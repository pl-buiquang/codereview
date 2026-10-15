use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::worktree::{self, WorktreeInfo};
use crate::worktree_gc::{self, GcOptions, GcReport, RepoCtx};

pub(crate) fn repo_local_path(db: &Db, repo_id: i64) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    let local_path: Option<String> = conn
        .query_row(
            "SELECT local_path FROM repository WHERE id = ?1",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )
        .map_err(AppError::from)?;
    local_path.ok_or_else(|| AppError::Other("operation requires a local clone".into()))
}

/// Resolves the checkout a graph command operates on: the repo's main clone, or a
/// `worktree_path` that must be one of the repo's registered worktrees.
pub(crate) fn resolve_checkout(
    db: &Db,
    repo_id: i64,
    worktree_path: Option<String>,
) -> AppResult<PathBuf> {
    let repo_path = repo_local_path(db, repo_id)?;
    match worktree_path {
        None => Ok(PathBuf::from(repo_path)),
        Some(p) => {
            let worktrees = worktree::list_worktrees(Path::new(&repo_path))?;
            match_worktree(&worktrees, &p)
        }
    }
}

fn match_worktree(worktrees: &[WorktreeInfo], path: &str) -> AppResult<PathBuf> {
    worktrees
        .iter()
        .find(|w| w.path == path)
        .map(|w| PathBuf::from(&w.path))
        .ok_or_else(|| AppError::Other("unknown worktree path".into()))
}

#[tauri::command]
pub fn list_worktrees(repo_id: i64, db: State<Db>) -> AppResult<Vec<WorktreeInfo>> {
    let path = repo_local_path(&db, repo_id)?;
    worktree::list_worktrees(Path::new(&path))
}

#[tauri::command]
pub fn remove_worktree(repo_id: i64, worktree_path: String, db: State<Db>) -> AppResult<()> {
    let repo_path = repo_local_path(&db, repo_id)?;
    worktree::remove_worktree(Path::new(&repo_path), Path::new(&worktree_path))
}

#[tauri::command]
pub fn prune_worktrees(repo_id: i64, db: State<Db>) -> AppResult<String> {
    let path = repo_local_path(&db, repo_id)?;
    worktree::prune_worktrees(Path::new(&path))
}

/// Scans every repo with a local clone (`remove`/`sync` = also apply those
/// actions). Manual only; the git/gh work runs off the UI thread.
#[tauri::command]
pub async fn worktree_gc_run(
    remove: bool,
    sync: bool,
    db: State<'_, Db>,
) -> AppResult<GcReport> {
    let (repos, chat_paths) = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, local_path, remote_owner, remote_name, default_branch
             FROM repository WHERE local_path IS NOT NULL ORDER BY local_path",
        )?;
        let repos = stmt
            .query_map([], |r| {
                Ok(RepoCtx {
                    repo_id: r.get(0)?,
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    owner: r.get(2)?,
                    name: r.get(3)?,
                    default_branch: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare(
            "SELECT c.worktree_path FROM chat c
             JOIN review rv ON rv.id = c.review_id
             JOIN target t ON t.id = rv.target_id
             WHERE c.worktree_path IS NOT NULL AND t.kind = 'local'",
        )?;
        let chat_paths = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        (repos, chat_paths)
    };
    let opts = GcOptions { remove, sync };
    tauri::async_runtime::spawn_blocking(move || worktree_gc::run(&repos, chat_paths, opts))
        .await
        .map_err(|e| AppError::Other(format!("worktree gc task failed: {e}")))?
}

#[tauri::command]
pub fn worktree_gc_last() -> Option<GcReport> {
    worktree_gc::last_report()
}

/// Fast-forwards (or resyncs after an upstream rewrite) one registered worktree.
#[tauri::command]
pub fn worktree_sync(repo_id: i64, worktree_path: String, db: State<Db>) -> AppResult<String> {
    let path = resolve_checkout(&db, repo_id, Some(worktree_path))?;
    worktree_gc::sync_one(&path)
}

/// Opens a terminal (tmux window when available, like the board's Claude
/// terminals) with a plain shell in one of the repo's registered worktrees.
#[tauri::command]
pub fn worktree_open_terminal(
    repo_id: i64,
    worktree_path: String,
    host: crate::terminal::TerminalHost,
    tmux_target: crate::terminal::TmuxTarget,
    tmux_session: String,
    db: State<Db>,
) -> AppResult<()> {
    let path = resolve_checkout(&db, repo_id, Some(worktree_path))?;
    let repo_label = {
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
            rusqlite::params![repo_id],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
        )?
    };
    let repo = match repo_label {
        (Some(o), Some(n)) => format!("{o}/{n}"),
        _ => repo_local_path(&db, repo_id)?,
    };
    let window_name = worktree::current_branch(&path).unwrap_or_else(|| {
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    crate::terminal::open_shell(
        host,
        tmux_target,
        &tmux_session,
        &repo,
        &window_name,
        &path.to_string_lossy(),
    )
}

#[tauri::command]
pub fn open_in_vscode(path: String, file: Option<String>) -> AppResult<()> {
    let mut cmd = Command::new("code");
    cmd.arg(&path);
    if let Some(file) = file {
        cmd.arg(Path::new(&path).join(file));
    }
    cmd.spawn()
        .map_err(|e| AppError::Other(format!("failed to open VSCode: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::WorktreeSource;

    fn wt(path: &str) -> WorktreeInfo {
        WorktreeInfo {
            path: path.into(),
            display_path: path.into(),
            head_sha: "abc".into(),
            branch: None,
            is_main: false,
            is_prunable: false,
            prunable_reason: None,
            is_locked: false,
            source: WorktreeSource::Manual,
        }
    }

    #[test]
    fn match_worktree_accepts_registered_path() {
        let list = [wt("/repo"), wt("/repo-wt")];
        assert_eq!(match_worktree(&list, "/repo-wt").unwrap(), PathBuf::from("/repo-wt"));
    }

    #[test]
    fn match_worktree_rejects_unknown_path() {
        let err = match_worktree(&[wt("/repo")], "/etc").unwrap_err();
        assert_eq!(err.to_string(), "unknown worktree path");
    }
}
