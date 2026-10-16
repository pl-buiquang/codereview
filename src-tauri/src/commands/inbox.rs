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
/// next sync fetches it.
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
        CASE WHEN EXISTS (SELECT 1 FROM gh_pr_file f WHERE f.pr_id = e.id) THEN (
            SELECT json_group_array(json_object('path', f.filename, 'additions', f.additions,
                                                'deletions', f.deletions, 'changes', f.changes))
            FROM (SELECT * FROM gh_pr_file WHERE pr_id = e.id ORDER BY changes DESC, filename LIMIT 5) f
        ) END AS top_files_json,
        LOWER(s.ci_state) AS ci_state,
        p.review_decision,
        e.untracked_at, e.closed_at, e.engaged_at, e.first_seen_at,
        COALESCE(p.fetched_at, i.fetched_at, e.first_seen_at) AS last_refreshed
    FROM inbox_entry e
    LEFT JOIN gh_pr p ON p.node_id = e.id
    LEFT JOIN gh_issue i ON i.node_id = e.id
    LEFT JOIN repository r ON r.id = COALESCE(p.repo_id, i.repo_id)
    LEFT JOIN gh_user u ON u.login = COALESCE(p.author_login, i.author_login)
    LEFT JOIN gh_pr_status s ON s.pr_id = e.id
)";

/// Track a surfaced item, reopening it if the inbox had seen it close.
pub(crate) fn upsert_entry(conn: &Connection, node: &Node, ts: &str) -> AppResult<()> {
    let typ = if node.is_pr() { "pr" } else { "issue" };
    conn.execute(
        "INSERT INTO inbox_entry (id, type, first_seen_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, closed_at = NULL",
        params![node.id, typ, ts],
    )?;
    Ok(())
}

pub(crate) fn replace_reasons(conn: &Connection, item_id: &str, reasons: &[inbox::Reason]) -> AppResult<()> {
    conn.execute("DELETE FROM item_reasons WHERE item_id = ?1", params![item_id])?;
    for r in reasons {
        conn.execute(
            "INSERT OR IGNORE INTO item_reasons (item_id, reason, detail) VALUES (?1, ?2, ?3)",
            params![item_id, r.reason, r.detail],
        )?;
    }
    Ok(())
}

pub(crate) fn mark_item_closed(conn: &Connection, id: &str, ts: &str) -> AppResult<bool> {
    Ok(conn.execute(
        "UPDATE inbox_entry SET closed_at = ?1 WHERE id = ?2 AND closed_at IS NULL",
        params![ts, id],
    )? > 0)
}

/// Ids currently in the inbox (tracked, not closed, with reasons): the
/// entities an inbox sync probes.
pub(crate) fn active_inbox_ids(conn: &Connection) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM inbox_entry
         WHERE untracked_at IS NULL
           AND closed_at IS NULL
           AND id IN (SELECT item_id FROM item_reasons)",
    )?;
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
        "WHERE untracked_at IS NULL
           AND closed_at IS NULL
           AND id IN (SELECT item_id FROM item_reasons)
         ORDER BY updated_at DESC",
        &[],
    )
}

#[tauri::command]
pub fn list_inbox(db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    list_inbox_impl(&conn)
}

#[tauri::command]
pub fn list_archive(search: Option<String>, db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    match search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(q) => {
            let like = format!("%{}%", q.to_lowercase());
            query_items(
                &conn,
                "WHERE untracked_at IS NOT NULL
                   AND (LOWER(title) LIKE ?1
                        OR LOWER(repo) LIKE ?1
                        OR LOWER(COALESCE(author_login, '')) LIKE ?1)
                 ORDER BY untracked_at DESC
                 LIMIT 200",
                &[&like],
            )
        }
        None => query_items(
            &conn,
            "WHERE untracked_at IS NOT NULL
             ORDER BY untracked_at DESC
             LIMIT 200",
            &[],
        ),
    }
}

#[tauri::command]
pub fn list_closed(db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    query_items(
        &conn,
        "WHERE closed_at IS NOT NULL
           AND untracked_at IS NULL
         ORDER BY closed_at DESC
         LIMIT 200",
        &[],
    )
}

fn set_item_timestamp(db: &State<Db>, id: &str, column: &str, value: Option<&str>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    // `column` is from a fixed internal set, never user input.
    let sql = format!("UPDATE inbox_entry SET {column} = ?1 WHERE id = ?2");
    conn.execute(&sql, params![value, id])?;
    Ok(())
}

#[tauri::command]
pub fn engage_item(id: String, db: State<Db>) -> AppResult<()> {
    set_item_timestamp(&db, &id, "engaged_at", Some(&now()))
}

#[tauri::command]
pub fn unengage_item(id: String, db: State<Db>) -> AppResult<()> {
    set_item_timestamp(&db, &id, "engaged_at", None)
}

#[tauri::command]
pub fn untrack_item(id: String, db: State<Db>) -> AppResult<()> {
    set_item_timestamp(&db, &id, "untracked_at", Some(&now()))
}

#[tauri::command]
pub fn retrack_item(id: String, db: State<Db>) -> AppResult<()> {
    set_item_timestamp(&db, &id, "untracked_at", None)
}

/// Open an inbox PR as a review: marks the item engaged (visited), then
/// creates/reuses a review for the PR (clone-less if the repo isn't added).
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
    {
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE inbox_entry SET engaged_at = ?1 WHERE id = ?2",
            params![now(), item_id],
        )?;
    }
    crate::commands::review::create_review_for_pr_impl(&db, &owner, &name, number, &base_paths, false)
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
