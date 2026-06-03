use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::db::models::{Comment, Review, ReviewDetail, ReviewSummary, Target};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::gh;
use crate::git;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn get_target(conn: &Connection, id: i64) -> AppResult<Target> {
    conn.query_row("SELECT * FROM target WHERE id = ?1", params![id], Target::from_row)
        .map_err(Into::into)
}

fn get_review_row(conn: &Connection, id: i64) -> AppResult<Review> {
    conn.query_row("SELECT * FROM review WHERE id = ?1", params![id], Review::from_row)
        .map_err(Into::into)
}

fn get_comment(conn: &Connection, id: i64) -> AppResult<Comment> {
    conn.query_row("SELECT * FROM comment WHERE id = ?1", params![id], Comment::from_row)
        .map_err(Into::into)
}

fn review_status(conn: &Connection, review_id: i64) -> AppResult<String> {
    conn.query_row(
        "SELECT status FROM review WHERE id = ?1",
        params![review_id],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

fn ensure_draft(conn: &Connection, review_id: i64) -> AppResult<()> {
    if review_status(conn, review_id)? == "published" {
        return Err(AppError::Other(
            "this review is published and can no longer be edited".into(),
        ));
    }
    Ok(())
}

/// Reuse one `target` per (repo, base, head) comparison so several reviews can
/// share it. Refreshes the resolved SHAs each time the comparison is opened.
fn get_or_create_local_target(
    conn: &Connection,
    repo_id: i64,
    repo_path: &str,
    base_ref: &str,
    head_ref: &str,
    three_dot: bool,
) -> AppResult<Target> {
    let base_sha = git::rev_parse(Path::new(repo_path), base_ref).ok();
    let head_sha = git::rev_parse(Path::new(repo_path), head_ref).ok();

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM target
             WHERE repo_id = ?1 AND kind = 'local' AND base_ref = ?2 AND head_ref = ?3",
            params![repo_id, base_ref, head_ref],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(id) = existing {
        conn.execute(
            "UPDATE target SET base_sha = ?1, head_sha = ?2, three_dot = ?3 WHERE id = ?4",
            params![base_sha, head_sha, three_dot as i64, id],
        )?;
        return get_target(conn, id);
    }

    let title = format!("{base_ref}...{head_ref}");
    conn.execute(
        "INSERT INTO target
            (repo_id, kind, github_pr_number, title, base_ref, head_ref, base_sha, head_sha, three_dot, created_at)
         VALUES (?1, 'local', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![repo_id, title, base_ref, head_ref, base_sha, head_sha, three_dot as i64, now()],
    )?;
    get_target(conn, conn.last_insert_rowid())
}

/// Reuse one `target` per GitHub PR number, refreshing title/refs/head sha.
pub(crate) fn get_or_create_pr_target(
    conn: &Connection,
    repo_id: i64,
    pr_number: i64,
    info: &gh::PrInfo,
) -> AppResult<Target> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM target WHERE repo_id = ?1 AND kind = 'github_pr' AND github_pr_number = ?2",
            params![repo_id, pr_number],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(id) = existing {
        conn.execute(
            "UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, head_sha = ?4 WHERE id = ?5",
            params![info.title, info.base_ref, info.head_ref, info.head_sha, id],
        )?;
        return get_target(conn, id);
    }

    conn.execute(
        "INSERT INTO target
            (repo_id, kind, github_pr_number, title, base_ref, head_ref, base_sha, head_sha, three_dot, created_at)
         VALUES (?1, 'github_pr', ?2, ?3, ?4, ?5, NULL, ?6, 1, ?7)",
        params![
            repo_id,
            pr_number,
            info.title,
            info.base_ref,
            info.head_ref,
            info.head_sha,
            now()
        ],
    )?;
    get_target(conn, conn.last_insert_rowid())
}

fn new_review_for_target(conn: &Connection, target_id: i64) -> AppResult<Review> {
    let ts = now();
    conn.execute(
        "INSERT INTO review (target_id, body, status, created_at, updated_at)
         VALUES (?1, '', 'draft', ?2, ?2)",
        params![target_id, ts],
    )?;
    get_review_row(conn, conn.last_insert_rowid())
}

/// Start a fresh draft review for a local virtual PR (creating/reusing its target).
#[tauri::command]
pub fn create_review(
    repo_id: i64,
    repo_path: String,
    base_ref: String,
    head_ref: String,
    three_dot: bool,
    db: State<Db>,
) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let target =
        get_or_create_local_target(&conn, repo_id, &repo_path, &base_ref, &head_ref, three_dot)?;
    new_review_for_target(&conn, target.id)
}

/// Start a fresh draft review against a real GitHub PR (creating/reusing its target).
#[tauri::command]
pub fn create_review_for_pr(
    repo_id: i64,
    repo_path: String,
    pr_number: i64,
    db: State<Db>,
) -> AppResult<Review> {
    let info = gh::pr_view(std::path::Path::new(&repo_path), pr_number)?;
    let conn = db.0.lock().unwrap();
    let target = get_or_create_pr_target(&conn, repo_id, pr_number, &info)?;
    new_review_for_target(&conn, target.id)
}

/// The diff for a review's target: GitHub PR diff via `gh`, or a local
/// base...head git diff for virtual PRs.
#[tauri::command]
pub fn review_diff(review_id: i64, db: State<Db>) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let path = std::path::Path::new(&detail.repo_path);
    match detail.target.kind.as_str() {
        "github_pr" => {
            let number = detail
                .target
                .github_pr_number
                .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
            gh::pr_diff(path, number)
        }
        _ => git::diff(
            path,
            &detail.target.base_ref,
            &detail.target.head_ref,
            detail.target.three_dot,
        ),
    }
}

