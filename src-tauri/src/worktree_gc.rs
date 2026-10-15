//! Worktree janitor: classifies every linked worktree of every local repo and
//! removes / syncs the ones where that is provably safe. Facts are gathered
//! from git + gh, then `classify` (pure) decides; nothing is ever forced.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::git::run_git;
use crate::worktree::{self, WorktreeInfo, WorktreeSource};

const FETCH_EVERY: Duration = Duration::from_secs(5 * 60);
const RUN_EVERY: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrFact {
    pub number: i64,
    pub state: String,
    #[serde(rename = "headRefOid")]
    pub head_oid: String,
    #[serde(rename = "headRefName")]
    pub head_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Upstream {
    None,
    Gone,
    Tracking { ahead: u32, behind: u32, head_was_pushed: bool },
}

#[derive(Debug, Clone)]
pub struct Facts {
    pub missing: bool,
    pub is_main: bool,
    pub is_locked: bool,
    pub in_use: Option<String>,
    pub dirty: bool,
    pub branch: Option<String>,
    pub head: String,
    pub upstream: Upstream,
    pub pr: Option<PrFact>,
    /// Every commit on HEAD is in the PR's final head (ancestor or same patch).
    pub head_in_pr: bool,
    pub in_trunk: bool,
    pub on_remote: bool,
    pub patches_in_trunk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    /// `git worktree remove`; `delete_branch` when the branch tip is exactly the
    /// merged/closed PR's head.
    Remove { reason: String, delete_branch: bool },
    /// Folder already gone: `git worktree prune` drops the metadata.
    Prune { reason: String },
    /// Clean and only behind its upstream: `git merge --ff-only @{u}`.
    FastForward { reason: String },
    /// Upstream was rewritten (e.g. GitHub rebased the stack) and the local tip
    /// was pushed before: `git reset --keep @{u}` loses nothing.
    Resync { reason: String },
    Flag { reason: String },
    Keep { reason: String },
}

pub fn classify(f: &Facts) -> Verdict {
    use Verdict::*;
    let keep = |r: &str| Keep { reason: r.into() };
    let flag = |r: String| Flag { reason: r };

    if f.missing {
        return Prune { reason: "folder no longer exists".into() };
    }
    if f.is_main {
        return keep("main checkout");
    }
    if f.is_locked {
        return keep("locked");
    }
    if let Some(r) = &f.in_use {
        return Keep { reason: r.clone() };
    }
    if f.dirty {
        return flag("uncommitted or untracked changes".into());
    }

    let Some(branch) = &f.branch else {
        return if f.on_remote {
            Remove { reason: "detached; commit is on a remote branch".into(), delete_branch: false }
        } else if f.in_trunk || f.patches_in_trunk {
            Remove { reason: "detached; changes are already in trunk".into(), delete_branch: false }
        } else {
            flag("detached HEAD with commits not on any remote branch".into())
        };
    };

    let sync = match f.upstream {
        Upstream::Tracking { ahead: 0, behind, .. } if behind > 0 => Some(FastForward {
            reason: format!("{behind} behind origin/{branch}"),
        }),
        Upstream::Tracking { ahead, behind, head_was_pushed: true } if ahead > 0 && behind > 0 => {
            Some(Resync { reason: format!("origin/{branch} was rewritten; local tip was pushed before") })
        }
        _ => None,
    };
    let unpushed = match f.upstream {
        Upstream::Tracking { ahead, head_was_pushed: false, .. } if ahead > 0 => Some(ahead),
        _ => None,
    };

    match &f.pr {
        Some(pr) if pr.state != "OPEN" => {
            let state = pr.state.to_lowercase();
            let fully_pushed = matches!(f.upstream, Upstream::Tracking { ahead: 0, .. });
            if f.head == pr.head_oid || f.head_in_pr {
                Remove { reason: format!("PR #{} {state}", pr.number), delete_branch: true }
            } else if fully_pushed || f.in_trunk {
                Remove { reason: format!("PR #{} {state}; nothing unpushed", pr.number), delete_branch: false }
            } else {
                flag(format!("PR #{} {state}, but the branch has commits that aren't in it", pr.number))
            }
        }
        Some(pr) => match (sync, unpushed) {
            (Some(v), _) => v,
            (None, Some(n)) if matches!(f.upstream, Upstream::Tracking { behind, .. } if behind > 0) => {
                flag(format!("diverged from origin/{branch} with {n} unpushed commits"))
            }
            _ => Keep { reason: format!("PR #{} open", pr.number) },
        },
        None if f.in_trunk => {
            Remove { reason: "no PR; already in trunk".into(), delete_branch: false }
        }
        None => match (sync, f.upstream) {
            (Some(v), _) => v,
            (None, Upstream::Tracking { ahead: 0, .. }) => keep("pushed, no PR yet"),
            (None, Upstream::Tracking { ahead, .. }) => flag(format!("{ahead} unpushed commits, no PR")),
            (None, _) => flag("not pushed, no PR".into()),
        },
    }
}

// ---------------------------------------------------------------------------
// Fact gathering

pub struct RepoCtx {
    pub repo_id: i64,
    pub path: PathBuf,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub default_branch: Option<String>,
}

fn git_ok(dir: &Path, args: &[&str]) -> Option<String> {
    run_git(dir, args).ok().map(|s| s.trim().to_string())
}

fn trunk_of(repo: &RepoCtx) -> String {
    repo.default_branch
        .clone()
        .or_else(|| {
            git_ok(&repo.path, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
                .map(|s| s.trim_start_matches("origin/").to_string())
        })
        .unwrap_or_else(|| "main".into())
}

fn upstream_of(wt: &Path, branch: &str) -> Upstream {
    if git_ok(wt, &["config", &format!("branch.{branch}.merge")]).is_none() {
        return Upstream::None;
    }
    let Some(full) = git_ok(wt, &["rev-parse", "--symbolic-full-name", &format!("{branch}@{{u}}")])
    else {
        return Upstream::Gone;
    };
    let Some(counts) = git_ok(wt, &["rev-list", "--left-right", "--count", &format!("{branch}...{full}")])
    else {
        return Upstream::Gone;
    };
    let mut it = counts.split_whitespace().map(|n| n.parse::<u32>().unwrap_or(0));
    let (ahead, behind) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    let head = git_ok(wt, &["rev-parse", "HEAD"]).unwrap_or_default();
    let head_was_pushed = ahead > 0
        && git_ok(wt, &["reflog", "show", "--format=%H", &full])
            .is_some_and(|log| log.lines().any(|l| l == head));
    Upstream::Tracking { ahead, behind, head_was_pushed }
}

fn gather(repo: &RepoCtx, wt: &WorktreeInfo, trunk: &str, prs: &HashMap<String, PrFact>, in_use: &InUse) -> Facts {
    let path = Path::new(&wt.path);
    let missing = wt.is_prunable || !path.exists();
    let mut f = Facts {
        missing,
        is_main: wt.is_main,
        is_locked: wt.is_locked,
        in_use: None,
        dirty: false,
        branch: wt.branch.clone(),
        head: wt.head_sha.clone(),
        upstream: Upstream::None,
        pr: wt.branch.as_ref().and_then(|b| prs.get(b).cloned()),
        head_in_pr: false,
        in_trunk: false,
        on_remote: false,
        patches_in_trunk: false,
    };
    if missing || wt.is_main {
        return f;
    }
    f.in_use = in_use.reason_for(path);
    f.dirty = git_ok(path, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty());
    let trunk_ref = format!("refs/remotes/origin/{trunk}");
    f.in_trunk = run_git(&repo.path, &["merge-base", "--is-ancestor", &wt.head_sha, &trunk_ref]).is_ok();
    if let Some(pr) = f.pr.as_ref().filter(|p| p.state != "OPEN" && p.head_oid != wt.head_sha) {
        // The PR may have moved on GitHub (update-branch, a push from elsewhere);
        // GitHub keeps `refs/pull/N/head` after merge, so fetch it to compare.
        // Patch-equivalence (`git cherry`) also covers a PR rebased on GitHub.
        let contained = |repo_path: &Path| {
            run_git(repo_path, &["merge-base", "--is-ancestor", &wt.head_sha, &pr.head_oid]).is_ok()
                || git_ok(repo_path, &["cherry", &pr.head_oid, &wt.head_sha])
                    .is_some_and(|s| !s.lines().any(|l| l.starts_with('+')))
        };
        let have_pr_head = run_git(&repo.path, &["cat-file", "-e", &format!("{}^{{commit}}", pr.head_oid)]).is_ok()
            || run_git(&repo.path, &["fetch", "--quiet", "origin", &format!("refs/pull/{}/head", pr.number)]).is_ok();
        f.head_in_pr = have_pr_head && contained(&repo.path);
    }
    match &wt.branch {
        Some(b) => f.upstream = upstream_of(path, b),
        None if !f.in_trunk => {
            f.on_remote = git_ok(
                &repo.path,
                &["for-each-ref", "--count=1", "--contains", &wt.head_sha, "refs/remotes/"],
            )
            .is_some_and(|s| !s.is_empty());
            if !f.on_remote {
                f.patches_in_trunk = git_ok(&repo.path, &["cherry", &trunk_ref, &wt.head_sha])
                    .is_some_and(|s| !s.lines().any(|l| l.starts_with('+')));
            }
        }
        None => {}
    }
    f
}

/// PR facts for the given branches: one bulk `gh pr list`, then a per-branch
/// lookup for branches older than the bulk page. OPEN wins over closed ones.
fn pr_facts(repo: &RepoCtx, branches: &[String]) -> HashMap<String, PrFact> {
    let mut out: HashMap<String, PrFact> = HashMap::new();
    let (Some(owner), Some(name)) = (&repo.owner, &repo.name) else { return out };
    let slug = format!("{owner}/{name}");
    let ctx = crate::gh::GhRepo::Local(repo.path.clone());
    let fields = "number,state,headRefOid,headRefName";
    let parse = |s: String| serde_json::from_str::<Vec<PrFact>>(&s).unwrap_or_default();
    if let Ok(s) = crate::gh::run_gh(&ctx, &["pr", "list", "-R", &slug, "--state", "all", "--limit", "200", "--json", fields]) {
        absorb(&mut out, parse(s), branches);
    }
    for b in branches {
        if out.contains_key(b) {
            continue;
        }
        if let Ok(s) = crate::gh::run_gh(&ctx, &["pr", "list", "-R", &slug, "--head", b, "--state", "all", "--limit", "5", "--json", fields]) {
            absorb(&mut out, parse(s), branches);
        }
    }
    out
}

fn absorb(out: &mut HashMap<String, PrFact>, list: Vec<PrFact>, branches: &[String]) {
    for pr in list {
        let better = match out.get(&pr.head_ref) {
            None => true,
            Some(cur) => cur.state != "OPEN" && pr.state == "OPEN",
        };
        if better && branches.contains(&pr.head_ref) {
            out.insert(pr.head_ref.clone(), pr);
        }
    }
}

/// Paths something is actively using: live Claude sessions, tmux panes, and
/// review chats on local (non-PR) targets.
#[derive(Default)]
pub struct InUse {
    sessions: Vec<PathBuf>,
    panes: Vec<PathBuf>,
    chats: Vec<PathBuf>,
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

impl InUse {
    pub fn collect(chat_paths: Vec<String>) -> Self {
        let sessions = crate::claude_sessions::live_sessions()
            .into_iter()
            .map(|s| canon(Path::new(&s.cwd)))
            .collect();
        let panes = std::process::Command::new("tmux")
            .args(["list-panes", "-a", "-F", "#{pane_current_path}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| canon(Path::new(l)))
                    .collect()
            })
            .unwrap_or_default();
        let chats = chat_paths.iter().map(|p| canon(Path::new(p))).collect();
        Self { sessions, panes, chats }
    }

    fn reason_for(&self, wt: &Path) -> Option<String> {
        let wt = canon(wt);
        if self.sessions.iter().any(|p| p.starts_with(&wt)) {
            Some("live Claude session".into())
        } else if self.panes.iter().any(|p| p.starts_with(&wt)) {
            Some("open tmux pane".into())
        } else if self.chats.iter().any(|p| p == &wt) {
            Some("used by a review chat".into())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Run + report

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeRow {
    pub path: String,
    pub display_path: String,
    pub branch: Option<String>,
    pub head_sha: String,
    pub source: WorktreeSource,
    pub is_main: bool,
    pub pr: Option<PrFact>,
    pub upstream: Upstream,
    pub last_commit_at: Option<String>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoReport {
    pub repo_id: i64,
    pub path: String,
    pub slug: Option<String>,
    pub error: Option<String>,
    pub worktrees: Vec<WorktreeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GcAction {
    pub repo_id: i64,
    pub path: String,
    pub action: String,
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GcReport {
    pub ran_at: String,
    pub repos: Vec<RepoReport>,
    pub actions: Vec<GcAction>,
}

#[derive(Debug, Clone, Copy)]
pub struct GcOptions {
    pub auto_remove: bool,
    pub auto_sync: bool,
    pub force: bool,
}

struct State {
    last_run: Option<Instant>,
    last_report: Option<GcReport>,
    fetched: BTreeMap<PathBuf, Instant>,
}

static STATE: Mutex<State> = Mutex::new(State { last_run: None, last_report: None, fetched: BTreeMap::new() });
static RUNNING: Mutex<()> = Mutex::new(());

pub fn last_report() -> Option<GcReport> {
    STATE.lock().unwrap().last_report.clone()
}

fn should_fetch(path: &Path) -> bool {
    let mut st = STATE.lock().unwrap();
    let due = st.fetched.get(path).is_none_or(|t| t.elapsed() >= FETCH_EVERY);
    if due {
        st.fetched.insert(path.to_path_buf(), Instant::now());
    }
    due
}

/// Scans every repo, and applies the actions `opts` allows. Throttled to one
/// run per `RUN_EVERY` unless `force`; concurrent calls return the last report.
pub fn run(repos: &[RepoCtx], chat_paths: Vec<String>, opts: GcOptions) -> Option<GcReport> {
    let Ok(_guard) = RUNNING.try_lock() else { return last_report() };
    if !opts.force {
        let st = STATE.lock().unwrap();
        if st.last_run.is_some_and(|t| t.elapsed() < RUN_EVERY) {
            return st.last_report.clone();
        }
    }

    let in_use = InUse::collect(chat_paths);
    let mut reports = Vec::new();
    let mut actions = Vec::new();

    for repo in repos {
        let slug = repo.owner.as_ref().zip(repo.name.as_ref()).map(|(o, n)| format!("{o}/{n}"));
        let mut report = RepoReport {
            repo_id: repo.repo_id,
            path: repo.path.to_string_lossy().into_owned(),
            slug,
            error: None,
            worktrees: Vec::new(),
        };
        let wts = match worktree::list_worktrees(&repo.path) {
            Ok(w) => w,
            Err(e) => {
                report.error = Some(e.to_string());
                reports.push(report);
                continue;
            }
        };
        if wts.len() <= 1 {
            continue;
        }
        if should_fetch(&repo.path) {
            if let Err(e) = run_git(&repo.path, &["fetch", "--quiet", "--prune", "origin"]) {
                report.error = Some(format!("fetch failed: {e}"));
            }
        }
        let trunk = trunk_of(repo);
        let branches: Vec<String> = wts.iter().filter(|w| !w.is_main).filter_map(|w| w.branch.clone()).collect();
        let prs = pr_facts(repo, &branches);

        let mut pruned = false;
        for wt in &wts {
            let facts = gather(repo, wt, &trunk, &prs, &in_use);
            let verdict = classify(&facts);
            if let Some(a) = apply(repo, wt, &verdict, opts, &mut pruned) {
                actions.push(a);
            }
            let last_commit_at = (!facts.missing)
                .then(|| git_ok(Path::new(&wt.path), &["log", "-1", "--format=%cI"]))
                .flatten();
            report.worktrees.push(WorktreeRow {
                path: wt.path.clone(),
                display_path: wt.display_path.clone(),
                branch: wt.branch.clone(),
                head_sha: wt.head_sha.clone(),
                source: wt.source,
                is_main: wt.is_main,
                pr: facts.pr.clone(),
                upstream: facts.upstream,
                last_commit_at,
                verdict,
            });
        }
        reports.push(report);
    }

    let report = GcReport { ran_at: chrono::Utc::now().to_rfc3339(), repos: reports, actions };
    let mut st = STATE.lock().unwrap();
    st.last_run = Some(Instant::now());
    st.last_report = Some(report.clone());
    Some(report)
}

fn apply(repo: &RepoCtx, wt: &WorktreeInfo, v: &Verdict, opts: GcOptions, pruned: &mut bool) -> Option<GcAction> {
    let path = Path::new(&wt.path);
    let (action, result): (&str, Result<(), String>) = match v {
        Verdict::Prune { .. } if opts.auto_remove => {
            if *pruned {
                return None;
            }
            *pruned = true;
            ("prune", worktree::prune_worktrees(&repo.path).map(|_| ()).map_err(|e| e.to_string()))
        }
        Verdict::Remove { delete_branch, .. } if opts.auto_remove => {
            let r = worktree::remove_worktree(&repo.path, path).map_err(|e| e.to_string());
            let r = match (r, delete_branch, &wt.branch) {
                (Ok(()), true, Some(b)) => run_git(&repo.path, &["branch", "-D", b])
                    .map(|_| ())
                    .map_err(|e| format!("worktree removed, branch kept: {e}")),
                (r, _, _) => r,
            };
            (if *delete_branch { "remove + delete branch" } else { "remove" }, r)
        }
        Verdict::FastForward { .. } if opts.auto_sync => {
            ("fast-forward", run_git(path, &["merge", "--ff-only", "--quiet", "@{u}"]).map(|_| ()).map_err(|e| e.to_string()))
        }
        Verdict::Resync { .. } if opts.auto_sync => {
            ("resync", run_git(path, &["reset", "--keep", "--quiet", "@{u}"]).map(|_| ()).map_err(|e| e.to_string()))
        }
        _ => return None,
    };
    if let Err(e) = &result {
        crate::logging::warn(&format!("[worktree-gc] {action} {} failed: {e}", wt.path));
    }
    Some(GcAction {
        repo_id: repo.repo_id,
        path: wt.path.clone(),
        action: action.into(),
        ok: result.is_ok(),
        message: result.err(),
    })
}

/// Manual sync of one worktree: fast-forward if only behind, reset --keep if
/// the upstream was rewritten after the local tip was pushed; refuses otherwise.
pub fn sync_one(path: &Path) -> crate::error::AppResult<String> {
    use crate::error::AppError;
    let branch = worktree::current_branch(path).ok_or_else(|| AppError::Git("detached HEAD".into()))?;
    if git_ok(path, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty()) {
        return Err(AppError::Git("worktree has uncommitted or untracked changes".into()));
    }
    match upstream_of(path, &branch) {
        Upstream::Tracking { ahead: 0, behind: 0, .. } => Ok("already up to date".into()),
        Upstream::Tracking { ahead: 0, .. } => {
            run_git(path, &["merge", "--ff-only", "--quiet", "@{u}"])?;
            Ok("fast-forwarded".into())
        }
        Upstream::Tracking { head_was_pushed: true, behind, .. } if behind > 0 => {
            run_git(path, &["reset", "--keep", "--quiet", "@{u}"])?;
            Ok("reset to rewritten upstream".into())
        }
        Upstream::Tracking { .. } => Err(AppError::Git("branch has unpushed commits".into())),
        _ => Err(AppError::Git("branch has no upstream".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Facts {
        Facts {
            missing: false,
            is_main: false,
            is_locked: false,
            in_use: None,
            dirty: false,
            branch: Some("pbq/x".into()),
            head: "aaa".into(),
            upstream: Upstream::Tracking { ahead: 0, behind: 0, head_was_pushed: false },
            pr: None,
            head_in_pr: false,
            in_trunk: false,
            on_remote: false,
            patches_in_trunk: false,
        }
    }

    fn pr(state: &str, oid: &str) -> Option<PrFact> {
        Some(PrFact { number: 7, state: state.into(), head_oid: oid.into(), head_ref: "pbq/x".into() })
    }

    fn kind(v: &Verdict) -> &'static str {
        match v {
            Verdict::Remove { delete_branch: true, .. } => "remove+branch",
            Verdict::Remove { .. } => "remove",
            Verdict::Prune { .. } => "prune",
            Verdict::FastForward { .. } => "ff",
            Verdict::Resync { .. } => "resync",
            Verdict::Flag { .. } => "flag",
            Verdict::Keep { .. } => "keep",
        }
    }

    #[test]
    fn missing_folder_is_pruned() {
        assert_eq!(kind(&classify(&Facts { missing: true, dirty: true, ..base() })), "prune");
    }

    #[test]
    fn main_locked_and_in_use_are_kept() {
        assert_eq!(kind(&classify(&Facts { is_main: true, pr: pr("MERGED", "aaa"), ..base() })), "keep");
        assert_eq!(kind(&classify(&Facts { is_locked: true, pr: pr("MERGED", "aaa"), ..base() })), "keep");
        let f = Facts { in_use: Some("live Claude session".into()), pr: pr("MERGED", "aaa"), ..base() };
        assert_eq!(classify(&f), Verdict::Keep { reason: "live Claude session".into() });
    }

    #[test]
    fn dirty_is_flagged_even_when_merged() {
        assert_eq!(kind(&classify(&Facts { dirty: true, pr: pr("MERGED", "aaa"), ..base() })), "flag");
    }

    #[test]
    fn merged_with_matching_head_removes_worktree_and_branch() {
        let f = Facts { upstream: Upstream::Gone, pr: pr("MERGED", "aaa"), ..base() };
        assert_eq!(kind(&classify(&f)), "remove+branch");
    }

    #[test]
    fn squash_merged_with_extra_local_commits_is_flagged() {
        let f = Facts {
            upstream: Upstream::Gone,
            pr: pr("MERGED", "bbb"),
            ..base()
        };
        assert_eq!(kind(&classify(&f)), "flag");
    }

    #[test]
    fn merged_after_pr_moved_on_github_removes_when_tip_is_in_pr() {
        let f = Facts { upstream: Upstream::Gone, pr: pr("MERGED", "bbb"), head_in_pr: true, ..base() };
        assert_eq!(kind(&classify(&f)), "remove+branch");
    }

    #[test]
    fn closed_with_everything_pushed_removes_but_keeps_branch() {
        let f = Facts { pr: pr("CLOSED", "bbb"), ..base() };
        assert_eq!(kind(&classify(&f)), "remove");
    }

    #[test]
    fn open_pr_behind_fast_forwards() {
        let f = Facts {
            upstream: Upstream::Tracking { ahead: 0, behind: 3, head_was_pushed: false },
            pr: pr("OPEN", "zzz"),
            ..base()
        };
        assert_eq!(kind(&classify(&f)), "ff");
    }

    #[test]
    fn open_pr_rewritten_upstream_resyncs_when_tip_was_pushed() {
        let f = Facts {
            upstream: Upstream::Tracking { ahead: 2, behind: 5, head_was_pushed: true },
            pr: pr("OPEN", "zzz"),
            ..base()
        };
        assert_eq!(kind(&classify(&f)), "resync");
    }

    #[test]
    fn open_pr_diverged_with_unpushed_commits_is_flagged() {
        let f = Facts {
            upstream: Upstream::Tracking { ahead: 2, behind: 5, head_was_pushed: false },
            pr: pr("OPEN", "zzz"),
            ..base()
        };
        assert_eq!(kind(&classify(&f)), "flag");
    }

    #[test]
    fn open_pr_ahead_only_is_kept() {
        let f = Facts {
            upstream: Upstream::Tracking { ahead: 1, behind: 0, head_was_pushed: false },
            pr: pr("OPEN", "zzz"),
            ..base()
        };
        assert_eq!(kind(&classify(&f)), "keep");
    }

    #[test]
    fn no_pr_cases() {
        assert_eq!(kind(&classify(&Facts { in_trunk: true, upstream: Upstream::None, ..base() })), "remove");
        assert_eq!(kind(&classify(&base())), "keep");
        let unpushed = Facts { upstream: Upstream::Tracking { ahead: 1, behind: 0, head_was_pushed: false }, ..base() };
        assert_eq!(kind(&classify(&unpushed)), "flag");
        assert_eq!(kind(&classify(&Facts { upstream: Upstream::None, ..base() })), "flag");
    }

    #[test]
    fn detached_cases() {
        let d = Facts { branch: None, upstream: Upstream::None, ..base() };
        assert_eq!(kind(&classify(&Facts { on_remote: true, ..d.clone() })), "remove");
        assert_eq!(kind(&classify(&Facts { patches_in_trunk: true, ..d.clone() })), "remove");
        assert_eq!(kind(&classify(&d)), "flag");
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Bare `origin`, a clone `a` that pushed `feat`, and a clone `b` that
    /// rewrites `feat` on origin (what a GitHub stack rebase does).
    fn rewritten_upstream() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::TempDir::new().unwrap();
        let origin = root.path().join("origin.git");
        let a = root.path().join("a");
        let b = root.path().join("b");
        git(root.path(), &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
        git(root.path(), &["clone", "-q", origin.to_str().unwrap(), a.to_str().unwrap()]);
        git(&a, &["config", "user.email", "t@e.com"]);
        git(&a, &["config", "user.name", "T"]);
        std::fs::write(a.join("f"), "1").unwrap();
        git(&a, &["add", "."]);
        git(&a, &["commit", "-qm", "init"]);
        git(&a, &["push", "-q", "origin", "main"]);
        git(&a, &["checkout", "-qb", "feat"]);
        std::fs::write(a.join("g"), "1").unwrap();
        git(&a, &["add", "."]);
        git(&a, &["commit", "-qm", "feat"]);
        git(&a, &["push", "-qu", "origin", "feat"]);

        git(root.path(), &["clone", "-q", origin.to_str().unwrap(), b.to_str().unwrap()]);
        git(&b, &["config", "user.email", "t@e.com"]);
        git(&b, &["config", "user.name", "T"]);
        std::fs::write(b.join("h"), "1").unwrap();
        git(&b, &["add", "."]);
        git(&b, &["commit", "-qm", "main moved"]);
        git(&b, &["push", "-q", "origin", "main"]);
        git(&b, &["checkout", "-q", "feat"]);
        git(&b, &["rebase", "-q", "main"]);
        git(&b, &["push", "-qf", "origin", "feat"]);

        git(&a, &["fetch", "-q", "origin"]);
        (root, a)
    }

    #[test]
    fn rewritten_upstream_of_pushed_tip_is_detected_and_resynced() {
        let (_root, a) = rewritten_upstream();
        let up = upstream_of(&a, "feat");
        assert_eq!(up, Upstream::Tracking { ahead: 1, behind: 2, head_was_pushed: true });

        assert_eq!(sync_one(&a).unwrap(), "reset to rewritten upstream");
        assert_eq!(upstream_of(&a, "feat"), Upstream::Tracking { ahead: 0, behind: 0, head_was_pushed: false });
    }

    #[test]
    fn unpushed_local_commit_on_rewritten_upstream_is_not_resynced() {
        let (_root, a) = rewritten_upstream();
        std::fs::write(a.join("local"), "1").unwrap();
        git(&a, &["add", "."]);
        git(&a, &["commit", "-qm", "local only"]);

        let up = upstream_of(&a, "feat");
        assert!(matches!(up, Upstream::Tracking { head_was_pushed: false, .. }));
        assert!(sync_one(&a).is_err());
    }
}
