//! Owns the `claude -p` processes of routine runs: FIFO queue under a concurrency cap, streaming of
//! the `stream-json` transcript into `routine_run_event`, stop / timeout, and either killing
//! everything when the app exits or leaving the runs behind (detaching) for the next launch to
//! re-adopt. Each run's process leads its own process group, so one signal reaches the tools it
//! spawned. Its stdout / stderr go to files in a per-run directory and the transcript is tailed from
//! there, so a run does not depend on the app that started it staying alive.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use super::context::{self, ContextRef, Light};
use super::handlers;
use super::{get as get_routine, Routine};
use crate::ai_usage::{self, RunUsage};
use crate::commands::chat::{new_session_id, ClaudeEnvelope};
use crate::commands::focus::child_path;
use crate::db::{Db, DbPath};
use crate::error::{AppError, AppResult};
use crate::prompts;

pub const DEFAULT_CONCURRENCY: i64 = 4;
const CONCURRENCY_KEY: &str = "routine_max_concurrency";
const MAX_EVENT_BYTES: usize = 100_000;
const KEEP_RUNS: i64 = 200;
const STOP_GRACE: Duration = Duration::from_secs(5);
const TAIL_POLL: Duration = Duration::from_millis(200);
const PROMPT_FILE: &str = "system.txt";
const STDOUT_FILE: &str = "stdout.jsonl";
const STDERR_FILE: &str = "stderr.log";
const STDERR_MAX_BYTES: usize = 16_384;

/// `<db dir>/routine_runs`: one subdirectory per live run (prompt, stdout, stderr).
fn runs_root(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap_or(Path::new(".")).join("routine_runs")
}

fn run_dir(app: &AppHandle, run_id: i64) -> PathBuf {
    runs_root(&app.state::<DbPath>().0).join(run_id.to_string())
}

#[derive(Default)]
pub struct RoutineManager {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    live: HashMap<i64, Live>,
    /// Stop requests (`killed` / `timeout`), consumed when the run finishes.
    stops: HashMap<i64, &'static str>,
    shutting_down: bool,
    /// Exiting with the runs left alive: workers stop tailing without finalizing and nothing is killed.
    detaching: bool,
    /// The quit confirmation is on screen: a second quit request must not stack another.
    quit_prompt_open: bool,
}

struct Live {
    pgid: i32,
    deadline: Instant,
}

/// A run as the UI sees it.
#[derive(Debug, Clone, Serialize)]
pub struct RoutineRun {
    pub id: i64,
    pub routine_id: i64,
    pub routine_key: String,
    pub routine_name: String,
    pub context_type: String,
    pub context_key: String,
    pub context_label: String,
    pub context: ContextRef,
    pub status: String,
    pub note: String,
    pub model: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub result_text: Option<String>,
    pub error: Option<String>,
    pub cost_usd: f64,
    pub num_turns: i64,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

const RUN_SELECT: &str =
    "SELECT r.*, ro.key AS routine_key, ro.name AS routine_name FROM routine_run r JOIN routine ro ON ro.id = r.routine_id";

impl RoutineRun {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        let json: String = row.get("context_json")?;
        Ok(Self {
            id: row.get("id")?,
            routine_id: row.get("routine_id")?,
            routine_key: row.get("routine_key")?,
            routine_name: row.get("routine_name")?,
            context_type: row.get("context_type")?,
            context_key: row.get("context_key")?,
            context_label: row.get("context_label")?,
            context: serde_json::from_str(&json)
                .unwrap_or(ContextRef { kind: String::new(), id: String::new(), base_paths: vec![] }),
            status: row.get("status")?,
            note: row.get("note")?,
            model: row.get("model")?,
            session_id: row.get("session_id")?,
            cwd: row.get("cwd")?,
            result_text: row.get("result_text")?,
            error: row.get("error")?,
            cost_usd: row.get("cost_usd")?,
            num_turns: row.get("num_turns")?,
            queued_at: row.get("queued_at")?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
        })
    }
}

pub fn get_run(conn: &Connection, id: i64) -> AppResult<RoutineRun> {
    conn.query_row(&format!("{RUN_SELECT} WHERE r.id = ?1"), params![id], RoutineRun::from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::Other(format!("routine run {id} not found")),
            other => other.into(),
        })
}

