//! Tauri commands for the kanban board. Owns the DB and orchestrates the
//! pure pipeline in `crate::board`, following the same split-lock discipline
//! as `commands/inbox.rs`: `gh` network I/O runs unlocked, writes happen in
//! short locked sections.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::board::{self, BoardNode, ThreadMode};
use crate::db::models::{BoardCardRow, BoardCardWithExtras, BoardMetaRow, Review};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::gh;

fn now() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// card read/write helpers
// ---------------------------------------------------------------------------

/// Insert or refresh a card from a fetched GitHub node. `clear_discard` is set
/// only by explicit tracking actions (Track / Link PR) — routine refreshes
/// never resurrect a discarded card, since discarded cards are excluded from
/// the refresh's refetch batch in the first place.
fn upsert_card(conn: &Connection, node: &BoardNode, viewer_login: &str, ts: &str, clear_discard: bool) -> AppResult<()> {
    let typ = if node.is_pr() { "pr" } else { "issue" };
    let repo = node.repo().unwrap_or_default();
    let author_login = node.author.as_ref().and_then(|a| a.login.clone());
    let is_mine = author_login.as_deref() == Some(viewer_login);
    let has_reviews = node.reviews.as_ref().map(|r| r.total_count > 0).unwrap_or(false);
    let ci_status = node.ci_status();
    let extra = if clear_discard { ", discarded_at = NULL" } else { "" };
    let sql = format!(
        "INSERT INTO board_card (
            id, type, repo, number, url, title, author_login, state, is_draft,
            review_decision, has_reviews, head_ref, is_mine, updated_at, closed_at,
            tracked_at, last_refreshed, ci_status, mergeable,
            merge_state_status, stack_number, stack_position, viewer_can_update_branch, auto_merge
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT(id) DO UPDATE SET
            type = excluded.type, repo = excluded.repo, number = excluded.number,
            url = excluded.url, title = excluded.title, author_login = excluded.author_login,
            state = excluded.state, is_draft = excluded.is_draft,
            review_decision = excluded.review_decision, has_reviews = excluded.has_reviews,
            head_ref = excluded.head_ref, is_mine = excluded.is_mine,
            updated_at = excluded.updated_at, closed_at = excluded.closed_at,
            last_refreshed = excluded.last_refreshed,
            ci_status = excluded.ci_status, mergeable = excluded.mergeable,
            merge_state_status = excluded.merge_state_status,
            stack_number = excluded.stack_number, stack_position = excluded.stack_position,
            viewer_can_update_branch = excluded.viewer_can_update_branch,
            auto_merge = excluded.auto_merge{extra}"
    );
    conn.execute(
        &sql,
        params![
            node.id,
            typ,
            repo,
            node.number,
            node.url,
            node.title,
            author_login,
            node.normalized_state(),
            node.is_draft as i64,
            node.review_decision,
            has_reviews as i64,
            node.head_ref_name,
            is_mine as i64,
            node.updated_at,
            node.closed_at,
            ts,
            ci_status,
            node.mergeable,
            node.merge_state_status,
            node.stack.as_ref().map(|s| s.number),
            node.stack_entry.as_ref().map(|s| s.position),
            node.viewer_can_update_branch as i64,
            node.auto_merge(),
        ],
    )?;
    Ok(())
}