/// All reviews (optionally filtered to one repo), newest first, for the Reviews list.
#[tauri::command]
pub fn list_reviews(repo_id: Option<i64>, db: State<Db>) -> AppResult<Vec<ReviewSummary>> {
    let conn = db.0.lock().unwrap();

    let reviews: Vec<Review> = {
        if let Some(rid) = repo_id {
            let mut stmt = conn.prepare(
                "SELECT rv.* FROM review rv JOIN target t ON rv.target_id = t.id
                 WHERE t.repo_id = ?1 ORDER BY rv.updated_at DESC",
            )?;
            let rows = stmt
                .query_map(params![rid], Review::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            rows
        } else {
            let mut stmt = conn.prepare("SELECT * FROM review ORDER BY updated_at DESC")?;
            let rows = stmt
                .query_map([], Review::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            rows
        }
    };

    let mut out = Vec::with_capacity(reviews.len());
    for review in reviews {
        let target = get_target(&conn, review.target_id)?;
        let (repo_id_v, repo_label): (i64, String) = conn.query_row(
            "SELECT id, COALESCE(remote_owner || '/' || remote_name, path) FROM repository WHERE id = ?1",
            params![target.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let comment_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM comment WHERE review_id = ?1",
            params![review.id],
            |r| r.get(0),
        )?;
        out.push(ReviewSummary {
            review,
            target,
            repo_id: repo_id_v,
            repo_label,
            comment_count,
        });
    }
    Ok(out)
}

/// Human-friendly repo label: `owner/name` if known, else the path.
pub(crate) fn repo_label(conn: &Connection, repo_id: i64) -> AppResult<String> {
    conn.query_row(
        "SELECT COALESCE(remote_owner || '/' || remote_name, path) FROM repository WHERE id = ?1",
        params![repo_id],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// Load the full review state — shared by the get_review command and exporters.
pub(crate) fn load_detail(conn: &Connection, review_id: i64) -> AppResult<ReviewDetail> {
    let review = get_review_row(conn, review_id)?;
    let target = get_target(conn, review.target_id)?;
    let repo_path: String = conn.query_row(
        "SELECT path FROM repository WHERE id = ?1",
        params![target.repo_id],
        |r| r.get(0),
    )?;
    let comments: Vec<Comment> = {
        let mut stmt = conn.prepare(
            "SELECT * FROM comment WHERE review_id = ?1 ORDER BY file_path, line, created_at",
        )?;
        let rows = stmt
            .query_map(params![review_id], Comment::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    Ok(ReviewDetail {
        review,
        target,
        repo_path,
        comments,
    })
}

/// Full review state (review + target + repo path + comments) for the Review screen.
#[tauri::command]
pub fn get_review(review_id: i64, db: State<Db>) -> AppResult<ReviewDetail> {
    let conn = db.0.lock().unwrap();
    load_detail(&conn, review_id)
}

/// Autosave the review summary and/or verdict. Pass `event = ""` to clear the verdict.
#[tauri::command]
pub fn update_review(
    review_id: i64,
    body: Option<String>,
    event: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    if let Some(b) = body {
        conn.execute(
            "UPDATE review SET body = ?1, updated_at = ?2 WHERE id = ?3",
            params![b, now(), review_id],
        )?;
    }
    if let Some(e) = event {
        let ev = if e.is_empty() { None } else { Some(e) };
        conn.execute(
            "UPDATE review SET event = ?1, updated_at = ?2 WHERE id = ?3",
            params![ev, now(), review_id],
        )?;
    }
    Ok(())
}

/// Publish a draft review to its GitHub PR via the line-based reviews API, then
/// lock it (published reviews can't be edited or re-published). Returns the
/// updated review.
#[tauri::command]
pub fn publish_review(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    if detail.review.status == "published" {
        return Err(AppError::Other("this review is already published".into()));
    }
    if detail.target.kind != "github_pr" {
        return Err(AppError::Other(
            "only GitHub PR reviews can be published; export local reviews instead".into(),
        ));
    }
    let number = detail
        .target
        .github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name): (Option<String>, Option<String>) = conn.query_row(
        "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
        params![detail.target.repo_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (owner, name) = match (owner, name) {
        (Some(o), Some(n)) => (o, n),
        _ => return Err(AppError::Other("repository has no GitHub remote".into())),
    };

    let event = match detail.review.event.as_deref() {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    };

    let comments: Vec<serde_json::Value> = detail
        .comments
        .iter()
        .map(|c| {
            let mut obj = serde_json::json!({
                "path": c.file_path,
                "side": c.side,
                "line": c.line,
                "body": c.body,
            });
            if let Some(start) = c.start_line {
                if start != c.line {
                    obj["start_line"] = serde_json::json!(start);
                    obj["start_side"] = serde_json::json!(c.side);
                }
            }
            obj
        })
        .collect();

    let mut payload = serde_json::json!({ "event": event, "comments": comments });
    if !detail.review.body.trim().is_empty() {
        payload["body"] = serde_json::json!(detail.review.body);
    }
    if let Some(sha) = &detail.target.head_sha {
        payload["commit_id"] = serde_json::json!(sha);
    }

    let gh_id = gh::post_review(
        std::path::Path::new(&detail.repo_path),
        &owner,
        &name,
        number,
        &payload.to_string(),
    )?;

    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'published', published_at = ?1, github_review_id = ?2, updated_at = ?1
         WHERE id = ?3",
        params![ts, gh_id, review_id],
    )?;
    get_review_row(&conn, review_id)
}

#[tauri::command]
pub fn delete_review(review_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM review WHERE id = ?1", params![review_id])?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn add_comment(
    review_id: i64,
    file_path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
    diff_hunk: Option<String>,
    body: String,
    anchored_head_sha: Option<String>,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let ts = now();
    conn.execute(
        "INSERT INTO comment
            (review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, ts],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    get_comment(&conn, conn.last_insert_rowid())
}

#[tauri::command]
pub fn update_comment(comment_id: i64, body: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    let review_id: i64 = conn.query_row(
        "SELECT review_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| r.get(0),
    )?;
    ensure_draft(&conn, review_id)?;
    conn.execute(
        "UPDATE comment SET body = ?1, updated_at = ?2 WHERE id = ?3",
        params![body, now(), comment_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn delete_comment(comment_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    let review_id: i64 = conn.query_row(
        "SELECT review_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| r.get(0),
    )?;
    ensure_draft(&conn, review_id)?;
    conn.execute("DELETE FROM comment WHERE id = ?1", params![comment_id])?;
    Ok(())
}
