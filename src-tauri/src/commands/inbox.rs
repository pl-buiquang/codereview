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

fn now() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// meta key/value helpers
// ---------------------------------------------------------------------------

fn get_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// items read/write helpers
// ---------------------------------------------------------------------------

fn upsert_item(conn: &Connection, it: &inbox::ItemInput, ts: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO items (
            id, type, number, repo, title, url,
            author_login, author_avatar, state, is_draft, body,
            latest_comment, latest_actor, updated_at,
            files_changed, additions, deletions, top_files_json, ci_state, review_decision,
            first_seen_at, last_refreshed
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20,
            ?21, ?21
         )
         ON CONFLICT(id) DO UPDATE SET
            type            = excluded.type,
            number          = excluded.number,
            repo            = excluded.repo,
            title           = excluded.title,
            url             = excluded.url,
            author_login    = excluded.author_login,
            author_avatar   = excluded.author_avatar,
            state           = excluded.state,
            is_draft        = excluded.is_draft,
            body            = excluded.body,
            latest_comment  = excluded.latest_comment,
            latest_actor    = excluded.latest_actor,
            updated_at      = excluded.updated_at,
            files_changed   = excluded.files_changed,
            additions       = excluded.additions,
            deletions       = excluded.deletions,
            top_files_json  = COALESCE(excluded.top_files_json, items.top_files_json),
            ci_state        = excluded.ci_state,
            review_decision = excluded.review_decision,
            last_refreshed  = excluded.last_refreshed",
        params![
            it.id,
            it.typ,
            it.number,
            it.repo,
            it.title,
            it.url,
            it.author_login,
            it.author_avatar,
            it.state,
            it.is_draft as i64,
            it.body,
            it.latest_comment,
            it.latest_actor,
            it.updated_at,
            it.files_changed,
            it.additions,
            it.deletions,
            it.top_files_json,
            it.ci_state,
            it.review_decision,
            ts,
        ],
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

fn reopen_item(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("UPDATE items SET closed_at = NULL WHERE id = ?1", params![id])?;
    Ok(())
}

fn mark_item_closed(conn: &Connection, id: &str, ts: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE items SET closed_at = ?1 WHERE id = ?2 AND closed_at IS NULL",
        params![ts, id],
    )?;
    Ok(())
}

/// `(updated_at, has_top_files)` for the given ids — used to skip re-enriching
/// PRs that haven't changed since last refresh.
fn existing_item_stats(conn: &Connection, ids: &[String]) -> AppResult<HashMap<String, (String, bool)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("SELECT id, updated_at, top_files_json FROM items WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        let id: String = r.get(0)?;
        let updated: String = r.get(1)?;
        let top: Option<String> = r.get(2)?;
        Ok((id, (updated, top.is_some())))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, v) = row?;
        map.insert(id, v);
    }
    Ok(map)
}

