use std::path::Path;

use crate::error::AppResult;
use crate::gh;

#[tauri::command]
pub fn gh_auth_status() -> bool {
    gh::auth_status()
}

#[tauri::command]
pub fn list_prs(repo_path: String) -> AppResult<Vec<gh::PrSummary>> {
    gh::list_prs(Path::new(&repo_path))
}
