//! Tauri commands for routines: definitions (Config tab), launching from a context menu, and the
//! run list / transcript (Routines tab).

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::routines::context::{self, ContextRef};
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

#[tauri::command]
pub fn routine_launch(
    routine_id: i64,
    context: ContextRef,
    note: Option<String>,
    model: Option<String>,
    app: AppHandle,
    db: State<Db>,
    manager: State<RoutineManager>,
) -> AppResult<RoutineRun> {
    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    let routine = routines::get(&db.0.lock().unwrap(), routine_id)?;
    let light = context::light(&db, &context)?;
    if !context::eligible(&routine, &light) {
        return Err(AppError::Other(format!("“{}” can't run on {}", routine.name, light.label)));
    }
    manager.launch(
        &app,
        &routine,
        &context,
        &light,
        note.as_deref().unwrap_or("").trim(),
        model.as_deref().unwrap_or("").trim(),
    )
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
pub fn routine_run_relaunch(
    run_id: i64,
    app: AppHandle,
    db: State<Db>,
    manager: State<RoutineManager>,
) -> AppResult<RoutineRun> {
    let run = manager::get_run(&db.0.lock().unwrap(), run_id)?;
    routine_launch(run.routine_id, run.context, Some(run.note), Some(run.model), app, db, manager)
}

#[tauri::command]
pub fn routine_max_concurrency(db: State<Db>) -> i64 {
    manager::max_concurrency(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn routine_set_max_concurrency(n: i64, app: AppHandle, db: State<Db>, manager: State<RoutineManager>) -> AppResult<()> {
    manager::set_max_concurrency(&db.0.lock().unwrap(), n)?;
    // A higher cap may let queued runs start.
    manager.pump(&app);
    Ok(())
}
