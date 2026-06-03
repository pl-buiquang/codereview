use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Run a git command inside `repo` and return stdout on success.
pub fn run_git(repo: &Path, args: &[&str]) -> AppResult<String> {
    let output = Command::new("git")
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