/// Active (`queued` / `running`) run of a routine on a context, if any.
pub fn active_run(conn: &Connection, routine_id: i64, context_key: &str) -> AppResult<Option<RoutineRun>> {
    Ok(conn
        .query_row(
            &format!(
                "{RUN_SELECT} WHERE r.routine_id = ?1 AND r.context_key = ?2 AND r.status IN ('queued', 'running')"
            ),
            params![routine_id, context_key],
            RoutineRun::from_row,
        )
        .optional()?)
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct RunFilter {
    pub statuses: Vec<String>,
    pub routine_id: Option<i64>,
    pub context_key: Option<String>,
    pub limit: Option<i64>,
}

pub fn list_runs(conn: &Connection, f: &RunFilter) -> AppResult<Vec<RoutineRun>> {
    let mut sql = format!("{RUN_SELECT} WHERE 1 = 1");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if !f.statuses.is_empty() {
        let marks = vec!["?"; f.statuses.len()].join(",");
        sql.push_str(&format!(" AND r.status IN ({marks})"));
        args.extend(f.statuses.iter().cloned().map(rusqlite::types::Value::from));
    }
    if let Some(id) = f.routine_id {
        sql.push_str(" AND r.routine_id = ?");
        args.push(id.into());
    }
    if let Some(key) = &f.context_key {
        sql.push_str(" AND r.context_key = ?");
        args.push(key.clone().into());
    }
    sql.push_str(" ORDER BY r.id DESC LIMIT ?");
    args.push(f.limit.unwrap_or(200).clamp(1, 1000).into());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), RoutineRun::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[derive(Debug, Serialize)]
pub struct RunEvent {
    pub seq: i64,
    pub ts: String,
    pub kind: String,
    pub json: String,
}

pub fn run_events(conn: &Connection, run_id: i64, after_seq: i64) -> AppResult<Vec<RunEvent>> {
    let mut stmt = conn
        .prepare("SELECT seq, ts, kind, json FROM routine_run_event WHERE run_id = ?1 AND seq > ?2 ORDER BY seq")?;
    let rows = stmt
        .query_map(params![run_id, after_seq], |r| {
            Ok(RunEvent { seq: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, json: r.get(3)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn max_concurrency(conn: &Connection) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![CONCURRENCY_KEY], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .map_or(DEFAULT_CONCURRENCY, |n| n.max(1))
}

pub fn set_max_concurrency(conn: &Connection, n: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CONCURRENCY_KEY, n.clamp(1, 16).to_string()],
    )?;
    Ok(())
}

/// Where the issue-plan routine writes plans for repos without a tracker root, and the folder it
/// may write to at all. Both are stored with a leading `~/` and expanded on read.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RoutineSettings {
    pub plans_dir: String,
    pub notes_root: String,
}

const PLANS_DIR_KEY: &str = "routine_plans_dir";
const NOTES_ROOT_KEY: &str = "routine_notes_root";
pub const DEFAULT_PLANS_DIR: &str = "~/Nextcloud/Notes/_work/plans";
pub const DEFAULT_NOTES_ROOT: &str = "~/Nextcloud/Notes";

fn meta_value(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get::<_, String>(0))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn routine_settings(conn: &Connection) -> RoutineSettings {
    RoutineSettings {
        plans_dir: meta_value(conn, PLANS_DIR_KEY).unwrap_or_else(|| DEFAULT_PLANS_DIR.into()),
        notes_root: meta_value(conn, NOTES_ROOT_KEY).unwrap_or_else(|| DEFAULT_NOTES_ROOT.into()),
    }
}

pub fn set_routine_settings(conn: &Connection, s: &RoutineSettings) -> AppResult<()> {
    for (key, value) in [(PLANS_DIR_KEY, &s.plans_dir), (NOTES_ROOT_KEY, &s.notes_root)] {
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.trim()],
        )?;
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Serialize, Clone)]
struct RunUpdated {
    run_id: i64,
    status: String,
}

#[derive(Serialize, Clone)]
struct RunEventPayload<'a> {
    run_id: i64,
    seq: i64,
    kind: &'a str,
    json: &'a str,
}

fn emit_updated(app: &AppHandle, run_id: i64, status: &str) {
    let _ = app.emit("routine-run-updated", RunUpdated { run_id, status: status.to_string() });
}

