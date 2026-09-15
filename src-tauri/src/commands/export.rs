use std::fs;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use tauri::State;

use crate::commands::review::{load_detail, repo_label};
use crate::db::models::Review;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::export;

pub fn render(conn: &rusqlite::Connection, review_id: i64, format: &str) -> AppResult<String> {
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
pub fn export_review_impl(
    conn: &rusqlite::Connection,
    review_id: i64,
    dest_path: &str,
    format: &str,
) -> AppResult<()> {
    let content = render(conn, review_id, format)?;
    fs::write(dest_path, content)?;
    conn.execute(
        "UPDATE review SET last_exported_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), review_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn export_review(
    review_id: i64,
    dest_path: String,
    format: String,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    export_review_impl(&conn, review_id, &dest_path, &format)
}

// ---- import ----

#[derive(Deserialize)]
struct ImportReply {
    body: String,
    #[allow(dead_code)]
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct ImportComment {
    file: String,
    #[serde(default = "default_subject_type")]
    subject_type: String,
    #[serde(default = "default_origin")]
    origin: String,
    #[serde(default = "default_side")]
    side: String,
    #[serde(default)]
    line: i64,
    #[serde(default)]
    start_line: Option<i64>,
    #[serde(default)]
    diff_hunk: Option<String>,
    body: String,
    #[serde(default)]
    resolved_at: Option<String>,
    #[serde(default)]
    replies: Vec<ImportReply>,
}

fn default_subject_type() -> String {
    "line".into()
}
fn default_origin() -> String {
    "diff".into()
}
fn default_side() -> String {
    "RIGHT".into()
}

#[derive(Deserialize)]
struct ImportPayload {
    title: String,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    github_pr_number: Option<i64>,
    #[serde(default)]
    base_ref: Option<String>,
    #[serde(default)]
    head_ref: Option<String>,
    #[serde(default)]
    base_sha: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    comments: Vec<ImportComment>,
}

fn find_or_create_repo(conn: &Connection, label: &str) -> AppResult<i64> {
    let (owner, name) = label
        .split_once('/')
        .unwrap_or(("", label));
    if !owner.is_empty() && !name.is_empty() {
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM repository WHERE remote_owner = ?1 AND remote_name = ?2 LIMIT 1",
                params![owner, name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let sentinel = format!("github:{owner}/{name}");
        let ts = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO repository (path, remote_owner, remote_name, added_at) VALUES (?1, ?2, ?3, ?4)",
            params![sentinel, owner, name, ts],
        )?;
        return Ok(conn.last_insert_rowid());
    }
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM repository WHERE path = ?1 LIMIT 1",
            params![label],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let ts = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO repository (path, added_at) VALUES (?1, ?2)",
        params![label, ts],
    )?;
    Ok(conn.last_insert_rowid())
}

fn import_review_impl(conn: &Connection, payload: ImportPayload) -> AppResult<Review> {
    let ts = Utc::now().to_rfc3339();

    let repo_id = find_or_create_repo(conn, payload.repo.as_deref().unwrap_or("imported"))?;

    let kind = payload.kind.as_deref().unwrap_or("local");
    if kind != "local" && kind != "github_pr" {
        return Err(AppError::Other(format!("invalid target kind: {kind}")));
    }
    let base_ref = payload.base_ref.as_deref().unwrap_or("unknown");
    let head_ref = payload.head_ref.as_deref().unwrap_or("unknown");

    conn.execute(
        "INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, base_sha, head_sha, three_dot, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)",
        params![
            repo_id,
            kind,
            payload.github_pr_number,
            payload.title,
            base_ref,
            head_ref,
            payload.base_sha,
            payload.head_sha,
            ts,
        ],
    )?;
    let target_id = conn.last_insert_rowid();

    let event = payload.verdict.as_deref().and_then(|v| {
        match v {
            "approve" | "request_changes" | "comment" => Some(v),
            _ => None,
        }
    });
    let body = payload.summary.as_deref().unwrap_or("");

    conn.execute(
        "INSERT INTO review (target_id, body, event, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'draft', ?4, ?4)",
        params![target_id, body, event, ts],
    )?;
    let review_id = conn.last_insert_rowid();

    for c in &payload.comments {
        conn.execute(
            "INSERT INTO comment
                (review_id, file_path, subject_type, origin, side, line, start_line,
                 diff_hunk, body, resolved_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                review_id,
                c.file,
                c.subject_type,
                c.origin,
                c.side,
                c.line,
                c.start_line,
                c.diff_hunk,
                c.body,
                c.resolved_at,
                ts,
            ],
        )?;
        let root_id = conn.last_insert_rowid();
        for r in &c.replies {
            conn.execute(
                "INSERT INTO comment
                    (review_id, file_path, subject_type, origin, side, line, start_line,
                     diff_hunk, body, parent_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![
                    review_id,
                    c.file,
                    c.subject_type,
                    c.origin,
                    c.side,
                    c.line,
                    c.start_line,
                    c.diff_hunk,
                    r.body,
                    root_id,
                    ts,
                ],
            )?;
        }
    }

    conn.query_row(
        "SELECT * FROM review WHERE id = ?1",
        params![review_id],
        Review::from_row,
    )
    .map_err(Into::into)
}

/// Import a review from a JSON file (the same format `export_review` produces,
/// plus optional extra fields). Creates a new draft review with all comments.
#[tauri::command]
pub fn import_review(src_path: String, db: State<Db>) -> AppResult<Review> {
    let content = fs::read_to_string(&src_path)?;
    let payload: ImportPayload =
        serde_json::from_str(&content).map_err(|e| AppError::Other(format!("invalid JSON: {e}")))?;
    let conn = db.0.lock().unwrap();
    import_review_impl(&conn, payload)
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

    // ---- import tests ----

    fn minimal_import() -> ImportPayload {
        serde_json::from_str(
            r#"{
                "title": "Improve thing",
                "repo": "owner/name",
                "kind": "github_pr",
                "github_pr_number": 42,
                "base_ref": "main",
                "head_ref": "feature",
                "base_sha": "abcd1234",
                "head_sha": "5678efgh",
                "verdict": "approve",
                "summary": "Looks good overall.",
                "comments": [
                    {
                        "file": "src/main.rs",
                        "side": "RIGHT",
                        "line": 5,
                        "start_line": 3,
                        "diff_hunk": "@@ -1,2 +1,3 @@\n line1\n+line3",
                        "body": "Consider renaming this.",
                        "replies": [
                            { "body": "Good point", "created_at": "2026-01-01T00:00:00Z" }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn import_creates_review_with_comments() {
        let conn = open_memory();
        let review = import_review_impl(&conn, minimal_import()).unwrap();

        assert_eq!(review.status, "draft");
        assert_eq!(review.body, "Looks good overall.");
        assert_eq!(review.event.as_deref(), Some("approve"));

        let comment_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comment WHERE review_id = ?1",
                params![review.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(comment_count, 2, "root + 1 reply");

        let root_body: String = conn
            .query_row(
                "SELECT body FROM comment WHERE review_id = ?1 AND parent_id IS NULL",
                params![review.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(root_body, "Consider renaming this.");

        let reply_body: String = conn
            .query_row(
                "SELECT body FROM comment WHERE review_id = ?1 AND parent_id IS NOT NULL",
                params![review.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reply_body, "Good point");
    }

    #[test]
    fn import_reuses_existing_repo() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (path, remote_owner, remote_name, added_at)
             VALUES ('github:owner/name', 'owner', 'name', 'now')",
            [],
        )
        .unwrap();
        let existing_id = conn.last_insert_rowid();

        let review = import_review_impl(&conn, minimal_import()).unwrap();
        let target_repo: i64 = conn
            .query_row(
                "SELECT t.repo_id FROM target t JOIN review r ON r.target_id = t.id WHERE r.id = ?1",
                params![review.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(target_repo, existing_id);
    }

    #[test]
    fn import_minimal_json_with_defaults() {
        let conn = open_memory();
        let payload: ImportPayload =
            serde_json::from_str(r#"{ "title": "bare review" }"#).unwrap();
        let review = import_review_impl(&conn, payload).unwrap();
        assert_eq!(review.body, "");
        assert_eq!(review.event, None);
    }

    #[test]
    fn import_roundtrip_matches_export() {
        let conn = open_memory();
        let review = import_review_impl(&conn, minimal_import()).unwrap();
        let exported = render(&conn, review.id, "json").unwrap();
        let v: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(v["title"], "Improve thing");
        assert_eq!(v["repo"], "owner/name");
        assert_eq!(v["verdict"], "approve");
        assert_eq!(v["comments"][0]["file"], "src/main.rs");
        assert_eq!(v["comments"][0]["line"], 5);
        assert_eq!(v["comments"][0]["replies"][0]["body"], "Good point");
    }
}
