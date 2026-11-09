//! Plain routine runs on a remote host. The local engine is not involved: the run gets its
//! `routine_run` row (with `host_id`), a worker thread renders the request, posts it to the host and
//! then mirrors the run ([`crate::remote::mirror`]) into the same row and events the local engine would
//! have written, emitting the usual Tauri events.
//!
//! The row's life: `queued` (worker preparing / connecting) -> `running` (the host has the run;
//! `remote_run_id` is set) -> a terminal status copied from the host. A host that cannot be reached
//! at launch fails the run at once with `host_unreachable`; one lost during the run leaves it `running`
//! while the mirror reconnects.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{params, Connection};
use tauri::{AppHandle, Emitter, Manager};

use super::context::{self, ContextRef, Light};
use super::manager::{emit_updated, get_run, now, run_finished, RoutineManager, RoutineRun, RunEventPayload};
use super::{get as get_routine, launch_needs, Routine};
use crate::ai::profiles;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::prompts;
use crate::remote::client::RemoteClient;
use crate::remote::error::{readable, RemoteError};
use crate::remote::launch::{dropped_settings, run_request, LaunchInput};
use crate::remote::mirror::{self, finish_local, MirrorConfig, MirrorHooks};
use crate::remote::runtime;
use crate::run_engine::Finished;
use crate::run_usage::RunUsage;

/// The workers of this session, by local run id.
#[derive(Default)]
pub struct RemoteRuns {
    workers: Mutex<HashMap<i64, Arc<Flags>>>,
}

#[derive(Default)]
struct Flags {
    /// The user stopped the run before the host had it.
    cancel: AtomicBool,
    /// Stop following (the run is settled some other way).
    abandon: AtomicBool,
}

struct AppHooks {
    app: AppHandle,
}

impl MirrorHooks for AppHooks {
    fn db(&self) -> MutexGuard<'_, Connection> {
        self.app.state::<Db>().inner().0.lock().unwrap()
    }

    fn client(&self, host_id: i64) -> Result<RemoteClient, RemoteError> {
        runtime::global().ok_or_else(|| RemoteError::disabled("remote hosts are not ready"))?.client(host_id)
    }

    fn event(&self, run_id: i64, seq: i64, kind: &str, json: &str) {
        let _ = self.app.emit("routine-run-event", RunEventPayload { run_id, seq, kind, json });
    }

    fn status_changed(&self, run_id: i64, status: &str) {
        emit_updated(&self.app, run_id, status);
    }

    fn finished(&self, run_id: i64, fin: &Finished) {
        let loaded = {
            let db = self.app.state::<Db>();
            let conn = db.0.lock().unwrap();
            get_run(&conn, run_id).and_then(|run| get_routine(&conn, run.routine_id).map(|r| (run, r))).ok()
        };
        run_finished(&self.app, run_id, loaded, fin);
    }
}

impl RoutineManager {
    #[allow(clippy::too_many_arguments)]
    /// Queue a run of `routine` on `ctx` that runs on `host_id`. The row exists when this returns; the
    /// connection, the request and the mirror are the worker's business.
    pub fn launch_remote(
        &self,
        app: &AppHandle,
        routine: &Routine,
        ctx: &ContextRef,
        light: &Light,
        note: &str,
        model: &str,
        host_id: i64,
        harness_id: &str,
    ) -> AppResult<RoutineRun> {
        let db = app.state::<Db>();
        let id = {
            let conn = db.0.lock().unwrap();
            let inserted = conn.execute(
                "INSERT INTO routine_run
                   (routine_id, context_type, context_key, context_label, context_json, status, note, model, queued_at, harness, host_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8, ?9, ?10)",
                params![
                    routine.id,
                    routine.context_type,
                    light.key,
                    light.label,
                    serde_json::to_string(ctx).unwrap_or_default(),
                    note,
                    model,
                    now(),
                    harness_id,
                    host_id
                ],
            );
            match inserted {
                Ok(_) => conn.last_insert_rowid(),
                Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                    return Err(AppError::Other(format!("“{}” is already running on {}", routine.name, light.label)))
                }
                Err(e) => return Err(e.into()),
            }
        };
        emit_updated(app, id, "queued");
        self.spawn_worker(app, id);
        let run = get_run(&db.0.lock().unwrap(), id);
        run
    }

    fn spawn_worker(&self, app: &AppHandle, run_id: i64) {
        let flags = Arc::new(Flags::default());
        match self.remote.workers.lock().unwrap().entry(run_id) {
            std::collections::hash_map::Entry::Occupied(_) => return, // already driven
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(flags.clone());
            }
        }
        let app = app.clone();
        std::thread::spawn(move || {
            drive(&app, run_id, &flags);
            app.state::<RoutineManager>().remote.workers.lock().unwrap().remove(&run_id);
        });
    }

    /// Startup: re-attach to every remote run a previous session left active, whether the host still
    /// has it, finished it meanwhile, or lost it.
    pub fn resume_remote(app: &AppHandle) {
        let ids: Vec<i64> = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let mut stmt = match conn.prepare("SELECT id FROM routine_run WHERE host_id IS NOT NULL AND status IN ('queued', 'running') ORDER BY id") {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows = stmt.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default();
            rows
        };
        let manager = app.state::<RoutineManager>();
        for id in ids {
            manager.spawn_worker(app, id);
        }
    }

    /// Stop a remote run. `Ok(false)`: the run is not a remote one. The request goes out on a worker
    /// thread (it may need to open the tunnel); the mirror then sees the host report `killed`.
    pub(super) fn stop_remote(&self, app: &AppHandle, run_id: i64) -> AppResult<bool> {
        let (host_id, remote_id, status) = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let row: Option<(Option<i64>, Option<i64>, String)> = conn
                .query_row("SELECT host_id, remote_run_id, status FROM routine_run WHERE id = ?1", params![run_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .ok();
            match row {
                Some((Some(host), remote, status)) => (host, remote, status),
                _ => return Ok(false),
            }
        };
        if !matches!(status.as_str(), "queued" | "running") {
            return Err(AppError::Other("this run is not active".into()));
        }
        let flags = self.remote.workers.lock().unwrap().get(&run_id).cloned();
        let Some(remote_id) = remote_id else {
            // The host does not have it yet: the worker notices before it posts.
            if let Some(f) = flags {
                f.cancel.store(true, Ordering::Relaxed);
            }
            return Ok(true);
        };
        let app = app.clone();
        std::thread::spawn(move || {
            let hooks = AppHooks { app: app.clone() };
            let sent = hooks.client(host_id).and_then(|c| c.stop(remote_id));
            match sent {
                Ok(()) => {}
                // Already over on the host: the mirror settles the row.
                Err(e) if e.code == "bad_request" => {}
                Err(e) => {
                    let why = format!("stopped locally; {} The run may still be going on the host.", readable(&e.to_string()));
                    if let Some(f) = app.state::<RoutineManager>().remote.workers.lock().unwrap().get(&run_id) {
                        f.abandon.store(true, Ordering::Relaxed);
                    }
                    if finish_local(&hooks, run_id, "killed", "", Some(&why), RunUsage::default()) {
                        emit_updated(&app, run_id, "killed");
                    }
                }
            }
        });
        Ok(true)
    }
}

