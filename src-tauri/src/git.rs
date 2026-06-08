use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Run a git command inside `repo` and return stdout on success.
pub fn run_git(repo: &Path, args: &[&str]) -> AppResult<String> {
    let output = Command::new(crate::tools::git_bin())
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| AppError::Git(format!("failed to spawn git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Git(stderr.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn is_git_repo(path: &Path) -> bool {
    run_git(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

#[derive(Debug, Default)]
pub struct RemoteInfo {
    pub owner: Option<String>,
    pub name: Option<String>,
}

/// Best-effort parse of `origin`'s owner/repo (GitHub ssh or https form).
pub fn remote_info(path: &Path) -> RemoteInfo {
    match run_git(path, &["remote", "get-url", "origin"]) {
        Ok(url) => parse_owner_repo(&url),
        Err(_) => RemoteInfo::default(),
    }
}

pub fn parse_owner_repo(url: &str) -> RemoteInfo {
    let u = url.trim().trim_end_matches(".git");
    // For `git@github.com:owner/repo` or `https://github.com/owner/repo`,
    // take everything after the host, then the last two path segments.
    let tail = match u.find("github.com") {
        Some(idx) => u[idx + "github.com".len()..].trim_start_matches([':', '/']),
        None => u,
    };
    let parts: Vec<&str> = tail.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        RemoteInfo {
            owner: Some(parts[parts.len() - 2].to_string()),
            name: Some(parts[parts.len() - 1].to_string()),
        }
    } else {
        RemoteInfo::default()
    }
}

/// Resolve the repository's default branch (origin/HEAD, falling back to the
/// current branch).
pub fn default_branch(path: &Path) -> Option<String> {
    run_git(path, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .map(|s| s.trim().trim_start_matches("origin/").to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
                .ok()
                .map(|s| s.trim().to_string())
        })
}

#[derive(Debug, Serialize)]
pub struct Branch {
    pub name: String,
    pub is_remote: bool,
    pub sha: String,
}

/// List local and remote-tracking branches (remote HEAD pointers excluded).
pub fn list_branches(path: &Path) -> AppResult<Vec<Branch>> {
    let out = run_git(
        path,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname)%09%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let mut branches = Vec::new();
    for line in out.lines() {
        let mut cols = line.split('\t');
        let (Some(name), Some(sha), Some(full)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        // Skip symbolic refs like `origin/HEAD`.
        if name.ends_with("/HEAD") {
            continue;
        }
        branches.push(Branch {
            name: name.to_string(),
            is_remote: full.starts_with("refs/remotes"),
            sha: sha.to_string(),
        });
    }
    Ok(branches)
}

pub fn rev_parse(path: &Path, rev: &str) -> AppResult<String> {
    Ok(run_git(path, &["rev-parse", rev])?.trim().to_string())
}

/// Full contents of `file_path` as of `sha` (`git show <sha>:<file_path>`),
/// used to reveal collapsed context lines around a diff.
pub fn show_file(path: &Path, sha: &str, file_path: &str) -> AppResult<String> {
    run_git(path, &["show", &format!("{sha}:{file_path}")])
}

/// Unified diff between two refs. `three_dot` uses the merge-base (GitHub PR
/// semantics); otherwise a plain two-dot diff.
pub fn diff(path: &Path, base: &str, head: &str, three_dot: bool) -> AppResult<String> {
    let range = if three_dot {
        format!("{base}...{head}")
    } else {
        format!("{base}..{head}")
    };
    run_git(path, &["diff", "--no-color", &range])
}

/// Plain two-dot diff between two commits (literal line evolution old→new).
/// Two-dot, NOT three-dot: merge-base semantics are irrelevant here. Kept
/// alongside the path-scoped variant the re-anchor helper uses (Spec 01 §2).
#[allow(dead_code)]
pub fn diff_shas(repo: &Path, old_sha: &str, new_sha: &str) -> AppResult<String> {
    run_git(repo, &["diff", "--no-color", &format!("{old_sha}..{new_sha}")])
}

/// Two-dot diff scoped to a single file, so the diff parser only sees one file.
pub fn diff_shas_path(
    repo: &Path,
    old_sha: &str,
    new_sha: &str,
    file_path: &str,
) -> AppResult<String> {
    run_git(
        repo,
        &[
            "diff",
            "--no-color",
            &format!("{old_sha}..{new_sha}"),
            "--",
            file_path,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    // ---- pure parsing -----------------------------------------------------

    #[test]
    fn parse_owner_repo_ssh() {
        let r = parse_owner_repo("git@github.com:owner/repo.git");
        assert_eq!(r.owner.as_deref(), Some("owner"));
        assert_eq!(r.name.as_deref(), Some("repo"));
    }

    #[test]
    fn parse_owner_repo_https() {
        let r = parse_owner_repo("https://github.com/some-org/some-repo.git");
        assert_eq!(r.owner.as_deref(), Some("some-org"));
        assert_eq!(r.name.as_deref(), Some("some-repo"));
    }

    #[test]
    fn parse_owner_repo_https_without_git_suffix() {
        let r = parse_owner_repo("https://github.com/a/b");
        assert_eq!(r.owner.as_deref(), Some("a"));
        assert_eq!(r.name.as_deref(), Some("b"));
    }

    #[test]
    fn parse_owner_repo_trims_whitespace_and_trailing_newline() {
        let r = parse_owner_repo("  git@github.com:owner/repo.git\n");
        assert_eq!(r.owner.as_deref(), Some("owner"));
        assert_eq!(r.name.as_deref(), Some("repo"));
    }

    #[test]
    fn parse_owner_repo_non_github_falls_back_to_last_two_segments() {
        let r = parse_owner_repo("https://gitlab.com/group/project.git");
        assert_eq!(r.owner.as_deref(), Some("group"));
        assert_eq!(r.name.as_deref(), Some("project"));
    }

    #[test]
    fn parse_owner_repo_garbage_yields_none() {
        let r = parse_owner_repo("not-a-url");
        assert!(r.owner.is_none());
        assert!(r.name.is_none());
    }

    // ---- integration against a real temporary git repo --------------------

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// Build a repo with a `main` branch (one commit) and a `feature` branch
    /// that adds a line, so diffs/branches are deterministic.
    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "line1\nline2\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "initial"]);

        git(p, &["checkout", "-q", "-b", "feature"]);
        fs::write(p.join("file.txt"), "line1\nline2\nline3\n").unwrap();
        git(p, &["commit", "-q", "-am", "add line3"]);
        git(p, &["checkout", "-q", "main"]);
        dir
    }

    /// Build a repo with two linear commits on `main`. H1 has a few lines; H2
    /// inserts a line above an existing one, so the SHA-to-SHA diff is
    /// deterministic. Returns `(dir, h1, h2)`.
    fn fixture_repo_two_heads() -> (TempDir, String, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "alpha\nbeta\ngamma\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "h1"]);
        let h1 = rev_parse(p, "HEAD").unwrap();

        fs::write(p.join("file.txt"), "alpha\nINSERTED\nbeta\ngamma\n").unwrap();
        git(p, &["commit", "-q", "-am", "h2"]);
        let h2 = rev_parse(p, "HEAD").unwrap();
        (dir, h1, h2)
    }

    #[test]
    fn is_git_repo_true_for_repo_false_otherwise() {
        let repo = fixture_repo();
        assert!(is_git_repo(repo.path()));

        let empty = TempDir::new().unwrap();
        assert!(!is_git_repo(empty.path()));
    }

    #[test]
    fn rev_parse_returns_a_sha() {
        let repo = fixture_repo();
        let sha = rev_parse(repo.path(), "main").unwrap();
        assert_eq!(sha.len(), 40, "expected full sha, got {sha:?}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn list_branches_includes_local_heads() {
        let repo = fixture_repo();
        let names: Vec<String> = list_branches(repo.path())
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(names.contains(&"main".to_string()), "got {names:?}");
        assert!(names.contains(&"feature".to_string()), "got {names:?}");
        // No symbolic HEAD pointers leak through.
        assert!(!names.iter().any(|n| n.ends_with("/HEAD")));
    }

    #[test]
    fn diff_shows_added_line() {
        let repo = fixture_repo();
        let out = diff(repo.path(), "main", "feature", false).unwrap();
        assert!(out.contains("+line3"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }

    #[test]
    fn diff_empty_when_no_changes() {
        let repo = fixture_repo();
        let out = diff(repo.path(), "main", "main", false).unwrap();
        assert!(out.trim().is_empty(), "expected empty diff, got: {out}");
    }

    #[test]
    fn run_git_surfaces_error_on_bad_ref() {
        let repo = fixture_repo();
        let err = rev_parse(repo.path(), "no-such-ref").unwrap_err();
        assert!(matches!(err, AppError::Git(_)));
    }

    #[test]
    fn show_file_returns_contents_at_ref() {
        let repo = fixture_repo();
        let out = show_file(repo.path(), "main", "file.txt").unwrap();
        assert_eq!(out, "line1\nline2\n");

        let feature = show_file(repo.path(), "feature", "file.txt").unwrap();
        assert_eq!(feature, "line1\nline2\nline3\n");
    }

    #[test]
    fn show_file_errors_on_missing_path() {
        let repo = fixture_repo();
        let err = show_file(repo.path(), "main", "no-such-file.txt").unwrap_err();
        assert!(matches!(err, AppError::Git(_)));
    }

    #[test]
    fn diff_shas_shows_inserted_line() {
        let (repo, h1, h2) = fixture_repo_two_heads();
        let out = diff_shas(repo.path(), &h1, &h2).unwrap();
        assert!(out.contains("+INSERTED"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }

    #[test]
    fn diff_shas_path_scopes_to_one_file() {
        let (repo, h1, h2) = fixture_repo_two_heads();
        let out = diff_shas_path(repo.path(), &h1, &h2, "file.txt").unwrap();
        assert!(out.contains("+INSERTED"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }
}