/// Runs left `running` / `queued` by a previous session never finished: they are `interrupted`.
/// Except a `running` one whose output file is still there: it may have outlived the app (detached
/// on quit, or the app crashed), and `adopt_detached` decides once the app state is up. A process
/// that left no output file (older runs) is terminated first when it still looks like our `claude -p`.
pub fn mark_interrupted(conn: &Connection, db_path: &Path) -> AppResult<usize> {
    let root = runs_root(db_path);
    let mut stmt = conn.prepare("SELECT id, pid FROM routine_run WHERE status = 'running'")?;
    let rows: Vec<(i64, Option<i64>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut n = conn.execute("UPDATE routine_run SET status = 'interrupted', ended_at = ?1 WHERE status = 'queued'", params![now()])?;
    for (id, pid) in rows {
        if root.join(id.to_string()).join(STDOUT_FILE).exists() {
            continue;
        }
        if let Some(pid) = pid {
            kill_leftover(pid as i32);
        }
        n += conn.execute(
            "UPDATE routine_run SET status = 'interrupted', ended_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
    }
    Ok(n)
}

/// Whether `pid` is still a `claude -p` (of that session, when known): guards against pid reuse.
#[cfg(unix)]
fn is_our_claude(pid: i32, session_id: Option<&str>) -> bool {
    let Ok(out) = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output() else { return false };
    let cmd = String::from_utf8_lossy(&out.stdout);
    cmd.contains("claude") && cmd.contains(" -p") && session_id.is_none_or(|s| cmd.contains(s))
}

#[cfg(not(unix))]
fn is_our_claude(_pid: i32, _session_id: Option<&str>) -> bool {
    false
}

#[cfg(unix)]
fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

#[cfg(not(unix))]
fn process_alive(_pid: i32) -> bool {
    false
}

fn kill_leftover(pid: i32) {
    if is_our_claude(pid, None) {
        signal_group(pid, SIGTERM);
    }
}

#[cfg(unix)]
fn signal_group(pgid: i32, sig: i32) {
    // SAFETY: plain syscall; a stale pgid at worst yields ESRCH.
    unsafe {
        libc::kill(-pgid, sig);
    }
}

#[cfg(not(unix))]
fn signal_group(_pgid: i32, _sig: i32) {}

#[cfg(unix)]
const SIGTERM: i32 = libc::SIGTERM;
#[cfg(unix)]
const SIGKILL: i32 = libc::SIGKILL;
#[cfg(not(unix))]
const SIGTERM: i32 = 15;
#[cfg(not(unix))]
const SIGKILL: i32 = 9;

impl RoutineManager {
    /// Spawn the timeout watcher. Call once, after the manager is managed.
    pub fn start_ticker(app: AppHandle) {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            let manager = app.state::<RoutineManager>();
            let expired: Vec<i64> = {
                let inner = manager.inner.lock().unwrap();
                inner
                    .live
                    .iter()
                    .filter(|(id, l)| Instant::now() > l.deadline && !inner.stops.contains_key(*id))
                    .map(|(id, _)| *id)
                    .collect()
            };
            for id in expired {
                manager.request_stop(id, "timeout");
            }
        });
    }

    /// Queue a run of `routine` on `ctx`. Fails when that routine already has an active run there.
    pub fn launch(
        &self,
        app: &AppHandle,
        routine: &Routine,
        ctx: &ContextRef,
        light: &Light,
        note: &str,
        model: &str,
    ) -> AppResult<RoutineRun> {
        let db = app.state::<Db>();
        let id = {
            let conn = db.0.lock().unwrap();
            let inserted = conn.execute(
                "INSERT INTO routine_run
                   (routine_id, context_type, context_key, context_label, context_json, status, note, model, queued_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8)",
                params![
                    routine.id,
                    routine.context_type,
                    light.key,
                    light.label,
                    serde_json::to_string(ctx).unwrap_or_default(),
                    note,
                    model,
                    now()
                ],
            );
            match inserted {
                Ok(_) => conn.last_insert_rowid(),
                Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                    return Err(AppError::Other(format!(
                        "“{}” is already running on {}",
                        routine.name, light.label
                    )))
                }
                Err(e) => return Err(e.into()),
            }
        };
        emit_updated(app, id, "queued");
        self.pump(app);
        let run = get_run(&db.0.lock().unwrap(), id);
        run
    }

    /// Start queued runs while below the concurrency cap.
    pub fn pump(&self, app: &AppHandle) {
        let inner = self.inner.lock().unwrap();
        // Queued runs wait while AI is off; `set_ai_enabled(true)` pumps again.
        if inner.shutting_down || !crate::ai_gate::is_enabled() {
            return;
        }
        let db = app.state::<Db>();
        loop {
            let next: AppResult<Option<i64>> = (|| {
                let conn = db.0.lock().unwrap();
                let running: i64 =
                    conn.query_row("SELECT COUNT(*) FROM routine_run WHERE status = 'running'", [], |r| r.get(0))?;
                if running >= max_concurrency(&conn) {
                    return Ok(None);
                }
                let Some(id) = conn
                    .query_row("SELECT id FROM routine_run WHERE status = 'queued' ORDER BY id LIMIT 1", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .optional()?
                else {
                    return Ok(None);
                };
                conn.execute(
                    "UPDATE routine_run SET status = 'running', started_at = ?2 WHERE id = ?1",
                    params![id, now()],
                )?;
                Ok(Some(id))
            })();
            match next {
                Ok(Some(id)) => {
                    emit_updated(app, id, "running");
                    let app = app.clone();
                    std::thread::spawn(move || execute(app, id));
                }
                Ok(None) => break,
                Err(e) => {
                    log::warn!(target: "routines", "pump failed: {e}");
                    break;
                }
            }
        }
    }

    /// Stop a run: a queued one is dropped, a running one is terminated.
    pub fn stop(&self, app: &AppHandle, run_id: i64) -> AppResult<()> {
        let db = app.state::<Db>();
        let dropped = db.0.lock().unwrap().execute(
            "UPDATE routine_run SET status = 'killed', ended_at = ?2 WHERE id = ?1 AND status = 'queued'",
            params![run_id, now()],
        )?;
        if dropped > 0 {
            emit_updated(app, run_id, "killed");
            handlers::finish_dropped(app, run_id);
            return Ok(());
        }
        let running: bool = db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT status = 'running' FROM routine_run WHERE id = ?1", params![run_id], |r| r.get(0))
            .unwrap_or(false);
        if !running {
            return Err(AppError::Other("this run is not active".into()));
        }
        self.request_stop(run_id, "killed");
        Ok(())
    }

    fn request_stop(&self, run_id: i64, reason: &'static str) {
        let pgid = {
            let mut inner = self.inner.lock().unwrap();
            inner.stops.entry(run_id).or_insert(reason);
            inner.live.get(&run_id).map(|l| l.pgid)
        };
        if let Some(pgid) = pgid {
            terminate(pgid);
        }
    }

    /// Runs a quit would interrupt: started ones plus those still queued.
    pub fn active_count(&self, app: &AppHandle) -> usize {
        let Some(db) = app.try_state::<Db>() else { return self.inner.lock().unwrap().live.len() };
        let n: i64 = db
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM routine_run WHERE status IN ('queued', 'running')", [], |r| r.get(0))
            .unwrap_or(0);
        n as usize
    }

    /// Claim the quit prompt; false when one is already open.
    pub fn begin_quit_prompt(&self) -> bool {
        !std::mem::replace(&mut self.inner.lock().unwrap().quit_prompt_open, true)
    }

    pub fn end_quit_prompt(&self) {
        self.inner.lock().unwrap().quit_prompt_open = false;
    }

    /// App is exiting but the runs go on: workers stop tailing (leaving their rows `running` for the
    /// next launch to adopt) and nothing is signalled. Runs that never started are dropped.
    pub fn detach_all(&self, app: &AppHandle) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            inner.detaching = true;
        }
        if let Some(db) = app.try_state::<Db>() {
            let _ = db.0.lock().unwrap().execute(
                "UPDATE routine_run SET status = 'killed', ended_at = ?1 WHERE status = 'queued'",
                params![now()],
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !self.inner.lock().unwrap().live.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Startup: pick up the runs a previous session left alive (`mark_interrupted` spared them) and
    /// sweep the files of every other run.
    pub fn adopt_detached(app: &AppHandle) {
        let db = app.state::<Db>();
        let running: Vec<i64> = {
            let conn = db.0.lock().unwrap();
            let Ok(mut stmt) = conn.prepare("SELECT id FROM routine_run WHERE status = 'running'") else { return };
            let ids = stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default();
            ids
        };
        if let Ok(entries) = std::fs::read_dir(runs_root(&app.state::<DbPath>().0)) {
            for entry in entries.flatten() {
                let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else { continue };
                if !running.contains(&id) {
                    discard_run_files(app, id);
                }
            }
        }
        for id in running {
            let app = app.clone();
            std::thread::spawn(move || adopt(app, id));
        }
    }

    /// App is exiting: terminate every process group and settle the rows ourselves (the worker
    /// threads won't get to).
    pub fn kill_all(&self, app: &AppHandle) {
        let pgids: Vec<i32> = {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            if inner.detaching {
                return;
            }
            inner.live.values().map(|l| l.pgid).collect()
        };
        if let Some(db) = app.try_state::<Db>() {
            let _ = db.0.lock().unwrap().execute(
                "UPDATE routine_run SET status = 'killed', ended_at = ?1 WHERE status IN ('queued', 'running')",
                params![now()],
            );
        }
        for pgid in &pgids {
            signal_group(*pgid, SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !self.inner.lock().unwrap().live.is_empty() {
            std::thread::sleep(Duration::from_millis(100));
        }
        for pgid in pgids {
            signal_group(pgid, SIGKILL);
        }
    }
}

/// SIGTERM the group (claude ends its bash children and exits 143), SIGKILL it if it lingers.
fn terminate(pgid: i32) {
    signal_group(pgid, SIGTERM);
    std::thread::spawn(move || {
        std::thread::sleep(STOP_GRACE);
        signal_group(pgid, SIGKILL);
    });
}

// ---------------------------------------------------------------------------
// One run
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Outcome {
    result: Option<Value>,
    usage: RunUsage,
    /// Why there is no usable result: spawn failure, context resolution failure, exit status…
    failure: Option<String>,
    /// The app is quitting with this run left alive: nothing to settle, the next launch adopts it.
    detached: bool,
}

fn execute(app: AppHandle, run_id: i64) {
    let started = Instant::now();
    let started_at = ai_usage::now_stamp();
    let db = app.state::<Db>();
    let loaded = {
        let conn = db.0.lock().unwrap();
        get_run(&conn, run_id).and_then(|run| get_routine(&conn, run.routine_id).map(|r| (run, r)))
    };
    let (run, routine) = match loaded {
        Ok(x) => x,
        Err(e) => {
            finalize(&app, run_id, None, None, &Outcome { failure: Some(e.to_string()), ..Outcome::default() });
            return;
        }
    };
    let outcome = run_process(&app, &run, &routine);
    if outcome.detached {
        return;
    }
    settle(&app, &run, &routine, &outcome, started, &started_at);
}

/// Finalize a finished run, record its usage and start the next queued one.
fn settle(app: &AppHandle, run: &RoutineRun, routine: &Routine, outcome: &Outcome, started: Instant, started_at: &str) {
    let status = finalize(app, run.id, Some(routine), Some(run), outcome);
    let usage_error = match status.as_str() {
        "ok" | "needs_attention" => None,
        "error" => Some(outcome.failure.clone().unwrap_or_else(|| "error".into())),
        other => Some(other.to_string()),
    };
    let mut tag = handlers::usage_tag(routine, run);
    tag.model = if run.model.is_empty() { routine.model.clone() } else { run.model.clone() };
    ai_usage::record(&tag, started_at, started.elapsed().as_millis() as i64, &outcome.usage, usage_error.as_deref());
    app.state::<RoutineManager>().pump(app);
}

fn run_process(app: &AppHandle, run: &RoutineRun, routine: &Routine) -> Outcome {
    let manager = app.state::<RoutineManager>();
    let fail = |msg: String| Outcome { failure: Some(msg), ..Outcome::default() };

    let resolved = match context::resolve(app, routine, &run.context) {
        Ok(r) => r,
        Err(e) => return fail(e.to_string()),
    };
    if manager.inner.lock().unwrap().stops.contains_key(&run.id) {
        return Outcome::default();
    }

    let prep = match handlers::prepare(app, routine, run) {
        Ok(p) => p,
        Err(e) => return fail(e.to_string()),
    };
    if !prep.cleanup.is_empty() {
        let _ = app.state::<Db>().0.lock().unwrap().execute(
            "UPDATE routine_run SET cleanup_json = ?2 WHERE id = ?1",
            params![run.id, serde_json::to_string(&prep.cleanup).unwrap_or_default()],
        );
    }
    let cleanup = || prep.cleanup.iter().for_each(|d| drop(std::fs::remove_dir_all(d)));
    let cwd = prep.cwd.clone().unwrap_or_else(|| resolved.cwd.clone());
    let vars: Vec<(&str, &str)> =
        resolved.vars.iter().chain(&prep.vars).map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let prompt = prompts::render(&routine.prompt_template, &vars);
    let mut system = prep.system.clone().unwrap_or_else(|| prompts::render(&routine.system_prompt, &vars));
    system.push_str(&prep.system_suffix);
    // Handler routines feed app features and keep exactly the prompt they always had.
    if routine.handler.is_none() {
        if !system.is_empty() {
            system.push_str("\n\n");
        }
        system.push_str(&resolved.block);
    }
    if !run.note.trim().is_empty() {
        system.push_str(&format!("\n\n## Extra instructions for this run\n{}", run.note.trim()));
    }
    let dir = run_dir(app, run.id);
    let prompt_path = dir.join(PROMPT_FILE);
    let files = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&prompt_path, &system))
        .and_then(|_| Ok((File::create(dir.join(STDOUT_FILE))?, File::create(dir.join(STDERR_FILE))?)));
    let (stdout_file, stderr_file) = match files {
        Ok(f) => f,
        Err(e) => {
            cleanup();
            return fail(format!("failed to prepare the run directory: {e}"));
        }
    };

    let session_id = new_session_id();
    let model = handlers::effective_model(run, routine);
    let mut cmd = Command::new(crate::tools::claude_bin());
    cmd.current_dir(&cwd)
        .args(["-p", "--output-format", "stream-json", "--verbose", "--strict-mcp-config"])
        .arg("--permission-mode")
        .arg(&routine.permission_mode)
        .arg("--add-dir")
        .arg(&cwd);
    for dir in &prep.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    if !routine.allowed_tools.is_empty() || !prep.extra_tools.is_empty() {
        cmd.arg("--allowedTools").args(routine.allowed_tools.iter().chain(&prep.extra_tools));
    }
    if !model.is_empty() {
        cmd.arg("--model").arg(&model);
    }
    cmd.args(&routine.extra_args).arg("--session-id").arg(&session_id).arg(
        if routine.system_prompt_mode == "replace" { "--system-prompt-file" } else { "--append-system-prompt-file" },
    );
    cmd.arg(&prompt_path);
    // A self-paced /loop or background wait must not hit the 10-minute idle cap of `-p`.
    cmd.env("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS", "0");
    cmd.env("CODEREVIEW_DB", &app.state::<DbPath>().0);
    if let Some(path) = child_path() {
        cmd.env("PATH", path);
    }
    // Files, not pipes: the process must outlive us when the app quits with runs left alive.
    cmd.stdin(Stdio::piped()).stdout(Stdio::from(stdout_file)).stderr(Stdio::from(stderr_file));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    log::info!(target: "routines", "run {}: starting {} on {}", run.id, routine.key, run.context_label);
    let spawned = cmd.spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&prompt_path);
            cleanup();
            return fail(format!("failed to spawn claude: {e}"));
        }
    };
    let pid = child.id();
    {
        let deadline = Instant::now() + Duration::from_secs(routine.timeout_min.max(1) as u64 * 60);
        let mut inner = manager.inner.lock().unwrap();
        inner.live.insert(run.id, Live { pgid: pid as i32, deadline });
        if inner.stops.contains_key(&run.id) {
            drop(inner);
            terminate(pid as i32);
        }
    }
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let _ = conn.execute(
            "UPDATE routine_run SET pid = ?2, cwd = ?3, session_id = ?4 WHERE id = ?1",
            params![run.id, pid, cwd.to_string_lossy(), session_id],
        );
    }

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let tail = tail_events(app, run.id, &dir.join(STDOUT_FILE), 0, &mut || matches!(child.try_wait(), Ok(None)));
    let result = match tail {
        Tail::Detached => {
            // Dropping `child` neither kills nor reaps it; the next launch adopts the run.
            manager.inner.lock().unwrap().live.remove(&run.id);
            return Outcome { detached: true, ..Outcome::default() };
        }
        Tail::Done(result) => result,
    };
    let status = child.wait().map(|s| s.to_string()).map_err(|e| e.to_string());
    manager.inner.lock().unwrap().live.remove(&run.id);
    let _ = std::fs::remove_file(&prompt_path);
    cleanup();
    outcome_from(result, read_stderr(&dir), status)
}

