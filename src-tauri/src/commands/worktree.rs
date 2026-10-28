use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use tauri::{Emitter, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::mirror;
use crate::repo_sync;
use crate::sync::{MirrorUpdated, Scope, MIRROR_UPDATED_EVENT};
use crate::watch::Watchers;
use crate::worktree::{self, WorktreeInfo};
use crate::worktree_gc::{self, CacheEntry, GcOptions, GcReport, PrFact, RepoCache, RepoCtx, ScanHooks};

const SCAN_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(120);
const ROW_SYNC_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(60);

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

fn scan_inputs(conn: &Connection) -> AppResult<(Vec<RepoCtx>, Vec<String>)> {
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
    Ok((repos, chat_paths))
}

fn load_cache(conn: &Connection) -> AppResult<HashMap<String, CacheEntry>> {
    let mut stmt = conn.prepare("SELECT path, cache_key, facts_json FROM worktree_state")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(path, key, facts)| Some((path, CacheEntry { key, facts: serde_json::from_str(&facts).ok()? })))
        .collect())
}

/// Replaces the cache rows of every scanned repo and stores the report.
/// `prune_others` (full scan) also drops rows of repos that weren't scanned.
fn store_scan(conn: &mut Connection, caches: &[RepoCache], report: &GcReport, prune_others: bool) -> AppResult<()> {
    let tx = conn.transaction()?;
    let now = chrono::Utc::now().to_rfc3339();
    if prune_others {
        let ids: Vec<i64> = caches.iter().map(|c| c.repo_id).collect();
        let ph = vec!["?"; ids.len()].join(",");
        tx.execute(&format!("DELETE FROM worktree_state WHERE repo_id NOT IN ({ph})"), params_from_iter(&ids))?;
    }
    for c in caches {
        tx.execute("DELETE FROM worktree_state WHERE repo_id = ?1", params![c.repo_id])?;
        for (path, e) in &c.entries {
            tx.execute(
                "INSERT INTO worktree_state (path, repo_id, cache_key, facts_json, computed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path) DO UPDATE SET repo_id = excluded.repo_id, cache_key = excluded.cache_key,
                    facts_json = excluded.facts_json, computed_at = excluded.computed_at",
                params![path, c.repo_id, e.key, serde_json::to_string(&e.facts)?, now],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO worktree_gc_report (id, report, created_at) VALUES (1, ?1, datetime('now'))
         ON CONFLICT(id) DO UPDATE SET report = excluded.report, created_at = excluded.created_at",
        params![serde_json::to_string(report)?],
    )?;
    tx.commit()?;
    Ok(())
}

/// Fills each row's `review_id`: the review whose chat lives in the worktree,
/// else the latest review of the row's PR. Two queries for the whole report.
fn attach_reviews(conn: &Connection, report: &mut GcReport) -> AppResult<()> {
    let mut by_chat: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT worktree_path, MAX(review_id) FROM chat WHERE worktree_path IS NOT NULL GROUP BY worktree_path",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (path, id) = row?;
        by_chat.insert(path, id);
    }
    let mut by_pr: HashMap<(i64, i64), i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT t.repo_id, t.github_pr_number, MAX(rv.id) FROM review rv
         JOIN target t ON t.id = rv.target_id
         WHERE t.kind = 'github_pr' AND t.github_pr_number IS NOT NULL
         GROUP BY t.repo_id, t.github_pr_number",
    )?;
    for row in stmt.query_map([], |r| Ok(((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?), r.get::<_, i64>(2)?)))? {
        let (key, id) = row?;
        by_pr.insert(key, id);
    }
    for repo in &mut report.repos {
        for wt in &mut repo.worktrees {
            wt.review_id = by_chat
                .get(&wt.path)
                .or_else(|| wt.pr.as_ref().and_then(|pr| by_pr.get(&(repo.repo_id, pr.number))))
                .copied();
        }
    }
    Ok(())
}

fn with_reviews(db: &Db, report: Option<GcReport>) -> AppResult<Option<GcReport>> {
    let Some(mut report) = report else { return Ok(None) };
    attach_reviews(&db.0.lock().unwrap(), &mut report)?;
    Ok(Some(report))
}

fn load_last_report(conn: &Connection) -> AppResult<Option<GcReport>> {
    if let Some(report) = worktree_gc::last_report() {
        return Ok(Some(report));
    }
    let stored: Option<String> = conn
        .query_row("SELECT report FROM worktree_gc_report WHERE id = 1", [], |r| r.get(0))
        .optional()?;
    Ok(stored.and_then(|s| serde_json::from_str(&s).ok()))
}

