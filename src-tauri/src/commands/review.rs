use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::anchor::{self, Remap};
use crate::db::models::{Comment, Repository, Review, ReviewDetail, ReviewSummary, Target};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::export;
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

/// The GitHub owner/name of a repository, if it has one: the stored remote
/// columns, else parsed from the clone-less `github:owner/name` path sentinel.
/// None for purely local repos (callers skip GitHub-API work gracefully).
fn repo_owner_name(conn: &Connection, repo_id: i64) -> AppResult<Option<(String, String)>> {
    let (path, owner, name): (String, Option<String>, Option<String>) = conn.query_row(
        "SELECT path, remote_owner, remote_name FROM repository WHERE id = ?1",
        params![repo_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if let (Some(o), Some(n)) = (owner, name) {
        return Ok(Some((o, n)));
    }
    Ok(path
        .strip_prefix("github:")
        .and_then(|rest| rest.split_once('/'))
        .map(|(o, n)| (o.to_string(), n.to_string())))
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
/// `merge_base_sha` is the resolved merge-base of `base_ref...head` (the LEFT
/// side of the three-dot PR diff — NOT the base-branch tip); None means
/// resolution failed/was skipped and the stored value is preserved (COALESCE).
pub(crate) fn get_or_create_pr_target(
    conn: &Connection,
    repo_id: i64,
    pr_number: i64,
    info: &gh::PrInfo,
    merge_base_sha: Option<&str>,
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
            "UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, head_sha = ?4,
                               base_sha = COALESCE(?5, base_sha)
             WHERE id = ?6",
            params![info.title, info.base_ref, info.head_ref, info.head_sha, merge_base_sha, id],
        )?;
        return get_target(conn, id);
    }

    conn.execute(
        "INSERT INTO target
            (repo_id, kind, github_pr_number, title, base_ref, head_ref, base_sha, head_sha, three_dot, created_at)
         VALUES (?1, 'github_pr', ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
        params![
            repo_id,
            pr_number,
            info.title,
            info.base_ref,
            info.head_ref,
            merge_base_sha,
            info.head_sha,
            now()
        ],
    )?;
    get_target(conn, conn.last_insert_rowid())
}

/// Outcome of re-resolving a target's head SHA: whether it moved since the last
/// resolution, plus the previous and current values for the frontend badge.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessResult {
    pub head_moved: bool,
    pub previous_head_sha: Option<String>,
    pub current_head_sha: Option<String>,
}

/// Re-resolve a target's SHAs (and refs/title for PRs) and persist them,
/// reporting whether the head moved. Mirrors how each kind resolves on open
/// (`get_or_create_local_target` / `get_or_create_pr_target`) but updates an
/// existing row in place.
///
/// Follows the split-lock pattern: resolve repo/`gh` context under the lock, drop
/// it for the `git rev-parse` / `gh pr view` subprocess, then re-lock to UPDATE.
/// Shared with `publish_review` so publish can pin the freshest head, and called
/// by the Refresh button via `refresh_review` — both therefore also heal a
/// PR target's `base_sha` (merge-base) as a side effect.
fn refresh_target_shas(db: &Db, target: &Target) -> AppResult<FreshnessResult> {
    let previous_head_sha = target.head_sha.clone();

    let current_head_sha = match target.kind.as_str() {
        "github_pr" => {
            let number = target
                .github_pr_number
                .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
            let (ctx, owner_name) = {
                let conn = db.0.lock().unwrap();
                (
                    gh_ctx_for_repo(&conn, target.repo_id)?,
                    repo_owner_name(&conn, target.repo_id)?,
                )
            };
            let info = gh::pr_view(&ctx, number)?;
            // PR diffs are three-dot: the LEFT side is the merge-base, not the
            // base-branch tip. Resolution failure degrades to None and the
            // stored base_sha is preserved (COALESCE).
            let merge_base = owner_name
                .and_then(|(o, n)| gh::merge_base_sha(&o, &n, &info.base_ref, &info.head_sha).ok());
            let conn = db.0.lock().unwrap();
            conn.execute(
                "UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, head_sha = ?4,
                                   base_sha = COALESCE(?5, base_sha)
                 WHERE id = ?6",
                params![info.title, info.base_ref, info.head_ref, info.head_sha, merge_base, target.id],
            )?;
            Some(info.head_sha)
        }
        _ => {
            let repo_path = {
                let conn = db.0.lock().unwrap();
                let path: String = conn.query_row(
                    "SELECT path FROM repository WHERE id = ?1",
                    params![target.repo_id],
                    |r| r.get(0),
                )?;
                path
            };
            let repo = Path::new(&repo_path);
            let base_sha = git::rev_parse(repo, &target.base_ref).ok();
            let head_sha = git::rev_parse(repo, &target.head_ref).ok();
            let conn = db.0.lock().unwrap();
            conn.execute(
                "UPDATE target SET base_sha = ?1, head_sha = ?2 WHERE id = ?3",
                params![base_sha, head_sha, target.id],
            )?;
            head_sha
        }
    };

    Ok(FreshnessResult {
        head_moved: current_head_sha.is_some() && current_head_sha != previous_head_sha,
        previous_head_sha,
        current_head_sha,
    })
}

fn refresh_review_impl(review_id: i64, db: &Db) -> AppResult<FreshnessResult> {
    let target = {
        let conn = db.0.lock().unwrap();
        let review = get_review_row(&conn, review_id)?;
        get_target(&conn, review.target_id)?
    };
    refresh_target_shas(db, &target)
}

/// Re-resolve a review's target SHAs from the source of truth (`git`/`gh`) and
/// persist them, surfacing whether the head moved. Does not re-anchor comments —
/// that is a separate explicit action.
#[tauri::command]
pub fn refresh_review(review_id: i64, db: State<Db>) -> AppResult<FreshnessResult> {
    refresh_review_impl(review_id, &db)
}

/// Tally of what `reanchor_review_comments` did to a review's diff comments.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReanchorResult {
    pub reanchored: usize,
    pub lost: usize,
    pub skipped_no_change: usize,
}

