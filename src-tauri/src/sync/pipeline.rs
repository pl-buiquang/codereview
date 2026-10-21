//! The sync pipeline: discovery → probe → refetch what changed → facets → GC.
//! Network I/O runs without the DB lock; writes happen in short transactions.
//! Fetches go through `inflight`, so overlapping syncs never fetch the same
//! (facet, id) twice at once.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rusqlite::{params_from_iter, Connection};

use super::{inflight, MirrorUpdated, Mode, Scope, SyncReport, SyncRequest};
use crate::commands::{board as board_cmd, inbox as inbox_cmd};
use crate::db::Db;
use crate::error::AppResult;
use crate::inbox::{self, ViewerInfo};
use crate::mirror::{self, fetch::Probe, Node};

const AUTHORED_SEARCH: &str = "is:pr is:open author:@me";
const AUTHORED_SEARCH_LIMIT: i64 = 50;
const THREADS_CONCURRENCY: usize = 8;
const FETCH_CONCURRENCY: usize = 4;
/// Resolving or unresolving a thread does not bump the PR's `updatedAt`, so an
/// unchanged PR still gets its threads refetched once this old.
pub const THREADS_MAX_AGE: chrono::Duration = chrono::Duration::minutes(15);
const FULL_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(30);

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

fn updated(scope: Scope, ids: impl IntoIterator<Item = String>) -> MirrorUpdated {
    let scopes = match scope {
        Scope::All => vec![Scope::Inbox, Scope::Board],
        s => vec![s],
    };
    MirrorUpdated { scopes, ids: ids.into_iter().collect() }
}

