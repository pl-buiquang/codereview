use std::fs;

use chrono::Utc;
use rusqlite::params;
use tauri::State;

use crate::commands::review::{load_detail, repo_label};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::export;

fn render(conn: &rusqlite::Connection, review_id: i64, format: &str) -> AppResult<String> {
    let detail = load_detail(conn, review_id)?;
    let label = repo_label(conn, detail.target.repo_id)?;
    match format {
        "json" => Ok(export::render_json(&detail, &label)),
        "markdown" | "md" => Ok(export::render_markdown(&detail, &label)),
        other => Err(AppError::Other(format!("unknown export format: {other}"))),
    }
}

/// Render the export to a string for preview (no file written, no state change).
#[tauri::command]
pub fn preview_review(review_id: i64, format: String, db: State<Db>) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    render(&conn, review_id, &format)
}

/// Write the export to `dest_path` and stamp `last_exported_at`. Repeatable;
/// never changes the review's published/draft status.
#[tauri::command]
pub fn export_review(
    review_id: i64,
    dest_path: String,
    format: String,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    let content = render(&conn, review_id, &format)?;
    fs::write(&dest_path, content)?;
    conn.execute(
        "UPDATE review SET last_exported_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), review_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use crate::error::AppError;

    /// Seed a minimal repo → local target → draft review, returning the review id.
    fn seed_review(conn: &rusqlite::Connection) -> i64 {
        conn.execute(
            "INSERT INTO repository (path, remote_owner, remote_name, added_at)
             VALUES ('/repo', 'owner', 'name', 'now')",
            [],
        )
        .unwrap();
        let repo_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (?1, 'local', 'main...feature', 'main', 'feature', 'now')",
            params![repo_id],
        )
        .unwrap();
        let target_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (?1, 'A summary', 'draft', 'now', 'now')",
            params![target_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn render_markdown_format() {
        let conn = open_memory();
        let id = seed_review(&conn);
        let out = render(&conn, id, "markdown").unwrap();
        assert!(out.starts_with("# Review: main...feature"));
        assert!(out.contains("Repo: owner/name"));
    }

    #[test]
    fn render_md_alias_works() {
        let conn = open_memory();
        let id = seed_review(&conn);
        assert!(render(&conn, id, "md").is_ok());
    }

    #[test]
    fn render_json_format() {
        let conn = open_memory();
        let id = seed_review(&conn);
        let out = render(&conn, id, "json").unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["repo"], "owner/name");
        assert_eq!(v["summary"], "A summary");
    }

    #[test]
    fn render_unknown_format_errors() {
        let conn = open_memory();
        let id = seed_review(&conn);
        let err = render(&conn, id, "yaml").unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }
}