/// Re-anchor a review's RIGHT-side diff comments from their stored
/// `anchored_head_sha` to the target's current `head_sha` using the intervening
/// per-file diff. Pure helper: the caller holds the DB lock.
fn reanchor_review_comments(conn: &Connection, detail: &ReviewDetail) -> AppResult<ReanchorResult> {
    let mut result = ReanchorResult {
        reanchored: 0,
        lost: 0,
        skipped_no_change: 0,
    };

    let Some(current) = detail.target.head_sha.as_deref() else {
        return Ok(result);
    };

    let candidates: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| {
            c.side == "RIGHT"
                && c.subject_type == "line"
                && c.origin != "file_view"
                && c.parent_id.is_none()
        })
        .collect();

    // Group the comments that need remapping by (anchored_sha, file) so each
    // intervening per-file diff is fetched at most once.
    let mut groups: std::collections::HashMap<(String, String), Vec<&Comment>> =
        std::collections::HashMap::new();
    for c in candidates {
        match c.anchored_head_sha.as_deref() {
            None => result.skipped_no_change += 1,
            Some(sha) if sha == current => result.skipped_no_change += 1,
            Some(sha) => groups
                .entry((sha.to_string(), c.file_path.clone()))
                .or_default()
                .push(c),
        }
    }

    let is_remote = detail.repo_path.starts_with("github:");
    let mut compare_cache: std::collections::HashMap<String, Vec<gh::ComparedFile>> =
        std::collections::HashMap::new();

    for ((anchored_sha, file_path), comments) in &groups {
        let patch: Option<String> = if is_remote {
            let (owner, name) = match (&detail.remote_owner, &detail.remote_name) {
                (Some(o), Some(n)) => (o.clone(), n.clone()),
                _ => {
                    return Err(AppError::Other(
                        "clone-less re-anchor requires a GitHub remote".into(),
                    ))
                }
            };
            let files = match compare_cache.get(anchored_sha) {
                Some(f) => f,
                None => {
                    let f = gh::compare(&owner, &name, anchored_sha, current)?;
                    compare_cache.entry(anchored_sha.clone()).or_insert(f)
                }
            };
            files
                .iter()
                .find(|f| &f.filename == file_path)
                .and_then(|f| f.patch.clone())
        } else {
            Some(git::diff_shas_path(
                Path::new(&detail.repo_path),
                anchored_sha,
                current,
                file_path,
            )?)
        };

        // A missing patch on a clone-less PR means the file isn't in the compare
        // (renamed/binary/absent): every comment on it is Lost. An empty local
        // diff means the file is unchanged, so lines map to themselves.
        let hunks = match &patch {
            Some(p) => Some(anchor::parse_file_patch(p)),
            None if is_remote => None,
            None => Some(anchor::FileHunks::default()),
        };

        for c in comments {
            let Some(hunks) = &hunks else {
                result.lost += 1;
                continue;
            };

            // A true multi-line range remaps both endpoints; otherwise only `line`.
            let range_start = c.start_line.filter(|&s| s != c.line);

            let new_line = match anchor::remap_right_line(c.line, hunks) {
                Remap::Shifted(l) => l,
                Remap::Lost => {
                    result.lost += 1;
                    continue;
                }
            };
            let new_start = match range_start {
                Some(s) => match anchor::remap_right_line(s, hunks) {
                    Remap::Shifted(l) => Some(l),
                    Remap::Lost => {
                        result.lost += 1;
                        continue;
                    }
                },
                None => c.start_line,
            };

            conn.execute(
                "UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE id = ?5",
                params![new_line, new_start, current, now(), c.id],
            )?;
            // The whole thread moves with its root: replies inherit the root's
            // anchor, so overwriting their line/start/sha wholesale is correct.
            conn.execute(
                "UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE parent_id = ?5",
                params![new_line, new_start, current, now(), c.id],
            )?;
            result.reanchored += 1;
        }
    }

    Ok(result)
}

