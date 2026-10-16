//! Worktree janitor: classifies every linked worktree of every local repo and
//! removes / syncs the ones where that is provably safe. Facts are gathered
//! from git + the GitHub mirror, then `classify` (pure) decides; nothing is
//! ever forced. The expensive git facts are cached per worktree under a key
//! read from one `for-each-ref` per repo, so a rescan of an unchanged repo
//! costs a couple of spawns plus one `git status` per worktree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::git::run_git;
use crate::worktree::{self, WorktreeInfo, WorktreeSource};

/// Bump when what `CachedFacts` means (or how it is computed) changes.
const CACHE_VERSION: u32 = 1;

/// GitHub-cased state (`OPEN` / `CLOSED` / `MERGED`), as `gh pr list` reported it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrFact {
    pub number: i64,
    pub state: String,
    #[serde(rename = "headRefOid")]
    pub head_oid: String,
    #[serde(rename = "headRefName")]
    pub head_ref: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Upstream {
    #[default]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// The facts that cost git spawns, reused while the worktree's cache key is
/// unchanged. Everything else (dirty, in use, missing) is rechecked each scan.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedFacts {
    pub upstream: Upstream,
    pub head_in_pr: bool,
    pub in_trunk: bool,
    pub on_remote: bool,
    pub patches_in_trunk: bool,
    pub last_commit_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub key: String,
    pub facts: CachedFacts,
}

/// Every local branch and remote-tracking ref of a repo, from one `for-each-ref`.
#[derive(Debug, Default)]
pub struct RefSnapshot {
    shas: HashMap<String, String>,
    /// Branch name → configured upstream refname (present even when gone).
    upstreams: HashMap<String, String>,
    /// Over every remote-tracking ref: "is this commit on a remote" can change
    /// with any of them.
    remotes_digest: u64,
}

impl RefSnapshot {
    pub fn read(repo: &Path) -> crate::error::AppResult<Self> {
        let out = run_git(
            repo,
            &["for-each-ref", "--format=%(refname)%09%(objectname)%09%(upstream)", "refs/heads", "refs/remotes"],
        )?;
        Ok(Self::parse(&out))
    }

    fn parse(out: &str) -> Self {
        let mut s = Self::default();
        // FNV-1a: stable across builds, since keys are persisted.
        let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
        for line in out.lines() {
            let mut it = line.split('\t');
            let (Some(name), Some(sha)) = (it.next(), it.next()) else { continue };
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                if let Some(up) = it.next().filter(|u| !u.is_empty()) {
                    s.upstreams.insert(branch.to_string(), up.to_string());
                }
            } else {
                for b in line.bytes().chain([b'\n']) {
                    digest = (digest ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
                }
            }
            s.shas.insert(name.to_string(), sha.to_string());
        }
        s.remotes_digest = digest;
        s
    }
}

/// Everything the cached facts depend on.
fn cache_key(wt: &WorktreeInfo, trunk_ref: &str, refs: &RefSnapshot, pr: Option<&PrFact>) -> String {
    let upstream = wt.branch.as_ref().and_then(|b| refs.upstreams.get(b)).map(|up| (up, refs.shas.get(up)));
    serde_json::json!({
        "v": CACHE_VERSION,
        "head": wt.head_sha,
        "branch": wt.branch,
        "upstream": upstream,
        "trunk": refs.shas.get(trunk_ref),
        "pr": pr,
        "remotes": wt.branch.is_none().then_some(refs.remotes_digest),
    })
    .to_string()
}

fn compute_facts(repo: &RepoCtx, wt: &WorktreeInfo, trunk_ref: &str, pr: Option<&PrFact>) -> CachedFacts {
    let path = Path::new(&wt.path);
    let mut f = CachedFacts {
        in_trunk: run_git(&repo.path, &["merge-base", "--is-ancestor", &wt.head_sha, trunk_ref]).is_ok(),
        last_commit_at: git_ok(path, &["log", "-1", "--format=%cI"]),
        ..Default::default()
    };
    if let Some(pr) = pr.filter(|p| p.state != "OPEN" && p.head_oid != wt.head_sha) {
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
                f.patches_in_trunk = git_ok(&repo.path, &["cherry", trunk_ref, &wt.head_sha])
                    .is_some_and(|s| !s.lines().any(|l| l.starts_with('+')));
            }
        }
        None => {}
    }
    f
}

