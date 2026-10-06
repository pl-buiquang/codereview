use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::models::Target;
use crate::error::{AppError, AppResult};
use crate::gh::{run_gh, GhRepo};
use crate::git::run_git;

fn worktrees_base() -> PathBuf {
    worktrees_base_in(&dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp")))
}

fn worktrees_base_in(home: &Path) -> PathBuf {
    home.join(".codereview").join("worktrees")
}

/// Returns a worktree checked out on `target.head_ref`, for the chat's edit
/// session. Reuses an existing worktree already on that branch — including
/// the main checkout — found via `list_worktrees`; otherwise creates one
/// under `~/.codereview/worktrees/<repo_name>/<branch>/`.
pub fn ensure_branch_worktree(repo: &Path, target: &Target) -> AppResult<PathBuf> {
    ensure_branch_worktree_in(repo, target, &worktrees_base())
}

fn ensure_branch_worktree_in(repo: &Path, target: &Target, base: &Path) -> AppResult<PathBuf> {
    let branch = &target.head_ref;

    for wt in list_worktrees(repo)? {
        if !wt.is_prunable && wt.branch.as_deref() == Some(branch.as_str()) {
            return Ok(PathBuf::from(wt.path));
        }
    }

    let repo_name = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    let worktree_path = base.join(repo_name).join(branch.replace('/', "-"));

    if worktree_path.exists() {
        return Ok(worktree_path);
    }

    std::fs::create_dir_all(base.join(repo_name))
        .map_err(|e| AppError::Git(format!("failed to create worktrees dir: {e}")))?;

    if target.kind == "github_pr" {
        let head_sha = target
            .head_sha
            .as_deref()
            .ok_or_else(|| AppError::Git("PR head SHA is not resolved yet".into()))?;

        if run_git(repo, &["cat-file", "-e", head_sha]).is_err() {
            run_git(repo, &["fetch", "origin", head_sha])
                .map_err(|e| AppError::Git(format!("failed to fetch PR head {head_sha}: {e}")))?;
        }

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

        // `gh pr checkout` turns the detached worktree into a real branch
        // checkout tracking the PR (handles fork PRs' remote naming too).
        let number = target
            .github_pr_number
            .ok_or_else(|| AppError::Git("github_pr target is missing a PR number".into()))?;
        run_gh(
            &GhRepo::Local(worktree_path.clone()),
            &["pr", "checkout", &number.to_string()],
        )
        .map_err(|e| AppError::Git(format!("gh pr checkout failed: {e}")))?;
    } else {
        if run_git(repo, &["rev-parse", "--verify", &format!("refs/heads/{branch}")]).is_err() {
            return Err(AppError::Git(format!(
                "`{branch}` is not a local branch; check it out in the repo to chat-edit"
            )));
        }
        run_git(
            repo,
            &[
                "worktree",
                "add",
                worktree_path.to_str().unwrap_or_default(),
                branch,
            ],
        )?;
    }

    Ok(worktree_path)
}

/// Removes a worktree created by `ensure_branch_worktree`, but only if it has
/// no uncommitted changes and lives under the codereview worktrees dir — it
/// never touches a worktree the user set up themselves, and never deletes
/// the branch (it belongs to the PR, not to the chat).
pub fn cleanup_chat_worktree(repo: &Path, worktree_path: &Path) -> AppResult<()> {
    cleanup_chat_worktree_in(repo, worktree_path, &worktrees_base())
}

fn cleanup_chat_worktree_in(repo: &Path, worktree_path: &Path, base: &Path) -> AppResult<()> {
    if worktree_path == repo || !worktree_path.exists() || !worktree_path.starts_with(base) {
        return Ok(());
    }

    let status = run_git(worktree_path, &["status", "--porcelain"]).unwrap_or_default();
    if !status.trim().is_empty() {
        return Ok(());
    }

    let _ = run_git(
        repo,
        &["worktree", "remove", worktree_path.to_str().unwrap_or_default()],
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Repo with `main` (one commit) and `feature` (a second commit), left
    /// checked out on `main`.
    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "line1\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "initial"]);
        git(p, &["checkout", "-q", "-b", "feature"]);
        fs::write(p.join("file.txt"), "line1\nline2\n").unwrap();
        git(p, &["commit", "-q", "-am", "add line2"]);
        git(p, &["checkout", "-q", "main"]);
        dir
    }

    fn local_target(head_ref: &str) -> Target {
        Target {
            id: 1,
            repo_id: 1,
            kind: "local".into(),
            github_pr_number: None,
            title: "t".into(),
            base_ref: "main".into(),
            head_ref: head_ref.into(),
            base_sha: None,
            head_sha: None,
            three_dot: true,
            created_at: "now".into(),
        }
    }

    #[test]
    fn reuses_main_checkout_when_branch_already_there() {
        let repo = fixture_repo();
        git(repo.path(), &["checkout", "-q", "feature"]);
        let base = TempDir::new().unwrap();

        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();

        // Compare canonicalized paths — on macOS `TempDir` lives under a `/tmp`
        // symlink, but `git worktree list` reports the resolved `/private/...` path.
        assert_eq!(fs::canonicalize(wt).unwrap(), fs::canonicalize(repo.path()).unwrap());
    }

    #[test]
    fn creates_worktree_when_branch_not_checked_out_anywhere() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();

        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();

        assert!(wt.starts_with(base.path()));
        assert_eq!(fs::read_to_string(wt.join("file.txt")).unwrap(), "line1\nline2\n");

        // Second call reuses the same worktree instead of erroring/duplicating.
        // (Found via `git worktree list` this time, which reports the canonicalized
        // path, hence comparing canonicalized paths on both sides.)
        let wt2 = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();
        assert_eq!(fs::canonicalize(wt).unwrap(), fs::canonicalize(wt2).unwrap());
    }

    #[test]
    fn errors_when_head_ref_is_not_a_local_branch() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();

        let err = ensure_branch_worktree_in(repo.path(), &local_target("no-such-branch"), base.path())
            .unwrap_err();
        assert!(matches!(err, AppError::Git(_)));
    }

    #[test]
    fn cleanup_removes_clean_worktree_under_base() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();

        cleanup_chat_worktree_in(repo.path(), &wt, base.path()).unwrap();

        assert!(!wt.exists());
    }

    #[test]
    fn cleanup_keeps_dirty_worktree() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();
        fs::write(wt.join("untracked.txt"), "wip").unwrap();

        cleanup_chat_worktree_in(repo.path(), &wt, base.path()).unwrap();

        assert!(wt.exists());
    }

    #[test]
    fn cleanup_keeps_worktree_outside_base() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), outside.path())
            .unwrap();

        // `base` here is unrelated to where `wt` was created.
        cleanup_chat_worktree_in(repo.path(), &wt, base.path()).unwrap();

        assert!(wt.exists());
    }
}