fn registered_repo(conn: &Connection, owner: &str, name: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM repository WHERE remote_owner = ?1 AND remote_name = ?2",
        params![owner, name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

/// `(owner, name, number)` for a tracked PR card, erroring on an issue card.
fn pr_card_coords(db: &Db, card_id: &str) -> AppResult<(String, String, i64)> {
    let conn = db.0.lock().unwrap();
    let (repo, number, typ): (String, i64, String) = conn.query_row(
        "SELECT repo, number, type FROM board_card WHERE id = ?1",
        params![card_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if typ != "pr" {
        return Err(AppError::Other("only PR cards have a review".into()));
    }
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| AppError::Other(format!("malformed repo label: {repo}")))?;
    Ok((owner.to_string(), name.to_string(), number))
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardRefreshResult {
    pub card_count: i64,
    pub auto_tracked: i64,
    pub duration_ms: i64,
}

/// Refresh the board: auto-track new open PRs of mine in registered repos,
/// refetch every tracked card's state, reconcile issue<->PR auto-links, and
/// recompute each open PR's pending-threads comment badge.
pub fn board_refresh_impl(db: &Db) -> AppResult<BoardRefreshResult> {
    let start = Instant::now();
    let viewer = crate::commands::inbox::cached_or_fetch_viewer(db)?;

    let tracked_ids: Vec<String> = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM board_card WHERE archived_at IS NULL AND discarded_at IS NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        rows
    };
    let tracked_set: HashSet<&str> = tracked_ids.iter().map(String::as_str).collect();

    let authored = board::search_authored_open_prs().unwrap_or_else(|e| {
        crate::logging::warn(&format!("[board.refresh] authored search failed: {e}"));
        Vec::new()
    });
    let mut auto_tracked = 0i64;
    let ts = now();
    {
        let conn = db.0.lock().unwrap();
        for node in &authored {
            if tracked_set.contains(node.id.as_str()) {
                continue;
            }
            let Some(repo) = node.repo() else { continue };
            let Some((owner, name)) = repo.split_once('/') else { continue };
            if registered_repo(&conn, owner, name)? {
                upsert_card(&conn, node, &viewer.login, &ts, false)?;
                auto_tracked += 1;
            }
        }
    }

    let refetched = board::refetch_nodes(&tracked_ids);
    let ts = now();
    {
        let conn = db.0.lock().unwrap();
        for node in refetched.values() {
            upsert_card(&conn, node, &viewer.login, &ts, false)?;
        }
    }

    reconcile_links(db, refetched.values().chain(authored.iter().filter(|n| !tracked_set.contains(n.id.as_str()))))?;
    refresh_pending_threads(db, &viewer.login)?;

    let card_count: i64 = {
        let conn = db.0.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM board_card WHERE archived_at IS NULL", [], |r| r.get(0))?
    };
    {
        let conn = db.0.lock().unwrap();
        crate::commands::inbox::set_meta(&conn, "board_last_refresh_at", &now())?;
    }

    Ok(BoardRefreshResult {
        card_count,
        auto_tracked,
        duration_ms: start.elapsed().as_millis() as i64,
    })
}

#[tauri::command]
pub async fn board_refresh(db: State<'_, Db>) -> AppResult<BoardRefreshResult> {
    board_refresh_impl(&db)
}

#[tauri::command]
pub fn board_last_refresh_at(db: State<Db>) -> AppResult<Option<String>> {
    let conn = db.0.lock().unwrap();
    Ok(crate::commands::inbox::get_meta(&conn, "board_last_refresh_at"))
}

/// Rebuild every auto-detected issue<->PR link from this refresh's PR nodes:
/// GitHub's own `closingIssuesReferences`, plus the `issue-<n>` branch-name
/// convention within the same repo. Only links against issue cards that are
/// themselves tracked (not discarded/archived) — an untracked issue never
/// gets a card just because a PR closes it.
fn reconcile_links<'a>(db: &Db, prs: impl Iterator<Item = &'a BoardNode>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM board_link WHERE source = 'auto'", [])?;

    let issues: Vec<(String, String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, repo, number FROM board_card
             WHERE type = 'issue' AND discarded_at IS NULL AND archived_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };

    for pr in prs.filter(|n| n.is_pr()) {
        let pr_repo = pr.repo();

        if let Some(refs) = &pr.closing_issues_references {
            for closing in &refs.nodes {
                if issues.iter().any(|(id, _, _)| id == &closing.id) {
                    conn.execute(
                        "INSERT OR IGNORE INTO board_link (issue_id, pr_id, source) VALUES (?1, ?2, 'auto')",
                        params![closing.id, pr.id],
                    )?;
                }
            }
        }

        if let (Some(head_ref), Some(pr_repo)) = (&pr.head_ref_name, &pr_repo) {
            for (issue_id, issue_repo, issue_number) in &issues {
                if issue_repo == pr_repo && board::branch_matches_issue(head_ref, *issue_number) {
                    conn.execute(
                        "INSERT OR IGNORE INTO board_link (issue_id, pr_id, source) VALUES (?1, ?2, 'auto')",
                        params![issue_id, pr.id],
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// For every open, tracked PR card, fetch its review threads and recompute
/// the pending-threads comment badge (concurrently, like the inbox's
/// top-files enrichment).
fn refresh_pending_threads(db: &Db, viewer_login: &str) -> AppResult<()> {
    struct ThreadJob {
        id: String,
        owner: String,
        name: String,
        number: i64,
        is_mine: bool,
    }

    let jobs: Vec<ThreadJob> = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo, number, is_mine FROM board_card
             WHERE type = 'pr' AND archived_at IS NULL AND discarded_at IS NULL
               AND state NOT IN ('merged', 'closed')",
        )?;
        let rows: Vec<(String, String, i64, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .filter_map(|(id, repo, number, is_mine)| {
                let (owner, name) = repo.split_once('/')?;
                Some(ThreadJob {
                    id,
                    owner: owner.to_string(),
                    name: name.to_string(),
                    number,
                    is_mine: is_mine != 0,
                })
            })
            .collect()
    };

    let results = crate::inbox::concurrent_map(jobs, 8, |job| {
        let threads = gh::pr_review_threads(&job.owner, &job.name, job.number).unwrap_or_default();
        let mode = if job.is_mine { ThreadMode::Mine } else { ThreadMode::Reviewing };
        (job.id.clone(), board::pending_threads(&threads, viewer_login, mode))
    });

    let conn = db.0.lock().unwrap();
    for (id, count) in results {
        conn.execute(
            "UPDATE board_card SET pending_threads = ?1 WHERE id = ?2",
            params![count as i64, id],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

pub fn board_list_impl(conn: &Connection) -> AppResult<Vec<BoardCardWithExtras>> {
    let cards: Vec<BoardCardRow> = {
        let mut stmt = conn.prepare("SELECT * FROM board_card WHERE archived_at IS NULL ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], BoardCardRow::from_row)?.collect::<rusqlite::Result<_>>()?;
        rows
    };

    let mut links_by_issue: HashMap<String, Vec<String>> = HashMap::new();
    let mut links_by_pr: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT issue_id, pr_id FROM board_link")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (issue_id, pr_id) = row?;
            links_by_issue.entry(issue_id.clone()).or_default().push(pr_id.clone());
            links_by_pr.entry(pr_id).or_default().push(issue_id);
        }
    }

    let card_state: HashMap<&str, Option<&str>> =
        cards.iter().map(|c| (c.id.as_str(), c.state.as_deref())).collect();
    let issue_has_open_pr: HashMap<String, bool> = links_by_issue
        .iter()
        .map(|(issue_id, prs)| {
            let has_open = prs.iter().any(|pr_id| {
                !matches!(card_state.get(pr_id.as_str()), Some(Some("merged")) | Some(Some("closed")))
            });
            (issue_id.clone(), has_open)
        })
        .collect();

    let mut meta_by_card: HashMap<String, Vec<BoardMetaRow>> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT * FROM board_meta ORDER BY created_at")?;
        let rows = stmt.query_map([], BoardMetaRow::from_row)?;
        for row in rows {
            let m = row?;
            meta_by_card.entry(m.card_id.clone()).or_default().push(m);
        }
    }

    let mut draft_reviews: HashMap<(String, i64), i64> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT r.remote_owner, r.remote_name, t.github_pr_number, rv.id
             FROM review rv
             JOIN target t ON t.id = rv.target_id
             JOIN repository r ON r.id = t.repo_id
             WHERE t.kind = 'github_pr' AND rv.status = 'draft'
             ORDER BY rv.updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (owner, name, number, review_id) = row?;
            if let (Some(owner), Some(name)) = (owner, name) {
                draft_reviews.entry((format!("{owner}/{name}"), number)).or_insert(review_id);
            }
        }
    }

    Ok(cards
        .into_iter()
        .map(|card| {
            let has_open_linked_pr = issue_has_open_pr.get(&card.id).copied().unwrap_or(false);
            let lane = board::lane_for(&card, has_open_linked_pr).map(|l| l.as_str().to_string());
            let meta = meta_by_card.get(&card.id).cloned().unwrap_or_default();
            let linked_issue_ids = links_by_pr.get(&card.id).cloned().unwrap_or_default();
            let linked_pr_ids = links_by_issue.get(&card.id).cloned().unwrap_or_default();
            let inherited_meta = linked_issue_ids
                .iter()
                .flat_map(|iid| meta_by_card.get(iid).cloned().unwrap_or_default())
                .collect();
            let existing_review_id = if card.typ == "pr" {
                draft_reviews.get(&(card.repo.clone(), card.number)).copied()
            } else {
                None
            };
            BoardCardWithExtras {
                card,
                lane,
                meta,
                inherited_meta,
                linked_issue_ids,
                linked_pr_ids,
                existing_review_id,
            }
        })
        .collect())
}

#[tauri::command]
pub fn board_list(db: State<Db>) -> AppResult<Vec<BoardCardWithExtras>> {
    let conn = db.0.lock().unwrap();
    board_list_impl(&conn)
}

/// Ids currently tracked (not discarded/archived) — the Inbox's "Tracking"
/// filter and Track-button visibility are keyed off this set.
#[tauri::command]
pub fn board_tracked_ids(db: State<Db>) -> AppResult<Vec<String>> {
    let conn = db.0.lock().unwrap();
    let mut stmt =
        conn.prepare("SELECT id FROM board_card WHERE discarded_at IS NULL AND archived_at IS NULL")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// A single card by id, with the same lane/meta/link enrichment as
/// `board_list_impl` — for `cr board show`. Falls back to a minimal
/// (lane-less) result for archived/discarded cards, which `board_list_impl`
/// excludes.
pub fn board_show_impl(conn: &Connection, card_id: &str) -> AppResult<BoardCardWithExtras> {
    if let Some(found) = board_list_impl(conn)?.into_iter().find(|c| c.card.id == card_id) {
        return Ok(found);
    }
    let card = conn
        .query_row("SELECT * FROM board_card WHERE id = ?1", params![card_id], BoardCardRow::from_row)
        .map_err(AppError::from)?;
    let meta = board_meta_list_impl(conn, card_id)?;
    let linked_issue_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT issue_id FROM board_link WHERE pr_id = ?1")?;
        let rows = stmt.query_map(params![card_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        rows
    };
    let inherited_meta = linked_issue_ids
        .iter()
        .map(|iid| board_meta_list_impl(conn, iid))
        .collect::<AppResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(BoardCardWithExtras {
        card,
        lane: None,
        meta,
        inherited_meta,
        linked_issue_ids,
        linked_pr_ids: Vec::new(),
        existing_review_id: None,
    })
}

/// The board card for a PR, if it's tracked — including discarded/archived
/// ones, so the review's Context tab can still show their metadata. Used by
/// `review_board_card` for the review view's left-pane Context tab.
pub fn board_card_for_pr_impl(
    conn: &Connection,
    owner: &str,
    name: &str,
    number: i64,
) -> AppResult<Option<BoardCardWithExtras>> {
    match find_card_id(conn, owner, name, number)? {
        Some(id) => Ok(Some(board_show_impl(conn, &id)?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn board_card_for_pr(owner: String, name: String, number: i64, db: State<Db>) -> AppResult<Option<BoardCardWithExtras>> {
    let conn = db.0.lock().unwrap();
    board_card_for_pr_impl(&conn, &owner, &name, number)
}

// ---------------------------------------------------------------------------
// Track / discard / link
// ---------------------------------------------------------------------------

/// A card's own GitHub node id for a given `owner/name#number`, if tracked.
pub fn find_card_id(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<String>> {
    let repo = format!("{owner}/{name}");
    conn.query_row(
        "SELECT id FROM board_card WHERE repo = ?1 AND number = ?2",
        params![repo, number],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Track an issue or PR by URL / `owner/repo#n`. Returns the card id. Clears
/// a prior discard, so re-tracking from the Inbox restores the same card
/// (and its metadata).
pub fn board_track_impl(db: &Db, reference: &str) -> AppResult<String> {
    let (owner, name, number) = board::parse_ref(reference)?;
    let node = board::resolve_node(&owner, &name, number)?;
    let viewer = crate::commands::inbox::cached_or_fetch_viewer(db)?;
    let ts = now();
    let conn = db.0.lock().unwrap();
    upsert_card(&conn, &node, &viewer.login, &ts, true)?;
    Ok(node.id)
}

#[tauri::command]
pub async fn board_track(reference: String, db: State<'_, Db>) -> AppResult<String> {
    board_track_impl(&db, &reference)
}

// ---------------------------------------------------------------------------
// GitHub PR actions (squash merge, draft toggle, update branch)
// ---------------------------------------------------------------------------

/// Run `gh pr <args> <number> --repo owner/name` for a PR card, then refetch
/// that one card so the board reflects the new state.
fn run_pr_action(db: &Db, card_id: &str, args: &[&str]) -> AppResult<()> {
    let (owner, name, number) = pr_card_coords(db, card_id)?;
    let ctx = crate::gh::GhRepo::Remote {
        owner: owner.clone(),
        name: name.clone(),
    };
    let number_s = number.to_string();
    let repo = format!("{owner}/{name}");
    let mut full = vec!["pr", args[0], number_s.as_str(), "--repo", repo.as_str()];
    full.extend_from_slice(&args[1..]);
    crate::gh::run_gh(&ctx, &full)?;

    let node = board::resolve_node(&owner, &name, number)?;
    let viewer = crate::commands::inbox::cached_or_fetch_viewer(db)?;
    let conn = db.0.lock().unwrap();
    upsert_card(&conn, &node, &viewer.login, &now(), false)
}

#[tauri::command]
pub async fn board_pr_set_auto_merge(card_id: String, enable: bool, db: State<'_, Db>) -> AppResult<()> {
    run_pr_action(&db, &card_id, crate::gh::auto_merge_args(enable))
}

#[tauri::command]
pub async fn board_pr_set_draft(card_id: String, draft: bool, db: State<'_, Db>) -> AppResult<()> {
    let args: &[&str] = if draft { &["ready", "--undo"] } else { &["ready"] };
    run_pr_action(&db, &card_id, args)
}

#[tauri::command]
pub async fn board_pr_update_branch(card_id: String, rebase: bool, db: State<'_, Db>) -> AppResult<()> {
    let args: &[&str] = if rebase { &["update-branch", "--rebase"] } else { &["update-branch"] };
    run_pr_action(&db, &card_id, args)
}

/// A card's id for `owner/repo#number`, but only if it's currently visible on
/// the board (not discarded/archived) — used to detect duplicates before
/// tracking a PR from the "+ Track" dialog.
fn tracked_card_id(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<String>> {
    let repo = format!("{owner}/{name}");
    conn.query_row(
        "SELECT id FROM board_card WHERE repo = ?1 AND number = ?2 AND discarded_at IS NULL AND archived_at IS NULL",
        params![repo, number],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Track a PR by URL / `owner/repo#n` from the board's "+ Track" dialog.
/// Unlike `board_track_impl`, this refuses issues and already-tracked PRs
/// instead of silently refreshing them, so the dialog can surface a clear
/// warning rather than a no-op success.
pub fn board_track_pr_impl(db: &Db, reference: &str) -> AppResult<String> {
    let (owner, name, number) = board::parse_ref(reference)?;
    {
        let conn = db.0.lock().unwrap();
        if tracked_card_id(&conn, &owner, &name, number)?.is_some() {
            return Err(AppError::Other(format!("{owner}/{name}#{number} is already tracked on the board")));
        }
    }
    let node = board::resolve_node(&owner, &name, number)?;
    if !node.is_pr() {
        return Err(AppError::Other(format!("{owner}/{name}#{number} is not a pull request")));
    }
    let viewer = crate::commands::inbox::cached_or_fetch_viewer(db)?;
    let ts = now();
    let conn = db.0.lock().unwrap();
    upsert_card(&conn, &node, &viewer.login, &ts, true)?;
    conn.execute("UPDATE board_card SET archived_at = NULL WHERE id = ?1", params![node.id])?;
    Ok(node.id)
}

#[tauri::command]
pub async fn board_track_pr(reference: String, db: State<'_, Db>) -> AppResult<String> {
    board_track_pr_impl(&db, &reference)
}

/// Resolve a reference to its card id, tracking it first if `track` is set
/// and it isn't tracked yet. Used by `cr board meta add` so a single command
/// can both track and annotate a card.
pub fn resolve_or_track_card_id(db: &Db, reference: &str, track: bool) -> AppResult<String> {
    let (owner, name, number) = board::parse_ref(reference)?;
    let existing = {
        let conn = db.0.lock().unwrap();
        find_card_id(&conn, &owner, &name, number)?
    };
    match existing {
        Some(id) => Ok(id),
        None if track => board_track_impl(db, reference),
        None => Err(AppError::Other(format!(
            "{owner}/{name}#{number} isn't tracked on the board yet — pass --track to track it"
        ))),
    }
}

/// Soft-discard a card: hidden from the board, metadata kept, re-trackable.
/// Rejected for your own PRs, which are always on the board.
pub fn board_discard_impl(conn: &Connection, card_id: &str) -> AppResult<()> {
    let is_mine: i64 = conn
        .query_row("SELECT is_mine FROM board_card WHERE id = ?1", params![card_id], |r| r.get(0))
        .map_err(AppError::from)?;
    if is_mine != 0 {
        return Err(AppError::Other("your own PRs can't be discarded".into()));
    }
    conn.execute("UPDATE board_card SET discarded_at = ?1 WHERE id = ?2", params![now(), card_id])?;
    Ok(())
}

#[tauri::command]
pub fn board_discard(card_id: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    board_discard_impl(&conn, &card_id)
}

/// Manually link a PR (by URL / `owner/repo#n`) to a backlog issue, tracking
/// the PR if it wasn't already.
pub fn board_link_pr_impl(db: &Db, issue_id: &str, pr_reference: &str) -> AppResult<()> {
    let (owner, name, number) = board::parse_ref(pr_reference)?;
    let node = board::resolve_node(&owner, &name, number)?;
    if !node.is_pr() {
        return Err(AppError::Other("that reference is an issue, not a PR".into()));
    }
    let viewer = crate::commands::inbox::cached_or_fetch_viewer(db)?;
    let ts = now();
    let conn = db.0.lock().unwrap();
    upsert_card(&conn, &node, &viewer.login, &ts, true)?;
    conn.execute(
        "INSERT OR IGNORE INTO board_link (issue_id, pr_id, source) VALUES (?1, ?2, 'manual')",
        params![issue_id, node.id],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn board_link_pr(issue_id: String, pr_reference: String, db: State<'_, Db>) -> AppResult<()> {
    board_link_pr_impl(&db, &issue_id, &pr_reference)
}

/// Remove a manual issue<->PR link. Auto-detected links can't be unlinked
/// this way — they are a derived fact, not a user choice.
pub fn board_unlink_impl(conn: &Connection, issue_id: &str, pr_id: &str) -> AppResult<()> {
    let affected = conn.execute(
        "DELETE FROM board_link WHERE issue_id = ?1 AND pr_id = ?2 AND source = 'manual'",
        params![issue_id, pr_id],
    )?;
    if affected == 0 {
        return Err(AppError::Other("only manually-added links can be removed".into()));
    }
    Ok(())
}

#[tauri::command]
pub fn board_unlink(issue_id: String, pr_id: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    board_unlink_impl(&conn, &issue_id, &pr_id)
}

/// Archive (purge) Done cards whose PR closed/merged more than `older_than_days`
/// ago. Archiving is permanent in v1 — there is no restore UI yet.
pub fn board_archive_done_impl(conn: &Connection, older_than_days: i64) -> AppResult<usize> {
    let cutoff = (Utc::now() - chrono::Duration::days(older_than_days)).to_rfc3339();
    let n = conn.execute(
        "UPDATE board_card SET archived_at = ?1
         WHERE archived_at IS NULL AND type = 'pr' AND state IN ('merged', 'closed')
           AND closed_at IS NOT NULL AND closed_at < ?2",
        params![now(), cutoff],
    )?;
    Ok(n)
}

#[tauri::command]
pub fn board_archive_done(older_than_days: i64, db: State<Db>) -> AppResult<usize> {
    let conn = db.0.lock().unwrap();
    board_archive_done_impl(&conn, older_than_days)
}

// ---------------------------------------------------------------------------
// Review / VSCode actions
// ---------------------------------------------------------------------------

/// Create-or-resume the draft review for a PR card.
#[tauri::command]
pub async fn board_open_review(card_id: String, base_paths: Vec<String>, db: State<'_, Db>) -> AppResult<Review> {
    let (owner, name, number) = pr_card_coords(&db, &card_id)?;
    crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, true)
}

/// Create-or-resume the draft review for a PR card, ensure its chat/edit
/// worktree exists, then open VSCode on it.
#[tauri::command]
pub async fn board_open_vscode(card_id: String, base_paths: Vec<String>, db: State<'_, Db>) -> AppResult<()> {
    let (owner, name, number) = pr_card_coords(&db, &card_id)?;
    let review = crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, true)?;
    let worktree_path = crate::commands::chat::ensure_chat_worktree(review.id, db).await?;
    crate::commands::worktree::open_in_vscode(worktree_path, None)
}

/// Ensure the PR card's chat worktree exists, then open a plain shell there.
#[tauri::command]
pub async fn board_open_shell(
    card_id: String,
    base_paths: Vec<String>,
    host: crate::terminal::TerminalHost,
    tmux_target: crate::terminal::TmuxTarget,
    tmux_session: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    let (owner, name, number) = pr_card_coords(&db, &card_id)?;
    let review = crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, true)?;
    let worktree_path = crate::commands::chat::ensure_chat_worktree(review.id, db).await?;
    crate::terminal::open_shell(
        host,
        tmux_target,
        &tmux_session,
        &format!("{owner}/{name}"),
        &format!("#{number}"),
        &worktree_path,
    )
}

/// Ensure the PR card's chat worktree exists, then open a tmux window there running Claude —
/// resuming the card's latest recorded session when it isn't already live.
#[tauri::command]
pub async fn board_open_agent(
    card_id: String,
    base_paths: Vec<String>,
    host: crate::terminal::TerminalHost,
    tmux_target: crate::terminal::TmuxTarget,
    tmux_session: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    let (owner, name, number) = pr_card_coords(&db, &card_id)?;
    let review = crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, true)?;
    let worktree_path = crate::commands::chat::ensure_chat_worktree(review.id, db.clone()).await?;
    let live: std::collections::HashSet<String> =
        crate::claude_sessions::live_sessions().into_iter().map(|s| s.session_id).collect();
    let resume = {
        let conn = db.0.lock().unwrap();
        crate::commands::sessions::resumable_card_session(&conn, &card_id, &live)?
    };
    let mut argv = vec!["claude".to_string()];
    if let Some(id) = resume {
        argv.extend(["--resume".to_string(), id]);
    }
    crate::terminal::open_session(
        host,
        tmux_target,
        &tmux_session,
        &format!("{owner}/{name}"),
        &format!("#{number}"),
        &worktree_path,
        &argv,
    )
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

fn validate_meta_kind(kind: &str) -> AppResult<()> {
    match kind {
        "file" | "link" | "note" | "claude_session" => Ok(()),
        other => Err(AppError::Other(format!("invalid metadata kind '{other}' (expected file, link, note or claude_session)"))),
    }
}

/// Attach a metadata item to a card. Adding the same `claude_session` value
/// twice is a no-op (returns the existing row) so a session hook can call
/// this unconditionally.
#[allow(clippy::too_many_arguments)]
pub fn board_meta_add_impl(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    value: &str,
    tag: Option<&str>,
    label: Option<&str>,
    cwd: Option<&str>,
) -> AppResult<BoardMetaRow> {
    validate_meta_kind(kind)?;

    let value = value.trim();
    let has_tag = tag.is_some_and(|t| !t.trim().is_empty());
    if value.is_empty() && !(kind == "note" && has_tag) {
        return Err(AppError::Other("a note needs text or a tag".into()));
    }

    if kind == "claude_session" {
        let existing = conn
            .query_row(
                "SELECT * FROM board_meta WHERE card_id = ?1 AND kind = 'claude_session' AND value = ?2",
                params![card_id, value],
                BoardMetaRow::from_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
    }

    let ts = now();
    conn.execute(
        "INSERT INTO board_meta (card_id, kind, value, tag, label, cwd, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![card_id, kind, value, tag, label, cwd, ts],
    )?;
    conn.query_row(
        "SELECT * FROM board_meta WHERE id = ?1",
        params![conn.last_insert_rowid()],
        BoardMetaRow::from_row,
    )
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn board_meta_add(
    card_id: String,
    kind: String,
    value: String,
    tag: Option<String>,
    label: Option<String>,
    cwd: Option<String>,
    db: State<Db>,
) -> AppResult<BoardMetaRow> {
    let conn = db.0.lock().unwrap();
    board_meta_add_impl(&conn, &card_id, &kind, &value, tag.as_deref(), label.as_deref(), cwd.as_deref())
}

pub fn board_meta_update_impl(
    conn: &Connection,
    id: i64,
    value: Option<&str>,
    tag: Option<&str>,
    label: Option<&str>,
) -> AppResult<()> {
    if let Some(v) = value {
        conn.execute("UPDATE board_meta SET value = ?1 WHERE id = ?2", params![v, id])?;
    }
    if let Some(t) = tag {
        conn.execute("UPDATE board_meta SET tag = ?1 WHERE id = ?2", params![t, id])?;
    }
    if let Some(l) = label {
        conn.execute("UPDATE board_meta SET label = ?1 WHERE id = ?2", params![l, id])?;
    }
    Ok(())
}

#[tauri::command]
pub fn board_meta_update(
    id: i64,
    value: Option<String>,
    tag: Option<String>,
    label: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    board_meta_update_impl(&conn, id, value.as_deref(), tag.as_deref(), label.as_deref())
}

pub fn board_meta_delete_impl(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM board_meta WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn board_meta_delete(id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    board_meta_delete_impl(&conn, id)
}

/// A card's metadata, oldest first (for `cr board meta list`).
pub fn board_meta_list_impl(conn: &Connection, card_id: &str) -> AppResult<Vec<BoardMetaRow>> {
    let mut stmt = conn.prepare("SELECT * FROM board_meta WHERE card_id = ?1 ORDER BY created_at")?;
    let rows = stmt.query_map(params![card_id], BoardMetaRow::from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Distinct tags already used, for the tag autocomplete.
pub fn board_meta_tags_impl(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT tag FROM board_meta WHERE tag IS NOT NULL AND tag != '' ORDER BY tag")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[tauri::command]
pub fn board_meta_tags(db: State<Db>) -> AppResult<Vec<String>> {
    let conn = db.0.lock().unwrap();
    board_meta_tags_impl(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn seed_card(conn: &Connection, id: &str, repo: &str, number: i64, typ: &str) {
        conn.execute(
            "INSERT INTO board_card (id, type, repo, number, url, title, updated_at, tracked_at)
             VALUES (?1, ?2, ?3, ?4, 'https://example.com', 'title', 'now', 'now')",
            params![id, typ, repo, number],
        )
        .unwrap();
    }

    fn seed_meta(conn: &Connection, card_id: &str, value: &str) {
        conn.execute(
            "INSERT INTO board_meta (card_id, kind, value, created_at) VALUES (?1, 'note', ?2, 'now')",
            params![card_id, value],
        )
        .unwrap();
    }

    #[test]
    fn board_card_for_pr_returns_none_when_untracked() {
        let conn = open_memory();
        assert!(board_card_for_pr_impl(&conn, "owner", "repo", 1).unwrap().is_none());
    }

    #[test]
    fn board_card_for_pr_finds_tracked_card_by_coords() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 42, "pr");
        seed_meta(&conn, "pr1", "a note");

        let found = board_card_for_pr_impl(&conn, "owner", "repo", 42).unwrap().unwrap();
        assert_eq!(found.card.id, "pr1");
        assert_eq!(found.meta.len(), 1);
    }

    #[test]
    fn board_card_for_pr_includes_discarded_cards_with_inherited_meta() {
        let conn = open_memory();
        seed_card(&conn, "issue1", "owner/repo", 1, "issue");
        seed_card(&conn, "pr1", "owner/repo", 2, "pr");
        seed_meta(&conn, "issue1", "from issue");
        conn.execute(
            "INSERT INTO board_link (issue_id, pr_id, source) VALUES ('issue1', 'pr1', 'auto')",
            [],
        )
        .unwrap();
        conn.execute("UPDATE board_card SET discarded_at = 'now' WHERE id = 'pr1'", [])
            .unwrap();

        let found = board_card_for_pr_impl(&conn, "owner", "repo", 2).unwrap().unwrap();
        assert_eq!(found.card.id, "pr1");
        assert!(found.card.discarded_at.is_some());
        assert_eq!(found.inherited_meta.len(), 1);
        assert_eq!(found.inherited_meta[0].value, "from issue");
    }

    #[test]
    fn board_meta_add_allows_tag_only_note() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 1, "pr");

        let m = board_meta_add_impl(&conn, "pr1", "note", "", Some("urgent"), None, None).unwrap();
        assert_eq!(m.value, "");
        assert_eq!(m.tag.as_deref(), Some("urgent"));
    }

    #[test]
    fn board_meta_add_rejects_empty_note_without_tag() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 1, "pr");

        assert!(board_meta_add_impl(&conn, "pr1", "note", "", None, None, None).is_err());
        assert!(board_meta_add_impl(&conn, "pr1", "note", "  ", Some(""), None, None).is_err());
    }

    #[test]
    fn board_meta_add_rejects_empty_link() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 1, "pr");

        assert!(board_meta_add_impl(&conn, "pr1", "link", "", Some("urgent"), None, None).is_err());
    }

    #[test]
    fn tracked_card_id_finds_a_live_card() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 42, "pr");

        assert_eq!(tracked_card_id(&conn, "owner", "repo", 42).unwrap(), Some("pr1".to_string()));
    }

    #[test]
    fn tracked_card_id_ignores_discarded_and_archived_cards() {
        let conn = open_memory();
        seed_card(&conn, "pr1", "owner/repo", 42, "pr");
        seed_card(&conn, "pr2", "owner/repo", 43, "pr");
        conn.execute("UPDATE board_card SET discarded_at = 'now' WHERE id = 'pr1'", [])
            .unwrap();
        conn.execute("UPDATE board_card SET archived_at = 'now' WHERE id = 'pr2'", [])
            .unwrap();

        assert_eq!(tracked_card_id(&conn, "owner", "repo", 42).unwrap(), None);
        assert_eq!(tracked_card_id(&conn, "owner", "repo", 43).unwrap(), None);
    }
}
