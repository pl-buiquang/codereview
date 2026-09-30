use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::git::run_git;

fn worktrees_base() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".codereview")
        .join("worktrees")
}

/// Returns the path where code at `head_sha` is available for reading:
/// - If the repo's HEAD already points at `head_sha`, returns `repo` directly.
/// - Otherwise creates (or reuses) a detached worktree at
///   `~/.codereview/worktrees/<repo_name>/<sha_short>/`.
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
    if run_git(repo, &["cat-file", "-e", head_sha]).is_err()
        && run_git(repo, &["fetch", "origin", head_sha]).is_err()
    {
        return Ok(repo.to_path_buf());
    }

    let sha_short = &head_sha[..head_sha.len().min(8)];
    let repo_name = repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let worktree_path = worktrees_base().join(repo_name).join(sha_short);

    if worktree_path.exists() {
        return Ok(worktree_path);
    }

    std::fs::create_dir_all(worktrees_base().join(repo_name))
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeSource {
    Claude,
    Codex,
    Codereview,
    Manual,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeInfo {
    pub path: String,
    /// Path with home directory replaced by `~` for display.
    pub display_path: String,
    pub head_sha: String,
    /// Branch name (without `refs/heads/` prefix), or `None` if detached.
    pub branch: Option<String>,
    pub is_main: bool,
    pub is_prunable: bool,
    pub prunable_reason: Option<String>,
    pub source: WorktreeSource,
}

/// Returns all worktrees registered for `repo` by parsing
/// `git worktree list --porcelain`.
pub fn list_worktrees(repo: &Path) -> AppResult<Vec<WorktreeInfo>> {
    let raw = run_git(repo, &["worktree", "list", "--porcelain"])?;
    let home = dirs::home_dir();
    let mut result = Vec::new();
    let mut is_first = true;

    for block in raw.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut path: Option<String> = None;
        let mut head_sha = String::new();
        let mut branch: Option<String> = None;
        let mut is_prunable = false;
        let mut prunable_reason: Option<String> = None;

        for line in block.lines() {
            if let Some(v) = line.strip_prefix("worktree ") {
                path = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("HEAD ") {
                head_sha = v.to_string();
            } else if let Some(v) = line.strip_prefix("branch ") {
                branch = Some(
                    v.strip_prefix("refs/heads/")
                        .unwrap_or(v)
                        .to_string(),
                );
            } else if let Some(v) = line.strip_prefix("prunable ") {
                is_prunable = true;
                prunable_reason = Some(v.to_string());
            } else if line == "prunable" {
                is_prunable = true;
            }
            // "detached", "locked", "bare" and unknown keys are ignored
        }

        let Some(path) = path else { continue };

        let display_path = match &home {
            Some(h) => {
                let h_str = h.to_string_lossy();
                if path.starts_with(h_str.as_ref()) {
                    format!("~{}", &path[h_str.len()..])
                } else {
                    path.clone()
                }
            }
            None => path.clone(),
        };

        let source = detect_source(&path);
        let is_main = is_first;
        is_first = false;

        result.push(WorktreeInfo {
            path,
            display_path,
            head_sha,
            branch,
            is_main,
            is_prunable,
            prunable_reason,
            source,
        });
    }

    Ok(result)
}

/// Prunes stale worktree metadata with `git worktree prune`.
pub fn prune_worktrees(repo: &Path) -> AppResult<String> {
    run_git(repo, &["worktree", "prune"])
}

fn detect_source(path: &str) -> WorktreeSource {
    if path.contains("/.claude/worktrees/") || path.contains("\\.claude\\worktrees\\") {
        WorktreeSource::Claude
    } else if path.contains("/.codex/worktrees/") || path.contains("\\.codex\\worktrees\\") {
        WorktreeSource::Codex
    } else if path.contains("/.codereview/worktrees/") || path.contains("\\.codereview\\worktrees\\") {
        WorktreeSource::Codereview
    } else {
        WorktreeSource::Manual
    }
}
