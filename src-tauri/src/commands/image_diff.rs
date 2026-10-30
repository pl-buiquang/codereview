//! Commands of the image diffs panel: status, (re)generation, cancel and the manifest of what was
//! generated. The pictures themselves reach the webview through the `crimg://` scheme (`lib.rs`).

use tauri::{AppHandle, State};

use crate::db::{Db, DbPath};
use crate::error::AppResult;
use crate::image_diff::runner::{self, Status};
use crate::image_diff::store::{self, Manifest};

#[tauri::command(async)]
pub fn image_diff_status(review_id: i64, db: State<Db>, path: State<DbPath>) -> AppResult<Status> {
    let pin = runner::current_pin(&db.0.lock().unwrap(), review_id)?;
    Ok(runner::status_of(&store::cache_dir(&path.0, review_id), &pin, runner::is_active(review_id)))
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
