//! Routine runs as the app sees them: the `routine_run` rows, their queries and settings, and the
//! adapter that plugs the app into the Tauri-free run engine (`crate::run_engine`).
//!
//! The engine owns the queue, the concurrency cap, the processes, the transcript tailing and
//! stop / timeout / detach / re-adopt. This module supplies what is specific to the app: a SQLite
//! [`RunStore`], a Tauri [`EventSink`] (events, notifications, handlers, usage log), and the
//! preparation of a run (context, handlers, prompt rendering) before its process starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use super::context::{self, ContextRef, Light};
use super::handlers;
use super::{get as get_routine, Routine};
use crate::ai::profiles;
use crate::ai::{self, Harness, ToolPolicy};
use crate::ai_usage;
use crate::run_usage::RunTag;
use crate::commands::chat::new_session_id;
use crate::commands::focus::child_path;
use crate::db::{Db, DbPath};
use crate::error::{AppError, AppResult};
use crate::prompts;
use crate::run_engine::engine::RunEngine;
use crate::run_engine::outcome::STDOUT_FILE;
use crate::run_engine::{proc, EventSink, Finished, Prepared, RunStore, RunningRun};

pub use crate::run_engine::engine::DEFAULT_CONCURRENCY;
const CONCURRENCY_KEY: &str = "routine_max_concurrency";

/// `<db dir>/routine_runs`: one subdirectory per live run (prompt, stdout, stderr).
fn runs_root(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap_or(Path::new(".")).join("routine_runs")
}

/// Owns the run engine of the app. The engine is built on first use (it needs the app handle).
#[derive(Default)]
pub struct RoutineManager {
    engine: OnceLock<Arc<RunEngine>>,
    /// The quit confirmation is on screen: a second quit request must not stack another.
    quit_prompt_open: Mutex<bool>,
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
    /// Id of the harness the run is on (`claude` for every run before harnesses existed).
    pub harness: String,
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
            harness: row.get("harness")?,
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
    /// Harness-neutral form of the line (`{"v":1,"events":[..]}`); `None` for runs recorded before it existed.
    pub norm: Option<String>,
}

