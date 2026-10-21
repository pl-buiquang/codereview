//! Tauri commands for the GitHub inbox: triage state on `inbox_entry`, read
//! through the mirror. Syncing it from GitHub lives in `crate::sync`.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::db::models::{ItemReason, ItemRow, ItemWithReasons, Review};
use crate::db::Db;
use crate::error::AppResult;
use crate::inbox::{self, ViewerInfo};
use crate::mirror::Node;

fn now() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// meta key/value helpers
// ---------------------------------------------------------------------------

pub(crate) fn get_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub(crate) fn set_meta(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// inbox_entry read/write helpers
// ---------------------------------------------------------------------------

/// An inbox entry joined with its mirrored facts, in the `ItemRow` shape. An
/// entry whose facts aren't mirrored yet renders as a placeholder until the
/// next sync fetches it. `stale` marks a New item that no search in the last
/// complete discovery still found.
const ITEM_VIEW: &str = "(
    SELECT e.id, e.type,
        COALESCE(p.number, i.number, 0) AS number,
        COALESCE(r.remote_owner || '/' || r.remote_name, '') AS repo,
        COALESCE(p.title, i.title, '(not synced yet)') AS title,
        COALESCE(p.url, i.url, '') AS url,
        COALESCE(p.author_login, i.author_login) AS author_login,
        u.avatar_url AS author_avatar,
        COALESCE(p.state, i.state) AS state,
        COALESCE(p.is_draft, 0) AS is_draft,
        COALESCE(p.body_snippet, i.body_snippet) AS body,
        COALESCE(p.latest_comment, i.latest_comment) AS latest_comment,
        COALESCE(p.latest_actor, i.latest_actor) AS latest_actor,
        COALESCE(p.gh_updated_at, i.gh_updated_at, e.first_seen_at) AS updated_at,
        p.changed_files AS files_changed, p.additions, p.deletions,
        LOWER(s.ci_state) AS ci_state,
        p.review_decision,
        e.state AS inbox_state, e.archive_cause, COALESCE(e.state_at, e.first_seen_at) AS state_at,
        e.first_seen_at, COALESCE(u.is_bot, 0) AS author_is_bot,
        CASE WHEN e.state = 'new'
              AND (SELECT value FROM meta WHERE key = 'last_complete_discovery_at') IS NOT NULL
              AND NOT EXISTS (SELECT 1 FROM item_reasons x WHERE x.item_id = e.id
                              AND x.last_seen_at >= (SELECT value FROM meta WHERE key = 'last_complete_discovery_at'))
             THEN 1 ELSE 0 END AS stale,
        COALESCE(p.fetched_at, i.fetched_at, e.first_seen_at) AS last_refreshed
    FROM inbox_entry e
    LEFT JOIN gh_pr p ON p.node_id = e.id
    LEFT JOIN gh_issue i ON i.node_id = e.id
    LEFT JOIN repository r ON r.id = COALESCE(p.repo_id, i.repo_id)
    LEFT JOIN gh_user u ON u.login = COALESCE(p.author_login, i.author_login)
    LEFT JOIN gh_pr_status s ON s.pr_id = e.id
)";

/// My own PRs are on the Board already, so they are never queued.
const NOT_MY_PR: &str = "NOT (type = 'pr' AND id IN (SELECT item_id FROM item_reasons WHERE reason = 'author'))";

/// Record a surfaced item. Its triage state is kept: a reopened item stays
/// where the user put it (only a new reason brings an archived one back).
pub(crate) fn upsert_entry(conn: &Connection, node: &Node, ts: &str) -> AppResult<()> {
    let typ = if node.is_pr() { "pr" } else { "issue" };
    conn.execute(
        "INSERT INTO inbox_entry (id, type, first_seen_at, state_at) VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET type = excluded.type",
        params![node.id, typ, ts],
    )?;
    Ok(())
}