fn drive(app: &AppHandle, run_id: i64, flags: &Flags) {
    let hooks = AppHooks { app: app.clone() };
    let run = mirror::load(&hooks.db(), run_id);
    let Some(run) = run else { return };
    if run.remote_run_id.is_none() {
        match create(app, &hooks, run_id, flags) {
            Ok(Some(())) => {}
            Ok(None) => {
                if finish_local(&hooks, run_id, "killed", "", Some("stopped before it started"), RunUsage::default()) {
                    emit_updated(app, run_id, "killed");
                }
                return;
            }
            Err(e) => {
                let msg = readable(&e.to_string());
                log::warn!(target: "routines", "remote run {run_id} failed to start: {msg}");
                if finish_local(&hooks, run_id, "error", "", Some(&msg), RunUsage::default()) {
                    emit_updated(app, run_id, "error");
                }
                return;
            }
        }
    }
    mirror::follow(&hooks, &MirrorConfig::default(), run_id, &flags.abandon);
}

/// Prepare the request and post it. `Ok(None)`: the user stopped the run first.
fn create(app: &AppHandle, hooks: &AppHooks, run_id: i64, flags: &Flags) -> AppResult<Option<()>> {
    let (run, routine, resolved) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let run = get_run(&conn, run_id)?;
        let routine = get_routine(&conn, run.routine_id)?;
        let resolved = profiles::resolve_routine(&conn, routine.profile_id);
        (run, routine, resolved)
    };
    let host_id = run.host_id.ok_or_else(|| AppError::Other("not a remote run".into()))?;
    if resolved.host_id != Some(host_id) {
        return Err(AppError::Other("the routine's profile no longer runs on this host".into()));
    }
    if routine.is_system() {
        resolved.ensure_local(&format!("“{}” (a system routine)", routine.name))?;
    }
    let harness = resolved.harness;
    if let Some(reason) = harness.unavailable_reason() {
        return Err(AppError::Other(reason));
    }
    crate::ai::check_caps(harness, &format!("“{}”", routine.name), &launch_needs(&routine, harness.id()))?;
    if flags.cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let ctx = context::resolve_remote(app, &routine, &run.context)?;
    let vars: Vec<(&str, &str)> = ctx.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let prompt = prompts::render(&routine.prompt_template, &vars);
    let mut system = prompts::render(&routine.system_prompt, &vars);
    if !system.is_empty() {
        system.push_str("\n\n");
    }
    system.push_str(&ctx.block);
    if !run.note.trim().is_empty() {
        system.push_str(&format!("\n\n## Extra instructions for this run\n{}", run.note.trim()));
    }
    let model = {
        let conn = app.state::<Db>().inner().0.lock().unwrap();
        super::handlers::effective_model(&conn, &run, &routine)
    };
    for dropped in dropped_settings(&routine, harness.id()) {
        log::warn!(target: "routines", "remote run {run_id}: {dropped} cannot be sent to a host and is ignored");
    }
    let request = run_request(LaunchInput {
        routine: &routine,
        harness_id: harness.id(),
        model,
        prompt,
        system_prompt: system,
        workspace: ctx.workspace,
        snapshot: ctx.snapshot,
        // Stable across restarts: a crash between the post and the write-back re-posts and gets the same run.
        idempotency_key: format!("cr:{}:{}", run.queued_at, run.id),
    });
    if flags.cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let created = hooks.client(host_id)?.create_run(&request)?;
    {
        let conn = hooks.db();
        conn.execute(
            "UPDATE routine_run SET remote_run_id = ?2, status = 'running', started_at = COALESCE(started_at, ?3)
             WHERE id = ?1 AND status = 'queued'",
            params![run_id, created.id, now()],
        )?;
    }
    emit_updated(app, run_id, "running");
    log::info!(target: "routines", "run {run_id}: started as run {} on host {host_id} ({})", created.id, if created.created { "new" } else { "found by key" });
    Ok(Some(()))
}