pub fn run_events(conn: &Connection, run_id: i64, after_seq: i64) -> AppResult<Vec<RunEvent>> {
    let mut stmt = conn
        .prepare("SELECT seq, ts, kind, json, norm FROM routine_run_event WHERE run_id = ?1 AND seq > ?2 ORDER BY seq")?;
    let rows = stmt
        .query_map(params![run_id, after_seq], |r| {
            Ok(RunEvent { seq: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, json: r.get(3)?, norm: r.get(4)? })
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
    let mut stmt = conn.prepare("SELECT id, pid, harness FROM routine_run WHERE status = 'running'")?;
    let rows: Vec<(i64, Option<i64>, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut n = conn.execute("UPDATE routine_run SET status = 'interrupted', ended_at = ?1 WHERE status = 'queued'", params![now()])?;
    for (id, pid, harness) in rows {
        if root.join(id.to_string()).join(STDOUT_FILE).exists() {
            continue;
        }
        if let Some(pid) = pid {
            // Only if it still looks like its harness' own process (an id we don't know is Claude's, as before).
            proc::kill_leftover(ai::harness_by_id(&harness).unwrap_or_else(ai::harness_for), pid as i32);
        }
        n += conn.execute(
            "UPDATE routine_run SET status = 'interrupted', ended_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
    }
    Ok(n)
}

impl RoutineManager {
    fn engine(&self, app: &AppHandle) -> &Arc<RunEngine> {
        self.engine.get_or_init(|| {
            let sink = Arc::new(AppSink { app: app.clone(), loaded: Mutex::default() });
            RunEngine::new(Arc::new(SqliteRunStore { app: app.clone() }), sink, ai::harness_for())
        })
    }

    /// Spawn the timeout watcher. Call once, after the manager is managed.
    pub fn start_ticker(app: AppHandle) {
        app.state::<RoutineManager>().engine(&app).start_ticker();
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
        self.engine(app).submit(id);
        let run = get_run(&db.0.lock().unwrap(), id);
        run
    }

    /// Start queued runs while below the concurrency cap.
    pub fn pump(&self, app: &AppHandle) {
        self.engine(app).pump();
    }

    /// Stop a run: a queued one is dropped, a running one is terminated.
    pub fn stop(&self, app: &AppHandle, run_id: i64) -> AppResult<()> {
        self.engine(app).stop(run_id)
    }

    /// Runs a quit would interrupt: started ones plus those still queued.
    pub fn active_count(&self, app: &AppHandle) -> usize {
        self.engine(app).active_count()
    }

    /// Claim the quit prompt; false when one is already open.
    pub fn begin_quit_prompt(&self) -> bool {
        !std::mem::replace(&mut *self.quit_prompt_open.lock().unwrap(), true)
    }

    pub fn end_quit_prompt(&self) {
        *self.quit_prompt_open.lock().unwrap() = false;
    }

    /// App is exiting but the runs go on: workers stop tailing (leaving their rows `running` for the
    /// next launch to adopt) and nothing is signalled. Runs that never started are dropped.
    pub fn detach_all(&self, app: &AppHandle) {
        self.engine(app).detach_all();
    }

    /// Startup: pick up the runs a previous session left alive (`mark_interrupted` spared them) and
    /// sweep the files of every other run.
    pub fn adopt_detached(app: &AppHandle) {
        app.state::<RoutineManager>().engine(app).adopt_detached();
    }

    /// App is exiting: terminate every process group and settle the rows ourselves.
    pub fn kill_all(&self, app: &AppHandle) {
        self.engine(app).kill_all();
    }
}

// ---------------------------------------------------------------------------
// The run store: the app's SQLite
// ---------------------------------------------------------------------------

struct SqliteRunStore {
    app: AppHandle,
}

impl SqliteRunStore {
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let db = self.app.try_state::<Db>().ok_or_else(|| AppError::Other("the database is not ready".into()))?;
        let conn = db.0.lock().unwrap();
        f(&conn)
    }
}

impl RunStore for SqliteRunStore {
    fn ai_enabled(&self) -> bool {
        crate::ai_gate::is_enabled()
    }

    fn max_concurrency(&self) -> i64 {
        self.with_conn(|c| Ok(max_concurrency(c))).unwrap_or(DEFAULT_CONCURRENCY)
    }

    fn running_count(&self) -> AppResult<i64> {
        self.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM routine_run WHERE status = 'running'", [], |r| r.get(0))?))
    }

    fn next_queued(&self) -> AppResult<Option<i64>> {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT id FROM routine_run WHERE status = 'queued' ORDER BY id LIMIT 1", [], |r| r.get(0))
                .optional()?)
        })
    }

    fn mark_running(&self, id: i64) -> AppResult<bool> {
        self.with_conn(|c| {
            Ok(c.execute(
                "UPDATE routine_run SET status = 'running', started_at = ?2 WHERE id = ?1 AND status = 'queued'",
                params![id, now()],
            )? > 0)
        })
    }

    fn is_running(&self, id: i64) -> bool {
        self.with_conn(|c| Ok(c.query_row("SELECT status = 'running' FROM routine_run WHERE id = ?1", params![id], |r| r.get(0))?))
            .unwrap_or(false)
    }

    fn drop_queued(&self, id: i64) -> AppResult<bool> {
        self.with_conn(|c| {
            Ok(c.execute(
                "UPDATE routine_run SET status = 'killed', ended_at = ?2 WHERE id = ?1 AND status = 'queued'",
                params![id, now()],
            )? > 0)
        })
    }

    fn active_count(&self) -> AppResult<usize> {
        self.with_conn(|c| {
            let n: i64 = c
                .query_row("SELECT COUNT(*) FROM routine_run WHERE status IN ('queued', 'running')", [], |r| r.get(0))
                .unwrap_or(0);
            Ok(n as usize)
        })
    }

    fn kill_queued(&self) -> AppResult<()> {
        self.with_conn(|c| {
            c.execute("UPDATE routine_run SET status = 'killed', ended_at = ?1 WHERE status = 'queued'", params![now()])?;
            Ok(())
        })
    }

    fn kill_active(&self) -> AppResult<()> {
        self.with_conn(|c| {
            c.execute(
                "UPDATE routine_run SET status = 'killed', ended_at = ?1 WHERE status IN ('queued', 'running')",
                params![now()],
            )?;
            Ok(())
        })
    }

    fn set_process(&self, id: i64, pid: u32, cwd: &Path, session_id: &str) {
        let _ = self.with_conn(|c| {
            c.execute(
                "UPDATE routine_run SET pid = ?2, cwd = ?3, session_id = ?4 WHERE id = ?1",
                params![id, pid, cwd.to_string_lossy(), session_id],
            )?;
            Ok(())
        });
    }

    fn set_cleanup(&self, id: i64, dirs: &[PathBuf]) {
        let _ = self.with_conn(|c| {
            c.execute(
                "UPDATE routine_run SET cleanup_json = ?2 WHERE id = ?1",
                params![id, serde_json::to_string(dirs).unwrap_or_default()],
            )?;
            Ok(())
        });
    }

    fn cleanup_dirs(&self, id: i64) -> Vec<PathBuf> {
        let json: Option<String> = self
            .with_conn(|c| Ok(c.query_row("SELECT cleanup_json FROM routine_run WHERE id = ?1", params![id], |r| r.get(0))?))
            .ok()
            .flatten();
        json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    fn append_event(&self, id: i64, seq: i64, kind: &str, json: &str, norm: &str) {
        let _ = self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO routine_run_event (run_id, seq, ts, kind, json, norm) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, seq, now(), kind, json, norm],
            )?;
            Ok(())
        });
    }

    fn last_seq(&self, id: i64) -> i64 {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT COALESCE(MAX(seq), 0) FROM routine_run_event WHERE run_id = ?1", params![id], |r| r.get(0))?)
        })
        .unwrap_or(0)
    }

    fn running_runs(&self) -> AppResult<Vec<RunningRun>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, pid, session_id, started_at, harness FROM routine_run WHERE status = 'running'")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(RunningRun {
                        id: r.get(0)?,
                        pid: r.get::<_, Option<i64>>(1)?.unwrap_or(0) as i32,
                        session_id: r.get(2)?,
                        started_at: r.get(3)?,
                        harness: r.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        })
    }

    fn finish(&self, id: i64, fin: &Finished) {
        let _ = self.with_conn(|c| {
            // Guarded: on exit `kill_all` already settled the row.
            c.execute(
                "UPDATE routine_run SET status = ?2, ended_at = ?3, result_text = ?4, error = ?5, cost_usd = ?6,
                        num_turns = ?7, session_id = COALESCE(?8, session_id)
                 WHERE id = ?1 AND status = 'running'",
                params![
                    id,
                    fin.status,
                    now(),
                    (!fin.text.is_empty()).then_some(fin.text.as_str()),
                    fin.error,
                    fin.usage.cost_usd,
                    fin.usage.num_turns,
                    fin.usage.session_id
                ],
            )?;
            Ok(())
        });
    }

    fn prune(&self, keep: i64) {
        let _ = self.with_conn(|c| {
            c.execute(
                "DELETE FROM routine_run WHERE status NOT IN ('queued', 'running')
                   AND id NOT IN (SELECT id FROM routine_run ORDER BY id DESC LIMIT ?1)",
                params![keep],
            )?;
            Ok(())
        });
    }

    fn mark_lost(&self, id: i64) {
        let _ = self.with_conn(|c| {
            c.execute(
                "UPDATE routine_run SET status = 'interrupted', ended_at = ?2 WHERE id = ?1 AND status = 'running'",
                params![id, now()],
            )?;
            Ok(())
        });
    }

    fn runs_root(&self) -> PathBuf {
        runs_root(&self.app.state::<DbPath>().0)
    }
}