/// Record the reasons a discovery found for an item. A reason that is new, or
/// that lapsed (not found since `prev_discovery`, the last complete one) and
/// is back, brings an archived item back to New. Reasons that stop matching
/// are kept so the item can show as stale.
pub(crate) fn record_reasons(
    conn: &Connection,
    item_id: &str,
    reasons: &[inbox::Reason],
    prev_discovery: Option<&str>,
    ts: &str,
) -> AppResult<()> {
    let mut gained = false;
    for r in reasons {
        let seen: Option<Option<String>> = conn
            .query_row(
                "SELECT last_seen_at FROM item_reasons WHERE item_id = ?1 AND reason = ?2 AND detail = ?3",
                params![item_id, r.reason, r.detail],
                |row| row.get(0),
            )
            .optional()?;
        let lapsed = match (&seen, prev_discovery) {
            (None, _) => true,
            (Some(last), Some(prev)) => last.as_deref().is_none_or(|l| l < prev),
            (Some(_), None) => false,
        };
        gained |= lapsed;
        conn.execute(
            "INSERT INTO item_reasons (item_id, reason, detail, last_seen_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(item_id, reason, detail) DO UPDATE SET last_seen_at = excluded.last_seen_at",
            params![item_id, r.reason, r.detail, ts],
        )?;
    }
    if gained {
        conn.execute(
            "UPDATE inbox_entry SET state = 'new', archive_cause = NULL, state_at = ?1
             WHERE id = ?2 AND state = 'archived'",
            params![ts, item_id],
        )?;
    }
    Ok(())
}

/// Move an item between triage states. A no-op for items the inbox never saw
/// (board-only cards).
pub(crate) fn set_inbox_state(conn: &Connection, id: &str, state: &str, cause: Option<&str>) -> AppResult<()> {
    conn.execute(
        "UPDATE inbox_entry SET state = ?1, archive_cause = ?2, state_at = ?3 WHERE id = ?4",
        params![state, cause, now(), id],
    )?;
    Ok(())
}

/// Archive a New item the inbox saw close on GitHub. Returns whether it changed.
pub(crate) fn archive_closed_item(conn: &Connection, id: &str, ts: &str) -> AppResult<bool> {
    Ok(conn.execute(
        "UPDATE inbox_entry SET state = 'archived', archive_cause = 'closed', state_at = ?1
         WHERE id = ?2 AND state = 'new'",
        params![ts, id],
    )? > 0)
}

/// Ids in the New queue: the entities an inbox sync probes.
pub(crate) fn active_inbox_ids(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM inbox_entry WHERE state = 'new' AND {NOT_MY_PR}"
    ))?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Attach each item's reasons (one extra grouped query, like the TS layer).
fn attach_reasons(conn: &Connection, items: Vec<ItemRow>) -> AppResult<Vec<ItemWithReasons>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT item_id, reason, detail FROM item_reasons WHERE item_id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((
            r.get::<_, String>(0)?,
            ItemReason {
                reason: r.get(1)?,
                detail: r.get(2)?,
            },
        ))
    })?;
    let mut by_id: HashMap<String, Vec<ItemReason>> = HashMap::new();
    for row in rows {
        let (item_id, reason) = row?;
        by_id.entry(item_id).or_default().push(reason);
    }
    Ok(items
        .into_iter()
        .map(|item| {
            let reasons = by_id.remove(&item.id).unwrap_or_default();
            ItemWithReasons { item, reasons }
        })
        .collect())
}

