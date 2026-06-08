use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::db::models::{Comment, Repository, Review, ReviewDetail, ReviewSummary, Target};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::gh::{self, GhRepo};
use crate::git;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn get_target(conn: &Connection, id: i64) -> AppResult<Target> {
    conn.query_row("SELECT * FROM target WHERE id = ?1", params![id], Target::from_row)
        .map_err(Into::into)
}

fn get_repository(conn: &Connection, id: i64) -> AppResult<Repository> {
    conn.query_row(
        "SELECT * FROM repository WHERE id = ?1",
        params![id],
        Repository::from_row,
    )
    .map_err(Into::into)
}

/// Find a repository row for a GitHub `owner/name`, preferring a real local clone
/// over a clone-less sentinel row, and creating a sentinel row when neither
/// exists. This lets inbox PRs be reviewed without first adding the repo. A
/// remote-only row stores `path = "github:{owner}/{name}"`.
pub(crate) fn get_or_create_remote_repository(
    conn: &Connection,
    owner: &str,
    name: &str,
) -> AppResult<Repository> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM repository
             WHERE remote_owner = ?1 AND remote_name = ?2
             ORDER BY (path LIKE 'github:%') ASC
             LIMIT 1",
            params![owner, name],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return get_repository(conn, id);
    }
    let sentinel = format!("github:{owner}/{name}");
    conn.execute(
        "INSERT INTO repository (path, remote_owner, remote_name, default_branch, added_at)
         VALUES (?1, ?2, ?3, NULL, ?4)",
        params![sentinel, owner, name, now()],
    )?;
    get_repository(conn, conn.last_insert_rowid())
}