/// Re-anchor a draft review's RIGHT-side diff comments to the target's current
/// head SHA, reporting how many moved, were lost, or were already current.
#[tauri::command]
pub fn reanchor_comments(review_id: i64, db: State<Db>) -> AppResult<ReanchorResult> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let detail = load_detail(&conn, review_id)?;
    reanchor_review_comments(&conn, &detail)
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
    // PR diffs are three-dot, so the LEFT side is the merge-base — resolve it
    // here (lock still dropped) and store it as the target's base_sha. Failure
    // degrades to None: the stored value is preserved (COALESCE) and a NULL
    // heals later via file_source's lazy backfill.
    let merge_base = gh::merge_base_sha(&owner, &name, &info.base_ref, &info.head_sha).ok();
    let conn = db.0.lock().unwrap();
    let target = get_or_create_pr_target(&conn, repo_id, pr_number, &info, merge_base.as_deref())?;
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
/// collapsed context between hunks and to render the full-file review pane.
/// For GitHub PR targets LEFT is the merge-base blob (`target.base_sha`, lazily
/// backfilled here for rows created before it was populated). Reads the local
/// blob via `git show`; for GitHub PR targets whose commit isn't checked out
/// locally, falls back to the GitHub contents API.
#[tauri::command]
pub fn file_source(
    review_id: i64,
    file_path: String,
    side: String,
    db: State<Db>,
) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let sha: Option<String> = match side.as_str() {
        "LEFT" => detail.target.base_sha.clone(),
        "RIGHT" => detail.target.head_sha.clone(),
        other => return Err(AppError::Other(format!("invalid side: {other}"))),
    };

    // Lazy backfill: github_pr targets created before base_sha was populated (or
    // whose resolution failed) store NULL. Resolve the merge-base now and persist
    // it so the row heals without a manual refresh. file_source already runs gh
    // under this lock (file_at_ref below), so this follows the same precedent.
    let sha = match (sha, side.as_str(), detail.target.kind.as_str()) {
        (None, "LEFT", "github_pr") => {
            let resolved = repo_owner_name(&conn, detail.target.repo_id)?.and_then(|(o, n)| {
                let head = detail.target.head_sha.as_deref().unwrap_or(&detail.target.head_ref);
                gh::merge_base_sha(&o, &n, &detail.target.base_ref, head).ok()
            });
            if let Some(mb) = &resolved {
                conn.execute(
                    "UPDATE target SET base_sha = ?1 WHERE id = ?2",
                    params![mb, detail.target.id],
                )?;
            }
            resolved.ok_or_else(|| {
                AppError::Other("could not resolve the PR merge-base for the base side".into())
            })?
        }
        (sha, _, _) => sha.ok_or_else(|| {
            AppError::Other("this side has no source (file added or deleted)".into())
        })?,
    };

    // Remote-only (clone-less) PR targets have no local blob, so skip the
    // guaranteed-to-fail `git show` and go straight to the GitHub contents API.
    if !detail.repo_path.starts_with("github:") {
        let repo = std::path::Path::new(&detail.repo_path);
        // Local commit is fastest and works for local targets and PRs that are
        // checked out; for PRs whose commit isn't local, fall through to the API.
        match git::show_file(repo, &sha, &file_path) {
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
        (Some(owner), Some(name)) => gh::file_at_ref(&owner, &name, &file_path, &sha),
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
/// Whether a comment is anchored to the given (fresh) head SHA and so safe to
/// post inline. A `None` `anchored_head_sha` is treated as anchored — legacy and
/// local rows keep today's behaviour.
fn is_anchored_to(c: &Comment, head_sha: Option<&str>) -> bool {
    match c.anchored_head_sha.as_deref() {
        None => true,
        Some(sha) => Some(sha) == head_sha,
    }
}

fn build_publish_payload(detail: &ReviewDetail) -> serde_json::Value {
    let event = match detail.review.event.as_deref() {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    };

    let replies = export::replies_by_root(&detail.comments);
    let comments: Vec<serde_json::Value> = detail
        .comments
        .iter()
        .filter(|c| {
            c.subject_type != "file"
                && c.origin != "file_view"
                && c.parent_id.is_none()
                && is_anchored_to(c, detail.target.head_sha.as_deref())
        })
        .map(|c| {
            let body = export::fold_replies(
                &c.body,
                replies.get(&c.id).map_or(&[][..], Vec::as_slice),
            );
            let mut obj = serde_json::json!({
                "path": c.file_path,
                "side": c.side,
                "line": c.line,
                "body": body,
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
    let replies = export::replies_by_root(&detail.comments);
    let folded = |c: &Comment| {
        export::fold_replies(&c.body, replies.get(&c.id).map_or(&[][..], Vec::as_slice))
    };

    let file_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| c.subject_type == "file" && c.parent_id.is_none())
        .collect();
    if !file_comments.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("## File-level comments");
        for c in file_comments {
            body.push_str(&format!("\n\n**{}**\n\n{}", c.file_path, folded(c)));
        }
    }

    let file_view_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| c.origin == "file_view" && c.parent_id.is_none())
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
            body.push_str(&format!("\n- {}: {}", line_label(c), folded(c)));
        }
    }

    // Diff line comments that re-anchoring could not move onto the fresh head
    // would 422 if posted inline, so they degrade into prose here.
    let lost_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| {
            c.subject_type != "file"
                && c.origin != "file_view"
                && c.parent_id.is_none()
                && !is_anchored_to(c, detail.target.head_sha.as_deref())
        })
        .collect();
    if !lost_comments.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("## Comments that could not be re-anchored");
        for c in lost_comments {
            body.push_str(&format!(
                "\n- **{}** {}: {}",
                c.file_path,
                line_label(c),
                folded(c)
            ));
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
// Async so the GitHub round-trip runs off the main thread — a sync command would
// block the webview and freeze the UI while publishing (see `refresh_inbox`).
#[tauri::command]
pub async fn publish_review(review_id: i64, db: State<'_, Db>) -> AppResult<Review> {
    let detail = {
        let conn = db.0.lock().unwrap();
        load_detail(&conn, review_id)?
    };
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
    let (owner, name): (Option<String>, Option<String>) = {
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
            params![detail.target.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
    };
    let (owner, name) = match (owner, name) {
        (Some(o), Some(n)) => (o, n),
        _ => return Err(AppError::Other("repository has no GitHub remote".into())),
    };

    // Pin the freshest head, re-anchor RIGHT comments onto it, then build the
    // payload against the verified head so an advanced PR never causes a 422.
    refresh_target_shas(&db, &detail.target)?;

    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    reanchor_review_comments(&conn, &detail)?;
    let detail = load_detail(&conn, review_id)?;

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

/// Validate a reply target: the parent must exist, belong to `review_id`, and be
/// a root comment (one level of nesting, GitHub-style). Returns the parent row so
/// the caller inherits its anchor columns.
fn parent_for_reply(conn: &Connection, review_id: i64, parent_id: i64) -> AppResult<Comment> {
    let parent = match conn
        .query_row("SELECT * FROM comment WHERE id = ?1", params![parent_id], Comment::from_row)
    {
        Ok(p) => p,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(AppError::Other("reply parent not found".into()))
        }
        Err(e) => return Err(e.into()),
    };
    if parent.review_id != review_id {
        return Err(AppError::Other(
            "reply parent belongs to a different review".into(),
        ));
    }
    if parent.parent_id.is_some() {
        return Err(AppError::Other(
            "replies can only target a top-level comment".into(),
        ));
    }
    Ok(parent)
}

/// Insert a comment (or a reply when `parent_id` is set). Replies inherit every
/// anchor column from their root, so the caller-supplied anchor args are ignored
/// for them — the thread can never straddle two anchors. Plain helper so tests
/// can drive it with a bare `Connection`.
#[allow(clippy::too_many_arguments)]
fn add_comment_impl(
    conn: &Connection,
    review_id: i64,
    file_path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
    diff_hunk: Option<String>,
    body: String,
    anchored_head_sha: Option<String>,
    parent_id: Option<i64>,
) -> AppResult<Comment> {
    ensure_draft(conn, review_id)?;
    let ts = now();
    match parent_id {
        None => {
            conn.execute(
                "INSERT INTO comment
                    (review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, ts],
            )?;
        }
        Some(pid) => {
            let p = parent_for_reply(conn, review_id, pid)?;
            conn.execute(
                "INSERT INTO comment
                    (review_id, file_path, subject_type, origin, side, line, start_line,
                     diff_hunk, body, parent_id, anchored_head_sha, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![
                    review_id,
                    p.file_path,
                    p.subject_type,
                    p.origin,
                    p.side,
                    p.line,
                    p.start_line,
                    p.diff_hunk,
                    body,
                    pid,
                    p.anchored_head_sha,
                    ts,
                ],
            )?;
        }
    }
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    get_comment(conn, conn.last_insert_rowid())
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
    parent_id: Option<i64>,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    add_comment_impl(
        &conn,
        review_id,
        file_path,
        side,
        line,
        start_line,
        diff_hunk,
        body,
        anchored_head_sha,
        parent_id,
    )
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

/// Mark a root comment's thread resolved/unresolved. Pure helper; caller holds
/// the lock. Idempotent: resolving an already-resolved root refreshes the
/// timestamp, unresolving an unresolved one is a no-op UPDATE.
fn set_resolved(conn: &Connection, comment_id: i64, resolved: bool) -> AppResult<()> {
    let (review_id, parent_id): (i64, Option<i64>) = conn.query_row(
        "SELECT review_id, parent_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure_draft(conn, review_id)?;
    if parent_id.is_some() {
        return Err(AppError::Other(
            "only the root comment of a thread can be resolved".into(),
        ));
    }
    let ts = now();
    let resolved_at: Option<&str> = resolved.then_some(ts.as_str());
    conn.execute(
        "UPDATE comment SET resolved_at = ?1, updated_at = ?2 WHERE id = ?3",
        params![resolved_at, ts, comment_id],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn set_comment_resolved(comment_id: i64, resolved: bool, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_resolved(&conn, comment_id, resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use std::fs;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::TempDir;

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

    /// Like `seed_comment` but pins `anchored_head_sha` so re-anchoring sees a
    /// prior revision to remap from.
    #[allow(clippy::too_many_arguments)]
    fn seed_comment_anchored(
        conn: &Connection,
        review_id: i64,
        file_path: &str,
        side: &str,
        line: i64,
        start_line: Option<i64>,
        body: &str,
        anchored_head_sha: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO comment
                (review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '@@ hunk @@', ?6, ?7, 'now', 'now')",
            params![review_id, file_path, side, line, start_line, body, anchored_head_sha],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// Insert a reply row directly under `parent_id`, inheriting the parent's
    /// anchor columns (mirrors add_comment_impl's reply branch).
    fn seed_reply(conn: &Connection, review_id: i64, parent_id: i64, body: &str) -> i64 {
        let p = get_comment(conn, parent_id).unwrap();
        conn.execute(
            "INSERT INTO comment
                (review_id, file_path, subject_type, origin, side, line, start_line,
                 diff_hunk, body, parent_id, anchored_head_sha, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'now', 'now')",
            params![
                review_id,
                p.file_path,
                p.subject_type,
                p.origin,
                p.side,
                p.line,
                p.start_line,
                p.diff_hunk,
                body,
                parent_id,
                p.anchored_head_sha,
            ],
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

        let t1 = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        assert_eq!(t1.kind, "github_pr");
        assert_eq!(t1.github_pr_number, Some(7));
        assert_eq!(t1.head_sha.as_deref(), Some("deadbeef"));
        assert!(t1.base_sha.is_none());

        let mut updated = pr_info();
        updated.title = "Renamed PR".into();
        updated.head_sha = "cafe".into();
        let t2 = get_or_create_pr_target(&conn, repo, 7, &updated, None).unwrap();
        assert_eq!(t1.id, t2.id);
        assert_eq!(t2.title, "Renamed PR");
        assert_eq!(t2.head_sha.as_deref(), Some("cafe"));
        assert!(t2.base_sha.is_none(), "a None merge-base never invents a base_sha");
    }

    #[test]
    fn pr_target_stores_merge_base_sha() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));

        let t = get_or_create_pr_target(&conn, repo, 7, &pr_info(), Some("mb1")).unwrap();
        assert_eq!(t.base_sha.as_deref(), Some("mb1"));
    }

    #[test]
    fn pr_target_refresh_preserves_base_sha_on_none() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));

        let t1 = get_or_create_pr_target(&conn, repo, 7, &pr_info(), Some("mb1")).unwrap();
        assert_eq!(t1.base_sha.as_deref(), Some("mb1"));

        // A failed/skipped resolution (None) keeps the stored value (COALESCE).
        let t2 = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        assert_eq!(t1.id, t2.id);
        assert_eq!(t2.base_sha.as_deref(), Some("mb1"));

        // A fresh resolution overwrites it.
        let t3 = get_or_create_pr_target(&conn, repo, 7, &pr_info(), Some("mb2")).unwrap();
        assert_eq!(t3.base_sha.as_deref(), Some("mb2"));
    }

    #[test]
    fn repo_owner_name_resolution() {
        let conn = open_memory();

        // Stored remote columns win.
        let with_remote = seed_repo(&conn, Some("owner"), Some("repo"));
        assert_eq!(
            repo_owner_name(&conn, with_remote).unwrap(),
            Some(("owner".into(), "repo".into()))
        );

        // Clone-less sentinel path with NULL columns parses owner/name.
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at)
             VALUES ('github:acme/widget', 'main', 'now')",
            [],
        )
        .unwrap();
        let clone_less = conn.last_insert_rowid();
        assert_eq!(
            repo_owner_name(&conn, clone_less).unwrap(),
            Some(("acme".into(), "widget".into()))
        );

        // Purely local repo has no GitHub identity.
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at)
             VALUES ('/local/only', 'main', 'now')",
            [],
        )
        .unwrap();
        let local = conn.last_insert_rowid();
        assert_eq!(repo_owner_name(&conn, local).unwrap(), None);
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

    // ---- threaded replies (spec 11) ----

    /// A draft local review with one root diff comment. Returns (conn, review_id, root_id).
    fn review_with_root_comment() -> (Connection, i64, i64) {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();
        let root = add_comment_impl(
            &conn,
            review.id,
            "a.rs".into(),
            "RIGHT".into(),
            5,
            Some(3),
            Some("@@ root hunk @@".into()),
            "root note".into(),
            Some("rootsha".into()),
            None,
        )
        .unwrap();
        (conn, review.id, root.id)
    }

    #[test]
    fn reply_inherits_root_anchor_columns() {
        let (conn, review_id, root_id) = review_with_root_comment();
        // Junk anchors must be ignored entirely in favour of the root's.
        let reply = add_comment_impl(
            &conn,
            review_id,
            "".into(),
            "LEFT".into(),
            0,
            None,
            None,
            "a reply".into(),
            None,
            Some(root_id),
        )
        .unwrap();

        assert_eq!(reply.parent_id, Some(root_id));
        assert_eq!(reply.file_path, "a.rs");
        assert_eq!(reply.side, "RIGHT");
        assert_eq!(reply.line, 5);
        assert_eq!(reply.start_line, Some(3));
        assert_eq!(reply.diff_hunk.as_deref(), Some("@@ root hunk @@"));
        assert_eq!(reply.subject_type, "line");
        assert_eq!(reply.origin, "diff");
        assert_eq!(reply.anchored_head_sha.as_deref(), Some("rootsha"));
        assert_eq!(reply.body, "a reply");
    }

    #[test]
    fn reply_to_reply_is_rejected() {
        let (conn, review_id, root_id) = review_with_root_comment();
        let reply_id = seed_reply(&conn, review_id, root_id, "first reply");
        let err = add_comment_impl(
            &conn,
            review_id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "second level".into(),
            None,
            Some(reply_id),
        )
        .unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("top-level"), "got: {m}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reply_across_reviews_is_rejected() {
        let (conn, _review_id, root_id) = review_with_root_comment();
        // A second review under the same repo/target.
        let target = get_target(&conn, get_review_row(&conn, 1).unwrap().target_id).unwrap();
        let other = new_review_for_target(&conn, target.id).unwrap();
        let err = add_comment_impl(
            &conn,
            other.id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "wrong review".into(),
            None,
            Some(root_id),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn reply_to_missing_parent_is_rejected() {
        let (conn, review_id, _root_id) = review_with_root_comment();
        let err = add_comment_impl(
            &conn,
            review_id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "no parent".into(),
            None,
            Some(9999),
        )
        .unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("not found"), "got: {m}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reply_on_published_review_is_rejected() {
        let (conn, review_id, root_id) = review_with_root_comment();
        conn.execute(
            "UPDATE review SET status = 'published' WHERE id = ?1",
            params![review_id],
        )
        .unwrap();
        let err = add_comment_impl(
            &conn,
            review_id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "late reply".into(),
            None,
            Some(root_id),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn reply_to_file_comment_inherits_subject_type() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();
        // Seed a whole-file root via a direct INSERT mirroring add_file_comment.
        conn.execute(
            "INSERT INTO comment (review_id, file_path, subject_type, body, line, created_at, updated_at)
             VALUES (?1, 'a.rs', 'file', 'whole file', 0, 'now', 'now')",
            params![review.id],
        )
        .unwrap();
        let root_id = conn.last_insert_rowid();

        let reply = add_comment_impl(
            &conn,
            review.id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "reply to file".into(),
            None,
            Some(root_id),
        )
        .unwrap();
        assert_eq!(reply.subject_type, "file");
        assert_eq!(reply.origin, "diff");
        assert_eq!(reply.file_path, "a.rs");
    }

    #[test]
    fn deleting_root_cascades_replies() {
        let (conn, review_id, root_id) = review_with_root_comment();
        seed_reply(&conn, review_id, root_id, "reply one");
        seed_reply(&conn, review_id, root_id, "reply two");
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, 3);

        conn.execute("DELETE FROM comment WHERE id = ?1", params![root_id])
            .unwrap();
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, 0, "replies cascade with their root");
    }

    // ---- resolve / unresolve threads (spec 12) ----

    fn resolved_at_of(conn: &Connection, id: i64) -> Option<String> {
        conn.query_row(
            "SELECT resolved_at FROM comment WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn set_resolved_roundtrip() {
        let (conn, _review_id, root_id) = review_with_root_comment();
        assert!(resolved_at_of(&conn, root_id).is_none());

        set_resolved(&conn, root_id, true).unwrap();
        assert!(resolved_at_of(&conn, root_id).is_some());

        set_resolved(&conn, root_id, false).unwrap();
        assert!(resolved_at_of(&conn, root_id).is_none());
    }

    #[test]
    fn set_resolved_rejects_reply() {
        let (conn, review_id, root_id) = review_with_root_comment();
        let reply_id = seed_reply(&conn, review_id, root_id, "a reply");
        let err = set_resolved(&conn, reply_id, true).unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("root"), "got: {m}"),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(resolved_at_of(&conn, reply_id).is_none(), "row untouched");
    }

    #[test]
    fn set_resolved_blocked_when_published() {
        let (conn, review_id, root_id) = review_with_root_comment();
        conn.execute(
            "UPDATE review SET status = 'published' WHERE id = ?1",
            params![review_id],
        )
        .unwrap();
        let err = set_resolved(&conn, root_id, true).unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn set_resolved_idempotent() {
        let (conn, _review_id, root_id) = review_with_root_comment();
        set_resolved(&conn, root_id, true).unwrap();
        let first = resolved_at_of(&conn, root_id);
        // A second resolve refreshes the timestamp without error.
        set_resolved(&conn, root_id, true).unwrap();
        assert!(first.is_some());
        assert!(resolved_at_of(&conn, root_id).is_some());
        // Unresolving an unresolved root is also a no-op (no error).
        set_resolved(&conn, root_id, false).unwrap();
        set_resolved(&conn, root_id, false).unwrap();
        assert!(resolved_at_of(&conn, root_id).is_none());
    }

    #[test]
    fn new_comment_defaults_unresolved() {
        let (conn, _review_id, root_id) = review_with_root_comment();
        assert!(resolved_at_of(&conn, root_id).is_none());
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
            resolved_at: None,
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

    fn anchored_payload_comment(
        line: i64,
        start_line: Option<i64>,
        anchored_head_sha: Option<&str>,
    ) -> Comment {
        Comment {
            anchored_head_sha: anchored_head_sha.map(Into::into),
            ..payload_comment(line, start_line, "RIGHT")
        }
    }

    #[test]
    fn payload_keeps_comment_inline_when_anchored_to_fresh_head() {
        let mut d = detail_with(None, "", vec![anchored_payload_comment(5, None, Some("freshsha"))]);
        d.target.head_sha = Some("freshsha".into());

        let p = build_publish_payload(&d);
        assert_eq!(p["commit_id"], "freshsha");
        assert_eq!(p["comments"].as_array().unwrap().len(), 1);
        assert_eq!(p["comments"][0]["line"], 5);
        assert!(p.get("body").is_none());
    }

    #[test]
    fn payload_folds_unanchored_comment_into_body_instead_of_inline() {
        let mut d = detail_with(
            None,
            "",
            vec![anchored_payload_comment(7, None, Some("stalesha"))],
        );
        d.target.head_sha = Some("freshsha".into());

        let p = build_publish_payload(&d);
        assert_eq!(p["commit_id"], "freshsha");
        assert!(
            p["comments"].as_array().unwrap().is_empty(),
            "stale comment must not be posted inline"
        );
        let body = p["body"].as_str().unwrap();
        assert!(body.contains("## Comments that could not be re-anchored"));
        assert!(body.contains("**src/lib.rs** L7: comment body"));
    }

    /// A reply Comment literal whose anchor mirrors its root (id 1).
    fn reply_comment(id: i64, parent_id: i64, body: &str) -> Comment {
        Comment {
            id,
            parent_id: Some(parent_id),
            body: body.into(),
            ..payload_comment(5, None, "RIGHT")
        }
    }

    #[test]
    fn publish_payload_includes_resolved_comments() {
        // A resolved RIGHT diff comment still publishes inline (GitHub has no
        // resolved-at-creation concept).
        let mut c = payload_comment(5, None, "RIGHT");
        c.resolved_at = Some("2026-01-01T00:00:00Z".into());
        let p = build_publish_payload(&detail_with(None, "", vec![c]));
        assert_eq!(p["comments"].as_array().unwrap().len(), 1);
        assert_eq!(p["comments"][0]["line"], 5);
    }

    #[test]
    fn publish_payload_excludes_replies_and_folds_them() {
        let p = build_publish_payload(&detail_with(
            None,
            "",
            vec![
                payload_comment(5, None, "RIGHT"),
                reply_comment(2, 1, "I disagree"),
            ],
        ));
        assert_eq!(
            p["comments"].as_array().unwrap().len(),
            1,
            "the reply must not be a separate inline comment"
        );
        let body = p["comments"][0]["body"].as_str().unwrap();
        assert!(body.contains("comment body"));
        assert!(body.contains("> **reply by me:**"), "got: {body}");
        assert!(body.contains("> I disagree"), "got: {body}");
    }

    #[test]
    fn publish_body_folds_replies_of_file_and_lost_comments() {
        let mut root = file_comment("src/lib.rs", "whole-file note");
        root.id = 10;
        let p = build_publish_payload(&detail_with(
            Some("comment"),
            "Summary",
            vec![root, reply_comment(11, 10, "follow-up")],
        ));
        let body = p["body"].as_str().unwrap();
        assert!(body.contains("## File-level comments"));
        assert!(body.contains("whole-file note"));
        assert!(body.contains("> **reply by me:**"), "got: {body}");
        assert!(body.contains("> follow-up"), "got: {body}");
        // The reply must not appear in the inline array either.
        assert!(p["comments"].as_array().unwrap().is_empty());
    }

    // ---- refresh helper (real git via the two-heads fixture) ----

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repo with two linear commits on `main`: H1 has three lines, H2 inserts a
    /// line above an existing one. Returns `(dir, h1, h2)`.
    fn fixture_repo_two_heads() -> (TempDir, String, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "alpha\nbeta\ngamma\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "h1"]);
        let h1 = git::rev_parse(p, "HEAD").unwrap();

        fs::write(p.join("file.txt"), "alpha\nINSERTED\nbeta\ngamma\n").unwrap();
        git(p, &["commit", "-q", "-am", "h2"]);
        let h2 = git::rev_parse(p, "HEAD").unwrap();
        (dir, h1, h2)
    }

    /// Insert a repository row pointing at `path` and return its id.
    fn seed_repo_at(conn: &Connection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES (?1, 'main', 'now')",
            params![path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn refresh_local_target_updates_shas_and_reports_move() {
        let (dir, h1, _h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        // Pin the target's head to H1 while the branch already sits at H2.
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", true).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h1, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        let db = Db(Mutex::new(conn));
        let pinned = {
            let conn = db.0.lock().unwrap();
            get_target(&conn, target.id).unwrap()
        };
        let fresh = refresh_target_shas(&db, &pinned).unwrap();

        let h2 = git::rev_parse(dir.path(), "main").unwrap();
        let base = git::rev_parse(dir.path(), "main~1").unwrap();
        let stored = {
            let conn = db.0.lock().unwrap();
            get_target(&conn, target.id).unwrap()
        };
        assert_eq!(stored.head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(stored.base_sha.as_deref(), Some(base.as_str()));

        assert!(fresh.head_moved);
        assert_eq!(fresh.previous_head_sha.as_deref(), Some(h1.as_str()));
        assert_eq!(fresh.current_head_sha.as_deref(), Some(h2.as_str()));

        // The command path resolves the same review's target.
        let again = refresh_review_impl(review.id, &db).unwrap();
        assert!(!again.head_moved, "second refresh is a no-op");
    }

    #[test]
    fn refresh_local_target_no_move_reports_false() {
        let (dir, _h1, h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        // Target head already matches the branch tip, so refresh is a no-op.
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", true).unwrap();
        assert_eq!(target.head_sha.as_deref(), Some(h2.as_str()));

        let db = Db(Mutex::new(conn));
        let fresh = refresh_target_shas(&db, &target).unwrap();

        assert!(!fresh.head_moved);
        assert_eq!(fresh.previous_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(fresh.current_head_sha.as_deref(), Some(h2.as_str()));
    }

    // ---- re-anchor helper (real git via an insert+replace fixture) ----

    /// A repo with two linear commits on `main`. H1 has four lines; H2 inserts a
    /// line above `beta` (shifting later lines) and replaces `gamma` in place.
    /// Returns `(dir, h1, h2)`.
    ///
    /// Mapping RIGHT lines H1 -> H2: 1(alpha)->1, 2(beta)->3 (shifted),
    /// 3(gamma)->Lost (deleted/replaced), 4(delta)->5 (shifted).
    fn fixture_repo_insert_and_replace() -> (TempDir, String, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "alpha\nbeta\ngamma\ndelta\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "h1"]);
        let h1 = git::rev_parse(p, "HEAD").unwrap();

        fs::write(p.join("file.txt"), "alpha\nINSERTED\nbeta\ngammaX\ndelta\n").unwrap();
        git(p, &["commit", "-q", "-am", "h2"]);
        let h2 = git::rev_parse(p, "HEAD").unwrap();
        (dir, h1, h2)
    }

    fn comment_row(conn: &Connection, id: i64) -> Comment {
        get_comment(conn, id).unwrap()
    }

    #[test]
    fn reanchor_shifts_moved_comment_and_advances_sha() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        // beta (H1 line 2) shifts down to line 3 in H2.
        let id = seed_comment_anchored(
            &conn,
            review.id,
            "file.txt",
            "RIGHT",
            2,
            None,
            "on beta",
            Some(&h1),
        );

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1);
        assert_eq!(result.lost, 0);
        assert_eq!(result.skipped_no_change, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 3);
        assert_eq!(c.anchored_head_sha.as_deref(), Some(h2.as_str()));
    }

    #[test]
    fn reanchor_leaves_replaced_comment_lost_with_sha_untouched() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        // gamma (H1 line 3) is replaced in H2 -> Lost.
        let id = seed_comment_anchored(
            &conn,
            review.id,
            "file.txt",
            "RIGHT",
            3,
            None,
            "on gamma",
            Some(&h1),
        );

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.lost, 1);
        assert_eq!(result.reanchored, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 3, "lost comment's line is left untouched");
        assert_eq!(
            c.anchored_head_sha.as_deref(),
            Some(h1.as_str()),
            "lost comment keeps its old sha"
        );
    }

    #[test]
    fn reanchor_ignores_left_side_comments() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        let id = seed_comment_anchored(
            &conn,
            review.id,
            "file.txt",
            "LEFT",
            2,
            None,
            "left side",
            Some(&h1),
        );

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 0);
        assert_eq!(result.lost, 0);
        assert_eq!(result.skipped_no_change, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 2);
        assert_eq!(c.anchored_head_sha.as_deref(), Some(h1.as_str()));
    }

    #[test]
    fn reanchor_skips_comment_already_on_current_head() {
        let (dir, _h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        let id = seed_comment_anchored(
            &conn,
            review.id,
            "file.txt",
            "RIGHT",
            3,
            None,
            "already current",
            Some(&h2),
        );

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.skipped_no_change, 1);
        assert_eq!(result.reanchored, 0);
        assert_eq!(result.lost, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 3);
        assert_eq!(c.anchored_head_sha.as_deref(), Some(h2.as_str()));
    }

    #[test]
    fn reanchor_moves_replies_with_their_root() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        // beta (H1 line 2) shifts to line 3 in H2; the reply rides along.
        let root_id = seed_comment_anchored(
            &conn, review.id, "file.txt", "RIGHT", 2, None, "on beta", Some(&h1),
        );
        let reply_id = seed_reply(&conn, review.id, root_id, "agreed");

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1, "only the root counts");
        assert_eq!(result.lost, 0);

        let root = comment_row(&conn, root_id);
        let reply = comment_row(&conn, reply_id);
        assert_eq!(root.line, 3);
        assert_eq!(reply.line, 3);
        assert_eq!(root.anchored_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(reply.anchored_head_sha.as_deref(), Some(h2.as_str()));
    }

    #[test]
    fn reanchor_lost_root_leaves_replies_untouched() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        conn.execute(
            "UPDATE target SET head_sha = ?1 WHERE id = ?2",
            params![h2, target.id],
        )
        .unwrap();
        let review = new_review_for_target(&conn, target.id).unwrap();

        // gamma (H1 line 3) is replaced in H2 -> Lost; the reply stays put.
        let root_id = seed_comment_anchored(
            &conn, review.id, "file.txt", "RIGHT", 3, None, "on gamma", Some(&h1),
        );
        let reply_id = seed_reply(&conn, review.id, root_id, "still here");

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.lost, 1);
        assert_eq!(result.reanchored, 0);

        let root = comment_row(&conn, root_id);
        let reply = comment_row(&conn, reply_id);
        assert_eq!(root.line, 3);
        assert_eq!(reply.line, 3);
        assert_eq!(root.anchored_head_sha.as_deref(), Some(h1.as_str()));
        assert_eq!(reply.anchored_head_sha.as_deref(), Some(h1.as_str()));
    }
}