/// Run one sync. `notify` is called whenever mirror rows of the scope were
/// written (the app emits `mirror-updated`; the CLI ignores it).
pub fn run(db: &Db, req: &SyncRequest, notify: &dyn Fn(MirrorUpdated)) -> AppResult<SyncReport> {
    let start = Instant::now();
    let scope = req.scope;
    log::debug!(target: "pipeline", "run {scope:?} {:?} discover={}", req.mode, req.discover);
    let viewer = inbox_cmd::cached_or_fetch_viewer(db)?;
    let mut report = SyncReport { viewer_login: viewer.login.clone(), ..Default::default() };
    // Fetched with the full fragment during this run, so not probed again.
    let mut fresh: HashSet<String> = HashSet::new();
    // Whose core facet changed in this run.
    let mut changed: HashSet<String> = HashSet::new();

    if req.discover || req.mode != Mode::Light {
        report.discovered = discover(db, &viewer, scope, &mut report, &mut fresh, &mut changed)?;
        if report.discovered {
            notify(updated(scope, changed.iter().cloned()));
        }
    }

    let (inbox_ids, board_ids, target_ids) = {
        let conn = db.0.lock().unwrap();
        (
            if scope.inbox() { inbox_cmd::active_inbox_ids(&conn)? } else { Vec::new() },
            if scope.board() { board_cmd::tracked_ids(&conn)? } else { Vec::new() },
            if req.mode == Mode::Full { target_pr_ids(&conn)? } else { Vec::new() },
        )
    };
    let mut seen = HashSet::new();
    let entities: Vec<String> = inbox_ids
        .iter()
        .chain(&board_ids)
        .chain(&target_ids)
        .filter(|id| !fresh.contains(*id) && seen.insert(id.as_str()))
        .cloned()
        .collect();
    let visible: HashSet<&str> = match req.mode {
        Mode::Forced => req.visible_ids.iter().map(String::as_str).collect(),
        _ => HashSet::new(),
    };

    let mut gone: Vec<String> = Vec::new();
    let to_fetch: Vec<String> = if req.mode == Mode::Full {
        entities.clone()
    } else {
        // Visible ids of a Forced sync are refetched anyway, so not probed.
        let (mut out, probed): (Vec<String>, Vec<String>) =
            entities.iter().cloned().partition(|id| visible.contains(id.as_str()));
        report.probed = probed.len();
        let keys = mirror_keys(&db.0.lock().unwrap(), &probed)?;
        let (full, cheap): (Vec<String>, Vec<String>) =
            probed.iter().cloned().partition(|id| keys.get(id).is_some_and(MirrorKey::is_open_pr));
        let probes = mirror::fetch::probe(&full, &cheap);
        for id in &probed {
            match probes.get(id) {
                Some(Some(p)) if needs_refetch(p, keys.get(id)) => out.push(id.clone()),
                Some(None) => gone.push(id.clone()),
                // Unchanged, or its probe batch failed (state unknown).
                _ => {}
            }
        }
        out
    };
    log::debug!(
        target: "pipeline",
        "{} entities ({} inbox, {} board, {} targets), probed {}, to refetch {}, gone {}",
        entities.len(), inbox_ids.len(), board_ids.len(), target_ids.len(),
        report.probed, to_fetch.len(), gone.len()
    );
    report.changed = to_fetch.len();
    refetch_into_mirror(db, &to_fetch, &mut gone)?;
    changed.extend(to_fetch);

    if scope.inbox() {
        let ts = now();
        let mut conn = db.0.lock().unwrap();
        let tx = conn.transaction()?;
        let closed = closed_in_mirror(&tx, &inbox_ids)?;
        for id in inbox_ids.iter().filter(|id| closed.contains(*id) || gone.contains(id)) {
            if inbox_cmd::archive_closed_item(&tx, id, &ts)? {
                report.closed_count += 1;
            }
        }
        tx.commit()?;
    }

    if scope.board() {
        board_cmd::reconcile_links(&db.0.lock().unwrap())?;
        report.threads = sync_threads(db, req.mode, &visible)?;
    }

    {
        let conn = db.0.lock().unwrap();
        mirror::store::gc(&conn)?;
        let ts = now();
        if scope.inbox() {
            inbox_cmd::set_meta(&conn, "last_refresh_at", &ts)?;
            report.item_count = inbox_cmd::active_inbox_ids(&conn)?.len() as i64;
        }
        if scope.board() {
            inbox_cmd::set_meta(&conn, "board_last_refresh_at", &ts)?;
            report.card_count =
                conn.query_row("SELECT COUNT(*) FROM board_card WHERE archived_at IS NULL", [], |r| r.get(0))?;
        }
    }
    if req.mode == Mode::Full {
        full_extras(db)?;
    }

    report.duration_ms = start.elapsed().as_millis() as i64;
    log::debug!(target: "pipeline", "run {scope:?} done: {} changed ids, {}ms", changed.len(), report.duration_ms);
    notify(updated(scope, changed));
    Ok(report)
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Search for items the mirror may not know yet: the inbox's attention
/// searches (which include my authored PRs), or for a board-only sync just my
/// authored PRs. Authored PRs in registered repos get auto-tracked. Returns
/// false when another sync was already discovering (this one waited for it).
fn discover(
    db: &Db,
    viewer: &ViewerInfo,
    scope: Scope,
    report: &mut SyncReport,
    fresh: &mut HashSet<String>,
    changed: &mut HashSet<String>,
) -> AppResult<bool> {
    let flight = inflight::global();
    let kind = if scope.inbox() { "inbox" } else { "board" };
    let (claim, mine, others) = flight.claim("discovery", &[kind.to_string()]);
    if mine.is_empty() {
        flight.wait("discovery", &others);
        return Ok(false);
    }

    let mut complete_discovery = false;
    let (merged, authored): (Vec<inbox::ReasonedNode>, Vec<Node>) = if scope.inbox() {
        let search_scope = std::env::var("CODEREVIEW_SEARCH_SCOPE").ok();
        let (merged, complete) = inbox::run_all_searches(viewer, search_scope.as_deref());
        complete_discovery = complete;
        let authored = merged
            .iter()
            .filter(|r| r.reasons.iter().any(|x| x.reason == "author"))
            .map(|r| r.node.clone())
            .collect();
        (merged, authored)
    } else {
        let authored = mirror::fetch::search(AUTHORED_SEARCH, AUTHORED_SEARCH_LIMIT).unwrap_or_else(|e| {
            crate::logging::warn(&format!("[sync.discover] authored search failed: {e}"));
            Vec::new()
        });
        (Vec::new(), authored)
    };

    let nodes: Vec<&Node> = merged.iter().map(|r| &r.node).chain(&authored).collect();
    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let ts = now();
    let mut conn = db.0.lock().unwrap();
    let tx = conn.transaction()?;
    let before = updated_at_of(&tx, &ids)?;
    let prev_discovery = inbox_cmd::get_meta(&tx, "last_complete_discovery_at");
    for r in &merged {
        mirror::store::upsert_node(&tx, &r.node, &ts)?;
        inbox_cmd::upsert_entry(&tx, &r.node, &ts)?;
        inbox_cmd::record_reasons(&tx, &r.node.id, &r.reasons, prev_discovery.as_deref(), &ts)?;
    }
    if !scope.inbox() {
        for node in &authored {
            mirror::store::upsert_node(&tx, node, &ts)?;
        }
    }
    report.auto_tracked = board_cmd::auto_track_authored(&tx, authored.iter(), &ts)?;
    if scope.inbox() {
        inbox_cmd::set_meta(&tx, "last_discovery_at", &ts)?;
        if complete_discovery {
            inbox_cmd::set_meta(&tx, "last_complete_discovery_at", &ts)?;
        }
    }
    tx.commit()?;
    drop(conn);
    drop(claim);

    for node in nodes {
        if before.get(&node.id).and_then(|u| u.as_deref()) != Some(node.updated_at.as_str()) {
            changed.insert(node.id.clone());
        }
        fresh.insert(node.id.clone());
    }
    Ok(true)
}

fn updated_at_of(conn: &Connection, ids: &[String]) -> AppResult<HashMap<String, Option<String>>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ph = placeholders(ids.len());
    let sql = format!(
        "SELECT node_id, gh_updated_at FROM gh_pr WHERE node_id IN ({ph})
         UNION ALL SELECT node_id, gh_updated_at FROM gh_issue WHERE node_id IN ({ph})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter().chain(ids)), |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn target_pr_ids(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT gh_pr_id FROM target WHERE gh_pr_id IS NOT NULL")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Probe diff
// ---------------------------------------------------------------------------

/// The mirror's copy of the fields the probe returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MirrorKey {
    pub is_pr: bool,
    pub updated_at: Option<String>,
    pub head_sha: Option<String>,
    pub state: Option<String>,
    pub is_draft: bool,
    pub review_decision: Option<String>,
    pub merge_state_status: Option<String>,
    pub auto_merge_method: Option<String>,
    pub ci_state: Option<String>,
    /// False for rows mirrored before closing references were stored.
    pub has_closing: bool,
}

fn mirror_keys(conn: &Connection, ids: &[String]) -> AppResult<HashMap<String, MirrorKey>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ph = placeholders(ids.len());
    let sql = format!(
        "SELECT p.node_id, 1, p.gh_updated_at, p.head_sha, p.state, p.is_draft, p.review_decision,
                s.merge_state_status, s.auto_merge_method, s.ci_state, p.closing_issue_ids IS NOT NULL
         FROM gh_pr p LEFT JOIN gh_pr_status s ON s.pr_id = p.node_id WHERE p.node_id IN ({ph})
         UNION ALL
         SELECT node_id, 0, gh_updated_at, NULL, state, 0, NULL, NULL, NULL, NULL, 1
         FROM gh_issue WHERE node_id IN ({ph})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter().chain(ids)), |r| {
        Ok((
            r.get::<_, String>(0)?,
            MirrorKey {
                is_pr: r.get::<_, i64>(1)? != 0,
                updated_at: r.get(2)?,
                head_sha: r.get(3)?,
                state: r.get(4)?,
                is_draft: r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                review_decision: r.get(6)?,
                merge_state_status: r.get(7)?,
                auto_merge_method: r.get(8)?,
                ci_state: r.get(9)?,
                has_closing: r.get::<_, i64>(10)? != 0,
            },
        ))
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

impl MirrorKey {
    /// Only open PRs get the full probe: closed ones and issues change little
    /// beyond what `updatedAt` and `state` show.
    fn is_open_pr(&self) -> bool {
        self.is_pr && self.state.as_deref() == Some("open")
    }
}

/// Whether the probe saw a change the mirror doesn't have (or the mirror has
/// no row at all, e.g. after Reset mirror).
pub fn needs_refetch(p: &Probe, m: Option<&MirrorKey>) -> bool {
    let Some(m) = m else { return true };
    if p.is_pr() != m.is_pr
        || m.updated_at.as_deref() != p.updated_at.as_deref()
        || m.state != p.normalized_state()
    {
        return true;
    }
    p.is_pr()
        && !p.cheap
        && (!m.has_closing
            || m.head_sha != p.head_ref_oid
            || m.is_draft != p.is_draft
            || m.review_decision != p.review_decision
            || m.merge_state_status != p.merge_state_status
            || m.auto_merge_method != p.auto_merge_method()
            || m.ci_state != p.ci_state())
}

/// Refetch `ids` with the full fragment into the mirror. Ids GitHub no longer
/// returns are pushed onto `gone`. Ids another sync is already fetching are
/// waited for instead.
fn refetch_into_mirror(db: &Db, ids: &[String], gone: &mut Vec<String>) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let flight = inflight::global();
    let (claim, mine, others) = flight.claim("node", ids);
    let fetched = mirror::fetch::refetch(&mine);
    let ts = now();
    {
        let mut conn = db.0.lock().unwrap();
        let tx = conn.transaction()?;
        for (id, node) in &fetched {
            match node {
                Some(node) => mirror::store::upsert_node(&tx, node, &ts)?,
                None => gone.push(id.clone()),
            }
        }
        tx.commit()?;
    }
    drop(claim);
    flight.wait("node", &others);
    Ok(())
}

fn closed_in_mirror(conn: &Connection, ids: &[String]) -> AppResult<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let ph = placeholders(ids.len());
    let sql = format!(
        "SELECT node_id FROM gh_pr WHERE state IN ('closed', 'merged') AND node_id IN ({ph})
         UNION SELECT node_id FROM gh_issue WHERE state = 'closed' AND node_id IN ({ph})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter().chain(ids)), |r| r.get(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Facets
// ---------------------------------------------------------------------------

/// Whether a PR's mirrored threads are stale: never counted, fetched before
/// the PR's current `updatedAt`, or older than [`THREADS_MAX_AGE`].
pub fn threads_need_fetch(
    stored_updated_at: Option<&str>,
    updated_at: &str,
    has_count: bool,
    fetched_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    let fetched = fetched_at.and_then(|t| DateTime::parse_from_rfc3339(t).ok());
    !has_count
        || stored_updated_at != Some(updated_at)
        || fetched.is_none_or(|t| now.signed_duration_since(t) >= THREADS_MAX_AGE)
}

/// Fetch the threads facet (and so the pending-threads badge) of the open
/// tracked PR cards that need it: stale ones, all of them in a Full sync, and
/// the visible ones in a Forced sync.
fn sync_threads(db: &Db, mode: Mode, visible: &HashSet<&str>) -> AppResult<usize> {
    struct Job {
        id: String,
        owner: String,
        name: String,
        number: i64,
    }

    let started = Utc::now();
    let jobs: Vec<Job> = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT c.id, c.repo, c.number, c.updated_at, th.gh_updated_at,
                    th.pending_count IS NOT NULL, th.fetched_at
             FROM {} c LEFT JOIN gh_pr_threads th ON th.pr_id = c.id
             WHERE c.type = 'pr' AND c.archived_at IS NULL AND c.discarded_at IS NULL
               AND c.state NOT IN ('merged', 'closed')",
            board_cmd::CARD_VIEW
        ))?;
        type Row = (String, String, i64, String, Option<String>, bool, Option<String>);
        let rows: Vec<Row> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .filter(|(id, _, _, updated_at, stored, has_count, fetched_at)| {
                mode == Mode::Full
                    || visible.contains(id.as_str())
                    || threads_need_fetch(stored.as_deref(), updated_at, *has_count, fetched_at.as_deref(), started)
            })
            .filter_map(|(id, repo, number, ..)| {
                let (owner, name) = repo.split_once('/')?;
                Some(Job { id, owner: owner.to_string(), name: name.to_string(), number })
            })
            .collect()
    };

    let ids: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
    let flight = inflight::global();
    let (claim, mine, others) = flight.claim("threads", &ids);
    let mine: HashSet<String> = mine.into_iter().collect();
    let jobs: Vec<Job> = jobs.into_iter().filter(|j| mine.contains(&j.id)).collect();
    let results = inbox::concurrent_map(jobs, THREADS_CONCURRENCY, |job| {
        (job.id.clone(), crate::gh::pr_review_threads(&job.owner, &job.name, job.number).ok())
    });
    let n = results.len();
    {
        let conn = db.0.lock().unwrap();
        let ts = started.to_rfc3339();
        for (id, threads) in results {
            // A failed fetch keeps the previous count rather than writing 0.
            if let Some(threads) = threads {
                mirror::store::upsert_threads(&conn, &id, &threads, &ts)?;
            }
        }
    }
    drop(claim);
    flight.wait("threads", &others);
    Ok(n)
}

