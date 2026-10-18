use std::time::Duration;

use tauri::State;

use crate::avatar;
use crate::commands::review::repo_owner_name;
use crate::commands::worktree::resolve_checkout;
use crate::db::Db;
use crate::error::AppResult;
use crate::git;
use crate::graph::{self, ops, CommitDetail, GraphCommit, RefInfo, Scope, StashEntry, WipKind, WipStatus};

const GONE_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// Fetch (unless fresh), then the local branches whose upstream is gone.
/// `async`: the fetch is network, kept off the UI thread.
#[tauri::command]
pub async fn list_gone_branches(repo_id: i64, db: State<'_, Db>) -> AppResult<Vec<String>> {
    crate::repo_sync::fetch_repo(&db, repo_id, GONE_FETCH_MIN_INTERVAL)?;
    let checkout = resolve_checkout(&db, repo_id, None)?;
    graph::list_gone(&checkout)
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

/// Async so `git status` on a large or busy checkout can't block the UI thread.
#[tauri::command]
pub async fn worktree_status(
    repo_id: i64,
    worktree_path: Option<String>,
    db: State<'_, Db>,
) -> AppResult<WipStatus> {
    let checkout = resolve_checkout(&db, repo_id, worktree_path)?;
    tauri::async_runtime::spawn_blocking(move || graph::worktree_status(&checkout))
        .await
        .map_err(|e| crate::error::AppError::Other(format!("worktree status task failed: {e}")))?
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

fn blocking_err(e: tauri::Error) -> crate::error::AppError {
    crate::error::AppError::Other(format!("git task failed: {e}"))
}

/// Runs a mutating op on the resolved checkout. Async + blocking pool: some ops hit the network
/// or run hooks, and none may hold up the UI thread.
async fn mutate<T: Send + 'static>(
    db: &Db,
    repo_id: i64,
    worktree_path: Option<String>,
    op: impl FnOnce(&std::path::Path) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let checkout = resolve_checkout(db, repo_id, worktree_path)?;
    tauri::async_runtime::spawn_blocking(move || op(&checkout))
        .await
        .map_err(blocking_err)?
}

#[tauri::command]
pub async fn git_checkout_branch(
    repo_id: i64,
    worktree_path: Option<String>,
    name: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::checkout_branch(c, &name)).await
}

#[tauri::command]
pub async fn git_checkout_remote_branch(
    repo_id: i64,
    worktree_path: Option<String>,
    remote_ref: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::checkout_remote_branch(c, &remote_ref)).await
}

#[tauri::command]
pub async fn git_checkout_detached(
    repo_id: i64,
    worktree_path: Option<String>,
    rev: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::checkout_detached(c, &rev)).await
}

#[tauri::command]
pub async fn git_create_branch(
    repo_id: i64,
    worktree_path: Option<String>,
    name: String,
    at: String,
    switch: bool,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::create_branch(c, &name, &at, switch)).await
}

#[tauri::command]
pub async fn git_delete_branch(
    repo_id: i64,
    worktree_path: Option<String>,
    name: String,
    force: bool,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::delete_branch(c, &name, force)).await
}

#[tauri::command]
pub async fn git_rename_branch(
    repo_id: i64,
    worktree_path: Option<String>,
    old: String,
    new: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::rename_branch(c, &old, &new)).await
}

#[tauri::command]
pub async fn git_create_tag(
    repo_id: i64,
    worktree_path: Option<String>,
    name: String,
    at: String,
    message: Option<String>,
    push: bool,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| {
        ops::create_tag(c, &name, &at, message.as_deref(), push)
    })
    .await
}

#[tauri::command]
pub async fn git_delete_tag(
    repo_id: i64,
    worktree_path: Option<String>,
    name: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::delete_tag(c, &name)).await
}

#[tauri::command]
pub async fn git_stage(
    repo_id: i64,
    worktree_path: Option<String>,
    paths: Vec<String>,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::stage(c, &paths)).await
}

#[tauri::command]
pub async fn git_unstage(
    repo_id: i64,
    worktree_path: Option<String>,
    paths: Vec<String>,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::unstage(c, &paths)).await
}

#[tauri::command]
pub async fn git_discard(
    repo_id: i64,
    worktree_path: Option<String>,
    paths: Vec<String>,
    kind: WipKind,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::discard(c, &paths, kind)).await
}

#[tauri::command]
pub async fn git_commit(
    repo_id: i64,
    worktree_path: Option<String>,
    message: String,
    amend: bool,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::commit(c, &message, amend)).await
}

#[tauri::command]
pub async fn git_pull(
    repo_id: i64,
    worktree_path: Option<String>,
    db: State<'_, Db>,
) -> AppResult<String> {
    mutate(&db, repo_id, worktree_path, ops::pull_ff).await
}

#[tauri::command]
pub async fn git_push(
    repo_id: i64,
    worktree_path: Option<String>,
    force_with_lease: bool,
    db: State<'_, Db>,
) -> AppResult<String> {
    mutate(&db, repo_id, worktree_path, move |c| ops::push(c, force_with_lease)).await
}

#[tauri::command]
pub async fn git_stash_push(
    repo_id: i64,
    worktree_path: Option<String>,
    message: Option<String>,
    include_untracked: bool,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| {
        ops::stash_push(c, message.as_deref(), include_untracked)
    })
    .await
}

#[tauri::command]
pub async fn git_stash_list(
    repo_id: i64,
    worktree_path: Option<String>,
    db: State<'_, Db>,
) -> AppResult<Vec<StashEntry>> {
    mutate(&db, repo_id, worktree_path, ops::stash_list).await
}

#[tauri::command]
pub async fn git_stash_apply(
    repo_id: i64,
    worktree_path: Option<String>,
    index: u32,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::stash_apply(c, index)).await
}

#[tauri::command]
pub async fn git_stash_pop(
    repo_id: i64,
    worktree_path: Option<String>,
    index: u32,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::stash_pop(c, index)).await
}

#[tauri::command]
pub async fn git_stash_drop(
    repo_id: i64,
    worktree_path: Option<String>,
    index: u32,
    db: State<'_, Db>,
) -> AppResult<()> {
    mutate(&db, repo_id, worktree_path, move |c| ops::stash_drop(c, index)).await
}

#[tauri::command]
pub async fn git_stash_diff(
    repo_id: i64,
    worktree_path: Option<String>,
    index: u32,
    db: State<'_, Db>,
) -> AppResult<String> {
    mutate(&db, repo_id, worktree_path, move |c| ops::stash_diff(c, index)).await
}
