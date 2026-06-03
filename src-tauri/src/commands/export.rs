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