/// Ids currently in the inbox (tracked, not closed, with reasons) that were NOT
/// returned by this refresh — candidates for closed/merged detection.
fn stale_inbox_ids(conn: &Connection, current: &[String]) -> AppResult<Vec<String>> {
    if current.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT id FROM items
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

fn query_items(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> AppResult<Vec<ItemWithReasons>> {
    let items: Vec<ItemRow> = {
        let mut stmt = conn.prepare(sql)?;
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

fn cached_viewer(conn: &Connection) -> Option<ViewerInfo> {
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
#[tauri::command]
pub async fn refresh_inbox(db: State<'_, Db>) -> AppResult<RefreshResult> {
    let start = Instant::now();

    // Phase 0: cached viewer (brief lock).
    let cached = {
        let conn = db.0.lock().unwrap();
        cached_viewer(&conn)
    };

    // Phase 1: viewer (if stale) + all searches, unlocked.
    let viewer = match cached {
        Some(v) => v,
        None => {
            let v = inbox::fetch_viewer()?;
            let conn = db.0.lock().unwrap();
            set_meta(&conn, "viewer_login", &v.login)?;
            set_meta(&conn, "viewer_team_slugs_json", &serde_json::to_string(&v.team_slugs)?)?;
            set_meta(&conn, "viewer_cached_at", &now())?;
            v
        }
    };
    let scope = std::env::var("CODEREVIEW_SEARCH_SCOPE").ok();
    let merged = inbox::run_all_searches(&viewer, scope.as_deref());
    let merged_ids: Vec<String> = merged.iter().map(|r| r.node.id.clone()).collect();

    // Phase 2: which PRs need (re-)enrichment (brief lock).
    let existing = {
        let conn = db.0.lock().unwrap();
        existing_item_stats(&conn, &merged_ids)?
    };
    let to_enrich: Vec<inbox::EnrichTarget> = merged
        .iter()
        .filter(|r| r.node.typename == "PullRequest")
        .filter(|r| {
            !existing
                .get(&r.node.id)
                .map(|(updated, has)| updated == &r.node.updated_at && *has)
                .unwrap_or(false)
        })
        .map(|r| inbox::EnrichTarget {
            id: r.node.id.clone(),
            repo: r.node.repository.name_with_owner.clone(),
            number: r.node.number,
        })
        .collect();

    // Phase 3: enrich top files, unlocked.
    let top_files: HashMap<String, Option<String>> = inbox::enrich_all(to_enrich).into_iter().collect();

    // Phase 4: stale ids (brief lock) then refetch, unlocked.
    let stale_ids = {
        let conn = db.0.lock().unwrap();
        stale_inbox_ids(&conn, &merged_ids)?
    };
    let refetched = if stale_ids.is_empty() {
        HashMap::new()
    } else {
        inbox::refetch_nodes(&stale_ids)
    };

    // Phase 5: single write transaction.
    let ts = now();
    let mut conn = db.0.lock().unwrap();
    let tx = conn.transaction()?;
    for r in &merged {
        let top = top_files.get(&r.node.id).cloned().flatten();
        upsert_item(&tx, &inbox::node_to_input(&r.node, top), &ts)?;
        replace_reasons(&tx, &r.node.id, &r.reasons)?;
        reopen_item(&tx, &r.node.id)?;
    }
    let mut closed_count = 0i64;
    for id in &stale_ids {
        match refetched.get(id) {
            Some(node) => {
                upsert_item(&tx, &inbox::node_to_input(node, None), &ts)?;
                let closed = matches!(node.state.as_deref(), Some("CLOSED") | Some("MERGED")) || node.merged;
                if closed {
                    mark_item_closed(&tx, id, &ts)?;
                    closed_count += 1;
                }
            }
            None => {
                // Node deleted/inaccessible — treat as closed.
                mark_item_closed(&tx, id, &ts)?;
                closed_count += 1;
            }
        }
    }
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
pub fn list_inbox(db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    query_items(
        &conn,
        "SELECT * FROM items
         WHERE untracked_at IS NULL
           AND closed_at IS NULL
           AND id IN (SELECT item_id FROM item_reasons)
         ORDER BY updated_at DESC",
        &[],
    )
}

#[tauri::command]
pub fn list_archive(search: Option<String>, db: State<Db>) -> AppResult<Vec<ItemWithReasons>> {
    let conn = db.0.lock().unwrap();
    match search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(q) => {
            let like = format!("%{}%", q.to_lowercase());
            query_items(
                &conn,
                "SELECT * FROM items
                 WHERE untracked_at IS NOT NULL
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
            "SELECT * FROM items
             WHERE untracked_at IS NOT NULL
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
        "SELECT * FROM items
         WHERE closed_at IS NOT NULL
           AND untracked_at IS NULL
         ORDER BY closed_at DESC
         LIMIT 200",
        &[],
    )
}

fn set_item_timestamp(db: &State<Db>, id: &str, column: &str, value: Option<&str>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    // `column` is from a fixed internal set, never user input.
    let sql = format!("UPDATE items SET {column} = ?1 WHERE id = ?2");
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
    db: State<'_, Db>,
) -> AppResult<Review> {
    {
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE items SET engaged_at = ?1 WHERE id = ?2",
            params![now(), item_id],
        )?;
    }
    crate::commands::review::create_review_for_pr(owner, name, number, db)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxMeta {
    pub last_refresh_at: Option<String>,
    pub viewer_login: Option<String>,
}

#[tauri::command]
pub fn inbox_meta(db: State<Db>) -> AppResult<InboxMeta> {
    let conn = db.0.lock().unwrap();
    Ok(InboxMeta {
        last_refresh_at: get_meta(&conn, "last_refresh_at"),
        viewer_login: get_meta(&conn, "viewer_login"),
    })
}