/// The `gh` invocation context for a repository: a local clone when one is on
/// disk, otherwise a clone-less remote context resolved from `owner/name`.
fn gh_ctx_for_repo(conn: &Connection, repo_id: i64) -> AppResult<GhRepo> {
    let (path, owner, name): (String, Option<String>, Option<String>) = conn.query_row(
        "SELECT path, remote_owner, remote_name FROM repository WHERE id = ?1",
        params![repo_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    match path.strip_prefix("github:") {
        Some(rest) => {
            // Prefer the stored remote columns; fall back to parsing the sentinel.
            let (owner, name) = match (owner, name) {
                (Some(o), Some(n)) => (o, n),
                _ => rest
                    .split_once('/')
                    .map(|(o, n)| (o.to_string(), n.to_string()))
                    .ok_or_else(|| AppError::Other("remote-only repo missing owner/name".into()))?,
            };
            Ok(GhRepo::Remote { owner, name })
        }
        None => Ok(GhRepo::Local(PathBuf::from(path))),
    }
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

/// Start a fresh draft review against a real GitHub PR (creating/reusing its
/// target). Identifies the PR by `owner/name` + number so any inbox PR can be
/// opened, with no local clone required: the repo is resolved to a local clone
/// if one is added, otherwise a clone-less remote context.
#[tauri::command]
pub fn create_review_for_pr(
    owner: String,
    name: String,
    pr_number: i64,
    db: State<Db>,
) -> AppResult<Review> {
    // Resolve the repo and build the gh context before the slow `gh` call so we
    // don't hold the DB lock across a subprocess.
    let (repo_id, ctx) = {
        let conn = db.0.lock().unwrap();
        let repo = get_or_create_remote_repository(&conn, &owner, &name)?;
        let ctx = gh_ctx_for_repo(&conn, repo.id)?;
        (repo.id, ctx)
    };
    let info = gh::pr_view(&ctx, pr_number)?;
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
    match detail.target.kind.as_str() {
        "github_pr" => {
            let number = detail
                .target
                .github_pr_number
                .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
            let ctx = gh_ctx_for_repo(&conn, detail.target.repo_id)?;
            gh::pr_diff(&ctx, number)
        }
        _ => git::diff(
            std::path::Path::new(&detail.repo_path),
            &detail.target.base_ref,
            &detail.target.head_ref,
            detail.target.three_dot,
        ),
    }
}

/// Full source of one side of a file (LEFT→base, RIGHT→head), used to reveal
/// collapsed context between hunks and to render the full-file review pane. Reads
/// the local blob via `git show`; for GitHub PR targets whose commit isn't checked
/// out locally, falls back to the GitHub contents API.
#[tauri::command]
pub fn file_source(
    review_id: i64,
    file_path: String,
    side: String,
    db: State<Db>,
) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let sha = match side.as_str() {
        "LEFT" => detail.target.base_sha.as_deref(),
        "RIGHT" => detail.target.head_sha.as_deref(),
        other => return Err(AppError::Other(format!("invalid side: {other}"))),
    };
    let sha = sha
        .ok_or_else(|| AppError::Other("this side has no source (file added or deleted)".into()))?;

    // Remote-only (clone-less) PR targets have no local blob, so skip the
    // guaranteed-to-fail `git show` and go straight to the GitHub contents API.
    if !detail.repo_path.starts_with("github:") {
        let repo = std::path::Path::new(&detail.repo_path);
        // Local commit is fastest and works for local targets and PRs that are
        // checked out; for PRs whose commit isn't local, fall through to the API.
        match git::show_file(repo, sha, &file_path) {
            Ok(source) => return Ok(source),
            Err(local_err) => {
                if detail.target.kind != "github_pr" {
                    return Err(local_err);
                }
            }
        }
    }

    let (owner, name): (Option<String>, Option<String>) = conn.query_row(
        "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
        params![detail.target.repo_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match (owner, name) {
        (Some(owner), Some(name)) => gh::file_at_ref(&owner, &name, &file_path, sha),
        _ => Err(AppError::Other(
            "file source unavailable: no local clone and no GitHub remote".into(),
        )),
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
    let (repo_path, remote_owner, remote_name): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT path, remote_owner, remote_name FROM repository WHERE id = ?1",
            params![target.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
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
    let viewed_files: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT file_path FROM file_view_state WHERE review_id = ?1 AND viewed = 1")?;
        let rows = stmt
            .query_map(params![review_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    Ok(ReviewDetail {
        review,
        target,
        repo_path,
        remote_owner,
        remote_name,
        comments,
        viewed_files,
    })
}

/// Full review state (review + target + repo path + comments) for the Review screen.
#[tauri::command]
pub fn get_review(review_id: i64, db: State<Db>) -> AppResult<ReviewDetail> {
    let conn = db.0.lock().unwrap();
    load_detail(&conn, review_id)
}

/// Persist the per-file "viewed"/collapsed toggle. This is UI state rather than
/// review content, so it is allowed even on published (locked) reviews.
#[tauri::command]
pub fn set_file_viewed(
    review_id: i64,
    file_path: String,
    viewed: bool,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO file_view_state (review_id, file_path, viewed, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(review_id, file_path)
         DO UPDATE SET viewed = excluded.viewed, updated_at = excluded.updated_at",
        params![review_id, file_path, viewed as i64, now()],
    )?;
    Ok(())
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

/// Build the GitHub "create review" JSON payload from a loaded review detail.
///
/// Maps the stored verdict to GitHub's `event` enum, projects each line comment
/// to `{path, side, line, body}` (adding `start_line`/`start_side` only for true
/// multi-line ranges), and attaches the summary body and head commit when
/// present.
///
/// GitHub's bulk reviews API can't anchor file-level comments (each comment
/// needs a `path` + `line`/`position`), so file comments are folded into the
/// review body under a "File-level comments" section instead.
fn build_publish_payload(detail: &ReviewDetail) -> serde_json::Value {
    let event = match detail.review.event.as_deref() {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    };

    let comments: Vec<serde_json::Value> = detail
        .comments
        .iter()
        .filter(|c| c.subject_type != "file" && c.origin != "file_view")
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
    let body = body_with_file_comments(detail);
    if !body.trim().is_empty() {
        payload["body"] = serde_json::json!(body);
    }
    if let Some(sha) = &detail.target.head_sha {
        payload["commit_id"] = serde_json::json!(sha);
    }
    payload
}

/// The review summary with any whole-file and full-file-pane comments appended as
/// trailing sections. GitHub can't anchor either kind inline (file comments have
/// no line; file-view comments may sit outside the diff), so they ride along in
/// the body instead.
fn body_with_file_comments(detail: &ReviewDetail) -> String {
    let mut body = detail.review.body.trim().to_string();

    let file_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| c.subject_type == "file")
        .collect();
    if !file_comments.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("## File-level comments");
        for c in file_comments {
            body.push_str(&format!("\n\n**{}**\n\n{}", c.file_path, c.body.trim()));
        }
    }

    let file_view_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| c.origin == "file_view")
        .collect();
    if !file_view_comments.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("## File-view comments");
        let mut current = "";
        for c in file_view_comments {
            if c.file_path != current {
                body.push_str(&format!("\n\n**{}**", c.file_path));
                current = &c.file_path;
            }
            body.push_str(&format!("\n- {}: {}", line_label(c), c.body.trim()));
        }
    }

    body
}

/// `L5` for a single line, `L3-L5` for a multi-line range.
fn line_label(c: &Comment) -> String {
    match c.start_line {
        Some(start) if start != c.line => format!("L{}-L{}", start, c.line),
        _ => format!("L{}", c.line),
    }
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

    let payload = build_publish_payload(&detail);

    let gh_id = gh::post_review(&owner, &name, number, &payload.to_string())?;

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

/// Add a comment attached to a whole file rather than a specific line. Stored
/// with `subject_type = 'file'`; side/line keep their column defaults and are
/// ignored for these.
#[tauri::command]
pub fn add_file_comment(
    review_id: i64,
    file_path: String,
    body: String,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let ts = now();
    conn.execute(
        "INSERT INTO comment (review_id, file_path, subject_type, body, line, created_at, updated_at)
         VALUES (?1, ?2, 'file', ?3, 0, ?4, ?4)",
        params![review_id, file_path, body, ts],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    get_comment(&conn, conn.last_insert_rowid())
}

/// Add a comment authored in the full-file pane, anchored to an absolute
/// head-file line. Stored with `subject_type = 'line'`, `side = 'RIGHT'`, and
/// `origin = 'file_view'` so publish/export fold it into the review body instead
/// of posting it as a (possibly un-anchorable) GitHub inline comment.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn add_file_view_comment(
    review_id: i64,
    file_path: String,
    line: i64,
    start_line: Option<i64>,
    body: String,
    anchored_head_sha: Option<String>,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let ts = now();
    conn.execute(
        "INSERT INTO comment
            (review_id, file_path, subject_type, origin, side, line, start_line, body, anchored_head_sha, created_at, updated_at)
         VALUES (?1, ?2, 'line', 'file_view', 'RIGHT', ?3, ?4, ?5, ?6, ?7, ?7)",
        params![review_id, file_path, line, start_line, body, anchored_head_sha, ts],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    /// Insert a repository row and return its id.
    fn seed_repo(conn: &Connection, owner: Option<&str>, name: Option<&str>) -> i64 {
        conn.execute(
            "INSERT INTO repository (path, remote_owner, remote_name, default_branch, added_at)
             VALUES ('/repo', ?1, ?2, 'main', 'now')",
            params![owner, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// Insert a comment directly (mirrors add_comment's INSERT) and return its id.
    fn seed_comment(
        conn: &Connection,
        review_id: i64,
        file_path: &str,
        side: &str,
        line: i64,
        start_line: Option<i64>,
        body: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO comment
                (review_id, file_path, side, line, start_line, diff_hunk, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '@@ hunk @@', ?6, 'now', 'now')",
            params![review_id, file_path, side, line, start_line, body],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn pr_info() -> gh::PrInfo {
        gh::PrInfo {
            title: "My PR".into(),
            base_ref: "main".into(),
            head_ref: "feature".into(),
            head_sha: "deadbeef".into(),
        }
    }

    #[test]
    fn local_target_is_created_then_reused() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);

        let t1 = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        assert_eq!(t1.kind, "local");
        assert_eq!(t1.title, "main...feature");
        assert!(t1.three_dot);

        // Same (repo, base, head) reuses the row and can flip three_dot.
        let t2 = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", false).unwrap();
        assert_eq!(t1.id, t2.id);
        assert!(!t2.three_dot);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM target", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn pr_target_is_created_then_refreshed() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));

        let t1 = get_or_create_pr_target(&conn, repo, 7, &pr_info()).unwrap();
        assert_eq!(t1.kind, "github_pr");
        assert_eq!(t1.github_pr_number, Some(7));
        assert_eq!(t1.head_sha.as_deref(), Some("deadbeef"));

        let mut updated = pr_info();
        updated.title = "Renamed PR".into();
        updated.head_sha = "cafe".into();
        let t2 = get_or_create_pr_target(&conn, repo, 7, &updated).unwrap();
        assert_eq!(t1.id, t2.id);
        assert_eq!(t2.title, "Renamed PR");
        assert_eq!(t2.head_sha.as_deref(), Some("cafe"));
    }

    #[test]
    fn new_review_starts_as_empty_draft() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();

        let review = new_review_for_target(&conn, target.id).unwrap();
        assert_eq!(review.status, "draft");
        assert_eq!(review.body, "");
        assert!(review.event.is_none());
    }

    #[test]
    fn ensure_draft_blocks_published_reviews() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        assert!(ensure_draft(&conn, review.id).is_ok());

        conn.execute(
            "UPDATE review SET status = 'published' WHERE id = ?1",
            params![review.id],
        )
        .unwrap();
        let err = ensure_draft(&conn, review.id).unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn load_detail_returns_comments_ordered_by_file_line() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        seed_comment(&conn, review.id, "b.rs", "RIGHT", 10, None, "second file");
        seed_comment(&conn, review.id, "a.rs", "RIGHT", 5, None, "first file");
        seed_comment(&conn, review.id, "a.rs", "RIGHT", 2, None, "earliest line");

        let detail = load_detail(&conn, review.id).unwrap();
        let order: Vec<(&str, i64)> = detail
            .comments
            .iter()
            .map(|c| (c.file_path.as_str(), c.line))
            .collect();
        assert_eq!(order, vec![("a.rs", 2), ("a.rs", 5), ("b.rs", 10)]);
        assert_eq!(detail.repo_path, "/repo");
    }

    #[test]
    fn repo_label_prefers_owner_name_then_path() {
        let conn = open_memory();
        let with_remote = seed_repo(&conn, Some("acme"), Some("widget"));
        assert_eq!(repo_label(&conn, with_remote).unwrap(), "acme/widget");

        conn.execute(
            "INSERT INTO repository (path, added_at) VALUES ('/only/path', 'now')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        assert_eq!(repo_label(&conn, id).unwrap(), "/only/path");
    }

    #[test]
    fn remote_repository_creates_sentinel_then_reuses_local_clone() {
        let conn = open_memory();

        // No matching row yet: a clone-less sentinel row is created.
        let r1 = get_or_create_remote_repository(&conn, "acme", "widget").unwrap();
        assert_eq!(r1.path, "github:acme/widget");
        assert_eq!(r1.remote_owner.as_deref(), Some("acme"));
        let ctx = gh_ctx_for_repo(&conn, r1.id).unwrap();
        assert!(matches!(ctx, GhRepo::Remote { .. }));

        // Calling again reuses the same row (no duplicate).
        let r2 = get_or_create_remote_repository(&conn, "acme", "widget").unwrap();
        assert_eq!(r1.id, r2.id);

        // Once a real local clone is added for that remote, it is preferred and
        // the context becomes Local.
        conn.execute(
            "INSERT INTO repository (path, remote_owner, remote_name, added_at)
             VALUES ('/clones/widget', 'acme', 'widget', 'now')",
            [],
        )
        .unwrap();
        let local_id = conn.last_insert_rowid();
        let r3 = get_or_create_remote_repository(&conn, "acme", "widget").unwrap();
        assert_eq!(r3.id, local_id, "should prefer the local clone over the sentinel");
        assert!(matches!(gh_ctx_for_repo(&conn, r3.id).unwrap(), GhRepo::Local(_)));
    }

    #[test]
    fn deleting_review_cascades_to_comments() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();
        seed_comment(&conn, review.id, "a.rs", "RIGHT", 1, None, "note");

        conn.execute("DELETE FROM review WHERE id = ?1", params![review.id])
            .unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn deleting_repo_cascades_through_target_and_review() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();
        seed_comment(&conn, review.id, "a.rs", "RIGHT", 1, None, "note");

        conn.execute("DELETE FROM repository WHERE id = ?1", params![repo])
            .unwrap();
        for table in ["target", "review", "comment"] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table} should have been cascaded");
        }
    }

    // ---- publish payload mapping (the roadmap's top unit-test priority) ----

    fn detail_with(event: Option<&str>, body: &str, comments: Vec<Comment>) -> ReviewDetail {
        ReviewDetail {
            review: Review {
                id: 1,
                target_id: 1,
                body: body.into(),
                event: event.map(Into::into),
                status: "draft".into(),
                published_at: None,
                github_review_id: None,
                last_exported_at: None,
                created_at: "now".into(),
                updated_at: "now".into(),
            },
            target: Target {
                id: 1,
                repo_id: 1,
                kind: "github_pr".into(),
                github_pr_number: Some(1),
                title: "t".into(),
                base_ref: "main".into(),
                head_ref: "feature".into(),
                base_sha: None,
                head_sha: Some("headsha".into()),
                three_dot: true,
                created_at: "now".into(),
            },
            repo_path: "/repo".into(),
            remote_owner: Some("owner".into()),
            remote_name: Some("name".into()),
            comments,
            viewed_files: vec![],
        }
    }

    fn payload_comment(line: i64, start_line: Option<i64>, side: &str) -> Comment {
        Comment {
            id: 1,
            review_id: 1,
            file_path: "src/lib.rs".into(),
            subject_type: "line".into(),
            origin: "diff".into(),
            side: side.into(),
            line,
            start_line,
            diff_hunk: None,
            body: "comment body".into(),
            parent_id: None,
            anchored_head_sha: None,
            github_comment_id: None,
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    #[test]
    fn payload_maps_event_verdicts() {
        assert_eq!(
            build_publish_payload(&detail_with(Some("approve"), "", vec![]))["event"],
            "APPROVE"
        );
        assert_eq!(
            build_publish_payload(&detail_with(Some("request_changes"), "", vec![]))["event"],
            "REQUEST_CHANGES"
        );
        assert_eq!(
            build_publish_payload(&detail_with(Some("comment"), "", vec![]))["event"],
            "COMMENT"
        );
        // Missing/unknown verdict defaults to COMMENT.
        assert_eq!(
            build_publish_payload(&detail_with(None, "", vec![]))["event"],
            "COMMENT"
        );
    }

    #[test]
    fn payload_single_line_comment_has_no_start_fields() {
        let p = build_publish_payload(&detail_with(None, "", vec![payload_comment(5, None, "RIGHT")]));
        let c = &p["comments"][0];
        assert_eq!(c["path"], "src/lib.rs");
        assert_eq!(c["side"], "RIGHT");
        assert_eq!(c["line"], 5);
        assert_eq!(c["body"], "comment body");
        assert!(c.get("start_line").is_none());
        assert!(c.get("start_side").is_none());
    }

    #[test]
    fn payload_multiline_comment_adds_start_line_and_side() {
        let p =
            build_publish_payload(&detail_with(None, "", vec![payload_comment(8, Some(3), "LEFT")]));
        let c = &p["comments"][0];
        assert_eq!(c["line"], 8);
        assert_eq!(c["start_line"], 3);
        assert_eq!(c["start_side"], "LEFT");
    }

    #[test]
    fn payload_start_line_equal_to_line_is_treated_as_single_line() {
        let p =
            build_publish_payload(&detail_with(None, "", vec![payload_comment(5, Some(5), "RIGHT")]));
        let c = &p["comments"][0];
        assert!(c.get("start_line").is_none());
    }

    #[test]
    fn payload_includes_body_and_commit_id_when_present() {
        let p = build_publish_payload(&detail_with(Some("approve"), "Nice work", vec![]));
        assert_eq!(p["body"], "Nice work");
        assert_eq!(p["commit_id"], "headsha");
    }

    #[test]
    fn payload_omits_blank_body() {
        let p = build_publish_payload(&detail_with(Some("approve"), "   ", vec![]));
        assert!(p.get("body").is_none());
    }

    #[test]
    fn payload_omits_commit_id_when_head_sha_missing() {
        let mut d = detail_with(None, "", vec![]);
        d.target.head_sha = None;
        let p = build_publish_payload(&d);
        assert!(p.get("commit_id").is_none());
    }

    fn file_comment(file_path: &str, body: &str) -> Comment {
        Comment {
            subject_type: "file".into(),
            file_path: file_path.into(),
            line: 0,
            start_line: None,
            diff_hunk: None,
            body: body.into(),
            ..payload_comment(0, None, "RIGHT")
        }
    }

    #[test]
    fn payload_excludes_file_comments_from_inline_array() {
        let p = build_publish_payload(&detail_with(
            None,
            "",
            vec![
                payload_comment(5, None, "RIGHT"),
                file_comment("src/lib.rs", "whole-file note"),
            ],
        ));
        assert_eq!(p["comments"].as_array().unwrap().len(), 1);
        assert_eq!(p["comments"][0]["line"], 5);
    }

    #[test]
    fn payload_folds_file_comments_into_body() {
        let p = build_publish_payload(&detail_with(
            Some("comment"),
            "Summary text",
            vec![file_comment("src/lib.rs", "whole-file note")],
        ));
        let body = p["body"].as_str().unwrap();
        assert!(body.contains("Summary text"));
        assert!(body.contains("## File-level comments"));
        assert!(body.contains("**src/lib.rs**"));
        assert!(body.contains("whole-file note"));
    }

    #[test]
    fn payload_file_comments_become_body_even_without_summary() {
        let p = build_publish_payload(&detail_with(
            None,
            "",
            vec![file_comment("a.rs", "note")],
        ));
        assert!(p["body"].as_str().unwrap().starts_with("## File-level comments"));
    }

    fn file_view_comment(file_path: &str, line: i64, start_line: Option<i64>, body: &str) -> Comment {
        Comment {
            origin: "file_view".into(),
            file_path: file_path.into(),
            line,
            start_line,
            body: body.into(),
            ..payload_comment(line, start_line, "RIGHT")
        }
    }

    #[test]
    fn payload_excludes_file_view_comments_from_inline_array() {
        let p = build_publish_payload(&detail_with(
            None,
            "",
            vec![
                payload_comment(5, None, "RIGHT"),
                file_view_comment("src/lib.rs", 12, None, "pane note"),
            ],
        ));
        assert_eq!(p["comments"].as_array().unwrap().len(), 1);
        assert_eq!(p["comments"][0]["line"], 5);
    }

    #[test]
    fn payload_folds_file_view_comments_into_body_with_line_label() {
        let p = build_publish_payload(&detail_with(
            Some("comment"),
            "Summary text",
            vec![
                file_view_comment("src/lib.rs", 12, None, "single note"),
                file_view_comment("src/lib.rs", 20, Some(18), "range note"),
            ],
        ));
        let body = p["body"].as_str().unwrap();
        assert!(body.contains("## File-view comments"));
        assert!(body.contains("**src/lib.rs**"));
        assert!(body.contains("- L12: single note"));
        assert!(body.contains("- L18-L20: range note"));
    }
}