struct Gathered {
    facts: Facts,
    last_commit_at: Option<String>,
    /// None for the main checkout and missing folders (nothing to cache).
    entry: Option<CacheEntry>,
    hit: bool,
}

fn gather(
    repo: &RepoCtx,
    wt: &WorktreeInfo,
    trunk_ref: &str,
    refs: &RefSnapshot,
    prs: &HashMap<String, PrFact>,
    in_use: &InUse,
    cached: Option<&CacheEntry>,
) -> Gathered {
    let path = Path::new(&wt.path);
    let missing = wt.is_prunable || !path.exists();
    let pr = wt.branch.as_ref().and_then(|b| prs.get(b).cloned());
    let mut f = Facts {
        missing,
        is_main: wt.is_main,
        is_locked: wt.is_locked,
        in_use: None,
        dirty: false,
        branch: wt.branch.clone(),
        head: wt.head_sha.clone(),
        upstream: Upstream::None,
        pr,
        head_in_pr: false,
        in_trunk: false,
        on_remote: false,
        patches_in_trunk: false,
    };
    if missing || wt.is_main {
        return Gathered { facts: f, last_commit_at: None, entry: None, hit: false };
    }
    f.in_use = in_use.reason_for(path);
    f.dirty = git_ok(path, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty());
    let key = cache_key(wt, trunk_ref, refs, f.pr.as_ref());
    let (cf, hit) = match cached.filter(|c| c.key == key) {
        Some(c) => (c.facts.clone(), true),
        None => (compute_facts(repo, wt, trunk_ref, f.pr.as_ref()), false),
    };
    f.upstream = cf.upstream;
    f.head_in_pr = cf.head_in_pr;
    f.in_trunk = cf.in_trunk;
    f.on_remote = cf.on_remote;
    f.patches_in_trunk = cf.patches_in_trunk;
    Gathered { facts: f, last_commit_at: cf.last_commit_at.clone(), entry: Some(CacheEntry { key, facts: cf }), hit }
}

// ---------------------------------------------------------------------------
// Dirty tracking

/// Which repos need a rescan. A repo is clean only when it was scanned while
/// the same watcher was running (so no change went unseen) and the watcher
/// reported nothing since. Process-local: everything is dirty after a restart.
#[derive(Default)]
struct DirtyTracker {
    changes: HashMap<i64, u64>,
    /// repo → (watcher epoch, change count) at the start of its last scan.
    scanned: HashMap<i64, (u64, u64)>,
}

impl DirtyTracker {
    fn note_change(&mut self, repo_id: i64) {
        *self.changes.entry(repo_id).or_default() += 1;
    }

    fn mark(&self, repo_id: i64, epoch: Option<u64>) -> Option<(u64, u64)> {
        epoch.map(|e| (e, self.changes.get(&repo_id).copied().unwrap_or(0)))
    }

    fn finish(&mut self, repo_id: i64, mark: Option<(u64, u64)>) {
        match mark {
            Some(m) => self.scanned.insert(repo_id, m),
            None => self.scanned.remove(&repo_id),
        };
    }

    fn is_dirty(&self, repo_id: i64, epoch: Option<u64>) -> bool {
        let changes = self.changes.get(&repo_id).copied().unwrap_or(0);
        match (self.scanned.get(&repo_id), epoch) {
            (Some(&(e, c)), Some(cur)) => e != cur || c != changes,
            _ => true,
        }
    }
}

static DIRTY: Mutex<Option<DirtyTracker>> = Mutex::new(None);

fn with_dirty<R>(f: impl FnOnce(&mut DirtyTracker) -> R) -> R {
    f(DIRTY.lock().unwrap().get_or_insert_with(DirtyTracker::default))
}

/// Called for every watcher event of the repo.
pub fn note_change(repo_id: i64) {
    with_dirty(|d| d.note_change(repo_id));
}

