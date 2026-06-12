use std::path::PathBuf;

use serde::Serialize;

use crate::error::AppResult;
use crate::gh::GhRepo;
use crate::provider::provider_for;
use crate::{gh, tools};

#[tauri::command]
pub fn gh_auth_status() -> bool {
    provider_for().auth_status()
}

#[tauri::command]
pub fn list_prs(repo_path: String) -> AppResult<Vec<gh::PrSummary>> {
    provider_for().list_prs(&GhRepo::Local(PathBuf::from(repo_path)))
}

/// `async` so Tauri runs the slow `gh` GraphQL call off the main (UI) thread,
/// letting it overlap the diff/threads fetches instead of freezing the webview
/// (same rationale as the inbox commands). The body is synchronous; it holds no
/// DB lock.
#[tauri::command]
pub async fn pr_meta(owner: String, name: String, number: i64) -> AppResult<gh::PrMeta> {
    provider_for().pr_meta(&owner, &name, number)
}

/// `async` for the same reason as `pr_meta` — keeps the (paginated) review-thread
/// fetch off the UI thread so it loads concurrently with the diff and metadata.
#[tauri::command]
pub async fn pr_review_threads(
    owner: String,
    name: String,
    number: i64,
) -> AppResult<Vec<gh::PrThread>> {
    provider_for().pr_review_threads(&owner, &name, number)
}

/// Reply to an existing GitHub review thread. `comment_id` is the databaseId of
/// the thread's first comment. Acts on GitHub directly; nothing touches SQLite.
#[tauri::command]
pub async fn reply_to_thread(
    owner: String,
    name: String,
    number: i64,
    comment_id: i64,
    body: String,
) -> AppResult<i64> {
    provider_for().reply_to_thread(&owner, &name, number, comment_id, &body)
}

/// Resolve/unresolve a GitHub review thread by node id. Returns new isResolved.
#[tauri::command]
pub async fn set_pr_thread_resolved(thread_id: String, resolved: bool) -> AppResult<bool> {
    provider_for().set_thread_resolved(&thread_id, resolved)
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
        gh_authed: provider_for().auth_status(),
    }
}