// ---------------------------------------------------------------------------
// Full sync extras
// ---------------------------------------------------------------------------

/// Fetch every clone (re-resolving its default branch), and drop the diff
/// cache and expired avatars.
fn full_extras(db: &Db) -> AppResult<()> {
    crate::diff_cache::clear();
    let repo_ids: Vec<i64> = {
        let conn = db.0.lock().unwrap();
        conn.execute("UPDATE repo_sync SET default_branch_checked_at = NULL", [])?;
        let mut stmt = conn.prepare("SELECT id FROM repository WHERE local_path IS NOT NULL")?;
        let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids
    };
    let results = inbox::concurrent_map(repo_ids, FETCH_CONCURRENCY, |id| {
        (*id, crate::repo_sync::fetch_repo(db, *id, FULL_FETCH_MIN_INTERVAL))
    });
    for (id, r) in results {
        if let Err(e) = r {
            crate::logging::warn(&format!("[sync.full] fetch of repo {id} failed: {e}"));
        }
    }
    crate::avatar::purge_stale(&db.0.lock().unwrap(), crate::avatar::now_secs())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn probe(json: serde_json::Value) -> Probe {
        serde_json::from_value(json).unwrap()
    }

    fn pr_probe() -> Probe {
        probe(serde_json::json!({
            "__typename": "PullRequest", "id": "PR_1", "updatedAt": "t1", "headRefOid": "h1",
            "state": "OPEN", "merged": false, "isDraft": false, "reviewDecision": "APPROVED",
            "mergeStateStatus": "CLEAN", "autoMergeRequest": null,
            "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "SUCCESS" } } } ] }
        }))
    }

    fn pr_key() -> MirrorKey {
        MirrorKey {
            is_pr: true,
            updated_at: Some("t1".into()),
            head_sha: Some("h1".into()),
            state: Some("open".into()),
            is_draft: false,
            review_decision: Some("APPROVED".into()),
            merge_state_status: Some("CLEAN".into()),
            auto_merge_method: None,
            ci_state: Some("SUCCESS".into()),
            has_closing: true,
        }
    }

    #[test]
    fn unchanged_probe_needs_no_refetch() {
        assert!(!needs_refetch(&pr_probe(), Some(&pr_key())));
    }

    #[test]
    fn any_probed_change_or_missing_row_refetches() {
        let p = pr_probe();
        assert!(needs_refetch(&p, None), "not mirrored yet");
        type Case = (&'static str, fn(&mut MirrorKey));
        let cases: Vec<Case> = vec![
            ("updatedAt", |k| k.updated_at = Some("t0".into())),
            ("head", |k| k.head_sha = Some("h0".into())),
            ("state", |k| k.state = Some("closed".into())),
            ("draft", |k| k.is_draft = true),
            ("review decision", |k| k.review_decision = None),
            ("merge state", |k| k.merge_state_status = Some("BEHIND".into())),
            ("auto-merge", |k| k.auto_merge_method = Some("SQUASH".into())),
            ("CI, which updatedAt misses", |k| k.ci_state = Some("PENDING".into())),
            ("closing refs never stored", |k| k.has_closing = false),
        ];
        for (what, change) in cases {
            let mut k = pr_key();
            change(&mut k);
            assert!(needs_refetch(&p, Some(&k)), "{what}");
        }
    }

    #[test]
    fn a_cheap_probe_compares_only_updated_at_and_state() {
        let p = Probe { cheap: true, ..probe(serde_json::json!({
            "__typename": "PullRequest", "id": "PR_1", "updatedAt": "t1", "state": "OPEN", "merged": false
        })) };
        let stale_keys = MirrorKey { ci_state: Some("PENDING".into()), has_closing: false, ..pr_key() };
        assert!(!needs_refetch(&p, Some(&stale_keys)));
        assert!(needs_refetch(&p, Some(&MirrorKey { updated_at: Some("t0".into()), ..pr_key() })));
    }

    #[test]
    fn issues_compare_only_updated_at_and_state() {
        let p = probe(serde_json::json!({ "__typename": "Issue", "id": "I_1", "updatedAt": "t1", "state": "OPEN" }));
        let key = MirrorKey { updated_at: Some("t1".into()), state: Some("open".into()), has_closing: true, ..Default::default() };
        assert!(!needs_refetch(&p, Some(&key)));
        assert!(needs_refetch(&p, Some(&MirrorKey { state: Some("closed".into()), ..key.clone() })));
        assert!(needs_refetch(&p, Some(&MirrorKey { is_pr: true, ..key })), "type changed");
    }

    #[test]
    fn mirror_keys_round_trip_through_the_probe() {
        let conn = open_memory();
        let node: Node = serde_json::from_value(serde_json::json!({
            "__typename": "PullRequest", "id": "PR_1", "number": 1, "title": "t", "url": "u",
            "state": "OPEN", "updatedAt": "t1", "headRefOid": "h1", "reviewDecision": "APPROVED",
            "mergeStateStatus": "CLEAN", "repository": { "nameWithOwner": "acme/widget" },
            "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "SUCCESS" } } } ] },
            "closingIssuesReferences": { "nodes": [] }
        }))
        .unwrap();
        mirror::store::upsert_node(&conn, &node, "now").unwrap();
        let keys = mirror_keys(&conn, &["PR_1".to_string(), "missing".to_string()]).unwrap();
        assert_eq!(keys.len(), 1);
        assert!(!needs_refetch(&pr_probe(), keys.get("PR_1")), "a fresh upsert matches its own probe");
    }

    #[test]
    fn closed_in_mirror_finds_merged_prs_and_closed_issues() {
        let conn = open_memory();
        let repo = mirror::store::ensure_repo(&conn, "acme", "widget", "now").unwrap();
        for (id, n, state) in [("PR_open", 1, "open"), ("PR_merged", 2, "merged")] {
            conn.execute(
                "INSERT INTO gh_pr (node_id, repo_id, number, title, url, state, gh_updated_at, fetched_at)
                 VALUES (?1, ?2, ?3, 't', 'u', ?4, 'now', 'now')",
                rusqlite::params![id, repo, n, state],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO gh_issue (node_id, repo_id, number, title, url, state, gh_updated_at, fetched_at)
             VALUES ('I_closed', ?1, 3, 't', 'u', 'closed', 'now', 'now')",
            [repo],
        )
        .unwrap();
        let ids: Vec<String> = ["PR_open", "PR_merged", "I_closed"].iter().map(|s| s.to_string()).collect();
        let closed = closed_in_mirror(&conn, &ids).unwrap();
        assert_eq!(closed, HashSet::from(["PR_merged".to_string(), "I_closed".to_string()]));
    }

    #[test]
    fn threads_skipped_only_when_unchanged_counted_and_recent() {
        let now = Utc::now();
        let recent = now.to_rfc3339();
        let recent = Some(recent.as_str());
        assert!(!threads_need_fetch(Some("t1"), "t1", true, recent, now));
        assert!(threads_need_fetch(Some("t0"), "t1", true, recent, now), "PR updated");
        assert!(threads_need_fetch(None, "t1", true, recent, now), "newly tracked");
        assert!(threads_need_fetch(Some("t1"), "t1", false, recent, now), "never counted");
        assert!(threads_need_fetch(Some("t1"), "t1", true, None, now), "never fetched");
        let later = now + THREADS_MAX_AGE;
        assert!(threads_need_fetch(Some("t1"), "t1", true, recent, later), "resolutions don't bump updatedAt");
    }
}
