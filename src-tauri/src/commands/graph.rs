use tauri::State;

use crate::commands::worktree::resolve_checkout;
use crate::db::Db;
use crate::error::AppResult;
use crate::graph::{self, CommitDetail, GraphCommit, RefInfo, Scope, WipKind, WipStatus};

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