pub fn is_dirty(repo_id: i64, watch_epoch: Option<u64>) -> bool {
    with_dirty(|d| d.is_dirty(repo_id, watch_epoch))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoReport {
    pub repo_id: i64,
    pub path: String,
    pub slug: Option<String>,
    pub error: Option<String>,
    pub worktrees: Vec<WorktreeRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcAction {
    pub repo_id: i64,
    pub path: String,
    pub action: String,
    pub ok: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcReport {
    pub ran_at: String,
    pub repos: Vec<RepoReport>,
    pub actions: Vec<GcAction>,
}

#[derive(Debug, Clone, Copy)]
pub struct GcOptions {
    pub remove: bool,
    pub sync: bool,
    /// Wait for a running scan instead of refusing.
    pub wait: bool,
}

pub type FetchHook<'a> = &'a dyn Fn(&RepoCtx) -> Result<(), String>;
pub type PrsHook<'a> = &'a dyn Fn(&RepoCtx, &[String]) -> HashMap<String, PrFact>;

/// What `run` needs from outside (DB, mirror, network, watchers).
pub struct ScanHooks<'a> {
    /// Brings a repo's `origin/*` up to date first. None judges the local refs
    /// as they are (Reload).
    pub fetch: Option<FetchHook<'a>>,
    /// PR facts for a repo's branches.
    pub prs: PrsHook<'a>,
    pub watch_epoch: &'a dyn Fn(i64) -> Option<u64>,
}

/// The new cache rows of one scanned repo (replacing all of its old ones).
pub struct RepoCache {
    pub repo_id: i64,
    pub entries: Vec<(String, CacheEntry)>,
}

pub struct ScanOutput {
    pub report: GcReport,
    pub caches: Vec<RepoCache>,
}

static LAST_REPORT: Mutex<Option<GcReport>> = Mutex::new(None);
static RUNNING: Mutex<()> = Mutex::new(());

pub fn last_report() -> Option<GcReport> {
    LAST_REPORT.lock().unwrap().clone()
}

pub fn set_last_report(report: GcReport) {
    *LAST_REPORT.lock().unwrap() = Some(report);
}

pub fn is_running() -> bool {
    RUNNING.try_lock().is_err()
}

/// A partial scan's report: `fresh` for the repos it scanned, `prev` for the
/// others (dropping repos that no longer exist), in repo path order. The last
/// cleanup log survives a scan that applied nothing.
pub fn merge_reports(prev: Option<GcReport>, fresh: GcReport, scanned: &HashSet<i64>, known: &HashSet<i64>) -> GcReport {
    let Some(prev) = prev else { return fresh };
    let mut repos: Vec<RepoReport> = prev
        .repos
        .into_iter()
        .filter(|r| !scanned.contains(&r.repo_id) && known.contains(&r.repo_id))
        .chain(fresh.repos)
        .collect();
    repos.sort_by(|a, b| a.path.cmp(&b.path));
    let actions = if fresh.actions.is_empty() { prev.actions } else { fresh.actions };
    GcReport { ran_at: fresh.ran_at, repos, actions }
}

/// Scans `repos` and applies the actions `opts` allows; one scan at a time.
/// Worktrees whose cache key matches `cache` reuse their cached facts.
pub fn run(
    repos: &[RepoCtx],
    chat_paths: Vec<String>,
    opts: GcOptions,
    cache: &HashMap<String, CacheEntry>,
    hooks: &ScanHooks,
) -> crate::error::AppResult<ScanOutput> {
    let _guard = match RUNNING.try_lock() {
        Ok(g) => g,
        Err(_) if opts.wait => RUNNING.lock().unwrap(),
        Err(_) => return Err(crate::error::AppError::Other("a worktree scan is already running".into())),
    };

    let started = Instant::now();
    let in_use = InUse::collect(chat_paths);
    let mut reports = Vec::new();
    let mut actions = Vec::new();
    let mut caches = Vec::new();
    let (mut hits, mut misses) = (0usize, 0usize);

    for repo in repos {
        let mark = with_dirty(|d| d.mark(repo.repo_id, (hooks.watch_epoch)(repo.repo_id)));
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
            caches.push(RepoCache { repo_id: repo.repo_id, entries: Vec::new() });
            with_dirty(|d| d.finish(repo.repo_id, mark));
            continue;
        }
        if let Some(fetch) = hooks.fetch {
            if let Err(e) = fetch(repo) {
                report.error = Some(format!("fetch failed: {e}"));
            }
        }
        let trunk_ref = format!("refs/remotes/origin/{}", trunk_of(repo));
        let refs = RefSnapshot::read(&repo.path).unwrap_or_else(|e| {
            crate::logging::warn(&format!("[worktree-gc] for-each-ref in {}: {e}", repo.path.display()));
            RefSnapshot::default()
        });
        let branches: Vec<String> = wts.iter().filter(|w| !w.is_main).filter_map(|w| w.branch.clone()).collect();
        let prs = (hooks.prs)(repo, &branches);

        let mut pruned = false;
        let mut entries = Vec::new();
        for wt in &wts {
            let g = gather(repo, wt, &trunk_ref, &refs, &prs, &in_use, cache.get(&wt.path));
            let verdict = classify(&g.facts);
            let action = apply(repo, wt, &verdict, opts, &mut pruned);
            let removed = action.as_ref().is_some_and(|a| a.ok && (a.action.starts_with("remove") || a.action == "prune"));
            if let Some(entry) = g.entry.filter(|_| !removed) {
                if g.hit { hits += 1 } else { misses += 1 }
                entries.push((wt.path.clone(), entry));
            }
            actions.extend(action);
            let facts = g.facts;
            let last_commit_at = g.last_commit_at;
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
        caches.push(RepoCache { repo_id: repo.repo_id, entries });
        with_dirty(|d| d.finish(repo.repo_id, mark));
        reports.push(report);
    }

    log::info!(
        "[worktree-gc] scanned {} repos: {hits} worktrees cached, {misses} computed, {} ms",
        repos.len(),
        started.elapsed().as_millis()
    );
    let report = GcReport { ran_at: chrono::Utc::now().to_rfc3339(), repos: reports, actions };
    Ok(ScanOutput { report, caches })
}

fn apply(repo: &RepoCtx, wt: &WorktreeInfo, v: &Verdict, opts: GcOptions, pruned: &mut bool) -> Option<GcAction> {
    let path = Path::new(&wt.path);
    let (action, result): (&str, Result<(), String>) = match v {
        Verdict::Prune { .. } if opts.remove => {
            if *pruned {
                return None;
            }
            *pruned = true;
            ("prune", worktree::prune_worktrees(&repo.path).map(|_| ()).map_err(|e| e.to_string()))
        }
        Verdict::Remove { delete_branch, .. } if opts.remove => {
            let r = worktree::remove_worktree(&repo.path, path).map_err(|e| e.to_string());
            let r = match (r, delete_branch, &wt.branch) {
                (Ok(()), true, Some(b)) => run_git(&repo.path, &["branch", "-D", b])
                    .map(|_| ())
                    .map_err(|e| format!("worktree removed, branch kept: {e}")),
                (r, _, _) => r,
            };
            (if *delete_branch { "remove + delete branch" } else { "remove" }, r)
        }
        Verdict::FastForward { .. } if opts.sync => {
            ("fast-forward", run_git(path, &["merge", "--ff-only", "--quiet", "@{u}"]).map(|_| ()).map_err(|e| e.to_string()))
        }
        Verdict::Resync { .. } if opts.sync => {
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
    fn ref_snapshot_reads_upstreams_and_digests_only_remote_refs() {
        let out = "refs/heads/feat\taaa\trefs/remotes/origin/feat\nrefs/heads/local\tbbb\t\nrefs/remotes/origin/feat\tccc\t\n";
        let s = RefSnapshot::parse(out);
        assert_eq!(s.upstreams.get("feat").map(String::as_str), Some("refs/remotes/origin/feat"));
        assert!(!s.upstreams.contains_key("local"));
        assert_eq!(s.shas.get("refs/remotes/origin/feat").map(String::as_str), Some("ccc"));
        let local_moved = RefSnapshot::parse(&out.replace("bbb", "ddd"));
        assert_eq!(local_moved.remotes_digest, s.remotes_digest);
        let remote_moved = RefSnapshot::parse(&out.replace("ccc", "eee"));
        assert_ne!(remote_moved.remotes_digest, s.remotes_digest);
    }

    #[test]
    fn dirty_until_scanned_under_the_same_watcher_with_no_change_since() {
        let mut d = DirtyTracker::default();
        assert!(d.is_dirty(1, Some(5)), "never scanned");
        let m = d.mark(1, Some(5));
        d.finish(1, m);
        assert!(!d.is_dirty(1, Some(5)));
        assert!(d.is_dirty(1, Some(6)), "watcher restarted: changes may have been missed");
        assert!(d.is_dirty(1, None), "not watched");
        d.note_change(1);
        assert!(d.is_dirty(1, Some(5)));

        let m = d.mark(1, Some(5));
        d.note_change(1);
        d.finish(1, m);
        assert!(d.is_dirty(1, Some(5)), "changed during the scan");

        let m = d.mark(2, None);
        d.finish(2, m);
        assert!(d.is_dirty(2, None), "scanned without a watcher");
    }

    fn report(repos: &[(i64, &str)], actions: usize) -> GcReport {
        GcReport {
            ran_at: "t".into(),
            repos: repos
                .iter()
                .map(|(id, path)| RepoReport { repo_id: *id, path: path.to_string(), slug: None, error: None, worktrees: vec![] })
                .collect(),
            actions: (0..actions)
                .map(|_| GcAction { repo_id: 1, path: "p".into(), action: "remove".into(), ok: true, message: None })
                .collect(),
        }
    }

    #[test]
    fn merge_replaces_scanned_repos_and_keeps_the_others() {
        let prev = report(&[(1, "/a"), (2, "/b"), (3, "/c")], 1);
        let fresh = GcReport { ran_at: "t2".into(), ..report(&[(2, "/b")], 0) };
        let scanned = HashSet::from([2, 4]);
        let known = HashSet::from([1, 2, 4]);
        let merged = merge_reports(Some(prev), fresh, &scanned, &known);
        let ids: Vec<i64> = merged.repos.iter().map(|r| r.repo_id).collect();
        assert_eq!(ids, [1, 2], "3 no longer exists; 4 has no worktrees any more");
        assert_eq!(merged.actions.len(), 1, "last cleanup log kept");
        assert_eq!(merged.ran_at, "t2");
    }

    #[test]
    fn unchanged_worktree_reuses_cached_facts_until_its_key_moves() {
        let (_root, a) = rewritten_upstream();
        git(&a, &["checkout", "-q", "main"]);
        let wt_path = a.parent().unwrap().join("wt");
        git(&a, &["worktree", "add", "-q", wt_path.to_str().unwrap(), "feat"]);
        let repo = RepoCtx { repo_id: 1, path: a.clone(), owner: None, name: None, default_branch: Some("main".into()) };
        let trunk_ref = "refs/remotes/origin/main";
        let wt = || worktree::list_worktrees(&a).unwrap().into_iter().find(|w| !w.is_main).unwrap();
        let gather_with = |cached: Option<&CacheEntry>| {
            let refs = RefSnapshot::read(&a).unwrap();
            gather(&repo, &wt(), trunk_ref, &refs, &HashMap::new(), &InUse::default(), cached)
        };

        let first = gather_with(None);
        assert!(!first.hit);
        let entry = first.entry.unwrap();
        assert_eq!(entry.facts.upstream, Upstream::Tracking { ahead: 1, behind: 2, head_was_pushed: true });
        assert!(entry.facts.last_commit_at.is_some());

        let second = gather_with(Some(&entry));
        assert!(second.hit);
        assert_eq!(second.facts.upstream, first.facts.upstream);

        let stale = CacheEntry { key: entry.key.clone(), facts: CachedFacts { in_trunk: true, ..entry.facts.clone() } };
        std::fs::write(wt_path.join("x"), "1").unwrap();
        let dirty = gather_with(Some(&stale));
        assert!(dirty.hit && dirty.facts.dirty, "dirtiness is rechecked even on a cache hit");
        git(&wt_path, &["add", "."]);
        git(&wt_path, &["commit", "-qm", "more"]);
        let moved = gather_with(Some(&stale));
        assert!(!moved.hit, "a new head invalidates the cache");
        assert!(!moved.facts.in_trunk);

        let pr = HashMap::from([("feat".to_string(), PrFact {
            number: 7,
            state: "MERGED".into(),
            head_oid: "x".into(),
            head_ref: "feat".into(),
        })]);
        let refs = RefSnapshot::read(&a).unwrap();
        let fresh = moved.entry.unwrap();
        let with_pr = gather(&repo, &wt(), trunk_ref, &refs, &pr, &InUse::default(), Some(&fresh));
        assert!(!with_pr.hit, "new PR facts invalidate the cache");
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
