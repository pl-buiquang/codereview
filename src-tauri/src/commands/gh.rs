use std::path::Path;

use serde::Serialize;

use crate::error::AppResult;
use crate::{gh, tools};

#[tauri::command]
pub fn gh_auth_status() -> bool {
    gh::auth_status()
}

#[tauri::command]
pub fn list_prs(repo_path: String) -> AppResult<Vec<gh::PrSummary>> {
    gh::list_prs(Path::new(&repo_path))
}

/// Detected external-tool environment, for the Settings diagnostics panel.
/// `git`/`gh` are the resolved absolute paths (or `null` if not found).
#[derive(Debug, Serialize)]
pub struct ToolEnv {
    pub git: Option<String>,
    pub gh: Option<String>,
    pub gh_authed: bool,
}

#[tauri::command]
pub fn check_environment() -> ToolEnv {
    ToolEnv {
        git: tools::git_path(),
        gh: tools::gh_path(),
        gh_authed: gh::auth_status(),
    }
}