/// `where_order` filters and orders the `ITEM_VIEW` columns.
fn query_items(conn: &Connection, where_order: &str, params: &[&dyn rusqlite::ToSql]) -> AppResult<Vec<ItemWithReasons>> {
    let items: Vec<ItemRow> = {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {ITEM_VIEW} {where_order}"))?;
        let rows = stmt
            .query_map(params, ItemRow::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    attach_reasons(conn, items)
}

// ---------------------------------------------------------------------------
// viewer cache
// ---------------------------------------------------------------------------

pub(crate) fn cached_viewer(conn: &Connection) -> Option<ViewerInfo> {
    let cached_at = get_meta(conn, "viewer_cached_at")?;
    let login = get_meta(conn, "viewer_login")?;
    let teams_json = get_meta(conn, "viewer_team_slugs_json")?;
    let cached_ms = chrono::DateTime::parse_from_rfc3339(&cached_at).ok()?.timestamp_millis();
    let now_ms = Utc::now().timestamp_millis();
    if now_ms - cached_ms < inbox::VIEWER_TTL_MS {
        let team_slugs: Vec<String> = serde_json::from_str(&teams_json).ok()?;
        Some(ViewerInfo { login, team_slugs })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Cached viewer (24h TTL), fetching and persisting it on a cache miss. Shared
/// by every sync; the board only needs the login (not the team slugs).
pub(crate) fn cached_or_fetch_viewer(db: &Db) -> AppResult<ViewerInfo> {
    let cached = {
        let conn = db.0.lock().unwrap();
        cached_viewer(&conn)
    };
    match cached {
        Some(v) => Ok(v),
        None => {
            let v = inbox::fetch_viewer()?;
            let conn = db.0.lock().unwrap();
            set_meta(&conn, "viewer_login", &v.login)?;
            set_meta(&conn, "viewer_team_slugs_json", &serde_json::to_string(&v.team_slugs)?)?;
            set_meta(&conn, "viewer_cached_at", &now())?;
            Ok(v)
        }
    }
}

pub fn list_inbox_impl(conn: &Connection) -> AppResult<Vec<ItemWithReasons>> {
    query_items(
        conn,
        &format!(
            "WHERE inbox_state = 'new'
               AND {NOT_MY_PR}
               AND id IN (SELECT item_id FROM item_reasons)
             ORDER BY updated_at DESC"
        ),
        &[],
    )
}

#[tauri::command]
pub fn list_inbox(db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    list_inbox_impl(&conn)
}

/// Everything triaged: archived items and items put on the Board. `cause`
/// narrows to `dismissed` / `closed` / `discarded` / `tracked`.
pub fn list_inbox_archive_impl(
    conn: &Connection,
    search: Option<&str>,
    cause: Option<&str>,
    limit: i64,
) -> AppResult<Vec<ItemWithReasons>> {
    let mut clauses = vec!["inbox_state IN ('archived', 'tracked')".to_string()];
    let mut args: Vec<String> = Vec::new();
    match cause {
        Some("tracked") => clauses.push("inbox_state = 'tracked'".into()),
        Some(c) => {
            args.push(c.to_string());
            clauses.push(format!("archive_cause = ?{}", args.len()));
        }
        None => {}
    }
    if let Some(q) = search.map(str::trim).filter(|s| !s.is_empty()) {
        args.push(format!("%{}%", q.to_lowercase()));
        let n = args.len();
        clauses.push(format!(
            "(LOWER(title) LIKE ?{n} OR LOWER(repo) LIKE ?{n} OR LOWER(COALESCE(author_login, '')) LIKE ?{n})"
        ));
    }
    let sql = format!("WHERE {} ORDER BY state_at DESC LIMIT {}", clauses.join(" AND "), limit.max(1));
    let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
    query_items(conn, &sql, &params)
}

#[tauri::command]
pub fn list_inbox_archive(
    search: Option<String>,
    cause: Option<String>,
    limit: Option<i64>,
    db: State<Db>,
) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    list_inbox_archive_impl(&conn, search.as_deref(), cause.as_deref(), limit.unwrap_or(100))
}

/// The inbox entry id for `owner/name#number`, if the inbox has seen it.
pub fn find_item_id(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<String>> {
    let repo = format!("{owner}/{name}");
    conn.query_row(
        &format!("SELECT id FROM {ITEM_VIEW} WHERE repo = ?1 COLLATE NOCASE AND number = ?2"),
        params![repo, number],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn archive_item_impl(conn: &Connection, id: &str) -> AppResult<()> {
    set_inbox_state(conn, id, "archived", Some("dismissed"))
}

pub fn restore_item_impl(conn: &Connection, id: &str) -> AppResult<()> {
    set_inbox_state(conn, id, "new", None)
}

#[tauri::command]
pub fn archive_item(id: String, db: State<Db>) -> AppResult<()> {
    archive_item_impl(&db.0.lock().unwrap(), &id)
}

#[tauri::command]
pub fn restore_item(id: String, db: State<Db>) -> AppResult<()> {
    restore_item_impl(&db.0.lock().unwrap(), &id)
}

/// Open an inbox PR as a review: creates/reuses a review for the PR
/// (clone-less if the repo isn't added), then tracks the PR on the Board.
///
/// `async` so the `gh pr view` lookup runs off the UI thread.
#[tauri::command]
pub async fn open_pr_review(
    item_id: String,
    owner: String,
    name: String,
    number: i64,
    base_paths: Vec<String>,
    db: State<'_, Db>,
) -> AppResult<Review> {
    let review = crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, false)?;
    // Starting a review means working on it. Opening must not fail on this.
    if let Err(e) = crate::commands::board::board_track_impl(&db, &format!("{owner}/{name}#{number}")) {
        crate::logging::warn(&format!("[inbox.open_pr_review] tracking {item_id} failed: {e}"));
    }
    Ok(review)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxMeta {
    pub last_refresh_at: Option<String>,
    pub viewer_login: Option<String>,
}

pub fn inbox_meta_impl(conn: &Connection) -> AppResult<InboxMeta> {
    Ok(InboxMeta {
        last_refresh_at: get_meta(conn, "last_refresh_at"),
        viewer_login: get_meta(conn, "viewer_login"),
    })
}

#[tauri::command]
pub fn inbox_meta(db: State<Db>) -> AppResult<InboxMeta> {
    let conn = db.0.lock().unwrap();
    inbox_meta_impl(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn entry(conn: &Connection, id: &str, typ: &str) {
        conn.execute(
            "INSERT INTO inbox_entry (id, type, first_seen_at, state_at) VALUES (?1, ?2, 't0', 't0')",
            params![id, typ],
        )
        .unwrap();
    }

    fn reason(name: &str) -> inbox::Reason {
        inbox::Reason { reason: name.into(), detail: String::new() }
    }

    fn state_of(conn: &Connection, id: &str) -> String {
        conn.query_row("SELECT state FROM inbox_entry WHERE id = ?1", params![id], |r| r.get(0)).unwrap()
    }

    fn new_ids(conn: &Connection) -> Vec<String> {
        list_inbox_impl(conn).unwrap().into_iter().map(|i| i.item.id).collect()
    }

    #[test]
    fn my_own_prs_are_never_queued_but_my_issues_are() {
        let conn = open_memory();
        entry(&conn, "PR_mine", "pr");
        entry(&conn, "I_mine", "issue");
        entry(&conn, "PR_review", "pr");
        record_reasons(&conn, "PR_mine", &[reason("author")], None, "t1").unwrap();
        record_reasons(&conn, "I_mine", &[reason("author")], None, "t1").unwrap();
        record_reasons(&conn, "PR_review", &[reason("direct_review")], None, "t1").unwrap();
        let mut ids = new_ids(&conn);
        ids.sort();
        assert_eq!(ids, ["I_mine", "PR_review"]);
        assert_eq!(active_inbox_ids(&conn).unwrap().len(), 2, "my PRs are not probed either");
    }

    #[test]
    fn archived_item_returns_only_on_a_new_or_lapsed_reason() {
        let conn = open_memory();
        entry(&conn, "PR_1", "pr");
        record_reasons(&conn, "PR_1", &[reason("mention")], None, "t1").unwrap();
        archive_item_impl(&conn, "PR_1").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "archived");

        // Same reason found again in the next complete discovery: stays archived.
        record_reasons(&conn, "PR_1", &[reason("mention")], Some("t1"), "t2").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "archived");

        // A reason it didn't have before brings it back.
        record_reasons(&conn, "PR_1", &[reason("mention"), reason("direct_review")], Some("t2"), "t3").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "new");

        // A reason that lapsed (missed by the last complete discovery) and returned counts as new.
        archive_item_impl(&conn, "PR_1").unwrap();
        record_reasons(&conn, "PR_1", &[reason("mention")], Some("t3"), "t4").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "archived", "mention was seen at t3");
        record_reasons(&conn, "PR_1", &[reason("direct_review")], Some("t4"), "t5").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "new", "direct_review was last seen at t3, before t4");
    }

    #[test]
    fn tracked_items_are_not_resurfaced() {
        let conn = open_memory();
        entry(&conn, "PR_1", "pr");
        set_inbox_state(&conn, "PR_1", "tracked", None).unwrap();
        record_reasons(&conn, "PR_1", &[reason("direct_review")], None, "t1").unwrap();
        assert_eq!(state_of(&conn, "PR_1"), "tracked");
        assert!(new_ids(&conn).is_empty());
    }

    #[test]
    fn closed_items_archive_with_a_cause_and_only_from_new() {
        let conn = open_memory();
        entry(&conn, "PR_1", "pr");
        entry(&conn, "PR_2", "pr");
        record_reasons(&conn, "PR_1", &[reason("mention")], None, "t1").unwrap();
        record_reasons(&conn, "PR_2", &[reason("mention")], None, "t1").unwrap();
        set_inbox_state(&conn, "PR_2", "tracked", None).unwrap();
        assert!(archive_closed_item(&conn, "PR_1", "t2").unwrap());
        assert!(!archive_closed_item(&conn, "PR_2", "t2").unwrap());
        let archived = list_inbox_archive_impl(&conn, None, Some("closed"), 100).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].item.id, "PR_1");
        let tracked = list_inbox_archive_impl(&conn, None, Some("tracked"), 100).unwrap();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].item.id, "PR_2");
        assert_eq!(list_inbox_archive_impl(&conn, None, None, 100).unwrap().len(), 2);
    }

    #[test]
    fn archive_has_no_hidden_cap() {
        let conn = open_memory();
        for i in 0..250 {
            let id = format!("I_{i}");
            entry(&conn, &id, "issue");
            archive_item_impl(&conn, &id).unwrap();
        }
        assert_eq!(list_inbox_archive_impl(&conn, None, None, 1000).unwrap().len(), 250);
        assert_eq!(list_inbox_archive_impl(&conn, None, None, 100).unwrap().len(), 100);
        assert!(list_inbox_archive_impl(&conn, Some("nomatch"), None, 100).unwrap().is_empty());
    }

    #[test]
    fn item_is_stale_when_no_reason_was_seen_in_the_last_complete_discovery() {
        let conn = open_memory();
        entry(&conn, "I_1", "issue");
        entry(&conn, "I_2", "issue");
        record_reasons(&conn, "I_1", &[reason("assigned")], None, "2024-01-01T00:00:00Z").unwrap();
        record_reasons(&conn, "I_2", &[reason("assigned")], None, "2024-01-01T00:00:00Z").unwrap();
        let stale = |conn: &Connection| -> Vec<(String, bool)> {
            let mut v: Vec<_> = list_inbox_impl(conn).unwrap().into_iter().map(|i| (i.item.id, i.item.stale)).collect();
            v.sort();
            v
        };
        assert_eq!(stale(&conn), [("I_1".into(), false), ("I_2".into(), false)], "no complete discovery yet");

        // Next complete discovery finds only I_1.
        record_reasons(&conn, "I_1", &[reason("assigned")], Some("2024-01-01T00:00:00Z"), "2024-01-02T00:00:00Z").unwrap();
        set_meta(&conn, "last_complete_discovery_at", "2024-01-02T00:00:00Z").unwrap();
        assert_eq!(stale(&conn), [("I_1".into(), false), ("I_2".into(), true)]);
    }
}