/// Usage and failure reason of a run that produced `result` (if it did) and then ended with `exit`.
fn outcome_from(result: Option<Value>, stderr: String, exit: Result<String, String>) -> Outcome {
    let usage = result
        .as_ref()
        .and_then(|v| serde_json::from_value::<ClaudeEnvelope>(v.clone()).ok())
        .map(|e| e.run_usage())
        .unwrap_or_default();
    let failure = match (&result, exit) {
        (Some(_), _) => None,
        (None, Ok(s)) => Some(format!("claude exited with {s} without a result. {}", stderr.trim())),
        (None, Err(e)) => Some(format!("claude process error: {e}")),
    };
    Outcome { result, usage, failure, detached: false }
}

fn read_stderr(dir: &Path) -> String {
    let mut buf = Vec::new();
    if let Ok(f) = File::open(dir.join(STDERR_FILE)) {
        let _ = f.take(STDERR_MAX_BYTES as u64).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

enum Tail {
    /// The process ended and the file is drained: the `result` line, if there was one.
    Done(Option<Value>),
    /// The app is quitting with the run left alive.
    Detached,
}

/// Follow the run's stdout file into `routine_run_event` until `alive` turns false. The first `skip`
/// lines were recorded by an earlier session (the `seq` numbering is continuous across adoption).
fn tail_events(app: &AppHandle, run_id: i64, path: &Path, skip: i64, alive: &mut dyn FnMut() -> bool) -> Tail {
    let manager = app.state::<RoutineManager>();
    let Ok(file) = File::open(path) else { return Tail::Done(None) };
    let mut reader = BufReader::new(file);
    let mut result: Option<Value> = None;
    let mut seq = 0i64;
    let mut pending = String::new();
    let mut handle = |line: &str, result: &mut Option<Value>| {
        if line.trim().is_empty() {
            return;
        }
        let (kind, value) = parse_line(line);
        seq += 1;
        if seq > skip {
            record_event(app, run_id, seq, &kind, line);
        }
        if kind == "result" {
            *result = value;
        }
    };
    loop {
        // Sampled before reading, so output written just before the exit is still drained.
        let dead = !alive();
        loop {
            match reader.read_line(&mut pending) {
                Ok(0) | Err(_) => break,
                Ok(_) if pending.ends_with('\n') => {
                    handle(&pending, &mut result);
                    pending.clear();
                }
                Ok(_) => {}
            }
        }
        if dead {
            handle(&pending, &mut result);
            return Tail::Done(result);
        }
        if manager.inner.lock().unwrap().detaching {
            return Tail::Detached;
        }
        std::thread::sleep(TAIL_POLL);
    }
}

/// Take over a run a previous session left alive: keep tailing it, then settle it like any other.
fn adopt(app: AppHandle, run_id: i64) {
    let started = Instant::now();
    let started_at = ai_usage::now_stamp();
    let db = app.state::<Db>();
    let loaded = {
        let conn = db.0.lock().unwrap();
        get_run(&conn, run_id).and_then(|run| get_routine(&conn, run.routine_id).map(|r| (run, r)))
    };
    let Ok((run, routine)) = loaded else { return settle_lost(&app, run_id) };
    let pid = db
        .0
        .lock()
        .unwrap()
        .query_row("SELECT pid FROM routine_run WHERE id = ?1", params![run_id], |r| r.get::<_, Option<i64>>(0))
        .ok()
        .flatten()
        .unwrap_or(0) as i32;
    let dir = run_dir(&app, run_id);
    let out = dir.join(STDOUT_FILE);
    let alive = pid > 0 && is_our_claude(pid, run.session_id.as_deref());
    // Gone without a result: it died with the app, or was killed meanwhile.
    if !alive && !file_has_result(&out) {
        return settle_lost(&app, run_id);
    }
    log::info!(target: "routines", "run {run_id}: adopted (process {})", if alive { "alive" } else { "finished" });
    let manager = app.state::<RoutineManager>();
    if alive {
        let spent = run
            .started_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .and_then(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).to_std().ok())
            .unwrap_or_default();
        let left = Duration::from_secs(routine.timeout_min.max(1) as u64 * 60).saturating_sub(spent);
        manager.inner.lock().unwrap().live.insert(run_id, Live { pgid: pid, deadline: Instant::now() + left });
    }
    let skip: i64 = db
        .0
        .lock()
        .unwrap()
        .query_row("SELECT COALESCE(MAX(seq), 0) FROM routine_run_event WHERE run_id = ?1", params![run_id], |r| r.get(0))
        .unwrap_or(0);
    let tail = tail_events(&app, run_id, &out, skip, &mut || alive && process_alive(pid));
    manager.inner.lock().unwrap().live.remove(&run_id);
    let Tail::Done(result) = tail else { return };
    let outcome = outcome_from(result, read_stderr(&dir), Ok("an unknown status".into()));
    settle(&app, &run, &routine, &outcome, started, &started_at);
}

fn file_has_result(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.lines().rev().any(|l| parse_line(l).0 == "result"))
        .unwrap_or(false)
}

