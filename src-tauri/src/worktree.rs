use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::db::models::Target;
use crate::error::{AppError, AppResult};
use crate::gh::{run_gh, GhRepo};
use crate::git::{ensure_commit, run_git};

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
        return switch_existing_to_branch(&worktree_path, target);
    }

    std::fs::create_dir_all(base.join(repo_name))
        .map_err(|e| AppError::Git(format!("failed to create worktrees dir: {e}")))?;

    if target.kind == "github_pr" {
        let head_sha = target
            .head_sha
            .as_deref()
            .ok_or_else(|| AppError::Git("PR head SHA is not resolved yet".into()))?;
        let number = target
            .github_pr_number
            .ok_or_else(|| AppError::Git("github_pr target is missing a PR number".into()))?;

        ensure_commit(repo, head_sha)?;

        let wt = worktree_path.to_str().unwrap_or_default();
        run_git(repo, &["worktree", "add", "--detach", wt, head_sha])?;

        // `gh pr checkout` turns the detached worktree into a real branch
        // checkout tracking the PR (handles fork PRs' remote naming too). If it
        // fails, drop the detached worktree so it isn't reused as if it were
        // on the branch.
        if let Err(e) = run_gh(
            &GhRepo::Local(worktree_path.clone()),
            &["pr", "checkout", &number.to_string()],
        ) {
            let _ = run_git(repo, &["worktree", "remove", "--force", wt]);
            return Err(AppError::Git(format!("gh pr checkout failed: {e}")));
        }
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

/// The slot path for `target.head_ref` already exists but isn't on that
/// branch (e.g. left detached by an earlier failure): move it onto the branch
/// if it's clean, otherwise refuse rather than hand back the wrong checkout.
fn switch_existing_to_branch(path: &Path, target: &Target) -> AppResult<PathBuf> {
    if is_on_branch(path, &target.head_ref) {
        return Ok(path.to_path_buf());
    }
    let status = run_git(path, &["status", "--porcelain"])?;
    if !status.trim().is_empty() {
        return Err(AppError::Git(format!(
            "{} is not on `{}` and has uncommitted changes",
            path.display(),
            target.head_ref
        )));
    }
    let switched = match (target.kind.as_str(), target.github_pr_number) {
        ("github_pr", Some(number)) => run_gh(
            &GhRepo::Local(path.to_path_buf()),
            &["pr", "checkout", &number.to_string()],
        )
        .map(|_| ())
        .map_err(|e| e.to_string()),
        _ => run_git(path, &["switch", &target.head_ref])
            .map(|_| ())
            .map_err(|e| e.to_string()),
    };
    switched.map_err(|e| {
        AppError::Git(format!(
            "could not switch {} to `{}`: {e}",
            path.display(),
            target.head_ref
        ))
    })?;
    Ok(path.to_path_buf())
}

/// Current branch of the checkout at `path`, `None` when detached.
pub fn current_branch(path: &Path) -> Option<String> {
    run_git(path, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether `path` is checked out on `branch`. Also accepts the names
/// `gh pr checkout` gives fork PR branches (`<owner>/<branch>`, `<owner>-<branch>`).
pub fn is_on_branch(path: &Path, branch: &str) -> bool {
    current_branch(path).is_some_and(|b| {
        b == branch || b.ends_with(&format!("/{branch}")) || b.ends_with(&format!("-{branch}"))
    })
}

/// Whether `path` is a repo's main checkout rather than a linked worktree.
pub fn is_main_checkout(path: &Path) -> bool {
    let dir = run_git(path, &["rev-parse", "--absolute-git-dir"]);
    let common = run_git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    matches!((dir, common), (Ok(d), Ok(c)) if d.trim() == c.trim())
}

/// Removes a linked worktree. Never forces: git refuses when the worktree has
/// modified or untracked files, and that refusal is surfaced to the caller.
pub fn remove_worktree(repo: &Path, worktree_path: &Path) -> AppResult<()> {
    if worktree_path == repo || !worktree_path.exists() {
        return Ok(());
    }
    run_git(
        repo,
        &["worktree", "remove", worktree_path.to_str().unwrap_or_default()],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    pub is_locked: bool,
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
        let mut is_locked = false;

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
            } else if line == "locked" || line.starts_with("locked ") {
                is_locked = true;
            }
            // "detached", "bare" and unknown keys are ignored
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
            is_locked,
            source,
        });
    }

    Ok(result)
}

/// Prunes stale worktree metadata with `git worktree prune`.
pub fn prune_worktrees(repo: &Path) -> AppResult<String> {
    run_git(repo, &["worktree", "prune"])
}

pub fn detect_source(path: &str) -> WorktreeSource {
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
    fn existing_detached_slot_is_switched_onto_branch() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let slot = base.path().join(repo.path().file_name().unwrap()).join("feature");
        git(repo.path(), &["worktree", "add", "--detach", slot.to_str().unwrap(), "main"]);

        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();

        assert_eq!(current_branch(&wt).as_deref(), Some("feature"));
    }

    #[test]
    fn existing_dirty_slot_on_wrong_ref_is_refused() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let slot = base.path().join(repo.path().file_name().unwrap()).join("feature");
        git(repo.path(), &["worktree", "add", "--detach", slot.to_str().unwrap(), "main"]);
        fs::write(slot.join("wip.txt"), "wip").unwrap();

        let err = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap_err();
        assert!(err.to_string().contains("uncommitted"));
    }

    #[test]
    fn detects_main_checkout_vs_linked_worktree() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();

        assert!(is_main_checkout(repo.path()));
        assert!(!is_main_checkout(&wt));
        assert!(is_on_branch(&wt, "feature"));
    }

    #[test]
    fn remove_refuses_dirty_worktree() {
        let repo = fixture_repo();
        let base = TempDir::new().unwrap();
        let wt = ensure_branch_worktree_in(repo.path(), &local_target("feature"), base.path())
            .unwrap();
        fs::write(wt.join("untracked.txt"), "wip").unwrap();

        assert!(remove_worktree(repo.path(), &wt).is_err());
        assert!(wt.exists());
    }
}