// ---------------------------------------------------------------------------
// The sink: Tauri events, handlers, notifications, usage log
// ---------------------------------------------------------------------------

struct AppSink {
    app: AppHandle,
    /// The run and routine of every run in flight, for the handlers and the notification.
    loaded: Mutex<HashMap<i64, (RoutineRun, Routine)>>,
}

impl AppSink {
    fn load(&self, id: i64) -> AppResult<(RoutineRun, Routine)> {
        let db = self.app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let loaded = get_run(&conn, id).and_then(|run| get_routine(&conn, run.routine_id).map(|r| (run, r)))?;
        self.loaded.lock().unwrap().insert(id, loaded.clone());
        Ok(loaded)
    }
}

impl EventSink for AppSink {
    fn queued(&self, id: i64) {
        emit_updated(&self.app, id, "queued");
    }

    fn started(&self, id: i64) {
        emit_updated(&self.app, id, "running");
    }

    fn status_changed(&self, id: i64, status: &str) {
        emit_updated(&self.app, id, status);
    }

    fn event(&self, id: i64, seq: i64, kind: &str, json: &str) {
        let _ = self.app.emit("routine-run-event", RunEventPayload { run_id: id, seq, kind, json });
    }

    fn dropped(&self, id: i64) {
        handlers::finish_dropped(&self.app, id);
    }