/// A run that can't be picked up again: it ended with the app.
fn settle_lost(app: &AppHandle, run_id: i64) {
    let _ = app.state::<Db>().0.lock().unwrap().execute(
        "UPDATE routine_run SET status = 'interrupted', ended_at = ?2 WHERE id = ?1 AND status = 'running'",
        params![run_id, now()],
    );
    discard_run_files(app, run_id);
    emit_updated(app, run_id, "interrupted");
}

/// Remove a run's directory and the scratch dirs it created.
fn discard_run_files(app: &AppHandle, run_id: i64) {
    let json: Option<String> = app
        .state::<Db>()
        .0
        .lock()
        .unwrap()
        .query_row("SELECT cleanup_json FROM routine_run WHERE id = ?1", params![run_id], |r| r.get(0))
        .ok()
        .flatten();
    if let Some(dirs) = json.and_then(|j| serde_json::from_str::<Vec<PathBuf>>(&j).ok()) {
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    let _ = std::fs::remove_dir_all(run_dir(app, run_id));
}

/// `(type, parsed line)` of one stream-json line; unparseable lines are kept as `raw`.
fn parse_line(line: &str) -> (String, Option<Value>) {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => (v.get("type").and_then(Value::as_str).unwrap_or("unknown").to_string(), Some(v)),
        Err(_) => ("raw".to_string(), None),
    }
}

