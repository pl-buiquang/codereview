//! Settings → CLI: the installed `cr`, the database each side uses, and the bundled-`cr` reinstall.

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use tauri::State;

use crate::cr_install;
use crate::db::DbPath;
use crate::error::{AppError, AppResult};

#[derive(Debug, Serialize)]
pub struct CrBinary {
    pub path: String,
    pub version: Option<String>,
}

/// What `cr info --json` reports. `None` for a `cr` older than the `info` command.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct CrInfo {
    pub version: String,
    pub build: String,
    pub db: String,
    pub db_source: String,
    pub db_exists: bool,
}

#[derive(Debug, Serialize)]
pub struct CliStatus {
    pub app_version: String,
    /// Debug build (`pnpm tauri dev`): no sidecar, nothing gets installed.
    pub debug_build: bool,
    /// The `cr` bundled with this app, if any.
    pub sidecar: Option<CrBinary>,
    /// The `cr` found on the login-shell `PATH`.
    pub installed: Option<CrBinary>,
    /// What the installed `cr` resolves (its own `cr info`).
    pub installed_info: Option<CrInfo>,
    /// Where an install or reinstall copies `cr`.
    pub install_dest: Option<String>,
    /// The database this app has open.
    pub app_db: String,
    /// `CODEREVIEW_DB` as this app sees it (from the login shell).
    pub env_db: Option<String>,
}

fn binary(path: &Path) -> CrBinary {
    CrBinary {
        path: path.to_string_lossy().into_owned(),
        version: cr_install::version_of(path),
    }
}

fn info_of(path: &Path) -> Option<CrInfo> {
    let out = Command::new(path).args(["info", "--json"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

#[tauri::command(async)]
pub fn cli_status(db_path: State<'_, DbPath>) -> CliStatus {
    let installed = which::which("cr").ok();
    CliStatus {
        app_version: env!("CODEREVIEW_VERSION").to_string(),
        debug_build: cfg!(debug_assertions),
        sidecar: cr_install::sidecar_path().as_deref().map(binary),
        installed_info: installed.as_deref().and_then(info_of),
        installed: installed.as_deref().map(binary),
        install_dest: cr_install::install_dest().map(|p| p.to_string_lossy().into_owned()),
        app_db: db_path.0.to_string_lossy().into_owned(),
        env_db: std::env::var("CODEREVIEW_DB").ok(),
    }
}

/// Copy the bundled `cr` over the installed one, whatever its version. Returns the note.
#[tauri::command(async)]
pub fn cr_reinstall() -> AppResult<String> {
    let result = cr_install::reinstall().map_err(AppError::Other)?;
    let note = result.note();
    log::info!(target: "cr_install", "{note}");
    Ok(note)
}