struct MirrorPr {
    id: String,
    updated_at: Option<String>,
    fact: PrFact,
}

/// PR facts per branch from the mirror: an open PR wins over closed ones,
/// then the newest.
fn mirror_pr_facts(conn: &Connection, repo_id: i64, branches: &[String]) -> AppResult<HashMap<String, MirrorPr>> {
    if branches.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT node_id, gh_updated_at, head_ref, number, state, head_sha FROM gh_pr
         WHERE repo_id = ? AND head_ref IN ({})
         ORDER BY (state = 'open') DESC, number DESC",
        vec!["?"; branches.len()].join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let args: Vec<rusqlite::types::Value> =
        std::iter::once(repo_id.into()).chain(branches.iter().map(|b| b.clone().into())).collect();
    let rows = stmt.query_map(params_from_iter(args), |r| {
        Ok(MirrorPr {
            id: r.get(0)?,
            updated_at: r.get(1)?,
            fact: PrFact {
                head_ref: r.get(2)?,
                number: r.get(3)?,
                state: r.get::<_, Option<String>>(4)?.unwrap_or_default().to_uppercase(),
                head_oid: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            },
        })
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let pr = row?;
        out.entry(pr.fact.head_ref.clone()).or_insert(pr);
    }
    Ok(out)
}

/// The non-main branches of every repo that has linked worktrees.
fn worktree_branches(repos: &[RepoCtx]) -> Vec<(&RepoCtx, Vec<String>)> {
    repos
        .iter()
        .filter_map(|r| {
            let wts = worktree::list_worktrees(&r.path).ok().filter(|w| w.len() > 1)?;
            Some((r, wts.into_iter().filter(|w| !w.is_main).filter_map(|w| w.branch).collect()))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrLookup {
    /// Probe the open PRs (refetching changed ones) and look up every branch
    /// the mirror doesn't know.
    Scan,
    /// Mirror only, plus branches not looked up within `NO_PR_TTL`.
    Reload,
}

/// Branches a lookup found no PR for, so a Reload doesn't ask again each time.
static NO_PR: Mutex<Option<HashMap<(i64, String), Instant>>> = Mutex::new(None);
const NO_PR_TTL: Duration = Duration::from_secs(3600);
const LOOKUP_CONCURRENCY: usize = 4;

type RepoPrFacts = HashMap<i64, HashMap<String, PrFact>>;

/// PR facts for every repo's worktree branches, from the GitHub mirror, in a
/// few batched GitHub calls across all repos. Branches the mirror doesn't know
/// are looked up by head branch and upserted. Also returns the ids of mirrored
/// PRs that were refetched.
fn resolve_prs(db: &Db, wanted: &[(&RepoCtx, Vec<String>)], lookup: PrLookup) -> (RepoPrFacts, Vec<String>) {
    let read = || -> HashMap<i64, HashMap<String, MirrorPr>> {
        let conn = db.0.lock().unwrap();
        wanted
            .iter()
            .map(|(r, branches)| (r.repo_id, mirror_pr_facts(&conn, r.repo_id, branches).unwrap_or_default()))
            .collect()
    };
    let facts = |m: HashMap<i64, HashMap<String, MirrorPr>>| -> RepoPrFacts {
        m.into_iter().map(|(id, prs)| (id, prs.into_iter().map(|(b, pr)| (b, pr.fact)).collect())).collect()
    };
    let known = read();

    let mut nodes = Vec::new();
    let mut refreshed = Vec::new();
    if lookup == PrLookup::Scan {
        // `updatedAt` moves on a push, close or merge: what the janitor needs.
        let open: Vec<&MirrorPr> = known.values().flat_map(|m| m.values()).filter(|p| p.fact.state == "OPEN").collect();
        let ids: Vec<String> = open.iter().map(|p| p.id.clone()).collect();
        let probes = mirror::fetch::probe(&[], &ids);
        refreshed = open
            .iter()
            .filter(|p| match probes.get(&p.id) {
                Some(Some(probe)) => probe.normalized_state().as_deref() != Some("open") || probe.updated_at != p.updated_at,
                _ => false,
            })
            .map(|p| p.id.clone())
            .collect();
        if !refreshed.is_empty() {
            nodes.extend(mirror::fetch::refetch(&refreshed).into_values().flatten());
        }
    }

    let now = Instant::now();
    let jobs: Vec<(i64, String, String, Vec<String>)> = {
        let mut no_pr = NO_PR.lock().unwrap();
        let no_pr = no_pr.get_or_insert_with(HashMap::new);
        wanted
            .iter()
            .filter_map(|(r, branches)| {
                let (owner, name) = r.owner.clone().zip(r.name.clone())?;
                let known = known.get(&r.repo_id);
                let unknown: Vec<String> = branches
                    .iter()
                    .filter(|b| !known.is_some_and(|k| k.contains_key(*b)))
                    .filter(|b| {
                        lookup == PrLookup::Scan
                            || no_pr.get(&(r.repo_id, (*b).clone())).is_none_or(|at| now.duration_since(*at) >= NO_PR_TTL)
                    })
                    .cloned()
                    .collect();
                (!unknown.is_empty()).then_some((r.repo_id, owner, name, unknown))
            })
            .collect()
    };
    let looked_up = crate::inbox::concurrent_map(jobs, LOOKUP_CONCURRENCY, |(repo_id, owner, name, unknown)| {
        (*repo_id, unknown.clone(), mirror::fetch::prs_by_head(owner, name, unknown))
    });
    {
        let mut no_pr = NO_PR.lock().unwrap();
        let no_pr = no_pr.get_or_insert_with(HashMap::new);
        for (repo_id, unknown, (found, failed)) in looked_up {
            let found_refs: HashSet<&str> = found.iter().filter_map(|n| n.head_ref_name.as_deref()).collect();
            for b in unknown {
                if found_refs.contains(b.as_str()) {
                    no_pr.remove(&(repo_id, b));
                } else if !failed.contains(&b) {
                    no_pr.insert((repo_id, b), now);
                }
            }
            nodes.extend(found);
        }
    }

    if nodes.is_empty() {
        return (facts(known), refreshed);
    }
    let ts = chrono::Utc::now().to_rfc3339();
    let written = (|| -> AppResult<()> {
        let mut conn = db.0.lock().unwrap();
        let tx = conn.transaction()?;
        for node in &nodes {
            mirror::store::upsert_node(&tx, node, &ts)?;
        }
        tx.commit()?;
        Ok(())
    })();
    if let Err(e) = written {
        crate::logging::warn(&format!("[worktree-gc] mirror upsert failed: {e}"));
    }
    (facts(read()), refreshed)
}

fn emit_refreshed(app: &tauri::AppHandle, ids: Vec<String>) {
    if !ids.is_empty() {
        log::debug!(target: "mirror", "emit {MIRROR_UPDATED_EVENT} from a worktree scan: {} ids", ids.len());
        let _ = app.emit(MIRROR_UPDATED_EVENT, MirrorUpdated { scopes: vec![Scope::Inbox, Scope::Board], ids });
    }
}

/// Scans every repo with a local clone: fetch, refresh the PR facts, judge
/// (`remove`/`sync` = also apply those actions). Unchanged worktrees reuse
/// their cached git facts. Blocking. Also returns the ids of mirrored PRs it
/// refetched.
pub fn scan_all(db: &Db, watchers: &Watchers, remove: bool, sync: bool) -> AppResult<(GcReport, Vec<String>)> {
    let (repos, chat_paths, cache) = {
        let conn = db.0.lock().unwrap();
        let (repos, chat_paths) = scan_inputs(&conn)?;
        (repos, chat_paths, load_cache(&conn)?)
    };
    let (pr_facts, refreshed) = resolve_prs(db, &worktree_branches(&repos), PrLookup::Scan);
    let fetch = |repo: &RepoCtx| {
        repo_sync::fetch_repo(db, repo.repo_id, SCAN_FETCH_MIN_INTERVAL).map(|_| ()).map_err(|e| e.to_string())
    };
    let prs = |repo: &RepoCtx, _: &[String]| pr_facts.get(&repo.repo_id).cloned().unwrap_or_default();
    let epoch = |id: i64| watchers.epoch(id);
    let hooks = ScanHooks { fetch: Some(&fetch), prs: &prs, watch_epoch: &epoch };
    let out = worktree_gc::run(&repos, chat_paths, GcOptions { remove, sync, wait: true }, &cache, &hooks)?;
    store_scan(&mut db.0.lock().unwrap(), &out.caches, &out.report, true)?;
    worktree_gc::set_last_report(out.report.clone());
    Ok((out.report, refreshed))
}

/// Rescans only the repos that may have changed since their last scan (see
/// `worktree_gc::is_dirty`): no fetch, PR facts from the mirror, nothing
/// applied. Returns the merged report (None if there never was one and no
/// repo has worktrees). While a scan runs, returns the last report. Blocking.
pub fn reload_dirty(db: &Db, watchers: &Watchers) -> AppResult<Option<GcReport>> {
    let (repos, chat_paths, cache, prev) = {
        let conn = db.0.lock().unwrap();
        let (repos, chat_paths) = scan_inputs(&conn)?;
        (repos, chat_paths, load_cache(&conn)?, load_last_report(&conn)?)
    };
    if worktree_gc::is_running() {
        return Ok(prev);
    }
    let known: HashSet<i64> = repos.iter().map(|r| r.repo_id).collect();
    let dirty: Vec<RepoCtx> =
        repos.into_iter().filter(|r| worktree_gc::is_dirty(r.repo_id, watchers.epoch(r.repo_id))).collect();
    if dirty.is_empty() {
        return Ok(prev);
    }
    let scanned: HashSet<i64> = dirty.iter().map(|r| r.repo_id).collect();
    let (pr_facts, _) = resolve_prs(db, &worktree_branches(&dirty), PrLookup::Reload);
    let prs = |repo: &RepoCtx, _: &[String]| pr_facts.get(&repo.repo_id).cloned().unwrap_or_default();
    let epoch = |id: i64| watchers.epoch(id);
    let hooks = ScanHooks { fetch: None, prs: &prs, watch_epoch: &epoch };
    let opts = GcOptions { remove: false, sync: false, wait: false };
    let out = worktree_gc::run(&dirty, chat_paths, opts, &cache, &hooks)?;
    let report = worktree_gc::merge_reports(prev, out.report, &scanned, &known);
    store_scan(&mut db.0.lock().unwrap(), &out.caches, &report, false)?;
    worktree_gc::set_last_report(report.clone());
    Ok(Some(report))
}

#[tauri::command]
pub async fn worktree_gc_run(app: tauri::AppHandle, remove: bool, sync: bool) -> AppResult<GcReport> {
    let handle = app.clone();
    let (report, refreshed) = tauri::async_runtime::spawn_blocking(move || {
        scan_all(&handle.state::<Db>(), &handle.state::<Watchers>(), remove, sync)
    })
    .await
    .map_err(|e| AppError::Other(format!("worktree gc task failed: {e}")))??;
    emit_refreshed(&app, refreshed);
    Ok(with_reviews(&app.state::<Db>(), Some(report))?.expect("report was Some"))
}

#[tauri::command]
pub async fn worktree_gc_reload(app: tauri::AppHandle) -> AppResult<Option<GcReport>> {
    tauri::async_runtime::spawn_blocking(move || {
        let report = reload_dirty(&app.state::<Db>(), &app.state::<Watchers>())?;
        with_reviews(&app.state::<Db>(), report)
    })
    .await
    .map_err(|e| AppError::Other(format!("worktree reload task failed: {e}")))?
}

#[tauri::command]
pub fn worktree_gc_last(db: State<Db>) -> AppResult<Option<GcReport>> {
    let report = load_last_report(&db.0.lock().unwrap())?;
    with_reviews(&db, report)
}

/// Fetches (unless fresh), then fast-forwards (or resyncs after an upstream
/// rewrite) one registered worktree. A failed fetch still syncs to the
/// upstream already known locally, and says so.
#[tauri::command]
pub async fn worktree_sync(repo_id: i64, worktree_path: String, db: State<'_, Db>) -> AppResult<String> {
    let path = resolve_checkout(&db, repo_id, Some(worktree_path))?;
    let fetch = repo_sync::fetch_repo(&db, repo_id, ROW_SYNC_FETCH_MIN_INTERVAL);
    let outcome = worktree_gc::sync_one(&path)?;
    Ok(match fetch {
        Ok(_) => outcome,
        Err(e) => format!("{outcome} (fetch failed: {e})"),
    })
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
    let (path, repo, window_name) = terminal_target(&db, repo_id, Some(worktree_path))?;
    crate::terminal::open_shell(host, tmux_target, &tmux_session, &repo, &window_name, &path)
}

/// Opens a new `claude` session (tmux window when available) in a checkout of the repo.
#[tauri::command]
pub fn checkout_open_claude(
    repo_id: i64,
    worktree_path: Option<String>,
    host: crate::terminal::TerminalHost,
    tmux_target: crate::terminal::TmuxTarget,
    tmux_session: String,
    db: State<Db>,
) -> AppResult<()> {
    let (path, repo, window_name) = terminal_target(&db, repo_id, worktree_path)?;
    crate::terminal::open_session(
        host,
        tmux_target,
        &tmux_session,
        &repo,
        &window_name,
        &path,
        &["claude".to_string()],
    )
}

/// `(cwd, repo label, window name)` for a terminal opened in one of the repo's checkouts.
fn terminal_target(
    db: &Db,
    repo_id: i64,
    worktree_path: Option<String>,
) -> AppResult<(String, String, String)> {
    let path = resolve_checkout(db, repo_id, worktree_path)?;
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
        _ => repo_local_path(db, repo_id)?,
    };
    let window_name = worktree::current_branch(&path).unwrap_or_else(|| {
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    });
    Ok((path.to_string_lossy().into_owned(), repo, window_name))
}

#[tauri::command]
pub fn open_in_vscode(path: String, file: Option<String>, line: Option<u32>) -> AppResult<()> {
    let mut cmd = Command::new("code");
    cmd.arg(&path);
    if let Some(file) = file {
        let target = Path::new(&path).join(file);
        match line {
            Some(line) => {
                cmd.arg("--goto").arg(format!("{}:{line}", target.display()));
            }
            None => {
                cmd.arg(target);
            }
        }
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
    fn mirror_pr_facts_prefer_open_then_newest() {
        let conn = crate::db::open_memory();
        let repo = mirror::store::ensure_repo(&conn, "acme", "widget", "now").unwrap();
        for (id, n, head, state) in
            [("A", 1, "feat", "merged"), ("B", 2, "feat", "open"), ("C", 3, "feat", "closed"), ("D", 4, "old", "closed"), ("E", 5, "old", "merged")]
        {
            conn.execute(
                "INSERT INTO gh_pr (node_id, repo_id, number, title, url, state, head_ref, head_sha, fetched_at)
                 VALUES (?1, ?2, ?3, 't', 'u', ?4, ?5, 'sha', 'now')",
                params![id, repo, n, state, head],
            )
            .unwrap();
        }
        let branches = ["feat", "old", "none"].map(String::from);
        let facts = mirror_pr_facts(&conn, repo, &branches).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts["feat"].id, "B");
        assert_eq!(facts["feat"].fact.state, "OPEN");
        assert_eq!(facts["old"].fact.number, 5);
    }

    #[test]
    fn scan_cache_round_trips_and_replaces_per_repo() {
        let mut conn = crate::db::open_memory();
        for id in [1, 2] {
            conn.execute("INSERT INTO repository (id, local_path, added_at) VALUES (?1, ?2, 'now')", params![id, format!("/r{id}")])
                .unwrap();
        }
        let entry = |key: &str| CacheEntry { key: key.into(), facts: Default::default() };
        let report = GcReport { ran_at: "t".into(), repos: vec![], actions: vec![] };
        let caches = [
            RepoCache { repo_id: 1, entries: vec![("/r1/a".into(), entry("k1")), ("/r1/b".into(), entry("k2"))] },
            RepoCache { repo_id: 2, entries: vec![("/r2/a".into(), entry("k3"))] },
        ];
        store_scan(&mut conn, &caches, &report, true).unwrap();
        assert_eq!(load_cache(&conn).unwrap().len(), 3);

        store_scan(&mut conn, &[RepoCache { repo_id: 1, entries: vec![("/r1/a".into(), entry("k4"))] }], &report, false).unwrap();
        let cache = load_cache(&conn).unwrap();
        assert_eq!(cache.len(), 2, "/r1/b is gone, repo 2 untouched");
        assert_eq!(cache["/r1/a"].key, "k4");

        store_scan(&mut conn, &[RepoCache { repo_id: 1, entries: vec![] }], &report, true).unwrap();
        assert!(load_cache(&conn).unwrap().is_empty(), "a full scan drops repos it didn't scan");
    }

    #[test]
    fn match_worktree_rejects_unknown_path() {
        let err = match_worktree(&[wt("/repo")], "/etc").unwrap_err();
        assert_eq!(err.to_string(), "unknown worktree path");
    }
}