fn record_event(app: &AppHandle, run_id: i64, seq: i64, kind: &str, line: &str) {
    let json = if line.len() > MAX_EVENT_BYTES {
        serde_json::json!({ "type": kind, "truncated": true, "bytes": line.len() }).to_string()
    } else {
        line.to_string()
    };
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO routine_run_event (run_id, seq, ts, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, seq, now(), kind, json],
        );
    }
    let _ = app.emit("routine-run-event", RunEventPayload { run_id, seq, kind, json: &json });
}

/// A `NEEDS_ATTENTION:` line anywhere in the final message.
fn needs_attention(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("NEEDS_ATTENTION:"))
}

fn result_text(result: &Value) -> Option<String> {
    match result.get("result")? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// The status a finished run settles on. A stop request wins over whatever the process reported.
fn final_status(stop: Option<&'static str>, result: Option<&Value>, text: &str) -> &'static str {
    if let Some(reason) = stop {
        return reason;
    }
    match result {
        None => "error",
        Some(v) if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) => "error",
        Some(_) if needs_attention(text) => "needs_attention",
        Some(_) => "ok",
    }
}

/// Write the final state, notify, prune old runs. Returns the status it settled on.
fn finalize(app: &AppHandle, run_id: i64, routine: Option<&Routine>, run: Option<&RoutineRun>, outcome: &Outcome) -> String {
    let manager = app.state::<RoutineManager>();
    let (stop, shutting_down) = {
        let mut inner = manager.inner.lock().unwrap();
        (inner.stops.remove(&run_id), inner.shutting_down)
    };
    discard_run_files(app, run_id);
    let text = outcome.result.as_ref().and_then(result_text).unwrap_or_default();
    let status = final_status(stop, outcome.result.as_ref(), &text);
    let error = match status {
        "error" => Some(outcome.failure.clone().unwrap_or_else(|| {
            if text.is_empty() { "the run reported an error".to_string() } else { text.clone() }
        })),
        _ => None,
    };
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        // Guarded: on exit `kill_all` already settled the row.
        let _ = conn.execute(
            "UPDATE routine_run SET status = ?2, ended_at = ?3, result_text = ?4, error = ?5, cost_usd = ?6,
                    num_turns = ?7, session_id = COALESCE(?8, session_id)
             WHERE id = ?1 AND status = 'running'",
            params![
                run_id,
                status,
                now(),
                (!text.is_empty()).then_some(text.as_str()),
                error,
                outcome.usage.cost_usd,
                outcome.usage.num_turns,
                outcome.usage.session_id
            ],
        );
        let _ = conn.execute(
            "DELETE FROM routine_run WHERE status NOT IN ('queued', 'running')
               AND id NOT IN (SELECT id FROM routine_run ORDER BY id DESC LIMIT ?1)",
            params![KEEP_RUNS],
        );
    }
    log::info!(target: "routines", "run {run_id}: {status}");
    if !shutting_down {
        if let (Some(routine), Some(run)) = (routine, run) {
            handlers::finish(app, routine, run, status, &text, error.as_deref(), &outcome.usage);
        }
        emit_updated(app, run_id, status);
        if let (Some(routine), Some(run), true) =
            (routine, run, matches!(status, "ok" | "needs_attention" | "error" | "timeout"))
        {
            // The board overview is a refresh button, not a task worth a banner.
            if routine.handler.as_deref() == Some("overview") && status == "ok" {
                return status.to_string();
            }
            notify(app, routine, &run.context_label, status, &text, error.as_deref());
        }
    }
    status.to_string()
}

