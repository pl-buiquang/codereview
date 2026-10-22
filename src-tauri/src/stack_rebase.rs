//! Local, all-or-nothing rebase of a PR stack onto its trunk.
//!
//! The whole rebase is computed in memory first (`git replay`, which needs no
//! checkout and writes no refs), so a conflict or a refusal leaves every branch
//! and worktree untouched. Only then are the branches pushed in one atomic
//! push with a lease per branch, and finally the local branches / worktrees
//! are moved to the new tips. A local state we can't move safely (dirty tree,
//! unpushed commits, operation in progress) is a refusal up front.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::git::run_git;
use crate::graph::ops::run_network;
use crate::worktree::list_worktrees;

/// Where a stack branch also lives locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LocalCopy {
    /// Only on the remote: nothing local to move.
    None,
    /// A local branch that is not checked out; `sha` is where it points now.
    Branch { sha: String },
    /// Checked out in a worktree; `sha` is its current HEAD.
    Worktree { path: String, sha: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchPlan {
    pub branch: String,
    pub old: String,
    pub new: String,
    /// Commits the branch carries on top of its parent.
    pub commits: u32,
    pub local: LocalCopy,
}

impl BranchPlan {
    pub fn changed(&self) -> bool {
        self.old != self.new
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub trunk: String,
    /// Bottom of the stack first.
    pub branches: Vec<BranchPlan>,
}

impl Plan {
    pub fn is_noop(&self) -> bool {
        self.branches.iter().all(|b| !b.changed())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub pushed: Vec<String>,
    /// Local branches / worktrees moved to the new tips.
    pub moved_local: Vec<String>,
    /// Local copies left behind because they changed while we worked; the
    /// worktree Resync picks them up.
    pub skipped_local: Vec<String>,
}

fn remote_ref(branch: &str) -> String {
    format!("refs/remotes/origin/{branch}")
}

fn rev(repo: &Path, spec: &str) -> Option<String> {
    run_git(repo, &["rev-parse", "--verify", "-q", spec]).ok().map(|s| s.trim().to_string())
}

fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
    run_git(repo, &["merge-base", "--is-ancestor", a, b]).is_ok()
}

fn count(repo: &Path, range: &str) -> u32 {
    run_git(repo, &["rev-list", "--count", range])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Replays `from..origin/<branch>` onto `onto`; returns the new tip. `replay`
/// only reports refs under `refs/heads/`, so the result is read back through a
/// scratch `--ref` name (print mode writes nothing). An empty range yields `onto`.
fn replay(repo: &Path, onto: &str, from: &str, branch: &str, commits: u32) -> Result<String, String> {
    let range = format!("{from}..{}", remote_ref(branch));
    let scratch = "refs/heads/cr-stack-rebase";
    let out = run_git(repo, &["replay", "--ref-action=print", "--ref", scratch, "--onto", onto, &range])
        .map_err(|_| format!("{branch}: conflicts when rebased onto its new parent — rebase this stack by hand"))?;
    let prefix = format!("update {scratch} ");
    let new = out
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next());
    match (new, commits) {
        (Some(sha), _) => Ok(sha.to_string()),
        (None, 0) => Ok(onto.to_string()),
        // Never guess a tip: pushing `onto` here would drop the branch's commits.
        (None, _) => Err(format!("{branch}: git replay returned no result for {commits} commits")),
    }
}

/// Why this worktree can't be moved, if it can't.
fn worktree_blocker(wt: &Path) -> Option<String> {
    let dirty = run_git(wt, &["status", "--porcelain"]).map(|s| !s.trim().is_empty()).unwrap_or(true);
    if dirty {
        return Some("has uncommitted changes".into());
    }
    for marker in ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
        if let Ok(p) = run_git(wt, &["rev-parse", "--git-path", marker]) {
            let p = PathBuf::from(p.trim());
            let p = if p.is_absolute() { p } else { wt.join(p) };
            if p.exists() {
                return Some("has a rebase / merge / cherry-pick in progress".into());
            }
        }
    }
    None
}

/// Local branch + worktree facts, or the reason it blocks the rebase.
fn local_copy(
    repo: &Path,
    branch: &str,
    origin_sha: &str,
    worktrees: &[crate::worktree::WorktreeInfo],
) -> Result<LocalCopy, String> {
    let Some(local) = rev(repo, &format!("refs/heads/{branch}")) else {
        return Ok(LocalCopy::None);
    };
    // A local branch strictly behind origin is safe: the new tip comes from origin.
    if local != origin_sha && !is_ancestor(repo, &local, origin_sha) {
        return Err(format!("{branch}: has local commits that aren't on origin — push or drop them first"));
    }
    match worktrees.iter().find(|w| w.branch.as_deref() == Some(branch)) {
        None => Ok(LocalCopy::Branch { sha: local }),
        Some(w) => {
            if let Some(why) = worktree_blocker(Path::new(&w.path)) {
                return Err(format!("{branch}: worktree {} {why}", w.display_path));
            }
            Ok(LocalCopy::Worktree { path: w.path.clone(), sha: local })
        }
    }
}

/// Computes the rebase of `branches` (bottom first) onto `origin/<trunk>`
/// without touching anything. `Err` carries every reason the stack can't be
/// rebased locally right now.
pub fn plan(repo: &Path, trunk: &str, branches: &[String]) -> Result<Plan, Vec<String>> {
    let Some(trunk_sha) = rev(repo, &remote_ref(trunk)) else {
        return Err(vec![format!("origin/{trunk} not found — fetch first")]);
    };
    let mut olds = Vec::new();
    let mut missing = Vec::new();
    for b in branches {
        match rev(repo, &remote_ref(b)) {
            Some(sha) => olds.push(sha),
            None => missing.push(format!("origin/{b} not found — fetch first")),
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    if branches.is_empty() {
        return Err(vec!["the stack has no branches".into()]);
    }

    // Shape: each branch must already contain its parent's current tip.
    for k in 1..branches.len() {
        if !is_ancestor(repo, &olds[k - 1], &olds[k]) {
            return Err(vec![format!(
                "{} doesn't contain the tip of {} (it was amended or pushed to separately) — rebase this stack by hand",
                branches[k],
                branches[k - 1]
            )]);
        }
    }

    let worktrees = list_worktrees(repo).map_err(|e| vec![e.to_string()])?;
    let mut refusals = Vec::new();
    let mut locals = Vec::new();
    for (b, old) in branches.iter().zip(&olds) {
        match local_copy(repo, b, old, &worktrees) {
            Ok(l) => locals.push(l),
            Err(why) => {
                refusals.push(why);
                locals.push(LocalCopy::None);
            }
        }
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }

    let mut plans: Vec<BranchPlan> = Vec::new();
    for (k, b) in branches.iter().enumerate() {
        let (parent_old, parent_new) = if k == 0 {
            let Some(base) = run_git(repo, &["merge-base", &trunk_sha, &olds[0]]).ok().map(|s| s.trim().to_string())
            else {
                return Err(vec![format!("{b} shares no history with origin/{trunk}")]);
            };
            (base, trunk_sha.clone())
        } else {
            (olds[k - 1].clone(), plans[k - 1].new.clone())
        };
        let commits = count(repo, &format!("{parent_old}..{}", olds[k]));
        let new = if parent_old == parent_new {
            olds[k].clone()
        } else {
            replay(repo, &parent_new, &parent_old, b, commits).map_err(|e| vec![e])?
        };
        plans.push(BranchPlan { branch: b.clone(), old: olds[k].clone(), new, commits, local: locals[k].clone() });
    }
    Ok(Plan { trunk: trunk.to_string(), branches: plans })
}

/// Pushes every changed branch atomically (a lease per branch), then moves the
/// local copies. Nothing is pushed unless all leases hold.
pub fn apply(repo: &Path, plan: &Plan) -> AppResult<ApplyReport> {
    let changed: Vec<&BranchPlan> = plan.branches.iter().filter(|b| b.changed()).collect();
    if changed.is_empty() {
        return Ok(ApplyReport { pushed: vec![], moved_local: vec![], skipped_local: vec![] });
    }

    let leases: Vec<String> =
        changed.iter().map(|b| format!("--force-with-lease=refs/heads/{}:{}", b.branch, b.old)).collect();
    let specs: Vec<String> = changed.iter().map(|b| format!("{}:refs/heads/{}", b.new, b.branch)).collect();
    let mut args: Vec<&str> = vec!["push", "--atomic"];
    args.extend(leases.iter().map(String::as_str));
    args.push("origin");
    args.extend(specs.iter().map(String::as_str));
    run_network(repo, &args)?;

    let mut moved = Vec::new();
    let mut skipped = Vec::new();
    for b in &changed {
        let ok = match &b.local {
            LocalCopy::None => continue,
            LocalCopy::Branch { sha } => {
                run_git(repo, &["update-ref", &format!("refs/heads/{}", b.branch), &b.new, sha]).is_ok()
            }
            LocalCopy::Worktree { path, sha } => {
                let wt = Path::new(path);
                let still_there = rev(wt, "HEAD").as_deref() == Some(sha.as_str()) && worktree_blocker(wt).is_none();
                still_there && run_git(wt, &["reset", "--keep", &b.new]).is_ok()
            }
        };
        if ok { moved.push(b.branch.clone()) } else { skipped.push(b.branch.clone()) }
    }
    Ok(ApplyReport { pushed: changed.iter().map(|b| b.branch.clone()).collect(), moved_local: moved, skipped_local: skipped })
}

pub fn refusal_error(reasons: &[String]) -> AppError {
    AppError::Other(reasons.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str, body: &str) {
        fs::write(dir.join(file), body).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-qm", file]);
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        origin: PathBuf,
        clone: PathBuf,
    }

    /// origin with main; stack a -> b on top of it; a clone with no local branches checked out.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        let clone = tmp.path().join("clone");
        git(tmp.path(), &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        git(tmp.path(), &["clone", "-q", origin.to_str().unwrap(), work.to_str().unwrap()]);
        git(&work, &["checkout", "-qb", "main"]);
        commit(&work, "base", "1");
        git(&work, &["push", "-q", "origin", "main"]);
        git(&work, &["checkout", "-qb", "a"]);
        commit(&work, "a", "1");
        git(&work, &["checkout", "-qb", "b"]);
        commit(&work, "b", "1");
        git(&work, &["push", "-q", "origin", "a", "b"]);
        git(&work, &["checkout", "-q", "main"]);
        commit(&work, "m", "1");
        git(&work, &["push", "-q", "origin", "main"]);
        git(tmp.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        Fixture { _tmp: tmp, origin, clone }
    }

    fn stack() -> Vec<String> {
        vec!["a".into(), "b".into()]
    }

    #[test]
    fn rebases_whole_stack_and_pushes() {
        let f = fixture();
        let plan = plan(&f.clone, "main", &stack()).unwrap();
        assert!(!plan.is_noop());
        let report = apply(&f.clone, &plan).unwrap();
        assert_eq!(report.pushed, vec!["a", "b"]);
        let main = git(&f.origin, &["rev-parse", "main"]);
        assert_eq!(git(&f.origin, &["merge-base", "main", "b"]), main);
        assert_eq!(git(&f.origin, &["rev-parse", "b~1"]), git(&f.origin, &["rev-parse", "a"]));
        // A second run has nothing to do.
        git(&f.clone, &["fetch", "-q", "origin"]);
        assert!(super::plan(&f.clone, "main", &stack()).unwrap().is_noop());
    }

    #[test]
    fn moves_clean_worktree_and_unchecked_out_branch() {
        let f = fixture();
        git(&f.clone, &["branch", "a", "origin/a"]);
        let wt = f.clone.parent().unwrap().join("wt-b");
        git(&f.clone, &["worktree", "add", "-q", "-b", "b", wt.to_str().unwrap(), "origin/b"]);
        let plan = plan(&f.clone, "main", &stack()).unwrap();
        let report = apply(&f.clone, &plan).unwrap();
        assert_eq!(report.moved_local, vec!["a", "b"]);
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), git(&f.origin, &["rev-parse", "b"]));
        assert_eq!(git(&f.clone, &["rev-parse", "a"]), git(&f.origin, &["rev-parse", "a"]));
        assert!(git(&wt, &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn dirty_worktree_is_refused() {
        let f = fixture();
        let wt = f.clone.parent().unwrap().join("wt-b");
        git(&f.clone, &["worktree", "add", "-q", "-b", "b", wt.to_str().unwrap(), "origin/b"]);
        fs::write(wt.join("b"), "edited").unwrap();
        let reasons = plan(&f.clone, "main", &stack()).unwrap_err();
        assert!(reasons[0].contains("uncommitted"), "{reasons:?}");
    }

    #[test]
    fn unpushed_local_commits_are_refused() {
        let f = fixture();
        git(&f.clone, &["checkout", "-q", "-b", "a", "origin/a"]);
        commit(&f.clone, "extra", "1");
        git(&f.clone, &["checkout", "-q", "main"]);
        let reasons = plan(&f.clone, "main", &stack()).unwrap_err();
        assert!(reasons[0].contains("local commits"), "{reasons:?}");
    }

    #[test]
    fn conflict_changes_nothing() {
        let f = fixture();
        let work = f.clone.parent().unwrap().join("work");
        git(&work, &["checkout", "-q", "main"]);
        commit(&work, "a", "conflicting");
        git(&work, &["push", "-q", "origin", "main"]);
        git(&f.clone, &["fetch", "-q", "origin"]);
        let before = git(&f.origin, &["rev-parse", "a", "b"]);
        let reasons = plan(&f.clone, "main", &stack()).unwrap_err();
        assert!(reasons[0].contains("conflicts"), "{reasons:?}");
        assert_eq!(git(&f.origin, &["rev-parse", "a", "b"]), before);
    }

    #[test]
    fn lost_lease_pushes_nothing() {
        let f = fixture();
        let plan = plan(&f.clone, "main", &stack()).unwrap();
        // Someone pushes to `b` after we planned.
        let work = f.clone.parent().unwrap().join("work");
        git(&work, &["checkout", "-q", "b"]);
        commit(&work, "late", "1");
        git(&work, &["push", "-q", "origin", "b"]);
        let before = git(&f.origin, &["rev-parse", "a", "b"]);
        assert!(apply(&f.clone, &plan).is_err());
        assert_eq!(git(&f.origin, &["rev-parse", "a", "b"]), before);
    }
}
