//! Commands of the image diffs panel: status, (re)generation, cancel and the manifest of what was
//! generated. The pictures themselves reach the webview through the `crimg://` scheme (`lib.rs`).

use tauri::{AppHandle, State};

use crate::db::{Db, DbPath};
use crate::error::AppResult;
use crate::commands::routines::{builtin, launch};
use crate::image_diff::report::{self, Report};
use crate::image_diff::runner::{self, Status};
use crate::image_diff::store::{self, Manifest};
use crate::routines::context::ContextRef;
use crate::routines::{handlers, IMAGE_DIFF_REPORT_KEY};

#[tauri::command(async)]
pub fn image_diff_status(review_id: i64, db: State<Db>, path: State<DbPath>) -> AppResult<Status> {
    let conn = db.0.lock().unwrap();
    let pin = runner::current_pin(&conn, review_id)?;
    let mut status = runner::status_of(&store::cache_dir(&path.0, review_id), &pin, runner::is_active(review_id));
    status.report_running = handlers::active_image_diff_report(&conn, review_id);
    Ok(status)
}

/// What was generated for the review; `None` before the first run finished.
#[tauri::command(async)]
pub fn image_diff_manifest(review_id: i64, path: State<DbPath>) -> AppResult<Option<Manifest>> {
    Ok(store::read_manifest(&store::cache_dir(&path.0, review_id)))
}

/// Generate the review's image diffs (again, if there are some): queued behind any running job.
#[tauri::command(async)]
pub fn image_diff_generate(review_id: i64, app: AppHandle) -> AppResult<()> {
    runner::start(&app, review_id, false);
    Ok(())
}

#[tauri::command(async)]
pub fn image_diff_cancel(review_id: i64, app: AppHandle) {
    runner::cancel_job(&app, review_id);
}

/// The AI report of the review's image diffs; `None` before one was written.
#[tauri::command(async)]
pub fn image_diff_report(review_id: i64, path: State<DbPath>) -> AppResult<Option<Report>> {
    Ok(report::read_final(&store::cache_dir(&path.0, review_id)))
}

/// Start the AI report run (the `image_diff_report` routine) on the review's generated image diffs.
#[tauri::command]
pub fn image_diff_run_report(review_id: i64, model: Option<String>, app: AppHandle, db: State<Db>) -> AppResult<()> {
    let routine = builtin(&db, IMAGE_DIFF_REPORT_KEY)?;
    let ctx = ContextRef { kind: "review".into(), id: review_id.to_string(), base_paths: vec![] };
    launch(&app, &routine, &ctx, "", model.as_deref().unwrap_or(""))?;
    Ok(())
}