fn notify(app: &AppHandle, routine: &Routine, label: &str, status: &str, text: &str, error: Option<&str>) {
    let headline = match status {
        "ok" => "finished",
        "needs_attention" => "needs your attention",
        "timeout" => "timed out",
        _ => "failed",
    };
    let detail = error.unwrap_or(text);
    let body: String = detail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(160).collect();
    let _ = app
        .notification()
        .builder()
        .title(format!("{} {headline}{}", routine.name, if label.is_empty() { String::new() } else { format!(" · {label}") }))
        .body(body)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use serde_json::json;

    fn insert_run(conn: &Connection, routine_id: i64, key: &str, status: &str) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at)
             VALUES (?1, 'pr', ?2, '{\"kind\":\"pr\"}', ?3, 'now')",
            params![routine_id, key, status],
        )
    }

    fn seeded() -> (Connection, i64) {
        let conn = open_memory();
        super::super::seed(&conn).unwrap();
        let id = super::super::list(&conn).unwrap()[0].id;
        (conn, id)
    }

    #[test]
    fn parse_line_extracts_type() {
        assert_eq!(parse_line(r#"{"type":"assistant","x":1}"#).0, "assistant");
        assert_eq!(parse_line(r#"{"x":1}"#).0, "unknown");
        assert_eq!(parse_line("not json").0, "raw");
    }

    #[test]
    fn needs_attention_is_a_line_prefix() {
        assert!(needs_attention("done\nNEEDS_ATTENTION: lint config is broken"));
        assert!(needs_attention("  NEEDS_ATTENTION: x"));
        assert!(!needs_attention("I will report NEEDS_ATTENTION: later"));
    }

    #[test]
    fn final_status_precedence() {
        let ok = json!({ "type": "result", "is_error": false, "result": "fine" });
        let err = json!({ "type": "result", "is_error": true, "result": "boom" });
        assert_eq!(final_status(None, Some(&ok), "fine"), "ok");
        assert_eq!(final_status(None, Some(&ok), "x\nNEEDS_ATTENTION: y"), "needs_attention");
        assert_eq!(final_status(None, Some(&err), "boom"), "error");
        assert_eq!(final_status(None, None, ""), "error");
        assert_eq!(final_status(Some("timeout"), Some(&ok), "fine"), "timeout");
        assert_eq!(final_status(Some("killed"), None, ""), "killed");
    }

    #[test]
    fn one_active_run_per_routine_and_context() {
        let (conn, rid) = seeded();
        insert_run(&conn, rid, "pr:o/n#1", "queued").unwrap();
        assert!(insert_run(&conn, rid, "pr:o/n#1", "running").is_err());
        // another context is fine, and so is the same context once the first run ended
        insert_run(&conn, rid, "pr:o/n#2", "running").unwrap();
        conn.execute("UPDATE routine_run SET status = 'ok' WHERE context_key = 'pr:o/n#1'", []).unwrap();
        insert_run(&conn, rid, "pr:o/n#1", "queued").unwrap();
        assert_eq!(active_run(&conn, rid, "pr:o/n#1").unwrap().unwrap().status, "queued");
    }

    #[test]
    fn mark_interrupted_settles_active_runs_only() {
        let (conn, rid) = seeded();
        insert_run(&conn, rid, "a", "queued").unwrap();
        insert_run(&conn, rid, "b", "running").unwrap();
        insert_run(&conn, rid, "c", "ok").unwrap();
        assert_eq!(mark_interrupted(&conn, Path::new("/nonexistent/db.sqlite")).unwrap(), 2);
        let statuses: Vec<String> = list_runs(&conn, &RunFilter::default()).unwrap().into_iter().map(|r| r.status).collect();
        assert_eq!(statuses, ["ok", "interrupted", "interrupted"]);
    }

    #[test]
    fn mark_interrupted_spares_runs_that_left_an_output_file() {
        let (conn, rid) = seeded();
        insert_run(&conn, rid, "a", "running").unwrap();
        insert_run(&conn, rid, "b", "running").unwrap();
        let tmp = std::env::temp_dir().join(format!("cr_detach_test_{}", std::process::id()));
        let kept = list_runs(&conn, &RunFilter::default()).unwrap()[1].id; // "a"
        let dir = runs_root(&tmp.join("db.sqlite")).join(kept.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STDOUT_FILE), "").unwrap();
        assert_eq!(mark_interrupted(&conn, &tmp.join("db.sqlite")).unwrap(), 1);
        let by_key: Vec<(String, String)> =
            list_runs(&conn, &RunFilter::default()).unwrap().into_iter().map(|r| (r.context_key, r.status)).collect();
        assert!(by_key.contains(&("a".into(), "running".into())));
        assert!(by_key.contains(&("b".into(), "interrupted".into())));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn file_has_result_looks_for_a_result_line() {
        let path = std::env::temp_dir().join(format!("cr_result_test_{}.jsonl", std::process::id()));
        std::fs::write(&path, "{\"type\":\"assistant\"}\n").unwrap();
        assert!(!file_has_result(&path));
        std::fs::write(&path, "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"result\":\"x\"}\n").unwrap();
        assert!(file_has_result(&path));
        let _ = std::fs::remove_file(&path);
        assert!(!file_has_result(&path));
    }

    #[test]
    fn outcome_without_a_result_reports_the_exit() {
        let o = outcome_from(None, " boom \n".into(), Ok("exit status: 1".into()));
        assert_eq!(o.failure.as_deref(), Some("claude exited with exit status: 1 without a result. boom"));
        assert!(!o.detached);
        let ok = outcome_from(Some(serde_json::json!({"type": "result"})), String::new(), Ok("0".into()));
        assert!(ok.failure.is_none());
    }

    #[test]
    fn concurrency_defaults_and_clamps() {
        let conn = open_memory();
        assert_eq!(max_concurrency(&conn), 4);
        set_max_concurrency(&conn, 0).unwrap();
        assert_eq!(max_concurrency(&conn), 1);
        set_max_concurrency(&conn, 99).unwrap();
        assert_eq!(max_concurrency(&conn), 16);
    }

    #[test]
    fn routine_settings_default_and_round_trip() {
        let conn = open_memory();
        let s = routine_settings(&conn);
        assert_eq!((s.plans_dir.as_str(), s.notes_root.as_str()), (DEFAULT_PLANS_DIR, DEFAULT_NOTES_ROOT));
        let custom = RoutineSettings { plans_dir: " ~/plans ".into(), notes_root: "".into() };
        set_routine_settings(&conn, &custom).unwrap();
        let s = routine_settings(&conn);
        assert_eq!(s.plans_dir, "~/plans");
        // A cleared field falls back to the default.
        assert_eq!(s.notes_root, DEFAULT_NOTES_ROOT);
    }

    #[test]
    fn events_are_returned_after_a_sequence_number() {
        let (conn, rid) = seeded();
        insert_run(&conn, rid, "a", "running").unwrap();
        let run_id = conn.last_insert_rowid();
        for seq in 1..=3 {
            conn.execute(
                "INSERT INTO routine_run_event (run_id, seq, ts, kind, json) VALUES (?1, ?2, 't', 'assistant', '{}')",
                params![run_id, seq],
            )
            .unwrap();
        }
        assert_eq!(run_events(&conn, run_id, 1).unwrap().len(), 2);
    }
}
