use tauri::State;

use crate::avatar;
use crate::commands::review::repo_owner_name;
use crate::commands::worktree::resolve_checkout;
use crate::db::Db;
use crate::error::AppResult;
use crate::git;
use crate::graph::{self, CommitDetail, GraphCommit, RefInfo, Scope, WipKind, WipStatus};

/// `async`: runs `git fetch --prune` (network) off the UI thread.
#[tauri::command]
pub async fn list_gone_branches(repo_id: i64, db: State<'_, Db>) -> AppResult<Vec<String>> {
    let checkout = resolve_checkout(&db, repo_id, None)?;
    graph::gone_branches(&checkout)
}

#[tauri::command]
pub fn delete_gone_branches(
    repo_id: i64,
    names: Vec<String>,
    db: State<Db>,
) -> AppResult<graph::branches::DeleteOutcome> {
    let checkout = resolve_checkout(&db, repo_id, None)?;
    graph::delete_gone_branches(&checkout, &names)
}

#[tauri::command]
pub fn graph_log(
    repo_id: i64,
    scope: Scope,
    worktree_path: Option<String>,
    skip: u32,
    limit: u32,
    db: State<Db>,
) -> AppResult<Vec<GraphCommit>> {
    let checkout = resolve_checkout(&db, repo_id, worktree_path)?;
    graph::graph_log(&checkout, scope, skip, limit)
}

#[tauri::command]
pub fn list_refs(
    repo_id: i64,
    worktree_path: Option<String>,
    db: State<Db>,
) -> AppResult<Vec<RefInfo>> {
    let checkout = resolve_checkout(&db, repo_id, worktree_path)?;
    graph::list_refs(&checkout)
}

/// Avatar data URL for a commit author, cached in SQLite; `None` falls back to a
/// generated identicon on the frontend. `async` so `gh`/HTTP stay off the UI thread.
#[tauri::command]
pub async fn commit_avatar(
    repo_id: i64,
    email: String,
    sha: String,
    db: State<'_, Db>,
) -> AppResult<Option<String>> {
    let key = email.trim().to_lowercase();
    if key.is_empty() {
        return Ok(None);
    }
    git::validate_commit_ref(&sha)?;
    let now = avatar::now_secs();
    let owner_name = {
        let conn = db.0.lock().unwrap();
        if let Some(hit) = avatar::cached(&conn, &key, now)? {
            return Ok(hit);
        }
        repo_owner_name(&conn, repo_id)?
    };
    match avatar::resolve(key.clone(), sha, owner_name).await {
        Ok(data_url) => {
            let conn = db.0.lock().unwrap();
            avatar::store(&conn, &key, data_url.as_deref(), now)?;
            Ok(data_url)
        }
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub fn commit_detail(repo_id: i64, sha: String, db: State<Db>) -> AppResult<CommitDetail> {
    let checkout = resolve_checkout(&db, repo_id, None)?;
    graph::commit_detail(&checkout, &sha)
}

#[tauri::command]
pub fn commit_file_diff(
    repo_id: i64,
    sha: String,
    path: String,
    old_path: Option<String>,
    db: State<Db>,
) -> AppResult<String> {
    let checkout = resolve_checkout(&db, repo_id, None)?;
    graph::commit_file_diff(&checkout, &sha, &path, old_path.as_deref())
}

#[tauri::command]
pub fn worktree_status(
    repo_id: i64,
    worktree_path: Option<String>,
    db: State<Db>,
) -> AppResult<WipStatus> {
    let checkout = resolve_checkout(&db, repo_id, worktree_path)?;
    graph::worktree_status(&checkout)
}

#[tauri::command]
pub fn worktree_file_diff(
    repo_id: i64,
    worktree_path: Option<String>,
    path: String,
    kind: WipKind,
    db: State<Db>,
) -> AppResult<String> {
    let checkout = resolve_checkout(&db, repo_id, worktree_path)?;
    graph::worktree_file_diff(&checkout, &path, kind)
}
