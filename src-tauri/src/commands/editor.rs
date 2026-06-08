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

/// Open a URL in the OS default browser. Uses the opener plugin's Rust API for
/// the same reason as `open_in_default_app`: it isn't gated by the opener IPC
/// scope, so no per-URL allowlist is needed.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> AppResult<()> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))
}
