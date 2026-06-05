use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

/// Open a path in the OS default application. Goes through the opener plugin's
/// Rust API rather than the `open_path` IPC command, so it isn't gated by the
/// (intentionally empty) opener path-scope — the frontend already constrains the
/// path to a file inside the review's working tree.
#[tauri::command]
pub fn open_in_default_app(app: AppHandle, path: String) -> AppResult<()> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))
}