    fn prepare(&self, id: i64, stopped: &dyn Fn() -> bool) -> Result<Option<Prepared>, String> {
        let app = &self.app;
        let (run, routine) = self.load(id).map_err(|e| e.to_string())?;
        let resolved = context::resolve(app, &routine, &run.context).map_err(|e| e.to_string())?;
        if stopped() {
            return Ok(None);
        }
        let prep = handlers::prepare(app, &routine, &run).map_err(|e| e.to_string())?;
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
        let (model, harness, profile_args) = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let resolved = profiles::resolve_routine(&conn, routine.profile_id);
            let harness = resolved.harness;
            let _ = conn.execute("UPDATE routine_run SET harness = ?1 WHERE id = ?2", params![harness.id(), run.id]);
            (handlers::effective_model(&conn, &run, &routine), harness, harness.settings_args(&resolved.settings))
        };

        log::info!(target: "routines", "run {}: starting {} on {}", run.id, routine.key, run.context_label);
        Ok(Some(build_prepared(
            harness,
            profile_args,
            &routine,
            &prep,
            cwd,
            model,
            new_session_id(),
            system,
            prompt,
            &app.state::<DbPath>().0,
            handlers::usage_tag(&routine, &run, harness.id()),
        )))
    }

    fn adopt(&self, id: i64) -> Option<Duration> {
        let (_, routine) = self.load(id).ok()?;
        Some(Duration::from_secs(routine.timeout_min.max(1) as u64 * 60))
    }

    fn finished(&self, id: i64, fin: &Finished) {
        let app = &self.app;
        let loaded = self.loaded.lock().unwrap().remove(&id);
        let error = fin.error.as_deref();
        if !fin.shutting_down {
            if let Some((run, routine)) = &loaded {
                handlers::finish(app, routine, run, fin.status, &fin.text, error, &fin.usage);
            }
            emit_updated(app, id, fin.status);
            if let (Some((run, routine)), true) =
                (&loaded, matches!(fin.status, "ok" | "needs_attention" | "error" | "timeout"))
            {
                // The board overview is a refresh button, not a task worth a banner.
                if !(routine.handler.as_deref() == Some("overview") && fin.status == "ok") {
                    notify(app, routine, &run.context_label, fin.status, &fin.text, error);
                }
            }
        }
        if let Some((run, routine)) = &loaded {
            let harness = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                profiles::resolve_routine(&conn, routine.profile_id).harness.id()
            };
            let mut tag = handlers::usage_tag(routine, run, harness);
            tag.model = if run.model.is_empty() { routine.model.clone() } else { run.model.clone() };
            ai_usage::record(&tag, &fin.started_at, fin.elapsed.as_millis() as i64, &fin.usage, fin.usage_error.as_deref());
        }
    }
}

