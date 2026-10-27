//! Tauri commands for routines: definitions (Config tab), launching from a context menu, and the
//! run list / transcript (Routines tab).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::routines::context::{self, ContextRef};
use crate::routines::handlers;
use crate::routines::manager::{self, RoutineManager, RoutineRun, RunEvent, RunFilter};
use crate::routines::{self, Routine, RoutineInput};

#[tauri::command]
pub fn routine_list(db: State<Db>) -> AppResult<Vec<Routine>> {
    routines::list(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn routine_save(routine: RoutineInput, db: State<Db>) -> AppResult<Routine> {
    routines::save(&db.0.lock().unwrap(), routine)
}

#[tauri::command]
pub fn routine_delete(id: i64, db: State<Db>) -> AppResult<()> {
    routines::delete(&db.0.lock().unwrap(), id)
}

#[tauri::command]
pub fn routine_reset(id: i64, db: State<Db>) -> AppResult<Routine> {
    routines::reset(&db.0.lock().unwrap(), id)
}

#[derive(Serialize)]
pub struct RoutineOption {
    pub routine: Routine,
    /// The routine's queued / running run on this context, if any.
    pub active_run: Option<RoutineRun>,
}

/// Routines the context satisfies, each with its active run, for the launch modal.
#[tauri::command]
pub fn routine_list_for_context(context: ContextRef, db: State<Db>) -> AppResult<Vec<RoutineOption>> {
    let light = context::light(&db, &context)?;
    let conn = db.0.lock().unwrap();
    let mut out = Vec::new();
    for routine in routines::list(&conn)? {
        if !context::eligible(&routine, &light) {
            continue;
        }
        let active_run = manager::active_run(&conn, routine.id, &light.key)?;
        out.push(RoutineOption { routine, active_run });
    }
    Ok(out)
}

/// Launch `routine` on `context`: eligibility, the handler's checks, then the manager's queue.
/// Also the entry point of the features that run on routines (overview, focus, AI review).
pub fn launch(
    app: &AppHandle,
    routine: &Routine,
    context: &ContextRef,
    note: &str,
    model: &str,
) -> AppResult<RoutineRun> {
    crate::ai_gate::ensure_enabled()?;
    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    let db = app.state::<Db>();
    let light = context::light(&db, context)?;
    if !context::eligible(routine, &light) {
        return Err(AppError::Other(format!("“{}” can't run on {}", routine.name, light.label)));
    }
    handlers::pre_launch(app, routine, context, &light, model)?;
    app.state::<RoutineManager>()
        .launch(app, routine, context, &light, note.trim(), model.trim())
        .inspect_err(|e| handlers::rollback(app, routine, context, &e.to_string()))
}

/// The built-in routine with this key.
pub fn builtin(db: &Db, key: &str) -> AppResult<Routine> {
    routines::list(&db.0.lock().unwrap())?
        .into_iter()
        .find(|r| r.key == key)
        .ok_or_else(|| AppError::Other(format!("routine `{key}` not found")))
}

#[tauri::command]
pub fn routine_launch(
    routine_id: i64,
    context: ContextRef,
    note: Option<String>,
    model: Option<String>,
    app: AppHandle,
    db: State<Db>,
) -> AppResult<RoutineRun> {
    let routine = routines::get(&db.0.lock().unwrap(), routine_id)?;
    launch(&app, &routine, &context, note.as_deref().unwrap_or(""), model.as_deref().unwrap_or(""))
}

#[tauri::command]
pub fn routine_run_list(filter: Option<RunFilter>, db: State<Db>) -> AppResult<Vec<RoutineRun>> {
    manager::list_runs(&db.0.lock().unwrap(), &filter.unwrap_or_default())
}

#[tauri::command]
pub fn routine_run_events(run_id: i64, after_seq: Option<i64>, db: State<Db>) -> AppResult<Vec<RunEvent>> {
    manager::run_events(&db.0.lock().unwrap(), run_id, after_seq.unwrap_or(0))
}

#[tauri::command]
pub fn routine_run_stop(run_id: i64, app: AppHandle, manager: State<RoutineManager>) -> AppResult<()> {
    manager.stop(&app, run_id)
}

/// Launch the same routine on the same context again, with the same note and model.
#[tauri::command]
pub fn routine_run_relaunch(run_id: i64, app: AppHandle, db: State<Db>) -> AppResult<RoutineRun> {
    let run = manager::get_run(&db.0.lock().unwrap(), run_id)?;
    routine_launch(run.routine_id, run.context, Some(run.note), Some(run.model), app, db)
}

#[tauri::command]
pub fn routine_max_concurrency(db: State<Db>) -> i64 {
    manager::max_concurrency(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn routine_settings(db: State<Db>) -> manager::RoutineSettings {
    manager::routine_settings(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn routine_set_settings(settings: manager::RoutineSettings, db: State<Db>) -> AppResult<()> {
    manager::set_routine_settings(&db.0.lock().unwrap(), &settings)
}

#[tauri::command]
pub fn routine_set_max_concurrency(n: i64, app: AppHandle, db: State<Db>, manager: State<RoutineManager>) -> AppResult<()> {
    manager::set_max_concurrency(&db.0.lock().unwrap(), n)?;
    // A higher cap may let queued runs start.
    manager.pump(&app);
    Ok(())
}
