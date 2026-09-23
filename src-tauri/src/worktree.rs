use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::git::run_git;

const WORKTREE_DIR: &str = ".codereview-worktrees";

/// Returns the path where code at `head_sha` is available for reading:
/// - If the repo's HEAD already points at `head_sha`, returns `repo` directly.
/// - Otherwise creates (or reuses) a detached worktree at
///   `<repo>/.codereview-worktrees/<sha_short>/`.
pub fn ensure_worktree(repo: &Path, head_sha: &str) -> AppResult<PathBuf> {
    let current_head = run_git(repo, &["rev-parse", "HEAD"])
        .unwrap_or_default();
    if current_head.trim() == head_sha {
        return Ok(repo.to_path_buf());
    }

    // `git cat-file -e <sha>` exits 0 if the object is present locally.
    // For GitHub PR reviews the head commit is often unfetched — try fetching
    // it from origin. If the fetch fails (no network, server doesn't support
    // SHA fetches, etc.) fall back to the repo as-is so chat still works.
    if run_git(repo, &["cat-file", "-e", head_sha]).is_err() {
        if run_git(repo, &["fetch", "origin", head_sha]).is_err() {
            return Ok(repo.to_path_buf());
        }
    }

    let sha_short = &head_sha[..head_sha.len().min(8)];
    let worktree_path = repo.join(WORKTREE_DIR).join(sha_short);

    if worktree_path.exists() {
        return Ok(worktree_path);
    }

    std::fs::create_dir_all(repo.join(WORKTREE_DIR))
        .map_err(|e| AppError::Git(format!("failed to create worktrees dir: {e}")))?;

    run_git(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().unwrap_or_default(),
            head_sha,
        ],
    )?;

    Ok(worktree_path)
}

/// Removes a worktree created by `ensure_worktree`. No-op if the path doesn't
/// exist or is the repo itself (meaning no worktree was created).
pub fn cleanup_worktree(repo: &Path, worktree_path: &Path) -> AppResult<()> {
    if worktree_path == repo || !worktree_path.exists() {
        return Ok(());
    }
    let _ = run_git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap_or_default(),
        ],
    );
    Ok(())
}
