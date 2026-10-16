//! Tauri commands for the GitHub inbox. These own the DB and orchestrate the
//! pure pipeline in `crate::inbox`. The refresh deliberately performs all `gh`
//! network I/O OUTSIDE the DB lock and writes the results in one transaction, so
//! the single `Mutex<Connection>` is never held across a subprocess.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::db::models::{ItemReason, ItemRow, ItemWithReasons, Review};
use crate::db::Db;
use crate::error::AppResult;
use crate::inbox::{self, ViewerInfo};
use crate::mirror::{self, Node};

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
fn upsert_entry(conn: &Connection, node: &Node, ts: &str) -> AppResult<()> {
    let typ = if node.is_pr() { "pr" } else { "issue" };
    conn.execute(
        "INSERT INTO inbox_entry (id, type, first_seen_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET type = excluded.type, closed_at = NULL",
        params![node.id, typ, ts],
    )?;
    Ok(())
}

fn replace_reasons(conn: &Connection, item_id: &str, reasons: &[inbox::Reason]) -> AppResult<()> {
    conn.execute("DELETE FROM item_reasons WHERE item_id = ?1", params![item_id])?;
    for r in reasons {
        conn.execute(
            "INSERT OR IGNORE INTO item_reasons (item_id, reason, detail) VALUES (?1, ?2, ?3)",
            params![item_id, r.reason, r.detail],
        )?;
    }
    Ok(())
}

fn mark_item_closed(conn: &Connection, id: &str, ts: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE inbox_entry SET closed_at = ?1 WHERE id = ?2 AND closed_at IS NULL",
        params![ts, id],
    )?;
    Ok(())
}

/// What the mirror knows of each PR's files facet: `(gh_updated_at, files
/// head_sha)`, the latter `Some(None)` for files without a head key.
type FilesKey = (Option<String>, Option<Option<String>>);

fn existing_files_keys(conn: &Connection, ids: &[String]) -> AppResult<HashMap<String, FilesKey>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT p.node_id, p.gh_updated_at,
                EXISTS (SELECT 1 FROM gh_pr_file f WHERE f.pr_id = p.node_id),
                (SELECT MAX(f.head_sha) FROM gh_pr_file f WHERE f.pr_id = p.node_id)
         FROM gh_pr p WHERE p.node_id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        let has_files: bool = r.get(2)?;
        let head: Option<String> = r.get(3)?;
        Ok((r.get::<_, String>(0)?, (r.get(1)?, has_files.then_some(head))))
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

/// The files facet is fresh when fetched at the node's head; rows backfilled
/// without a head key stay fresh until the PR changes.
fn files_need_fetch(node: &Node, key: Option<&FilesKey>) -> bool {
    match key {
        Some((_, Some(Some(head)))) => node.head_ref_oid.as_deref() != Some(head.as_str()),
        Some((updated_at, Some(None))) => updated_at.as_deref() != Some(node.updated_at.as_str()),
        _ => true,
    }
}

/// Ids currently in the inbox (tracked, not closed, with reasons) that were NOT
/// returned by this refresh — candidates for closed/merged detection.
fn stale_inbox_ids(conn: &Connection, current: &[String]) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM inbox_entry
         WHERE untracked_at IS NULL
           AND closed_at IS NULL
           AND id IN (SELECT item_id FROM item_reasons)",
    )?;
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let set: HashSet<&str> = current.iter().map(String::as_str).collect();
    Ok(ids.into_iter().filter(|id| !set.contains(id.as_str())).collect())
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    pub viewer_login: String,
    pub item_count: i64,
    pub closed_count: i64,
    pub duration_ms: i64,
}

/// Refresh the inbox from GitHub. Network I/O happens unlocked; the DB lock is
/// taken only for short reads and the final write transaction.
///
/// Declared `async` so Tauri runs it off the main (UI) thread — the body is
/// synchronous and contains no `.await`, but the `gh` fan-out is slow and would
/// otherwise freeze the webview.
/// Cached viewer (24h TTL), fetching and persisting it on a cache miss. Shared
/// with the board refresh, which only needs the login (not the team slugs).
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

pub fn refresh_inbox_impl(db: &Db) -> AppResult<RefreshResult> {
    let start = Instant::now();

    let viewer = cached_or_fetch_viewer(db)?;
    let scope = std::env::var("CODEREVIEW_SEARCH_SCOPE").ok();
    let merged = inbox::run_all_searches(&viewer, scope.as_deref());
    let merged_ids: Vec<String> = merged.iter().map(|r| r.node.id.clone()).collect();

    let (files_keys, stale_ids) = {
        let conn = db.0.lock().unwrap();
        (existing_files_keys(&conn, &merged_ids)?, stale_inbox_ids(&conn, &merged_ids)?)
    };
    let to_enrich: Vec<inbox::EnrichTarget> = merged
        .iter()
        .filter(|r| r.node.is_pr() && files_need_fetch(&r.node, files_keys.get(&r.node.id)))
        .filter_map(|r| {
            Some(inbox::EnrichTarget {
                id: r.node.id.clone(),
                repo: r.node.repo()?.to_string(),
                number: r.node.number,
                head_sha: r.node.head_ref_oid.clone(),
            })
        })
        .collect();
    let files = inbox::enrich_all(to_enrich);
    let refetched = if stale_ids.is_empty() {
        HashMap::new()
    } else {
        mirror::fetch::refetch(&stale_ids)
    };

    let ts = now();
    let mut conn = db.0.lock().unwrap();
    let tx = conn.transaction()?;
    for r in &merged {
        mirror::store::upsert_node(&tx, &r.node, &ts)?;
        upsert_entry(&tx, &r.node, &ts)?;
        replace_reasons(&tx, &r.node.id, &r.reasons)?;
    }
    for (target, files) in &files {
        if let Some(files) = files {
            mirror::store::upsert_files(&tx, &target.id, target.head_sha.as_deref(), files)?;
        }
    }
    let mut closed_count = 0i64;
    for id in &stale_ids {
        let closed = match refetched.get(id) {
            Some(Some(node)) => {
                mirror::store::upsert_node(&tx, node, &ts)?;
                node.is_closed()
            }
            Some(None) => true,
            // Its batch failed: state unknown, try again next sync.
            None => false,
        };
        if closed {
            mark_item_closed(&tx, id, &ts)?;
            closed_count += 1;
        }
    }
    mirror::store::gc(&tx)?;
    set_meta(&tx, "last_refresh_at", &ts)?;
    tx.commit()?;

    Ok(RefreshResult {
        viewer_login: viewer.login,
        item_count: merged.len() as i64,
        closed_count,
        duration_ms: start.elapsed().as_millis() as i64,
    })
}

#[tauri::command]
pub async fn refresh_inbox(db: State<'_, Db>) -> AppResult<RefreshResult> {
    refresh_inbox_impl(&db)
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
/// `async` so the `gh pr view` lookup runs off the UI thread (see `refresh_inbox`).
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