/// The process request of a run.
#[allow(clippy::too_many_arguments)]
fn build_prepared(
    harness: &'static dyn Harness,
    profile_args: Vec<String>,
    routine: &Routine,
    prep: &handlers::Prep,
    cwd: PathBuf,
    model: String,
    session_id: String,
    system_prompt: String,
    prompt: String,
    db_path: &Path,
    usage_tag: RunTag,
) -> Prepared {
    let mut opts = routine.run_options(usage_tag.harness);
    // The profile's own flags (an opencode agent) come after the routine's.
    opts.extra_args.extend(profile_args);
    let mut env: Vec<(String, std::ffi::OsString)> = vec![("CODEREVIEW_DB".into(), db_path.into())];
    if let Some(path) = child_path() {
        env.push(("PATH".into(), path));
    }
    Prepared {
        harness,
        model,
        system_prompt,
        system_prompt_mode: opts.system_prompt_mode,
        prompt,
        cwd,
        extra_dirs: prep.extra_dirs.clone(),
        tools: ToolPolicy::Allowlist(opts.allowed_tools.iter().chain(&prep.extra_tools).cloned().collect()),
        access: opts.access,
        session_id,
        env,
        extra_args: opts.extra_args,
        usage_tag,
        timeout: Duration::from_secs(routine.timeout_min.max(1) as u64 * 60),
        cleanup: prep.cleanup.clone(),
    }
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

    #[test]
    fn rows_without_a_normalized_form_are_still_returned() {
        let (conn, rid) = seeded();
        insert_run(&conn, rid, "a", "running").unwrap();
        let run_id = conn.last_insert_rowid();
        // seq 1 as an older version wrote it (no norm), seq 2 as the engine writes it now
        conn.execute("INSERT INTO routine_run_event (run_id, seq, ts, kind, json) VALUES (?1, 1, 't', 'assistant', '{}')", params![run_id])
            .unwrap();
        conn.execute(
            "INSERT INTO routine_run_event (run_id, seq, ts, kind, json, norm) VALUES (?1, 2, 't', 'result', '{}', ?2)",
            params![run_id, r#"{"v":1,"events":[]}"#],
        )
        .unwrap();
        let events = run_events(&conn, run_id, 0).unwrap();
        assert_eq!(events[0].norm, None);
        assert_eq!(events[1].norm.as_deref(), Some(r#"{"v":1,"events":[]}"#));
        assert_eq!(get_run(&conn, run_id).unwrap().harness, "claude");
    }

    fn test_routine(mode: &str) -> Routine {
        Routine {
            id: 1,
            key: "k".into(),
            name: "K".into(),
            description: String::new(),
            context_type: "pr".into(),
            prompt_template: String::new(),
            system_prompt: String::new(),
            kind: "plain".into(),
            needs_worktree: false,
            access: "edit".into(),
            harness_overrides: serde_json::json!({ "claude": {
                "permission_mode": "acceptEdits",
                "allowed_tools": ["Bash(git:*)", "Read"],
                "extra_args": ["--effort", "high"],
                "system_prompt_mode": mode,
            }}),
            profile_id: None,
            model: String::new(),
            timeout_min: 1,
            builtin: false,
            handler: None,
            enabled: true,
        }
    }

    #[test]
    fn stream_command_argv_and_env_per_system_prompt_mode() {
        use crate::test_support::{argv, envs, strs};
        let prep = handlers::Prep {
            extra_dirs: vec![PathBuf::from("/skills")],
            extra_tools: vec!["Skill".into()],
            ..handlers::Prep::default()
        };
        let cwd = Path::new("/repo");
        let prompt = Path::new("/runs/5/system.txt");
        let db = Path::new("/data/cr.db");
        for (mode, flag) in [("append", "--append-system-prompt-file"), ("replace", "--system-prompt-file")] {
            let routine = test_routine(mode);
            let p = build_prepared(ai::harness_for(), Vec::new(), &routine, &prep, cwd.into(), "opus".into(), "sid".into(), String::new(), "p".into(), db, RunTag::new("t"));
            let cmd = crate::ai::claude::command(&p.request(prompt), true, Some(prompt));
            assert_eq!(
                argv(&cmd),
                strs(&[
                    "-p", "--output-format", "stream-json", "--verbose", "--strict-mcp-config",
                    "--permission-mode", "acceptEdits", "--add-dir", "/repo", "--add-dir", "/skills",
                    "--allowedTools", "Bash(git:*)", "Read", "Skill", "--model", "opus", "--effort", "high",
                    "--session-id", "sid", flag, "/runs/5/system.txt",
                ])
            );
            assert_eq!(
                envs(&cmd),
                vec![
                    ("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS".to_string(), "0".to_string()),
                    ("CODEREVIEW_DB".to_string(), "/data/cr.db".to_string()),
                ]
            );
            assert_eq!(cmd.get_current_dir(), Some(cwd));
        }
    }

    #[test]
    fn stream_command_omits_empty_tools_and_model() {
        use crate::test_support::{argv, strs};
        let mut routine = test_routine("append");
        routine.harness_overrides = serde_json::json!({ "claude": { "permission_mode": "acceptEdits" } });
        let prep = handlers::Prep::default();
        let p = build_prepared(ai::harness_for(), Vec::new(), &routine, &prep, "/r".into(), String::new(), "s".into(), String::new(), "p".into(), Path::new("/d"), RunTag::new("t"));
        let cmd = crate::ai::claude::command(&p.request(Path::new("/p")), true, Some(Path::new("/p")));
        assert_eq!(
            argv(&cmd),
            strs(&[
                "-p", "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--permission-mode",
                "acceptEdits", "--add-dir", "/r", "--session-id", "s", "--append-system-prompt-file", "/p",
            ])
        );
    }

}
