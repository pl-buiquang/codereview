use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::anchor::{self, Remap};
use crate::db::models::{Comment, Repository, Review, ReviewDetail, ReviewSummary, Target};
use crate::db::Db;
use crate::diff_cache;
use crate::error::{AppError, AppResult};
use crate::export;
use crate::gh;
use crate::gh::ComparedFile;
use crate::gh::GhRepo;
use crate::gh::PrInfo;
use crate::gh::ReviewComment;
use crate::git;
use crate::provider::provider_for;

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// A target with a PR's title and refs read from the mirror; the target's own
/// columns are the label shown until the mirror has the PR.
fn get_target(conn: &Connection, id: i64) -> AppResult<Target> {
    conn.query_row(
        "SELECT t.id, t.repo_id, t.kind, t.github_pr_number, t.gh_pr_id,
                COALESCE(p.title, t.title) AS title,
                COALESCE(p.base_ref, t.base_ref) AS base_ref,
                COALESCE(p.head_ref, t.head_ref) AS head_ref,
                t.three_dot, t.created_at
         FROM target t LEFT JOIN gh_pr p ON p.node_id = t.gh_pr_id AND t.kind = 'github_pr'
         WHERE t.id = ?1",
        params![id],
        Target::from_row,
    )
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

fn find_remote_repository_id(conn: &Connection, owner: &str, name: &str) -> AppResult<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM repository
             WHERE remote_owner = ?1 COLLATE NOCASE AND remote_name = ?2 COLLATE NOCASE
             ORDER BY id LIMIT 1",
            params![owner, name],
            |r| r.get(0),
        )
        .optional()?)
}

/// Find a repository row for a GitHub `owner/name`, creating a remote-only row
/// when neither exists. Reviewing a repo registers it, including one the
/// mirror auto-added. Attempts auto-link from base_paths before inserting, so
/// new rows may already have a local_path set.
pub fn get_or_create_remote_repository(
    conn: &Connection,
    owner: &str,
    name: &str,
    base_paths: &[String],
) -> AppResult<Repository> {
    if let Some(id) = find_remote_repository_id(conn, owner, name)? {
        conn.execute(
            "UPDATE repository SET registered_at = ?1 WHERE id = ?2 AND registered_at IS NULL",
            params![now(), id],
        )?;
        return get_repository(conn, id);
    }
    // Auto-link: try to discover a local clone under configured base paths.
    let auto_path = crate::commands::repo::resolve_local_path(owner, name, base_paths);
    let default_branch = auto_path
        .as_deref()
        .and_then(|p| git::default_branch(std::path::Path::new(p)));
    conn.execute(
        "INSERT INTO repository (local_path, remote_owner, remote_name, default_branch, added_at, registered_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![auto_path, owner, name, default_branch, now()],
    )?;
    get_repository(conn, conn.last_insert_rowid())
}

/// The `gh` invocation context for a repository: a local clone when one is on
/// disk, otherwise a clone-less remote context resolved from `owner/name`.
pub(crate) fn gh_ctx_for_repo(conn: &Connection, repo_id: i64) -> AppResult<GhRepo> {
    let (local_path, owner, name): (Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT local_path, remote_owner, remote_name FROM repository WHERE id = ?1",
            params![repo_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
    match local_path {
        Some(path) => Ok(GhRepo::Local(PathBuf::from(path))),
        None => {
            let owner = owner
                .ok_or_else(|| AppError::Other("repo has no local clone or GitHub remote".into()))?;
            let name = name
                .ok_or_else(|| AppError::Other("repo has no local clone or GitHub remote".into()))?;
            Ok(GhRepo::Remote { owner, name })
        }
    }
}

/// The GitHub owner/name of a repository, if it has one.
/// None for purely local repos (callers skip GitHub-API work gracefully).
pub(crate) fn repo_owner_name(conn: &Connection, repo_id: i64) -> AppResult<Option<(String, String)>> {
    let (owner, name): (Option<String>, Option<String>) = conn.query_row(
        "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
        params![repo_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(owner.zip(name))
}

pub(crate) fn get_review_row(conn: &Connection, id: i64) -> AppResult<Review> {
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

pub(crate) fn ensure_draft(conn: &Connection, review_id: i64) -> AppResult<()> {
    match review_status(conn, review_id)?.as_str() {
        "draft" => Ok(()),
        "published_pending" => Err(AppError::Other(
            "this review is pending on GitHub — submit or discard it before editing".into(),
        )),
        _ => Err(AppError::Other(
            "this review is published and can no longer be edited".into(),
        )),
    }
}

/// `ai`, `ai:explain`, `ai:review`, `ai:codex` or `local`, or none. Tagged comments are never published and don't pin
/// a focus-hidden hunk/file visible.
fn validate_tag(tag: Option<&str>) -> AppResult<()> {
    match tag {
        None | Some("ai" | "ai:explain" | "ai:review" | "ai:codex" | "local") => Ok(()),
        Some(t) => Err(AppError::Other(format!(
            "invalid tag '{t}' (expected ai, ai:explain, ai:review, ai:codex or local)"
        ))),
    }
}

/// SQL predicate: the comment's `tag` is any AI tag (`ai` or `ai:*`).
pub(crate) const AI_TAG_SQL: &str = "(tag = 'ai' OR tag LIKE 'ai:%')";

/// Map the stored verdict to a GitHub review event. None/'comment' => COMMENT.
fn gh_event(event: Option<&str>) -> &'static str {
    match event {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    }
}

/// The SHA whose file contents the LEFT side of this local diff shows: the
/// merge-base for three-dot (GitHub-PR semantics), the base tip for two-dot.
/// This is the coordinate system a LEFT comment's line numbers are valid in, so
/// it is what `anchored_base_sha` pins to.
fn local_old_side_sha(repo: &Path, base_ref: &str, head_ref: &str, three_dot: bool) -> Option<String> {
    if three_dot {
        git::merge_base(repo, base_ref, head_ref).ok()
    } else {
        git::rev_parse(repo, base_ref).ok()
    }
}

/// The commits a target currently resolves to (the live head and its LEFT
/// side). A review's pin is a copy of this taken at creation or on Update review.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveShas {
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
}

/// Reuse one `target` per (repo, base, head) comparison so several reviews can
/// share it. The target never stores commits; the live SHAs are returned for
/// pinning a new review.
fn get_or_create_local_target(
    conn: &Connection,
    repo_id: i64,
    repo_path: &str,
    base_ref: &str,
    head_ref: &str,
    three_dot: bool,
) -> AppResult<(Target, LiveShas)> {
    let live = LiveShas {
        base_sha: local_old_side_sha(Path::new(repo_path), base_ref, head_ref, three_dot),
        head_sha: git::rev_parse(Path::new(repo_path), head_ref).ok(),
    };

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM target
             WHERE repo_id = ?1 AND kind = 'local' AND base_ref = ?2 AND head_ref = ?3",
            params![repo_id, base_ref, head_ref],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(id) = existing {
        conn.execute("UPDATE target SET three_dot = ?1 WHERE id = ?2", params![three_dot as i64, id])?;
        return Ok((get_target(conn, id)?, live));
    }

    let title = format!("{base_ref}...{head_ref}");
    conn.execute(
        "INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, three_dot, created_at)
         VALUES (?1, 'local', NULL, ?2, ?3, ?4, ?5, ?6)",
        params![repo_id, title, base_ref, head_ref, three_dot as i64, now()],
    )?;
    Ok((get_target(conn, conn.last_insert_rowid())?, live))
}

/// Reuse one `target` per GitHub PR number, refreshing its label and its
/// mirror reference.
pub(crate) fn get_or_create_pr_target(
    conn: &Connection,
    repo_id: i64,
    pr_number: i64,
    info: &PrInfo,
    gh_pr_id: Option<&str>,
) -> AppResult<Target> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM target WHERE repo_id = ?1 AND kind = 'github_pr' AND github_pr_number = ?2",
            params![repo_id, pr_number],
            |r| r.get(0),
        )
        .optional()?;

    if let Some(id) = existing {
        update_pr_target_info(conn, id, info, gh_pr_id)?;
        return get_target(conn, id);
    }

    conn.execute(
        "INSERT INTO target (repo_id, kind, github_pr_number, gh_pr_id, title, base_ref, head_ref, three_dot, created_at)
         VALUES (?1, 'github_pr', ?2, ?3, ?4, ?5, ?6, 1, ?7)",
        params![repo_id, pr_number, gh_pr_id, info.title, info.base_ref, info.head_ref, now()],
    )?;
    get_target(conn, conn.last_insert_rowid())
}

fn update_pr_target_info(conn: &Connection, target_id: i64, info: &PrInfo, gh_pr_id: Option<&str>) -> AppResult<()> {
    conn.execute(
        "UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, gh_pr_id = COALESCE(?4, gh_pr_id)
         WHERE id = ?5",
        params![info.title, info.base_ref, info.head_ref, gh_pr_id, target_id],
    )?;
    Ok(())
}

/// Fetch a PR into the mirror and return what a target needs from it.
fn sync_pr(db: &Db, owner: &str, name: &str, number: i64) -> AppResult<(PrInfo, String)> {
    let node = provider_for().pr_node(owner, name, number)?;
    let info = PrInfo::from_node(&node)?;
    crate::mirror::store::upsert_node(&db.0.lock().unwrap(), &node, &now())?;
    Ok((info, node.id))
}

/// A review's pin compared with its target's live head.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessResult {
    /// The live head or base differs from the pin.
    pub moved: bool,
    pub pinned_head_sha: Option<String>,
    pub live_head_sha: Option<String>,
    pub live_base_sha: Option<String>,
    /// Commits on the live head that the pinned head lacks; local targets only.
    pub new_commits: Option<i64>,
}

fn has_moved(pin: &LiveShas, live: &LiveShas) -> bool {
    let differs = |pinned: &Option<String>, current: &Option<String>| current.is_some() && current != pinned;
    differs(&pin.head_sha, &live.head_sha) || differs(&pin.base_sha, &live.base_sha)
}

fn review_pin(review: &Review) -> LiveShas {
    LiveShas { base_sha: review.base_sha.clone(), head_sha: review.head_sha.clone() }
}

fn repo_local_path_of(db: &Db, repo_id: i64) -> AppResult<Option<String>> {
    let conn = db.0.lock().unwrap();
    Ok(conn.query_row("SELECT local_path FROM repository WHERE id = ?1", params![repo_id], |r| r.get(0))?)
}

/// Resolve a target's live head and LEFT side from `git` / `gh`. Never touches
/// any pin; a PR is synced into the mirror on the way.
///
/// Split-lock: repo/`gh` context under the lock, subprocesses without it.
fn resolve_live(db: &Db, target: &Target) -> AppResult<LiveShas> {
    match target.kind.as_str() {
        "github_pr" => {
            let number = target
                .github_pr_number
                .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
            let (owner, name) = repo_owner_name(&db.0.lock().unwrap(), target.repo_id)?
                .ok_or_else(|| AppError::Other("PR target's repo has no GitHub remote".into()))?;
            let (info, node_id) = sync_pr(db, &owner, &name, number)?;
            // PR diffs are three-dot: the LEFT side is the merge-base, not the
            // base-branch tip.
            let base_sha = provider_for().merge_base_sha(&owner, &name, &info.base_ref, &info.head_sha).ok();
            update_pr_target_info(&db.0.lock().unwrap(), target.id, &info, Some(&node_id))?;
            Ok(LiveShas { base_sha, head_sha: Some(info.head_sha) })
        }
        _ => {
            let repo_path = repo_local_path_of(db, target.repo_id)?
                .ok_or_else(|| AppError::Other("local targets require a local clone".into()))?;
            let repo = Path::new(&repo_path);
            Ok(LiveShas {
                base_sha: local_old_side_sha(repo, &target.base_ref, &target.head_ref, target.three_dot),
                head_sha: git::rev_parse(repo, &target.head_ref).ok(),
            })
        }
    }
}

/// Commits reachable from `live` but not from `pinned`, when both are local.
fn count_new_commits(repo: &Path, pinned: &str, live: &str) -> Option<i64> {
    git::run_git(repo, &["rev-list", "--count", &format!("{pinned}..{live}")])
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn refresh_review_impl(review_id: i64, db: &Db) -> AppResult<FreshnessResult> {
    let (review, target) = {
        let conn = db.0.lock().unwrap();
        let review = get_review_row(&conn, review_id)?;
        let target = get_target(&conn, review.target_id)?;
        (review, target)
    };
    let live = resolve_live(db, &target)?;
    let pin = review_pin(&review);
    log::debug!(
        target: "review",
        "refresh_review {review_id} ({}): pinned head {:?}, live head {:?}",
        target.kind, pin.head_sha, live.head_sha
    );
    let new_commits = match (target.kind.as_str(), &pin.head_sha, &live.head_sha) {
        ("local", Some(pinned), Some(current)) if pinned != current => repo_local_path_of(db, target.repo_id)?
            .and_then(|p| count_new_commits(Path::new(&p), pinned, current)),
        ("local", Some(_), Some(_)) => Some(0),
        _ => None,
    };
    Ok(FreshnessResult {
        moved: has_moved(&pin, &live),
        pinned_head_sha: pin.head_sha,
        live_head_sha: live.head_sha,
        live_base_sha: live.base_sha,
        new_commits,
    })
}

/// Compare a review's pin with its target's live head. Moves nothing: moving a
/// draft's pin is the explicit `update_review`. `async`: PR targets hit `gh`.
#[tauri::command]
pub async fn refresh_review(review_id: i64, db: State<'_, Db>) -> AppResult<FreshnessResult> {
    refresh_review_impl(review_id, &db)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReviewResult {
    pub moved: bool,
    pub reanchor: ReanchorResult,
}

/// Update review: move a draft's pin to the live head (and its LEFT side),
/// then re-anchor its comments onto the new pin. Comments that can't follow
/// keep their old anchor and so show as outdated; a Focus planned against the
/// old pin becomes stale. Published reviews never move.
pub fn update_review_pin_impl(review_id: i64, db: &Db) -> AppResult<UpdateReviewResult> {
    let (review, target) = {
        let conn = db.0.lock().unwrap();
        ensure_draft(&conn, review_id)?;
        let review = get_review_row(&conn, review_id)?;
        let target = get_target(&conn, review.target_id)?;
        (review, target)
    };
    let live = resolve_live(db, &target)?;
    let head = live
        .head_sha
        .clone()
        .ok_or_else(|| AppError::Other(format!("could not resolve the live head of `{}`", target.head_ref)))?;
    let moved = has_moved(&review_pin(&review), &live);

    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    if moved {
        conn.execute(
            "UPDATE review SET head_sha = ?1, base_sha = COALESCE(?2, base_sha), updated_at = ?3 WHERE id = ?4",
            params![head, live.base_sha, now(), review_id],
        )?;
    }
    let detail = load_detail(&conn, review_id)?;
    let reanchor = reanchor_review_comments(&conn, &detail)?;
    Ok(UpdateReviewResult { moved, reanchor })
}

#[tauri::command]
pub async fn update_review_pin(review_id: i64, db: State<'_, Db>) -> AppResult<UpdateReviewResult> {
    update_review_pin_impl(review_id, &db)
}

/// Tally of what `reanchor_review_comments` did to a review's diff comments.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReanchorResult {
    pub reanchored: usize,
    pub lost: usize,
    pub skipped_no_change: usize,
    /// Of `reanchored`, those the diff could not map and that were found again by content.
    pub relocated: usize,
}

/// Which anchor a re-anchor pass operates on: the comment side it selects, the
/// anchor column it reads/advances, and the review pin SHA it maps onto. The
/// RIGHT/head pass maps `anchored_head_sha → review.head_sha`; the LEFT/base
/// pass maps `anchored_base_sha → review.base_sha`. Both feed the same `remap_line`, whose
/// input always lives on the patch's old side.
#[derive(Clone, Copy)]
enum AnchorPass {
    Head,
    Base,
}

impl AnchorPass {
    fn comment_side(self) -> &'static str {
        match self {
            AnchorPass::Head => "RIGHT",
            AnchorPass::Base => "LEFT",
        }
    }

    fn pin(self, c: &Comment) -> Option<&str> {
        match self {
            AnchorPass::Head => c.anchored_head_sha.as_deref(),
            AnchorPass::Base => c.anchored_base_sha.as_deref(),
        }
    }

    fn current(self, r: &Review) -> Option<&str> {
        match self {
            AnchorPass::Head => r.head_sha.as_deref(),
            AnchorPass::Base => r.base_sha.as_deref(),
        }
    }

    /// UPDATE advancing the root's line/start and this pass's pin column.
    fn root_update_sql(self) -> &'static str {
        match self {
            AnchorPass::Head => "UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE id = ?5",
            AnchorPass::Base => "UPDATE comment SET line = ?1, start_line = ?2, anchored_base_sha = ?3, updated_at = ?4 WHERE id = ?5",
        }
    }

    /// Same UPDATE cascaded onto the root's replies (spec 11): the whole thread
    /// moves together since replies inherit the root's anchor.
    fn reply_update_sql(self) -> &'static str {
        match self {
            AnchorPass::Head => "UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE parent_id = ?5",
            AnchorPass::Base => "UPDATE comment SET line = ?1, start_line = ?2, anchored_base_sha = ?3, updated_at = ?4 WHERE parent_id = ?5",
        }
    }
}

/// File contents at `pinned_sha` and `current`, for the content fallback. Best effort: any
/// failure (missing file, unreachable commit, no network) yields `None` and the comment stays lost.
fn read_versions(
    detail: &ReviewDetail,
    pinned_sha: &str,
    current: &str,
    file_path: &str,
) -> Option<(String, String)> {
    match detail.local_path.as_deref() {
        Some(local) => {
            let local = Path::new(local);
            Some((
                git::show_file(local, pinned_sha, file_path).ok()?,
                git::show_file(local, current, file_path).ok()?,
            ))
        }
        None => {
            let (owner, name) = (detail.remote_owner.as_deref()?, detail.remote_name.as_deref()?);
            Some((
                gh::file_at_ref(owner, name, file_path, pinned_sha).ok()?,
                gh::file_at_ref(owner, name, file_path, current).ok()?,
            ))
        }
    }
}

/// One side's re-anchor pass: walk this side's root diff comments from their
/// anchored SHA to the review's pin for that side through the intervening
/// per-file diff, advancing line numbers and the pin column. Replies ride along
/// with their root. Accumulates into `result`. The diff is always fetched in the
/// direction pin → current so `remap_line` reads its input on the patch's old
/// side. A missing current SHA counts every pinned candidate as
/// `skipped_no_change` and touches nothing.
fn reanchor_pass(
    conn: &Connection,
    detail: &ReviewDetail,
    pass: AnchorPass,
    result: &mut ReanchorResult,
) -> AppResult<()> {
    let candidates: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| {
            c.side == pass.comment_side()
                && c.subject_type == "line"
                && c.origin != "file_view"
                && c.parent_id.is_none()
        })
        .collect();

    let Some(current) = pass.current(&detail.review) else {
        // Current SHA unresolved (head unresolved, or base_sha NULL on a PR row
        // spec 10 hasn't backfilled): every pinned candidate is left untouched.
        for c in &candidates {
            if pass.pin(c).is_some() {
                result.skipped_no_change += 1;
            }
        }
        return Ok(());
    };

    // Group the comments that need remapping by (pinned_sha, file) so each
    // intervening per-file diff is fetched at most once.
    let mut groups: std::collections::HashMap<(String, String), Vec<&Comment>> =
        std::collections::HashMap::new();
    for c in candidates {
        match pass.pin(c) {
            None => result.skipped_no_change += 1,
            Some(sha) if sha == current => result.skipped_no_change += 1,
            Some(sha) => groups
                .entry((sha.to_string(), c.file_path.clone()))
                .or_default()
                .push(c),
        }
    }

    let is_remote = detail.local_path.is_none();
    let mut compare_cache: std::collections::HashMap<String, Vec<ComparedFile>> =
        std::collections::HashMap::new();
    let mut ensured_commits: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ((pinned_sha, file_path), comments) in &groups {
        let patch: Option<String> = if is_remote {
            let (owner, name) = match (&detail.remote_owner, &detail.remote_name) {
                (Some(o), Some(n)) => (o.clone(), n.clone()),
                _ => {
                    return Err(AppError::Other(
                        "clone-less re-anchor requires a GitHub remote".into(),
                    ))
                }
            };
            let files = match compare_cache.get(pinned_sha) {
                Some(f) => f,
                None => {
                    let f = provider_for().compare(&owner, &name, pinned_sha, current)?;
                    compare_cache.entry(pinned_sha.clone()).or_insert(f)
                }
            };
            files
                .iter()
                .find(|f| &f.filename == file_path)
                .and_then(|f| f.patch.clone())
        } else {
            let local = detail.local_path.as_deref().unwrap();
            if detail.target.kind == "github_pr" {
                // The repo's local clone may not have fetched the PR's current
                // or pinned head yet (e.g. after a force-push) — `diff_shas_path`
                // needs both as local objects.
                for sha in [pinned_sha.as_str(), current] {
                    if ensured_commits.insert(sha.to_string()) {
                        git::ensure_commit(Path::new(local), sha)?;
                    }
                }
            }
            Some(git::diff_shas_path(
                Path::new(local),
                pinned_sha,
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

        // File contents at both revisions, read only if a comment needs the content fallback.
        let mut versions: Option<Option<(String, String)>> = None;

        for c in comments {
            // A true multi-line range remaps both endpoints; otherwise only `line`.
            let range_start = c.start_line.filter(|&s| s != c.line);

            let direct = hunks.as_ref().and_then(|h| {
                let line = match anchor::remap_line(c.line, h) {
                    Remap::Shifted(l) => l,
                    Remap::Lost => return None,
                };
                match range_start {
                    Some(s) => match anchor::remap_line(s, h) {
                        Remap::Shifted(l) => Some((line, Some(l))),
                        Remap::Lost => None,
                    },
                    None => Some((line, c.start_line)),
                }
            });

            let (new_line, new_start) = match direct {
                Some(pair) => pair,
                None => {
                    // The diff could not follow the line (rewritten, or file not in the
                    // compare): look for the commented text in the new revision.
                    let block_start = range_start.unwrap_or(c.line);
                    let versions = versions.get_or_insert_with(|| {
                        read_versions(detail, pinned_sha, current, file_path)
                    });
                    let found = versions.as_ref().and_then(|(old, new)| {
                        let old: Vec<&str> = old.lines().collect();
                        let new: Vec<&str> = new.lines().collect();
                        let predicted = hunks
                            .as_ref()
                            .map_or(block_start, |h| anchor::predict_line(block_start, h));
                        match anchor::relocate(&old, &new, block_start, c.line, Some(predicted)) {
                            Remap::Shifted(l) => Some(l),
                            Remap::Lost => None,
                        }
                    });
                    let Some(first) = found else {
                        result.lost += 1;
                        continue;
                    };
                    result.relocated += 1;
                    (
                        first + (c.line - block_start),
                        range_start.map(|_| first).or(c.start_line),
                    )
                }
            };

            conn.execute(
                pass.root_update_sql(),
                params![new_line, new_start, current, now(), c.id],
            )?;
            // The whole thread moves with its root: replies inherit the root's
            // anchor, so overwriting their line/start/pin wholesale is correct.
            conn.execute(
                pass.reply_update_sql(),
                params![new_line, new_start, current, now(), c.id],
            )?;
            result.reanchored += 1;
        }
    }

    Ok(())
}

/// Re-anchor a review's diff comments onto the review's pin: RIGHT-side
/// comments from `anchored_head_sha → head_sha`, LEFT-side from
/// `anchored_base_sha → base_sha`, each through the intervening per-file diff.
/// Two passes over one shared helper; the result aggregates both. Pure helper:
/// the caller holds the DB lock.
fn reanchor_review_comments(conn: &Connection, detail: &ReviewDetail) -> AppResult<ReanchorResult> {
    let mut result = ReanchorResult {
        reanchored: 0,
        lost: 0,
        skipped_no_change: 0,
        relocated: 0,
    };
    reanchor_pass(conn, detail, AnchorPass::Head, &mut result)?;
    reanchor_pass(conn, detail, AnchorPass::Base, &mut result)?;
    Ok(result)
}

/// Re-anchor a draft review's diff comments onto its pin, reporting how many
/// moved, were lost, or were already current.
#[tauri::command(async)]
pub fn reanchor_comments(review_id: i64, db: State<Db>) -> AppResult<ReanchorResult> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let detail = load_detail(&conn, review_id)?;
    reanchor_review_comments(&conn, &detail)
}

/// What `import_comments` did: copies made, duplicates skipped, and how the
/// follow-up re-anchor went (`reanchor_error` when it failed after a good copy).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommentsResult {
    pub imported: usize,
    pub skipped: usize,
    pub reanchor: Option<ReanchorResult>,
    pub reanchor_error: Option<String>,
}

/// Copy the selected unpublished comment threads of `source_id` into the draft
/// `review_id` (same target). `types` holds `"mine"` (untagged) and/or comment
/// tags. Skips roots the destination already has (same file/side/line/body).
/// Returns `(imported, skipped)`. Pure DB work; the caller re-anchors.
fn copy_comments(
    conn: &Connection,
    review_id: i64,
    source_id: i64,
    types: &[String],
) -> AppResult<(usize, usize)> {
    ensure_draft(conn, review_id)?;
    if review_id == source_id {
        return Err(AppError::Other("pick a different review to import from".into()));
    }
    let source = get_review_row(conn, source_id)?;
    let dest = get_review_row(conn, review_id)?;
    if source.target_id != dest.target_id {
        return Err(AppError::Other("can only import from a review on the same PR".into()));
    }
    for t in types {
        if t != "mine" {
            validate_tag(Some(t))?;
        }
    }

    let mut stmt = conn.prepare(
        "SELECT * FROM comment
         WHERE review_id = ?1 AND parent_id IS NULL
           AND github_comment_id IS NULL AND github_thread_root_id IS NULL
         ORDER BY file_path, line, created_at",
    )?;
    let roots: Vec<Comment> = stmt
        .query_map(params![source_id], Comment::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut reply_stmt = conn.prepare(
        "SELECT * FROM comment WHERE parent_id = ?1 AND github_comment_id IS NULL ORDER BY created_at, id",
    )?;

    let tx = conn.unchecked_transaction()?;
    let (mut imported, mut skipped) = (0, 0);
    for root in roots {
        let kind = root.tag.as_deref().unwrap_or("mine");
        if !types.iter().any(|t| t == kind) {
            continue;
        }
        let dup: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM comment
                WHERE review_id = ?1 AND parent_id IS NULL AND file_path = ?2 AND subject_type = ?3
                  AND side = ?4 AND line = ?5 AND body = ?6)",
            params![review_id, root.file_path, root.subject_type, root.side, root.line, root.body],
            |r| r.get(0),
        )?;
        if dup {
            skipped += 1;
            continue;
        }
        // An unpinned comment was written against the source's commits, so
        // pin it there and let re-anchoring map it forward.
        let head = root.anchored_head_sha.clone().or_else(|| source.head_sha.clone());
        let base = root
            .anchored_base_sha
            .clone()
            .or_else(|| (root.side == "LEFT").then(|| source.base_sha.clone()).flatten());
        let ts = now();
        let insert = |parent: Option<i64>, c: &Comment| -> AppResult<i64> {
            tx.execute(
                "INSERT INTO comment
                    (review_id, file_path, subject_type, origin, side, line, start_line, diff_hunk, body,
                     parent_id, anchored_head_sha, anchored_base_sha, resolved_at, tag, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
                params![
                    review_id,
                    c.file_path,
                    c.subject_type,
                    c.origin,
                    c.side,
                    c.line,
                    c.start_line,
                    c.diff_hunk,
                    c.body,
                    parent,
                    head,
                    base,
                    c.resolved_at,
                    c.tag,
                    ts,
                ],
            )?;
            Ok(tx.last_insert_rowid())
        };
        let new_root = insert(None, &root)?;
        let replies: Vec<Comment> = reply_stmt
            .query_map(params![root.id], Comment::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        for reply in &replies {
            insert(Some(new_root), reply)?;
        }
        imported += 1;
    }
    if imported > 0 {
        tx.execute("UPDATE review SET updated_at = ?1 WHERE id = ?2", params![now(), review_id])?;
    }
    tx.commit()?;
    Ok((imported, skipped))
}

pub fn import_comments_impl(
    db: &Db,
    review_id: i64,
    source_review_id: i64,
    types: Vec<String>,
) -> AppResult<ImportCommentsResult> {
    let (imported, skipped, detail) = {
        let conn = db.0.lock().unwrap();
        let (imported, skipped) = copy_comments(&conn, review_id, source_review_id, &types)?;
        let detail = if imported > 0 { Some(load_detail(&conn, review_id)?) } else { None };
        (imported, skipped, detail)
    };
    let (mut reanchor, mut reanchor_error) = (None, None);
    if let Some(detail) = detail {
        // Same as `reanchor_comments`: the lock is held while it remaps.
        let conn = db.0.lock().unwrap();
        match reanchor_review_comments(&conn, &detail) {
            Ok(r) => reanchor = Some(r),
            Err(e) => reanchor_error = Some(e.to_string()),
        }
    }
    Ok(ImportCommentsResult { imported, skipped, reanchor, reanchor_error })
}

/// Import comments from an earlier review of the same target, then re-anchor
/// them onto this review's commits.
#[tauri::command(async)]
pub fn import_comments(
    review_id: i64,
    source_review_id: i64,
    types: Vec<String>,
    db: State<Db>,
) -> AppResult<ImportCommentsResult> {
    import_comments_impl(&db, review_id, source_review_id, types)
}

/// A new draft pinned at `pin` (the target's live SHAs right now).
fn new_review_for_target(conn: &Connection, target_id: i64, pin: &LiveShas) -> AppResult<Review> {
    let ts = now();
    conn.execute(
        "INSERT INTO review (target_id, body, status, base_sha, head_sha, created_at, updated_at)
         VALUES (?1, '', 'draft', ?2, ?3, ?4, ?4)",
        params![target_id, pin.base_sha, pin.head_sha, ts],
    )?;
    get_review_row(conn, conn.last_insert_rowid())
}

/// The target's open review (its pin untouched), else a fresh one pinned at
/// `pin`. With `reuse_published` (opening from the UI) a published review is
/// reopened too, so a target only gets a new review when it has none; without
/// it (`cr create`, which must write) only once all of its reviews are published.
fn review_for_target(conn: &Connection, target_id: i64, pin: &LiveShas, reuse_published: bool) -> AppResult<Review> {
    if let Some(review) = latest_review(conn, target_id, reuse_published)? {
        return Ok(review);
    }
    new_review_for_target(conn, target_id, pin)
}

/// `latest_open_review`, falling back to the most recently updated published
/// review when `reuse_published`.
fn latest_review(conn: &Connection, target_id: i64, reuse_published: bool) -> AppResult<Option<Review>> {
    if let Some(review) = latest_open_review(conn, target_id)? {
        return Ok(Some(review));
    }
    if !reuse_published {
        return Ok(None);
    }
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM review WHERE target_id = ?1 ORDER BY updated_at DESC, id DESC LIMIT 1",
            params![target_id],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|id| get_review_row(conn, id)).transpose()
}

/// The most recently updated unpublished review (draft or pending on GitHub)
/// on a target, if any.
fn latest_open_review(conn: &Connection, target_id: i64) -> AppResult<Option<Review>> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM review WHERE target_id = ?1 AND status != 'published'
             ORDER BY updated_at DESC, id DESC LIMIT 1",
            params![target_id],
            |r| r.get(0),
        )
        .optional()?;
    id.map(|id| get_review_row(conn, id)).transpose()
}

/// A PR's open review (or, with `reuse_published`, its latest published one),
/// found from local rows only: no network, no mirror refresh. `None` when the
/// repo, target or review isn't there yet.
fn existing_pr_review(
    conn: &Connection,
    owner: &str,
    name: &str,
    pr_number: i64,
    reuse_published: bool,
) -> AppResult<Option<Review>> {
    let Some(repo_id) = find_remote_repository_id(conn, owner, name)? else {
        return Ok(None);
    };
    let target_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM target WHERE repo_id = ?1 AND kind = 'github_pr' AND github_pr_number = ?2",
            params![repo_id, pr_number],
            |r| r.get(0),
        )
        .optional()?;
    match target_id {
        Some(id) => latest_review(conn, id, reuse_published),
        None => Ok(None),
    }
}

/// A PR's refs and live head as the mirror last saw them; `None` when the
/// mirror doesn't have the PR (or its refs) yet.
fn mirror_pr_info(conn: &Connection, repo_id: i64, pr_number: i64) -> AppResult<Option<(PrInfo, String)>> {
    let row: Option<(String, Option<PrInfo>)> = conn
        .query_row(
            "SELECT node_id, title, base_ref, head_ref, head_sha FROM gh_pr WHERE repo_id = ?1 AND number = ?2",
            params![repo_id, pr_number],
            |r| {
                let info = (|| {
                    Some(PrInfo { title: r.get(1).ok()?, base_ref: r.get(2).ok()?, head_ref: r.get(3).ok()?, head_sha: r.get(4).ok()? })
                })();
                Ok((r.get(0)?, info))
            },
        )
        .optional()?;
    Ok(row.and_then(|(node_id, info)| Some((info?, node_id))))
}

/// Open the review of a local virtual PR (creating/reusing its target): its
/// open review if any (or a published one with `reuse_published`), else a new draft.
pub fn create_review_impl(
    conn: &Connection,
    repo_id: i64,
    repo_path: &str,
    base_ref: &str,
    head_ref: &str,
    three_dot: bool,
    reuse_published: bool,
) -> AppResult<Review> {
    let (target, live) =
        get_or_create_local_target(conn, repo_id, repo_path, base_ref, head_ref, three_dot)?;
    review_for_target(conn, target.id, &live, reuse_published)
}

#[tauri::command]
pub async fn create_review(
    repo_id: i64,
    base_ref: String,
    head_ref: String,
    three_dot: bool,
    db: State<'_, Db>,
) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let local_path: Option<String> = conn
        .query_row(
            "SELECT local_path FROM repository WHERE id = ?1",
            params![repo_id],
            |r| r.get(0),
        )
        .map_err(AppError::from)?;
    let repo_path = local_path.ok_or_else(|| {
        AppError::Other("creating a local review requires a local clone".into())
    })?;
    create_review_impl(&conn, repo_id, &repo_path, &base_ref, &head_ref, three_dot, true)
}

/// Open the review of a real GitHub PR (creating/reusing its target): its open
/// review if any (or a published one with `reuse_published`), else a new draft.
/// Identifies the PR by `owner/name` + number so any inbox PR can be opened, with
/// no local clone required: the repo is resolved to a local clone if one is
/// added, otherwise a clone-less remote context.
pub fn create_review_for_pr_impl(
    db: &Db,
    owner: &str,
    name: &str,
    pr_number: i64,
    base_paths: &[String],
    reuse_published: bool,
) -> AppResult<Review> {
    // Reopening a review needs no GitHub round-trip: the tab refreshes the mirror itself.
    if let Some(review) = existing_pr_review(&db.0.lock().unwrap(), owner, name, pr_number, reuse_published)? {
        return Ok(review);
    }
    let repo_id = get_or_create_remote_repository(&db.0.lock().unwrap(), owner, name, base_paths)?.id;
    // A new review pins the mirror's head; the tab moves an untouched pin if it was stale.
    let cached = mirror_pr_info(&db.0.lock().unwrap(), repo_id, pr_number)?;
    let (info, node_id) = match cached {
        Some(c) => c,
        None => sync_pr(db, owner, name, pr_number)?,
    };
    let merge_base = provider_for()
        .merge_base_sha(owner, name, &info.base_ref, &info.head_sha)
        .ok();
    let conn = db.0.lock().unwrap();
    let target = get_or_create_pr_target(&conn, repo_id, pr_number, &info, Some(&node_id))?;
    let live = LiveShas { base_sha: merge_base, head_sha: Some(info.head_sha.clone()) };
    review_for_target(&conn, target.id, &live, reuse_published)
}

#[tauri::command]
pub async fn create_review_for_pr(
    owner: String,
    name: String,
    pr_number: i64,
    base_paths: Vec<String>,
    db: State<'_, Db>,
) -> AppResult<Review> {
    create_review_for_pr_impl(&db, &owner, &name, pr_number, &base_paths, true)
}

enum DiffCommand {
    /// The PR's live diff: only when the review has no resolved pin.
    LivePr { ctx: GhRepo, number: i64 },
    /// A PR's diff at the review's pin: from the local clone when it has (or
    /// can fetch) both commits, else the compare API.
    PinnedPr { clone: Option<PathBuf>, owner_name: Option<(String, String)>, base: String, head: String },
    /// What changed in a PR since an earlier pin, rebase-aware (`git::interdiff`);
    /// without a usable clone, the compare API between the two heads.
    Interdiff {
        clone: Option<PathBuf>,
        owner_name: Option<(String, String)>,
        since: (String, String),
        base: String,
        head: String,
    },
    Local { path: PathBuf, base: String, head: String, three_dot: bool },
}

/// Everything needed to produce a review's diff, read from the DB so the
/// `gh`/`git` call can run after the DB lock is released.
pub struct DiffSource {
    command: DiffCommand,
    /// None when a SHA is unresolved: the diff then follows a moving ref.
    key: Option<diff_cache::DiffKey>,
}

pub fn diff_source(conn: &Connection, review_id: i64) -> AppResult<DiffSource> {
    let detail = load_detail(conn, review_id)?;
    let t = &detail.target;
    let pin = review_pin(&detail.review);
    let since = review_since(&detail.review);
    let command = match (t.kind.as_str(), &pin.base_sha, &pin.head_sha) {
        ("github_pr", Some(base), Some(head)) if since.is_some() => DiffCommand::Interdiff {
            clone: detail.local_path.as_ref().map(PathBuf::from),
            owner_name: repo_owner_name(conn, t.repo_id)?,
            since: since.clone().unwrap_or_default(),
            base: base.clone(),
            head: head.clone(),
        },
        ("github_pr", Some(base), Some(head)) => DiffCommand::PinnedPr {
            clone: detail.local_path.as_ref().map(PathBuf::from),
            owner_name: repo_owner_name(conn, t.repo_id)?,
            base: base.clone(),
            head: head.clone(),
        },
        ("github_pr", _, _) => DiffCommand::LivePr {
            ctx: gh_ctx_for_repo(conn, t.repo_id)?,
            number: t
                .github_pr_number
                .ok_or_else(|| AppError::Other("PR target missing number".into()))?,
        },
        _ => {
            let local_path = detail.local_path.as_deref().ok_or_else(|| {
                AppError::Other("local diff requires a local clone".into())
            })?;
            // Prefer the pinned SHA over the branch name: a branch can be deleted
            // or moved out from under a review, but the SHA it was pinned to
            // still diffs correctly as long as the commit itself survives.
            DiffCommand::Local {
                path: PathBuf::from(local_path),
                base: pin.base_sha.clone().unwrap_or_else(|| t.base_ref.clone()),
                head: pin.head_sha.clone().unwrap_or_else(|| t.head_ref.clone()),
                three_dot: t.three_dot,
            }
        }
    };
    let key = match (&pin.base_sha, &pin.head_sha) {
        (Some(base), Some(head)) => Some(diff_cache::DiffKey {
            repo_id: t.repo_id,
            pr_number: t.github_pr_number.filter(|_| t.kind == "github_pr"),
            base_sha: base.clone(),
            head_sha: head.clone(),
            three_dot: t.three_dot,
            since: since.filter(|_| t.kind == "github_pr"),
        }),
        _ => None,
    };
    Ok(DiffSource { command, key })
}

/// The earlier pin a "changes since" review diffs from, when it has one.
pub(crate) fn review_since(review: &Review) -> Option<(String, String)> {
    Some((review.since_base_sha.clone()?, review.since_head_sha.clone()?))
}

/// The since side of a PR interdiff, resolved against `new_base`; `None` when it
/// can't be (no local clone, or the commits aren't reachable).
fn interdiff_since_side(
    clone: Option<&Path>,
    since: &(String, String),
    new_base: &str,
) -> Option<git::InterdiffBase> {
    git::interdiff_base(clone?, &since.0, &since.1, new_base).ok()
}

fn pinned_interdiff(
    clone: Option<&Path>,
    owner_name: Option<&(String, String)>,
    since: &(String, String),
    base: &str,
    head: &str,
) -> AppResult<String> {
    let local = clone.map(|repo| git::interdiff(repo, &since.0, &since.1, base, head));
    match (local, owner_name) {
        (Some(Ok(diff)), _) => Ok(diff),
        // No clone (or it lacks the commits): between the two heads, which also
        // shows whatever a rebase pulled in.
        (_, Some((owner, name))) => provider_for().compare_diff(owner, name, &since.1, head),
        (Some(Err(e)), None) => Err(e),
        (None, None) => Err(AppError::Other("interdiff needs a local clone or a GitHub remote".into())),
    }
}

fn pinned_pr_diff(
    clone: Option<&Path>,
    owner_name: Option<&(String, String)>,
    base: &str,
    head: &str,
) -> AppResult<String> {
    // A clone that lacks the commits would first download every object of the PR (screenshots
    // included) before the diff could show. The compare API serves the same diff right away while
    // the clone catches up in the background; if it fails, fall back to fetching first.
    if let (Some(repo), Some((owner, name))) = (clone, owner_name) {
        let missing: Vec<&str> = [base, head].into_iter().filter(|sha| !git::has_commit(repo, sha)).collect();
        if !missing.is_empty() {
            if let Ok(diff) = provider_for().compare_diff(owner, name, base, head) {
                git::fetch_commits_in_background(repo, &missing);
                return Ok(diff);
            }
        }
    }
    let local = clone.map(|repo| -> AppResult<String> {
        git::ensure_commits(repo, &[base, head])?;
        // `base` is already the merge-base, so two-dot is the PR's diff.
        git::diff(repo, base, head, false)
    });
    match (local, owner_name) {
        (Some(Ok(diff)), _) => Ok(diff),
        (_, Some((owner, name))) => provider_for().compare_diff(owner, name, base, head),
        (Some(Err(e)), None) => Err(e),
        (None, None) => Err(AppError::Other("PR diff needs a local clone or a GitHub remote".into())),
    }
}

impl DiffSource {
    pub fn load(&self) -> AppResult<Arc<String>> {
        let run = || match &self.command {
            DiffCommand::LivePr { ctx, number } => provider_for().pr_diff(ctx, *number),
            DiffCommand::PinnedPr { clone, owner_name, base, head } => {
                pinned_pr_diff(clone.as_deref(), owner_name.as_ref(), base, head)
            }
            DiffCommand::Interdiff { clone, owner_name, since, base, head } => {
                pinned_interdiff(clone.as_deref(), owner_name.as_ref(), since, base, head)
            }
            DiffCommand::Local { path, base, head, three_dot } => {
                git::diff(path, base, head, *three_dot)
            }
        };
        match &self.key {
            Some(key) => diff_cache::get_or_load(key, run),
            None => run().map(Arc::new),
        }
    }
}

/// The diff for a review's target: GitHub PR diff via `gh`, or a local
/// base...head git diff for virtual PRs. Runs under `conn`'s lock; Tauri
/// commands use [`diff_source`] + [`DiffSource::load`] instead.
pub fn review_diff_impl(conn: &Connection, review_id: i64) -> AppResult<Arc<String>> {
    diff_source(conn, review_id)?.load()
}

#[tauri::command(async)]
pub fn review_diff(review_id: i64, app: tauri::AppHandle, db: State<Db>) -> AppResult<String> {
    let source = diff_source(&db.0.lock().unwrap(), review_id)?;
    let diff = source.load()?;
    // A small set of changed images is diffed in the background without being asked.
    crate::image_diff::runner::maybe_auto(&app, review_id, diff.clone());
    Ok(diff.as_ref().clone())
}

/// Full source of one side of a file (LEFT→base, RIGHT→head), used to reveal
/// collapsed context between hunks and to render the full-file review pane.
/// For GitHub PR targets LEFT is the merge-base blob (`target.base_sha`, lazily
/// backfilled here for rows created before it was populated). Reads the local
/// blob via `git show`; for GitHub PR targets whose commit isn't checked out
/// locally, falls back to the GitHub contents API.
#[tauri::command(async)]
pub fn file_source(
    review_id: i64,
    file_path: String,
    side: String,
    db: State<Db>,
) -> AppResult<String> {
    let (detail, owner_name) = {
        let conn = db.0.lock().unwrap();
        let detail = load_detail(&conn, review_id)?;
        let owner_name = repo_owner_name(&conn, detail.target.repo_id)?;
        (detail, owner_name)
    };
    let sha: Option<String> = match side.as_str() {
        "LEFT" => detail.review.base_sha.clone(),
        "RIGHT" => detail.review.head_sha.clone(),
        other => return Err(AppError::Other(format!("invalid side: {other}"))),
    };

    // Lazy backfill: github_pr targets created before base_sha was populated (or
    // whose resolution failed) store NULL. Resolve the merge-base now and persist
    // it so the row heals without a manual refresh.
    let sha = match (sha, side.as_str(), detail.target.kind.as_str()) {
        (None, "LEFT", "github_pr") => {
            let resolved = owner_name.as_ref().and_then(|(o, n)| {
                let head = detail.review.head_sha.as_deref().unwrap_or(&detail.target.head_ref);
                provider_for()
                    .merge_base_sha(o, n, &detail.target.base_ref, head)
                    .ok()
            });
            if let Some(mb) = &resolved {
                db.0.lock().unwrap().execute(
                    "UPDATE review SET base_sha = ?1 WHERE id = ?2 AND base_sha IS NULL",
                    params![mb, review_id],
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

    // A "changes since" review's LEFT side is the earlier PR replayed onto the
    // new merge-base (a tree, not a commit); without a clone, the earlier head.
    let mut sha = sha;
    if side == "LEFT" && detail.target.kind == "github_pr" {
        if let Some(since) = review_since(&detail.review) {
            if let Some(repo) = detail.local_path.as_deref().map(std::path::Path::new) {
                if let Some(left) = interdiff_since_side(Some(repo), &since, &sha) {
                    if let Ok(source) = git::show_blob(repo, left.treeish(), &file_path) {
                        return Ok(source);
                    }
                }
            }
            sha = since.1;
        }
    }

    // Remote-only (clone-less) PR targets have no local blob, so skip the
    // guaranteed-to-fail `git show` and go straight to the GitHub contents API.
    if let Some(ref path) = detail.local_path {
        let repo = std::path::Path::new(path);
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

    match owner_name {
        Some((owner, name)) => provider_for().file_at_ref(&owner, &name, &file_path, &sha),
        None => Err(AppError::Other(
            "file source unavailable: no local clone and no GitHub remote".into(),
        )),
    }
}

/// Declarations of `symbol` in the pinned commit of one side (LEFT→base, RIGHT→head), for the
/// diff view's go-to-definition. `file_path` is the file the symbol was clicked in (it picks the
/// language and ranks same-file hits first, then `diff_files`). `hover` marks a speculative
/// lookup (Cmd held over a word) that is abandoned, with a "cancelled" error, as soon as a newer
/// lookup starts. Needs a local clone with the commit; otherwise there is nothing to search and
/// the result is empty.
#[tauri::command]
pub async fn find_definition(
    review_id: i64,
    symbol: String,
    file_path: String,
    side: String,
    diff_files: Vec<String>,
    hover: bool,
    db: State<'_, Db>,
) -> AppResult<Vec<crate::definitions::DefinitionHit>> {
    let detail = load_detail(&db.0.lock().unwrap(), review_id)?;
    let sha = match side.as_str() {
        "LEFT" => detail.review.base_sha.clone(),
        "RIGHT" => detail.review.head_sha.clone(),
        other => return Err(AppError::Other(format!("invalid side: {other}"))),
    };
    let (Some(repo), Some(sha)) = (detail.local_path.as_deref(), sha) else {
        return Ok(vec![]);
    };
    // `git grep` over a big tree can take a while; keep it off the command thread.
    let repo = PathBuf::from(repo);
    tauri::async_runtime::spawn_blocking(move || {
        crate::definitions::lookup(&repo, &sha, &symbol, &file_path, &diff_files, hover)
    })
    .await
    .map_err(|e| AppError::Other(format!("definition lookup task failed: {e}")))?
}

/// Enriches raw `review` rows with their target, repo label and comment count.
fn summarize_reviews(conn: &Connection, reviews: Vec<Review>) -> AppResult<Vec<ReviewSummary>> {
    let mut out = Vec::with_capacity(reviews.len());
    for review in reviews {
        let target = get_target(conn, review.target_id)?;
        let (repo_id_v, label, remote): (i64, String, Option<String>) = conn.query_row(
            "SELECT id, COALESCE(friendly_name, remote_owner || '/' || remote_name, local_path, 'unknown'),
                    remote_owner || '/' || remote_name
             FROM repository WHERE id = ?1",
            params![target.repo_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
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
            repo_label: label,
            repo_remote: remote,
            comment_count,
        });
    }
    Ok(out)
}

/// All reviews (optionally filtered to one repo), newest first, for the Reviews list.
pub fn list_reviews_impl(conn: &Connection, repo_id: Option<i64>) -> AppResult<Vec<ReviewSummary>> {
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
    summarize_reviews(conn, reviews)
}

#[tauri::command]
pub fn list_reviews(repo_id: Option<i64>, db: State<Db>) -> AppResult<Vec<ReviewSummary>> {
    let conn = db.0.lock().unwrap();
    list_reviews_impl(&conn, repo_id)
}

/// All reviews on a single target, newest first — for the review header's
/// same-target switcher.
pub fn list_target_reviews_impl(conn: &Connection, target_id: i64) -> AppResult<Vec<ReviewSummary>> {
    let mut stmt = conn.prepare("SELECT * FROM review WHERE target_id = ?1 ORDER BY updated_at DESC")?;
    let reviews = stmt
        .query_map(params![target_id], Review::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    summarize_reviews(conn, reviews)
}

#[tauri::command]
pub fn list_target_reviews(target_id: i64, db: State<Db>) -> AppResult<Vec<ReviewSummary>> {
    let conn = db.0.lock().unwrap();
    list_target_reviews_impl(&conn, target_id)
}

/// Start an additional draft on a target, pinned at `pin`, even while it
/// already has an open review — e.g. a second pass with a different kind of
/// comment. Reopening the target (`review_for_target`, the board's
/// `existing_review_id`) lands on the most recently updated open review.
pub fn new_review_on_target_impl(conn: &Connection, target_id: i64, pin: &LiveShas) -> AppResult<Review> {
    new_review_for_target(conn, target_id, pin)
}

/// `new_review_on_target_impl` at the target's live head; for a PR that means `gh`.
pub fn new_review_on_target_live(db: &Db, target_id: i64) -> AppResult<Review> {
    let target = get_target(&db.0.lock().unwrap(), target_id)?;
    let live = resolve_live(db, &target)?;
    new_review_on_target_impl(&db.0.lock().unwrap(), target_id, &live)
}

/// `async`: the new review pins at the live head, which for a PR means `gh`.
#[tauri::command]
pub async fn new_review_on_target(target_id: i64, db: State<'_, Db>) -> AppResult<Review> {
    new_review_on_target_live(&db, target_id)
}

/// A new draft on a PR target that covers only what changed since `review_id`'s
/// pin (works for a draft, a pending or a published review). The new review is
/// pinned at the PR's live commits; its diff starts from the source's pin
/// (rebase-aware, see `git::interdiff`), recorded in `since_*_sha`.
pub fn new_review_since_impl(db: &Db, review_id: i64, pin: Option<LiveShas>) -> AppResult<Review> {
    let (target, since, live) = since_plan(db, review_id, pin)?;
    let conn = db.0.lock().unwrap();
    let review = new_review_for_target(&conn, target.id, &live)?;
    conn.execute(
        "UPDATE review SET since_base_sha = ?1, since_head_sha = ?2 WHERE id = ?3",
        params![since.0, since.1, review.id],
    )?;
    get_review_row(&conn, review.id)
}

/// What a "changes since" review needs: the PR target, the source's pin and the
/// commits to review up to. `pin` (from a quick view) skips resolving the live
/// head, so the review covers exactly the diff that was previewed.
fn since_plan(db: &Db, review_id: i64, pin: Option<LiveShas>) -> AppResult<(Target, (String, String), LiveShas)> {
    let (source, target) = {
        let conn = db.0.lock().unwrap();
        let source = get_review_row(&conn, review_id)?;
        let target = get_target(&conn, source.target_id)?;
        (source, target)
    };
    if target.kind != "github_pr" {
        return Err(AppError::Other("only PR reviews can be continued from new commits".into()));
    }
    let (Some(since_base), Some(since_head)) = (source.base_sha.clone(), source.head_sha.clone()) else {
        return Err(AppError::Other("this review has no pinned commits to continue from".into()));
    };
    let live = match pin {
        Some(pin) => pin,
        None => resolve_live(db, &target)?,
    };
    if live.base_sha.is_none() || live.head_sha.is_none() {
        return Err(AppError::Other("could not resolve the PR's latest commits".into()));
    }
    if !has_moved(&review_pin(&source), &live) {
        return Err(AppError::Other("the PR has no new commits since this review".into()));
    }
    Ok((target, (since_base, since_head), live))
}

/// `async`: pins at the PR's live head (unless `base_sha`/`head_sha` are given), which means `gh`.
#[tauri::command]
pub async fn new_review_since(
    review_id: i64,
    base_sha: Option<String>,
    head_sha: Option<String>,
    db: State<'_, Db>,
) -> AppResult<Review> {
    let pin = match (base_sha, head_sha) {
        (Some(base_sha), Some(head_sha)) => Some(LiveShas { base_sha: Some(base_sha), head_sha: Some(head_sha) }),
        _ => None,
    };
    new_review_since_impl(&db, review_id, pin)
}

/// The commits between a review's pinned base and head (a PR's own commits, newest
/// first). Fetches missing PR commits from origin first. Needs a local clone.
#[tauri::command]
pub async fn review_commits(review_id: i64, db: State<'_, Db>) -> AppResult<Vec<crate::graph::GraphCommit>> {
    let (review, target) = {
        let conn = db.0.lock().unwrap();
        let review = get_review_row(&conn, review_id)?;
        let target = get_target(&conn, review.target_id)?;
        (review, target)
    };
    let (Some(base), Some(head)) = (review.base_sha, review.head_sha) else {
        return Err(AppError::Other("this review has no pinned commits".into()));
    };
    let clone = repo_local_path_of(&db, target.repo_id)?
        .ok_or_else(|| AppError::Other("the commit list requires a local clone".into()))?;
    let clone = Path::new(&clone);
    git::ensure_commit(clone, &base)?;
    git::ensure_commit(clone, &head)?;
    crate::graph::range_log(clone, &base, &head)
}

/// The preview behind "Quick view new commits": the diff a since-review would
/// start with, and the commits it would pin, with nothing stored.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickInterdiff {
    pub diff: String,
    pub base_sha: String,
    pub head_sha: String,
    pub rebased: bool,
    pub approximate: bool,
}

#[tauri::command]
pub async fn quick_interdiff(review_id: i64, db: State<'_, Db>) -> AppResult<QuickInterdiff> {
    let (target, since, live) = since_plan(&db, review_id, None)?;
    let (Some(base), Some(head)) = (live.base_sha, live.head_sha) else {
        return Err(AppError::Other("could not resolve the PR's latest commits".into()));
    };
    let (clone, owner_name, repo_id) = {
        let conn = db.0.lock().unwrap();
        let local_path: Option<String> =
            conn.query_row("SELECT local_path FROM repository WHERE id = ?1", params![target.repo_id], |r| r.get(0))?;
        (local_path.map(PathBuf::from), repo_owner_name(&conn, target.repo_id)?, target.repo_id)
    };
    let source = DiffSource {
        key: Some(diff_cache::DiffKey {
            repo_id,
            pr_number: target.github_pr_number,
            base_sha: base.clone(),
            head_sha: head.clone(),
            three_dot: target.three_dot,
            since: Some(since.clone()),
        }),
        command: DiffCommand::Interdiff {
            clone: clone.clone(),
            owner_name,
            since: since.clone(),
            base: base.clone(),
            head: head.clone(),
        },
    };
    let diff = source.load()?.as_ref().clone();
    let rebased = since.0 != base;
    let approximate = rebased
        && !matches!(
            interdiff_since_side(clone.as_deref(), &since, &base),
            Some(git::InterdiffBase::Same(_) | git::InterdiffBase::Replayed(_))
        );
    Ok(QuickInterdiff { diff, base_sha: base, head_sha: head, rebased, approximate })
}

/// How a "changes since" review's diff was derived, for its header chip.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterdiffMeta {
    /// The PR's merge-base moved since the earlier pin.
    pub rebased: bool,
    /// The rebase could not be replayed cleanly (or there is no clone): the diff
    /// may include upstream changes or miss some context.
    pub approximate: bool,
}

pub fn interdiff_meta_impl(db: &Db, review_id: i64) -> AppResult<Option<InterdiffMeta>> {
    let detail = load_detail(&db.0.lock().unwrap(), review_id)?;
    let Some(since) = review_since(&detail.review) else {
        return Ok(None);
    };
    let Some(new_base) = detail.review.base_sha.as_deref() else {
        return Ok(None);
    };
    let rebased = since.0 != new_base;
    let clone = detail.local_path.as_deref().map(Path::new);
    let approximate = rebased
        && !matches!(
            interdiff_since_side(clone, &since, new_base),
            Some(git::InterdiffBase::Same(_) | git::InterdiffBase::Replayed(_))
        );
    Ok(Some(InterdiffMeta { rebased, approximate }))
}

#[tauri::command(async)]
pub fn review_interdiff_meta(review_id: i64, db: State<Db>) -> AppResult<Option<InterdiffMeta>> {
    interdiff_meta_impl(&db, review_id)
}

/// Human-friendly repo label: the user-set friendly name if any, else
/// `owner/name` if known, else the local path.
pub fn repo_label(conn: &Connection, repo_id: i64) -> AppResult<String> {
    conn.query_row(
        "SELECT COALESCE(friendly_name, remote_owner || '/' || remote_name, local_path, 'unknown') FROM repository WHERE id = ?1",
        params![repo_id],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// Load the full review state — shared by the get_review command and exporters.
pub fn load_detail(conn: &Connection, review_id: i64) -> AppResult<ReviewDetail> {
    let review = get_review_row(conn, review_id)?;
    let target = get_target(conn, review.target_id)?;
    let (local_path, remote_owner, remote_name, friendly_name): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT local_path, remote_owner, remote_name, friendly_name FROM repository WHERE id = ?1",
        params![target.repo_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
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
        local_path,
        remote_owner,
        remote_name,
        friendly_name,
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

/// Persist panel widths. UI state — allowed on published reviews.
#[tauri::command]
pub fn set_panel_widths(
    review_id: i64,
    left_panel_width: i64,
    right_panel_width: i64,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE review SET left_panel_width = ?1, right_panel_width = ?2 WHERE id = ?3",
        params![left_panel_width, right_panel_width, review_id],
    )?;
    Ok(())
}

/// Persist the sidebar collapsed/expanded state. UI state — allowed on published reviews.
#[tauri::command]
pub fn set_sidebar_collapsed(review_id: i64, collapsed: bool, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE review SET sidebar_collapsed = ?1 WHERE id = ?2",
        params![collapsed as i64, review_id],
    )?;
    Ok(())
}

/// Persist whether the PR details panel is collapsed. UI state — allowed on published reviews.
#[tauri::command]
pub fn set_pr_details_collapsed(review_id: i64, collapsed: bool, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE review SET pr_details_collapsed = ?1 WHERE id = ?2",
        params![collapsed as i64, review_id],
    )?;
    Ok(())
}

/// Autosave the review summary and/or verdict. Pass `event = ""` to clear the verdict.
pub fn update_review_impl(
    conn: &Connection,
    review_id: i64,
    body: Option<String>,
    event: Option<String>,
) -> AppResult<()> {
    ensure_draft(conn, review_id)?;
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

#[tauri::command]
pub fn update_review(
    review_id: i64,
    body: Option<String>,
    event: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    update_review_impl(&conn, review_id, body, event)
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
/// Whether a comment is pinned to the SHA its side's line numbers are valid
/// against (LEFT → target.base_sha, RIGHT → target.head_sha) and so safe to
/// post inline. A NULL pin is treated as anchored — legacy rows keep today's
/// behaviour. LEFT comments therefore stop folding into the body just because
/// the head moved (their line numbers live in base coordinates).
fn is_anchored_to(c: &Comment, review: &Review) -> bool {
    // A "changes since" review's LEFT side is a replayed tree GitHub has never
    // seen, so its line numbers can't be posted inline.
    if c.side == "LEFT" && review_since(review).is_some() {
        return false;
    }
    let (pin, current) = if c.side == "LEFT" {
        (c.anchored_base_sha.as_deref(), review.base_sha.as_deref())
    } else {
        (c.anchored_head_sha.as_deref(), review.head_sha.as_deref())
    };
    match pin {
        None => true,
        Some(sha) => Some(sha) == current,
    }
}

/// Replies to fold under a published root: never a tagged (`ai`/`local`) reply,
/// which stays local the same as a tagged root would.
fn publishable_replies<'a>(
    replies: &'a HashMap<i64, Vec<&'a Comment>>,
    root_id: i64,
) -> Vec<&'a Comment> {
    replies
        .get(&root_id)
        .map(|rs| rs.iter().filter(|r| r.tag.is_none()).copied().collect())
        .unwrap_or_default()
}

/// The comments publish posts inline, each paired with the exact body text sent
/// to GitHub. Single source of truth shared by `build_publish_payload` and the
/// post-publish id capture (Spec 17), so the matcher compares like-for-like.
/// The posted body folds each root's replies (Spec 11) in below it; Spec 13's
/// suggestion handling must apply its transforms HERE too — keep the roots-only
/// filter and the body projection in this one place.
fn inline_publish_comments(detail: &ReviewDetail) -> Vec<(&Comment, String)> {
    let replies = export::replies_by_root(&detail.comments);
    detail
        .comments
        .iter()
        .filter(|c| {
            c.subject_type != "file"
                && c.origin != "file_view"
                && c.parent_id.is_none()
                && c.github_thread_root_id.is_none()
                && c.tag.is_none()
                && is_anchored_to(c, &detail.review)
        })
        .map(|c| {
            let body = export::fold_replies(&c.body, &publishable_replies(&replies, c.id));
            (c, body)
        })
        .collect()
}

fn build_publish_payload(detail: &ReviewDetail) -> serde_json::Value {
    let event = gh_event(detail.review.event.as_deref());

    let comments: Vec<serde_json::Value> = inline_publish_comments(detail)
        .into_iter()
        .map(|(c, body)| {
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
    if let Some(sha) = &detail.review.head_sha {
        payload["commit_id"] = serde_json::json!(sha);
    }
    payload
}

/// GitHub rejects a COMMENT-event review with neither a body nor inline comments
/// (422 Validation Failed) — catch that locally with a clear message instead of
/// surfacing GitHub's opaque error. APPROVE/REQUEST_CHANGES are fine empty.
fn ensure_submittable(detail: &ReviewDetail, event: &str) -> AppResult<()> {
    if event != "COMMENT" {
        return Ok(());
    }
    if !inline_publish_comments(detail).is_empty() {
        return Ok(());
    }
    if !body_with_file_comments(detail).trim().is_empty() {
        return Ok(());
    }
    Err(AppError::Other(
        "this review has no comments and no summary — add one, or pick Approve/Request changes, before submitting".into(),
    ))
}

/// Same payload as a normal publish but with `event` omitted: GitHub's REST API
/// creates a PENDING review exactly when `event` is absent.
fn build_pending_payload(detail: &ReviewDetail) -> serde_json::Value {
    let mut payload = build_publish_payload(detail);
    payload
        .as_object_mut()
        .expect("payload is an object")
        .remove("event");
    payload
}

/// The review summary with any whole-file and full-file-pane comments appended as
/// trailing sections. GitHub can't anchor either kind inline (file comments have
/// no line; file-view comments may sit outside the diff), so they ride along in
/// the body instead.
fn body_with_file_comments(detail: &ReviewDetail) -> String {
    let mut body = detail.review.body.trim().to_string();
    let replies = export::replies_by_root(&detail.comments);
    let folded = |c: &Comment| export::fold_replies(&c.body, &publishable_replies(&replies, c.id));

    let file_comments: Vec<&Comment> = detail
        .comments
        .iter()
        .filter(|c| c.subject_type == "file" && c.parent_id.is_none() && c.tag.is_none())
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
        .filter(|c| c.origin == "file_view" && c.parent_id.is_none() && c.tag.is_none())
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
                && c.github_thread_root_id.is_none()
                && c.tag.is_none()
                && !is_anchored_to(c, &detail.review)
        })
        .collect();
    if !lost_comments.is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(if review_since(&detail.review).is_some() {
            "## Comments that could not be posted inline"
        } else {
            "## Comments that could not be re-anchored"
        });
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

/// Normalized anchor of a posted inline comment. Locals normalize
/// `start_line == line` to None because `build_publish_payload` only sends
/// `start_line` for true ranges and GitHub returns null for single-line comments.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct AnchorKey {
    path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
}

impl AnchorKey {
    fn local(c: &Comment) -> Self {
        Self {
            path: c.file_path.clone(),
            side: c.side.clone(),
            line: c.line,
            start_line: c.start_line.filter(|s| *s != c.line),
        }
    }

    /// None when the remote item lacks side/line (defensive; such items are skipped).
    fn remote(rc: &ReviewComment) -> Option<Self> {
        let side = rc.side.clone()?;
        let line = rc.line?;
        Some(Self {
            path: rc.path.clone(),
            side,
            line,
            start_line: rc.start_line,
        })
    }
}

/// Pure matcher pairing local posted comments with the remote review comments
/// GitHub minted for them. `local` items are `(comment id, anchor, body-as-posted)`.
/// Returns `(local_comment_id, github_comment_id)` pairs; each remote id is
/// consumed at most once; anything ambiguous is left out (caller leaves NULL).
fn match_review_comments(
    local: &[(i64, AnchorKey, String)],
    remote: &[ReviewComment],
) -> Vec<(i64, i64)> {
    use std::collections::HashMap;

    // Bucket remotes by anchor, preserving API order. Items without an anchor are
    // skipped entirely (never paired). `consumed` enforces one-id-per-pairing.
    struct RemoteEntry {
        github_id: i64,
        body: String,
        consumed: bool,
    }
    let mut remote_buckets: HashMap<AnchorKey, Vec<RemoteEntry>> = HashMap::new();
    for rc in remote {
        if let Some(key) = AnchorKey::remote(rc) {
            remote_buckets.entry(key).or_default().push(RemoteEntry {
                github_id: rc.id,
                body: rc.body.clone(),
                consumed: false,
            });
        }
    }

    // Bucket locals by anchor, preserving input order.
    let mut local_buckets: HashMap<AnchorKey, Vec<(i64, &str)>> = HashMap::new();
    let mut local_order: Vec<AnchorKey> = Vec::new();
    for (id, key, body) in local {
        let bucket = local_buckets.entry(key.clone()).or_insert_with(|| {
            local_order.push(key.clone());
            Vec::new()
        });
        bucket.push((*id, body.as_str()));
    }

    let mut matches = Vec::new();
    for key in &local_order {
        let locals = &local_buckets[key];
        let Some(remotes) = remote_buckets.get_mut(key) else {
            continue;
        };

        // Pass 1: exact body equality (byte equality, no trimming). Identical
        // duplicates therefore zip in order.
        let mut unpaired: Vec<(i64, &str)> = Vec::new();
        for (id, body) in locals {
            match remotes
                .iter_mut()
                .find(|r| !r.consumed && r.body == *body)
            {
                Some(r) => {
                    r.consumed = true;
                    matches.push((*id, r.github_id));
                }
                None => unpaired.push((*id, body)),
            }
        }

        // Pass 2: singleton fallback — exactly one unpaired local and exactly one
        // unconsumed remote on the same anchor pair despite body drift.
        let remaining: Vec<&mut RemoteEntry> =
            remotes.iter_mut().filter(|r| !r.consumed).collect();
        if unpaired.len() == 1 && remaining.len() == 1 {
            let (id, _) = unpaired[0];
            let r = remaining.into_iter().next().unwrap();
            r.consumed = true;
            matches.push((id, r.github_id));
        }
    }

    matches
}

/// Write matched ids. Plain helper so the SQL is unit-testable without network.
/// Deliberately bypasses `ensure_draft` (review is already locked) and leaves
/// `updated_at` alone (bookkeeping about the publish, not a user edit).
fn store_comment_id_matches(conn: &Connection, matches: &[(i64, i64)]) -> AppResult<usize> {
    for (comment_id, gh_id) in matches {
        conn.execute(
            "UPDATE comment SET github_comment_id = ?1 WHERE id = ?2",
            params![gh_id, comment_id],
        )?;
    }
    Ok(matches.len())
}

/// Best-effort post-publish capture: fetch the inline comments GitHub created for
/// the just-posted review and store their ids on the matching local rows. Errors
/// bubble to the caller, which logs and swallows them.
fn capture_github_comment_ids(
    conn: &Connection,
    detail: &ReviewDetail, // the post-reanchor detail the payload was built from
    owner: &str,
    name: &str,
    number: i64,
    gh_review_id: i64,
) -> AppResult<usize> {
    let remote = provider_for().review_comments(owner, name, number, gh_review_id)?;
    let local: Vec<(i64, AnchorKey, String)> = inline_publish_comments(detail)
        .into_iter()
        .map(|(c, body)| (c.id, AnchorKey::local(c), body))
        .collect();
    let matches = match_review_comments(&local, &remote);
    store_comment_id_matches(conn, &matches)
}

/// (remote_owner, remote_name) for a repo, or an error if it has no GitHub remote.
fn pr_remote(conn: &Connection, repo_id: i64) -> AppResult<(String, String)> {
    let (owner, name): (Option<String>, Option<String>) = conn.query_row(
        "SELECT remote_owner, remote_name FROM repository WHERE id = ?1",
        params![repo_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match (owner, name) {
        (Some(o), Some(n)) => Ok((o, n)),
        _ => Err(AppError::Other("repository has no GitHub remote".into())),
    }
}

/// Guards + remote lookup + head refresh + re-anchor shared by both publish
/// flavours. Returns the post-re-anchor detail and (owner, name, pr_number).
/// Holds no lock on return — callers re-lock for the GitHub round-trip + UPDATE.
fn prepare_publish(review_id: i64, db: &Db) -> AppResult<(ReviewDetail, String, String, i64)> {
    let detail = {
        let conn = db.0.lock().unwrap();
        load_detail(&conn, review_id)?
    };
    match detail.review.status.as_str() {
        "draft" => {}
        "published_pending" => {
            return Err(AppError::Other(
                "this review is already pending on GitHub — submit or discard it".into(),
            ))
        }
        _ => return Err(AppError::Other("this review is already published".into())),
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
    let (owner, name) = {
        let conn = db.0.lock().unwrap();
        pr_remote(&conn, detail.target.repo_id)?
    };

    // Publish against the review's own pin (the commits it was written
    // against); moving it is Update review's job. Re-anchor first so comments
    // written before an earlier pin move land on the pin or fold into the body.
    let conn = db.0.lock().unwrap();
    reanchor_review_comments(&conn, &detail)?;
    let detail = load_detail(&conn, review_id)?;

    Ok((detail, owner, name, number))
}

/// Publish a draft review to its GitHub PR via the line-based reviews API, then
/// lock it (published reviews can't be edited or re-published). Returns the
/// updated review.
// Async so the GitHub round-trip runs off the main thread — a sync command would
// block the webview and freeze the UI while publishing (it shells out to `gh`).
#[tauri::command]
pub async fn publish_review(review_id: i64, db: State<'_, Db>) -> AppResult<Review> {
    let (detail, owner, name, number) = prepare_publish(review_id, &db)?;
    ensure_submittable(&detail, gh_event(detail.review.event.as_deref()))?;

    let conn = db.0.lock().unwrap();

    let payload = build_publish_payload(&detail);

    let gh_id = provider_for().post_review(&owner, &name, number, &payload.to_string())?;
    log::info!(target: "publish", "review {review_id}: published to {owner}/{name}#{number} (github review {gh_id})");

    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'published', published_at = ?1, github_review_id = ?2, updated_at = ?1
         WHERE id = ?3",
        params![ts, gh_id, review_id],
    )?;

    // Best-effort: correlate the inline comments GitHub just minted back to our
    // local rows so future thread reply/resolve calls have a comment id. Never
    // fails the publish — the review is already locked at this point.
    match capture_github_comment_ids(&conn, &detail, &owner, &name, number, gh_id) {
        Ok(n) => {
            log::debug!(target: "publish", "stored {n} github comment ids for review {review_id}")
        }
        Err(e) => crate::logging::warn(&format!(
            "[publish.capture_ids] best-effort capture failed for review {review_id}: {e}"
        )),
    }

    get_review_row(&conn, review_id)
}

/// Remap GitHub's "max one pending review per user per PR" 422 to a clear message.
/// Only that specific conflict is remapped — other 422s surface verbatim.
fn map_pending_conflict(e: AppError) -> AppError {
    match &e {
        AppError::Gh(msg) if msg.to_lowercase().contains("pending review") => AppError::Other(
            "GitHub allows one pending review per reviewer on a PR — submit or discard \
             your existing pending review (here or on github.com) and retry"
                .into(),
        ),
        _ => e,
    }
}

/// The review must be 'published_pending' with a stored GitHub review id.
fn ensure_pending(detail: &ReviewDetail) -> AppResult<i64> {
    if detail.review.status != "published_pending" {
        return Err(AppError::Other(
            "this review has no pending GitHub review".into(),
        ));
    }
    detail
        .review
        .github_review_id
        .ok_or_else(|| AppError::Other("pending review is missing its GitHub review id".into()))
}

/// published_pending -> published (sets published_at = now).
fn mark_submitted(conn: &Connection, review_id: i64) -> AppResult<()> {
    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'published', published_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    Ok(())
}

/// published_pending -> draft (clears github_review_id; published_at stays NULL).
fn mark_discarded(conn: &Connection, review_id: i64) -> AppResult<()> {
    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'draft', github_review_id = NULL, updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    Ok(())
}

/// Push the draft to GitHub as a PENDING review (event omitted), then lock it
/// locally as 'published_pending'. github_review_id stores the pending review.
#[tauri::command(async)]
pub fn publish_review_pending(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let (detail, owner, name, number) = prepare_publish(review_id, &db)?;
    let conn = db.0.lock().unwrap();
    let payload = build_pending_payload(&detail);
    let gh_id = provider_for()
        .post_review(&owner, &name, number, &payload.to_string())
        .map_err(map_pending_conflict)?;
    log::info!(target: "publish", "review {review_id}: pending review {gh_id} created on {owner}/{name}#{number}");
    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'published_pending', github_review_id = ?1, updated_at = ?2
         WHERE id = ?3",
        params![gh_id, ts, review_id],
    )?;
    get_review_row(&conn, review_id)
}

/// Submit the PENDING review on GitHub with the stored verdict; lock as 'published'.
#[tauri::command(async)]
pub fn submit_pending_review(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let gh_id = ensure_pending(&detail)?;
    let event = gh_event(detail.review.event.as_deref());
    ensure_submittable(&detail, event)?;
    let number = detail
        .target
        .github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name) = pr_remote(&conn, detail.target.repo_id)?;
    provider_for().submit_pending_review(&owner, &name, number, gh_id, event)?;
    log::info!(target: "publish", "review {review_id}: pending review {gh_id} submitted on {owner}/{name}#{number}");
    mark_submitted(&conn, review_id)?;
    get_review_row(&conn, review_id)
}

/// Delete the PENDING review on GitHub; unlock the local draft.
#[tauri::command(async)]
pub fn discard_pending_review(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let gh_id = ensure_pending(&detail)?;
    let number = detail
        .target
        .github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name) = pr_remote(&conn, detail.target.repo_id)?;
    provider_for().delete_pending_review(&owner, &name, number, gh_id)?;
    log::info!(target: "publish", "review {review_id}: pending review {gh_id} discarded on {owner}/{name}#{number}");
    mark_discarded(&conn, review_id)?;
    get_review_row(&conn, review_id)
}

pub fn delete_review_impl(conn: &Connection, review_id: i64) -> AppResult<()> {
    if review_status(conn, review_id)? == "published_pending" {
        return Err(AppError::Other(
            "this review is pending on GitHub — discard the pending review before deleting it"
                .into(),
        ));
    }
    conn.execute("DELETE FROM review WHERE id = ?1", params![review_id])?;
    // The generated image diffs go with the review.
    crate::image_diff::forget_review(conn, review_id);
    Ok(())
}

#[tauri::command]
pub fn delete_review(review_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    delete_review_impl(&conn, review_id)
}

/// GitHub-PR review candidates for the "clean up merged PRs" action: reviews
/// backed by a known remote, excluding ones pending on GitHub (`delete_review_impl`
/// refuses those anyway).
pub fn closed_pr_candidates_impl(conn: &Connection) -> AppResult<Vec<(i64, String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT rv.id, r.remote_owner, r.remote_name, t.github_pr_number
         FROM review rv
         JOIN target t ON t.id = rv.target_id
         JOIN repository r ON r.id = t.repo_id
         WHERE t.kind = 'github_pr'
           AND t.github_pr_number IS NOT NULL
           AND r.remote_owner IS NOT NULL
           AND r.remote_name IS NOT NULL
           AND rv.status != 'published_pending'",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Find reviews whose PR is MERGED or CLOSED on GitHub, for the Reviews list
/// "clean up" button. `async` so the batched GitHub lookup doesn't block the UI
/// thread (same rationale as `pr_meta`); the DB lock is dropped before it runs.
#[tauri::command]
pub async fn find_closed_pr_reviews(db: State<'_, Db>) -> AppResult<Vec<i64>> {
    let candidates = {
        let conn = db.0.lock().unwrap();
        closed_pr_candidates_impl(&conn)?
    };
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<(String, String, i64)> = candidates
        .iter()
        .map(|(_, owner, name, number)| (owner.clone(), name.clone(), *number))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let states = provider_for().pr_states(&keys)?;
    Ok(candidates
        .into_iter()
        .filter(|(_, owner, name, number)| {
            matches!(
                states.get(&(owner.clone(), name.clone(), *number)).map(String::as_str),
                Some("MERGED") | Some("CLOSED")
            )
        })
        .map(|(id, ..)| id)
        .collect())
}

/// Delete a batch of reviews (the "clean up" button). Skips any id that fails
/// to delete — e.g. one that turned `published_pending` since the caller
/// fetched its candidate list — instead of aborting the whole batch; returns
/// how many were actually removed.
#[tauri::command]
pub fn delete_reviews(review_ids: Vec<i64>, db: State<Db>) -> AppResult<usize> {
    let conn = db.0.lock().unwrap();
    Ok(review_ids
        .into_iter()
        .filter(|&id| delete_review_impl(&conn, id).is_ok())
        .count())
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
pub fn add_comment_impl(
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
    tag: Option<&str>,
) -> AppResult<Comment> {
    ensure_draft(conn, review_id)?;
    validate_tag(tag)?;
    let ts = now();
    match parent_id {
        None => {
            // LEFT line numbers live in the base/merge-base coordinate system, so
            // anchor them to the review's pinned base_sha (read from the same
            // DB snapshot ensure_draft just saw). RIGHT comments store NULL.
            let anchored_base_sha: Option<String> = if side == "LEFT" {
                conn.query_row(
                    "SELECT base_sha FROM review WHERE id = ?1",
                    params![review_id],
                    |r| r.get(0),
                )?
            } else {
                None
            };
            conn.execute(
                "INSERT INTO comment
                    (review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, anchored_base_sha, tag, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![review_id, file_path, side, line, start_line, diff_hunk, body, anchored_head_sha, anchored_base_sha, tag, ts],
            )?;
        }
        Some(pid) => {
            let p = parent_for_reply(conn, review_id, pid)?;
            conn.execute(
                "INSERT INTO comment
                    (review_id, file_path, subject_type, origin, side, line, start_line,
                     diff_hunk, body, parent_id, anchored_head_sha, anchored_base_sha, tag, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
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
                    p.anchored_base_sha,
                    tag,
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
        None,
    )
}

/// Add a comment attached to a whole file rather than a specific line. Stored
/// with `subject_type = 'file'`; side/line keep their column defaults and are
/// ignored for these.
pub fn add_file_comment_impl(
    conn: &Connection,
    review_id: i64,
    file_path: String,
    body: String,
    tag: Option<&str>,
) -> AppResult<Comment> {
    ensure_draft(conn, review_id)?;
    validate_tag(tag)?;
    let ts = now();
    conn.execute(
        "INSERT INTO comment (review_id, file_path, subject_type, body, line, tag, created_at, updated_at)
         VALUES (?1, ?2, 'file', ?3, 0, ?4, ?5, ?5)",
        params![review_id, file_path, body, tag, ts],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    get_comment(conn, conn.last_insert_rowid())
}

#[tauri::command]
pub fn add_file_comment(
    review_id: i64,
    file_path: String,
    body: String,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    add_file_comment_impl(&conn, review_id, file_path, body, None)
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

pub fn update_comment_impl(conn: &Connection, comment_id: i64, body: String) -> AppResult<()> {
    let review_id: i64 = conn.query_row(
        "SELECT review_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| r.get(0),
    )?;
    ensure_draft(conn, review_id)?;
    // Editing an AI-written comment makes it yours: drop the tag so it
    // publishes like any other comment. A `local` tag is a deliberate choice
    // and survives edits.
    conn.execute(
        &format!(
            "UPDATE comment SET body = ?1, updated_at = ?2,
                tag = CASE WHEN {AI_TAG_SQL} THEN NULL ELSE tag END
             WHERE id = ?3"
        ),
        params![body, now(), comment_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn update_comment(comment_id: i64, body: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    update_comment_impl(&conn, comment_id, body)
}

pub fn set_comment_tag_impl(conn: &Connection, comment_id: i64, tag: Option<&str>) -> AppResult<()> {
    let review_id: i64 = conn.query_row(
        "SELECT review_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| r.get(0),
    )?;
    ensure_draft(conn, review_id)?;
    validate_tag(tag)?;
    conn.execute(
        "UPDATE comment SET tag = ?1, updated_at = ?2 WHERE id = ?3",
        params![tag, now(), comment_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn set_comment_tag(comment_id: i64, tag: Option<String>, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_comment_tag_impl(&conn, comment_id, tag.as_deref())
}

pub fn delete_comment_impl(conn: &Connection, comment_id: i64) -> AppResult<()> {
    let review_id: i64 = conn.query_row(
        "SELECT review_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| r.get(0),
    )?;
    ensure_draft(conn, review_id)?;
    conn.execute("DELETE FROM comment WHERE id = ?1", params![comment_id])?;
    Ok(())
}

#[tauri::command]
pub fn delete_comment(comment_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    delete_comment_impl(&conn, comment_id)
}

/// Mark a root comment's thread resolved/unresolved. Pure helper; caller holds
/// the lock. Idempotent: resolving an already-resolved root refreshes the
/// timestamp, unresolving an unresolved one is a no-op UPDATE.
pub fn set_resolved(conn: &Connection, comment_id: i64, resolved: bool) -> AppResult<()> {
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

/// Insert a draft reply targeting an existing GitHub PR review thread. Unlike
/// regular `add_comment`, this bypasses `ensure_draft` so it works even when the
/// review is already published — draft replies are about GitHub interactions, not
/// the review's own content. The caller supplies the thread's anchor (file/side/line)
/// so the reply can be positioned correctly in the diff UI.
#[allow(clippy::too_many_arguments)]
pub fn add_draft_reply_impl(
    conn: &Connection,
    review_id: i64,
    github_thread_root_id: i64,
    body: String,
    file_path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
) -> AppResult<Comment> {
    let review_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM review WHERE id = ?1",
            params![review_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(AppError::from)?;
    if !review_exists {
        return Err(AppError::Other(format!("review {review_id} not found")));
    }
    let ts = now();
    conn.execute(
        "INSERT INTO comment
            (review_id, file_path, subject_type, origin, side, line, start_line,
             body, github_thread_root_id, created_at, updated_at)
         VALUES (?1, ?2, 'line', 'diff', ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![review_id, file_path, side, line, start_line, body, github_thread_root_id, ts],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    get_comment(conn, conn.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn add_draft_reply(
    review_id: i64,
    github_thread_root_id: i64,
    body: String,
    file_path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
    db: State<Db>,
) -> AppResult<Comment> {
    let conn = db.0.lock().unwrap();
    add_draft_reply_impl(&conn, review_id, github_thread_root_id, body, file_path, side, line, start_line)
}

/// Result of publishing all draft replies for a review.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDraftRepliesResult {
    pub published: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

/// Resolve the `(owner, name, pr_number)` a review's draft replies post to.
fn pr_coords(conn: &Connection, review_id: i64) -> AppResult<(String, String, i64)> {
    let detail = load_detail(conn, review_id)?;
    if detail.target.kind != "github_pr" {
        return Err(AppError::Other(
            "draft replies can only be published for GitHub PR reviews".into(),
        ));
    }
    let number = detail
        .target
        .github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name) = pr_remote(conn, detail.target.repo_id)?;
    Ok((owner, name, number))
}

/// Publish all pending draft replies (github_thread_root_id IS NOT NULL) for a
/// review to GitHub, then delete each local row on success. Continues on
/// per-reply failures and reports them in the result rather than aborting.
pub fn publish_draft_replies_impl(db: &Db, review_id: i64) -> AppResult<PublishDraftRepliesResult> {
    let (owner, name, number) = {
        let conn = db.0.lock().unwrap();
        pr_coords(&conn, review_id)?
    };

    // Collect pending draft replies outside the lock.
    let draft_replies: Vec<(i64, i64, String)> = {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, github_thread_root_id, body FROM comment
             WHERE review_id = ?1 AND github_thread_root_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![review_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };

    let mut published = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (comment_id, thread_root_id, body) in draft_replies {
        match provider_for().reply_to_thread(&owner, &name, number, thread_root_id, &body) {
            Ok(_gh_comment_id) => {
                let conn = db.0.lock().unwrap();
                let _ = conn.execute("DELETE FROM comment WHERE id = ?1", params![comment_id]);
                published += 1;
            }
            Err(e) => {
                failed += 1;
                errors.push(format!("comment #{comment_id}: {e}"));
            }
        }
    }

    Ok(PublishDraftRepliesResult { published, failed, errors })
}

#[tauri::command]
pub async fn publish_draft_replies(
    review_id: i64,
    db: State<'_, Db>,
) -> AppResult<PublishDraftRepliesResult> {
    publish_draft_replies_impl(&db, review_id)
}

/// Publish a single draft reply to GitHub, then delete its local row on success.
pub fn publish_draft_reply_impl(db: &Db, comment_id: i64) -> AppResult<()> {
    let (owner, name, number, thread_root_id, body) = {
        let conn = db.0.lock().unwrap();
        let (review_id, thread_root_id, body) = conn
            .query_row(
                "SELECT review_id, github_thread_root_id, body FROM comment
                 WHERE id = ?1 AND github_thread_root_id IS NOT NULL",
                params![comment_id],
                |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
                },
            )
            .map_err(|_| AppError::Other(format!("draft reply #{comment_id} not found")))?;
        let (owner, name, number) = pr_coords(&conn, review_id)?;
        (owner, name, number, thread_root_id, body)
    };
    provider_for().reply_to_thread(&owner, &name, number, thread_root_id, &body)?;
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM comment WHERE id = ?1", params![comment_id])?;
    Ok(())
}

#[tauri::command]
pub async fn publish_draft_reply(comment_id: i64, db: State<'_, Db>) -> AppResult<()> {
    publish_draft_reply_impl(&db, comment_id)
}

/// Update the body of a draft reply. Bypasses `ensure_draft` so it works even
/// on published reviews (draft replies live independently of review lifecycle).
pub fn update_draft_reply_impl(conn: &Connection, comment_id: i64, body: String) -> AppResult<()> {
    let affected = conn.execute(
        "UPDATE comment SET body = ?1, updated_at = ?2 WHERE id = ?3 AND github_thread_root_id IS NOT NULL",
        params![body, now(), comment_id],
    )?;
    if affected == 0 {
        return Err(AppError::Other(format!(
            "draft reply #{comment_id} not found"
        )));
    }
    Ok(())
}

#[tauri::command]
pub fn update_draft_reply(comment_id: i64, body: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    update_draft_reply_impl(&conn, comment_id, body)
}

/// Delete a draft reply. Bypasses `ensure_draft` — same rationale as update.
pub fn delete_draft_reply_impl(conn: &Connection, comment_id: i64) -> AppResult<()> {
    let affected = conn.execute(
        "DELETE FROM comment WHERE id = ?1 AND github_thread_root_id IS NOT NULL",
        params![comment_id],
    )?;
    if affected == 0 {
        return Err(AppError::Other(format!(
            "draft reply #{comment_id} not found"
        )));
    }
    Ok(())
}

#[tauri::command]
pub fn delete_draft_reply(comment_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    delete_draft_reply_impl(&conn, comment_id)
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
            "INSERT INTO repository (local_path, remote_owner, remote_name, default_branch, added_at)
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

    /// Like `seed_comment_anchored` but pins `anchored_base_sha` instead, so the
    /// LEFT/base re-anchor pass sees a prior base revision to remap from. Stored
    /// with `side = "LEFT"` and a NULL `anchored_head_sha` (the head pass ignores
    /// it).
    fn seed_comment_anchored_base(
        conn: &Connection,
        review_id: i64,
        file_path: &str,
        line: i64,
        start_line: Option<i64>,
        body: &str,
        anchored_base_sha: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO comment
                (review_id, file_path, side, line, start_line, diff_hunk, body, anchored_base_sha, created_at, updated_at)
             VALUES (?1, ?2, 'LEFT', ?3, ?4, '@@ hunk @@', ?5, ?6, 'now', 'now')",
            params![review_id, file_path, line, start_line, body, anchored_base_sha],
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
                 diff_hunk, body, parent_id, anchored_head_sha, anchored_base_sha, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'now', 'now')",
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
                p.anchored_base_sha,
            ],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn pr_info() -> PrInfo {
        PrInfo {
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

        let t1 = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        assert_eq!(t1.kind, "local");
        assert_eq!(t1.title, "main...feature");
        assert!(t1.three_dot);

        // Same (repo, base, head) reuses the row and can flip three_dot.
        let t2 = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", false).unwrap().0;
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

        let mut updated = pr_info();
        updated.title = "Renamed PR".into();
        updated.head_sha = "cafe".into();
        let t2 = get_or_create_pr_target(&conn, repo, 7, &updated, None).unwrap();
        assert_eq!(t1.id, t2.id);
        assert_eq!(t2.title, "Renamed PR");
    }

    #[test]
    fn pr_target_reads_title_and_refs_from_the_mirror() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));
        let t = get_or_create_pr_target(&conn, repo, 7, &pr_info(), Some("PR_7")).unwrap();
        assert_eq!(t.title, "My PR", "label until the mirror has the PR");
        conn.execute(
            "INSERT INTO gh_pr (node_id, repo_id, number, title, url, base_ref, head_ref, fetched_at)
             VALUES ('PR_7', ?1, 7, 'Mirrored', 'u', 'develop', 'feat-2', 'now')",
            params![repo],
        )
        .unwrap();
        let t = get_target(&conn, t.id).unwrap();
        assert_eq!((t.title.as_str(), t.base_ref.as_str(), t.head_ref.as_str()), ("Mirrored", "develop", "feat-2"));
    }

    #[test]
    fn reviewing_registers_an_auto_added_repo() {
        let conn = open_memory();
        let id = crate::mirror::store::ensure_repo(&conn, "acme", "widget", "now").unwrap();
        let repo = get_or_create_remote_repository(&conn, "Acme", "widget", &[]).unwrap();
        assert_eq!(repo.id, id);
        assert!(repo.registered_at.is_some());
    }

    fn pin(base: Option<&str>, head: Option<&str>) -> LiveShas {
        LiveShas { base_sha: base.map(Into::into), head_sha: head.map(Into::into) }
    }

    #[test]
    fn reopening_a_pr_target_never_moves_a_drafts_pin() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));
        let target = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        let first = review_for_target(&conn, target.id, &pin(Some("mb1"), Some("deadbeef")), false).unwrap();
        assert_eq!(first.head_sha.as_deref(), Some("deadbeef"));
        assert_eq!(first.base_sha.as_deref(), Some("mb1"));

        // The PR moved; reopening reuses the draft and leaves its pin alone.
        let mut moved = pr_info();
        moved.head_sha = "cafe".into();
        let again = get_or_create_pr_target(&conn, repo, 7, &moved, None).unwrap();
        let reused = review_for_target(&conn, again.id, &pin(Some("mb2"), Some("cafe")), false).unwrap();
        assert_eq!(reused.id, first.id);
        assert_eq!(reused.head_sha.as_deref(), Some("deadbeef"));
        assert_eq!(reused.base_sha.as_deref(), Some("mb1"));

        // Once it's published, the next review on the same target pins at the live head.
        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![first.id]).unwrap();
        let fresh = review_for_target(&conn, again.id, &pin(Some("mb2"), Some("cafe")), false).unwrap();
        assert_ne!(fresh.id, first.id);
        assert_eq!(fresh.head_sha.as_deref(), Some("cafe"));
        assert_eq!(get_review_row(&conn, first.id).unwrap().head_sha.as_deref(), Some("deadbeef"));
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

        // Remote-only row with remote columns set returns owner/name.
        conn.execute(
            "INSERT INTO repository (remote_owner, remote_name, default_branch, added_at)
             VALUES ('acme', 'widget', 'main', 'now')",
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
            "INSERT INTO repository (local_path, default_branch, added_at)
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
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;

        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        assert_eq!(review.status, "draft");
        assert_eq!(review.body, "");
        assert!(review.event.is_none());
        assert!(!review.sidebar_collapsed, "the side pane starts open");
    }

    #[test]
    fn review_for_target_creates_only_when_every_review_is_published() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let open = |conn: &Connection| review_for_target(conn, target.id, &LiveShas::default(), false).unwrap().id;

        let first = open(&conn);
        assert_eq!(open(&conn), first, "the draft is reused");

        conn.execute("UPDATE review SET status = 'published_pending' WHERE id = ?1", params![first])
            .unwrap();
        assert_eq!(open(&conn), first, "a review pending on GitHub is reused too");

        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![first]).unwrap();
        let second = open(&conn);
        assert_ne!(second, first, "all published: a new review");
        assert_eq!(open(&conn), second);
    }

    #[test]
    fn review_for_target_can_reopen_a_published_review() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let pin = LiveShas::default();
        let published = review_for_target(&conn, target.id, &pin, false).unwrap().id;
        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![published]).unwrap();

        assert_eq!(review_for_target(&conn, target.id, &pin, true).unwrap().id, published);
        let draft = new_review_for_target(&conn, target.id, &pin).unwrap().id;
        assert_eq!(
            review_for_target(&conn, target.id, &pin, true).unwrap().id,
            draft,
            "an unpublished review wins over a published one"
        );
    }

    #[test]
    fn list_target_reviews_returns_all_newest_first() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;

        let first = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET status = 'published', updated_at = '2020-01-01T00:00:00Z' WHERE id = ?1",
            params![first.id],
        )
        .unwrap();
        let second = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET updated_at = '2024-01-01T00:00:00Z' WHERE id = ?1",
            params![second.id],
        )
        .unwrap();

        let summaries = list_target_reviews_impl(&conn, target.id).unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].review.id, second.id);
        assert_eq!(summaries[1].review.id, first.id);

        let other_target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "other", true).unwrap().0;
        assert!(list_target_reviews_impl(&conn, other_target.id).unwrap().is_empty());
    }

    #[test]
    fn new_review_on_target_allows_a_second_open_draft() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;

        let first = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        let second = new_review_on_target_impl(&conn, target.id, &LiveShas::default()).unwrap();
        assert_ne!(second.id, first.id);
        assert_eq!(second.status, "draft");
        assert_eq!(list_target_reviews_impl(&conn, target.id).unwrap().len(), 2);
    }

    #[test]
    fn copy_comments_filters_types_skips_published_and_duplicates() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let src = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        let dest = new_review_on_target_impl(&conn, target.id, &LiveShas::default()).unwrap();

        let mine = seed_comment(&conn, src.id, "a.rs", "RIGHT", 1, None, "mine");
        seed_reply(&conn, src.id, mine, "reply");
        let ai = seed_comment(&conn, src.id, "a.rs", "RIGHT", 2, None, "ai");
        set_comment_tag_impl(&conn, ai, Some("ai:review")).unwrap();
        let local = seed_comment(&conn, src.id, "a.rs", "RIGHT", 3, None, "local");
        set_comment_tag_impl(&conn, local, Some("local")).unwrap();
        let published = seed_comment(&conn, src.id, "a.rs", "RIGHT", 4, None, "published");
        conn.execute("UPDATE comment SET github_comment_id = 9 WHERE id = ?1", params![published]).unwrap();

        let types = vec!["mine".to_string(), "ai:review".to_string()];
        assert_eq!(copy_comments(&conn, dest.id, src.id, &types).unwrap(), (2, 0));
        let copied = load_detail(&conn, dest.id).unwrap().comments;
        assert_eq!(copied.len(), 3, "two roots plus one reply");
        let root = copied.iter().find(|c| c.body == "mine").unwrap();
        let reply = copied.iter().find(|c| c.body == "reply").unwrap();
        assert_eq!(reply.parent_id, Some(root.id));
        assert!(copied.iter().all(|c| c.body != "local" && c.body != "published"));

        assert_eq!(copy_comments(&conn, dest.id, src.id, &types).unwrap(), (0, 2));
        assert!(copy_comments(&conn, dest.id, dest.id, &types).is_err());
        assert!(copy_comments(&conn, dest.id, src.id, &["bogus".to_string()]).is_err());
    }

    #[test]
    fn copy_comments_rejects_other_target_and_non_draft_destination() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let t1 = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let t2 = get_or_create_local_target(&conn, repo, "/nope", "main", "other", true).unwrap().0;
        let a = new_review_for_target(&conn, t1.id, &LiveShas::default()).unwrap();
        let b = new_review_for_target(&conn, t2.id, &LiveShas::default()).unwrap();
        let types = vec!["mine".to_string()];
        assert!(copy_comments(&conn, a.id, b.id, &types).is_err());

        let c = new_review_for_target(&conn, t1.id, &LiveShas::default()).unwrap();
        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![c.id]).unwrap();
        assert!(copy_comments(&conn, c.id, a.id, &types).is_err());
    }

    #[test]
    fn ai_subtags_are_valid_and_cleared_on_edit_but_local_survives() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();

        for tag in ["ai", "ai:explain", "ai:review", "ai:codex"] {
            let id = seed_comment(&conn, review.id, "a.rs", "RIGHT", 1, None, "x");
            set_comment_tag_impl(&conn, id, Some(tag)).unwrap();
            assert_eq!(get_comment(&conn, id).unwrap().tag.as_deref(), Some(tag));
            update_comment_impl(&conn, id, "edited".into()).unwrap();
            assert_eq!(get_comment(&conn, id).unwrap().tag, None, "{tag} cleared on edit");
        }

        let local = seed_comment(&conn, review.id, "a.rs", "RIGHT", 2, None, "y");
        set_comment_tag_impl(&conn, local, Some("local")).unwrap();
        update_comment_impl(&conn, local, "edited".into()).unwrap();
        assert_eq!(get_comment(&conn, local).unwrap().tag.as_deref(), Some("local"));

        assert!(set_comment_tag_impl(&conn, local, Some("ai:bogus")).is_err());
    }

    #[test]
    fn ensure_draft_blocks_published_reviews() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();

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
    fn ensure_draft_blocks_pending_reviews() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();

        conn.execute(
            "UPDATE review SET status = 'published_pending' WHERE id = ?1",
            params![review.id],
        )
        .unwrap();
        let err = ensure_draft(&conn, review.id).unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("pending"), "message mentions pending: {m}"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    // ---- pending (draft) GitHub reviews (spec 19) ----

    /// A draft github_pr review with a resolved head, ready for the publish guards.
    /// Returns (Db wrapping the connection, review_id).
    fn pending_test_db() -> (Db, i64) {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("name"));
        let target = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        (Db(Mutex::new(conn)), review.id)
    }

    fn set_status(db: &Db, review_id: i64, status: &str) {
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE review SET status = ?1 WHERE id = ?2",
            params![status, review_id],
        )
        .unwrap();
    }

    #[test]
    fn gh_event_mapping() {
        assert_eq!(gh_event(Some("approve")), "APPROVE");
        assert_eq!(gh_event(Some("request_changes")), "REQUEST_CHANGES");
        assert_eq!(gh_event(Some("comment")), "COMMENT");
        assert_eq!(gh_event(None), "COMMENT");
    }

    #[test]
    fn pending_payload_omits_event() {
        let detail = detail_with(
            Some("request_changes"),
            "Looks good",
            vec![payload_comment(5, None, "RIGHT")],
        );
        let publish = build_publish_payload(&detail);
        let pending = build_pending_payload(&detail);
        assert!(pending.get("event").is_none(), "pending payload has no event");
        assert_eq!(pending["comments"], publish["comments"]);
        assert_eq!(pending["body"], publish["body"]);
        assert_eq!(pending["commit_id"], publish["commit_id"]);
    }

    #[test]
    fn ensure_submittable_rejects_empty_comment_review() {
        let detail = detail_with(None, "", vec![]);
        let err = ensure_submittable(&detail, "COMMENT").unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("no comments"), "message: {m}"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn ensure_submittable_accepts_comment_review_with_body_or_comments() {
        let with_body = detail_with(None, "Looks good", vec![]);
        ensure_submittable(&with_body, "COMMENT").unwrap();

        let with_comment = detail_with(None, "", vec![payload_comment(5, None, "RIGHT")]);
        ensure_submittable(&with_comment, "COMMENT").unwrap();
    }

    #[test]
    fn ensure_submittable_allows_empty_approve_or_request_changes() {
        let detail = detail_with(Some("approve"), "", vec![]);
        ensure_submittable(&detail, "APPROVE").unwrap();
        ensure_submittable(&detail, "REQUEST_CHANGES").unwrap();
    }

    #[test]
    fn publish_rejects_pending_status() {
        // The shared guard `prepare_publish` runs before any network in both
        // publish_review and publish_review_pending; a pending review errors here.
        let (db, review_id) = pending_test_db();
        set_status(&db, review_id, "published_pending");
        let err = prepare_publish(review_id, &db).unwrap_err();
        match err {
            AppError::Other(m) => assert!(m.contains("pending"), "message: {m}"),
            other => panic!("expected Other, got {other:?}"),
        }

        // A published review still errors with the published message.
        set_status(&db, review_id, "published");
        let err = prepare_publish(review_id, &db).unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    fn pending_detail(status: &str, github_review_id: Option<i64>) -> ReviewDetail {
        let mut d = detail_with(Some("approve"), "", vec![]);
        d.review.status = status.into();
        d.review.github_review_id = github_review_id;
        d
    }

    #[test]
    fn submit_discard_reject_non_pending() {
        assert!(ensure_pending(&pending_detail("draft", None)).is_err());
        assert!(ensure_pending(&pending_detail("published", Some(9))).is_err());
        assert_eq!(
            ensure_pending(&pending_detail("published_pending", Some(42))).unwrap(),
            42
        );
    }

    #[test]
    fn ensure_pending_requires_github_id() {
        let err = ensure_pending(&pending_detail("published_pending", None)).unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn mark_submitted_transitions() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("name"));
        let target = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET status = 'published_pending', github_review_id = 99 WHERE id = ?1",
            params![review.id],
        )
        .unwrap();

        mark_submitted(&conn, review.id).unwrap();

        let r = get_review_row(&conn, review.id).unwrap();
        assert_eq!(r.status, "published");
        assert!(r.published_at.is_some(), "published_at set on submit");
        assert_eq!(r.github_review_id, Some(99), "github_review_id kept");
    }

    #[test]
    fn mark_discarded_transitions() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("name"));
        let target = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET status = 'published_pending', github_review_id = 99 WHERE id = ?1",
            params![review.id],
        )
        .unwrap();

        mark_discarded(&conn, review.id).unwrap();

        let r = get_review_row(&conn, review.id).unwrap();
        assert_eq!(r.status, "draft");
        assert_eq!(r.github_review_id, None, "github_review_id cleared");
        assert!(r.published_at.is_none(), "published_at stays NULL");
    }

    #[test]
    fn map_pending_conflict_remaps_only_pending_422() {
        let conflict =
            AppError::Gh("422: A pending review already exists for this pull request".into());
        match map_pending_conflict(conflict) {
            AppError::Other(m) => assert!(m.contains("one pending review"), "friendly text: {m}"),
            other => panic!("expected Other, got {other:?}"),
        }

        // Any other gh error passes through unchanged.
        let other = AppError::Gh("HTTP 422: other".into());
        match map_pending_conflict(other) {
            AppError::Gh(m) => assert_eq!(m, "HTTP 422: other"),
            other => panic!("expected Gh, got {other:?}"),
        }
    }

    #[test]
    fn delete_review_blocked_while_pending() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("name"));
        let target = get_or_create_pr_target(&conn, repo, 7, &pr_info(), None).unwrap();
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET status = 'published_pending' WHERE id = ?1",
            params![review.id],
        )
        .unwrap();

        // delete_review's guard runs before the DELETE. Exercise the same logic a
        // tauri State cannot reach here: status == pending must short-circuit with
        // an error and never run the DELETE, so the row survives.
        let result: AppResult<()> = (|| {
            if review_status(&conn, review.id)? == "published_pending" {
                return Err(AppError::Other("pending".into()));
            }
            conn.execute("DELETE FROM review WHERE id = ?1", params![review.id])?;
            Ok(())
        })();
        assert!(result.is_err(), "deleting a pending review is rejected");

        let surviving: i64 = conn
            .query_row("SELECT COUNT(*) FROM review WHERE id = ?1", params![review.id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(surviving, 1, "pending review row is not deleted");
    }

    // ---- threaded replies (spec 11) ----

    /// A draft local review with one root diff comment. Returns (conn, review_id, root_id).
    fn review_with_root_comment() -> (Connection, i64, i64) {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
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
            None,
        )
        .unwrap();
        (conn, review.id, root.id)
    }

    #[test]
    fn add_comment_left_anchors_to_review_base_sha() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET base_sha = 'basesha' WHERE id = ?1",
            params![review.id],
        )
        .unwrap();

        let left = add_comment_impl(
            &conn,
            review.id,
            "a.rs".into(),
            "LEFT".into(),
            7,
            None,
            None,
            "on the old side".into(),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(left.anchored_base_sha.as_deref(), Some("basesha"));

        let right = add_comment_impl(
            &conn,
            review.id,
            "a.rs".into(),
            "RIGHT".into(),
            9,
            None,
            None,
            "on the new side".into(),
            Some("headsha".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(right.anchored_base_sha, None, "RIGHT comments store no base pin");
    }

    #[test]
    fn reply_inherits_anchored_base_sha() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET base_sha = 'basesha' WHERE id = ?1",
            params![review.id],
        )
        .unwrap();

        let root = add_comment_impl(
            &conn,
            review.id,
            "a.rs".into(),
            "LEFT".into(),
            7,
            None,
            None,
            "left root".into(),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(root.anchored_base_sha.as_deref(), Some("basesha"));

        let reply = add_comment_impl(
            &conn,
            review.id,
            "".into(),
            "RIGHT".into(),
            0,
            None,
            None,
            "reply".into(),
            None,
            Some(root.id),
            None,
        )
        .unwrap();
        assert_eq!(
            reply.anchored_base_sha.as_deref(),
            Some("basesha"),
            "reply copies the root's base pin (spec 11 inheritance)"
        );
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
            None,
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
            None,
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
        let other = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
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
            None,
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
            None,
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
            None,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn reply_to_file_comment_inherits_subject_type() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
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
            None,
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
    fn deleting_a_review_removes_its_image_diff_cache() {
        let dir = std::env::temp_dir().join(format!("cr-delete-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("codereview.db");
        let conn = crate::db::open(&db_path).unwrap();
        let repo = seed_repo(&conn, None, None);
        let target = get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        let other = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        let cache = crate::image_diff::store::cache_dir(&db_path, review.id);
        let other_cache = crate::image_diff::store::cache_dir(&db_path, other.id);
        for c in [&cache, &other_cache] {
            std::fs::create_dir_all(c.join("f0")).unwrap();
            std::fs::write(c.join("f0/diff.png"), b"x").unwrap();
        }

        delete_review_impl(&conn, review.id).unwrap();

        assert!(!cache.exists(), "the deleted review's cache is gone");
        assert!(other_cache.exists(), "another review's cache is untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_detail_returns_comments_ordered_by_file_line() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();

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
        assert_eq!(detail.local_path.as_deref(), Some("/repo"));
    }

    #[test]
    fn repo_label_prefers_owner_name_then_local_path() {
        let conn = open_memory();
        let with_remote = seed_repo(&conn, Some("acme"), Some("widget"));
        assert_eq!(repo_label(&conn, with_remote).unwrap(), "acme/widget");

        conn.execute(
            "INSERT INTO repository (local_path, added_at) VALUES ('/only/path', 'now')",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        assert_eq!(repo_label(&conn, id).unwrap(), "/only/path");
    }

    #[test]
    fn repo_label_prefers_friendly_name_over_owner_name() {
        let conn = open_memory();
        let with_remote = seed_repo(&conn, Some("acme"), Some("widget"));
        conn.execute(
            "UPDATE repository SET friendly_name = 'Widget' WHERE id = ?1",
            params![with_remote],
        )
        .unwrap();
        assert_eq!(repo_label(&conn, with_remote).unwrap(), "Widget");
    }

    #[test]
    fn remote_repository_creates_remote_only_then_reuses() {
        let conn = open_memory();

        // No matching row yet: a remote-only row (local_path = NULL) is created.
        let r1 = get_or_create_remote_repository(&conn, "acme", "widget", &[]).unwrap();
        assert!(r1.local_path.is_none(), "should be remote-only");
        assert_eq!(r1.remote_owner.as_deref(), Some("acme"));
        let ctx = gh_ctx_for_repo(&conn, r1.id).unwrap();
        assert!(matches!(ctx, GhRepo::Remote { .. }));

        // Calling again reuses the same row (no duplicate).
        let r2 = get_or_create_remote_repository(&conn, "acme", "widget", &[]).unwrap();
        assert_eq!(r1.id, r2.id);

        // When add_repository_impl is called with a matching remote, it merges
        // the local path into the existing remote-only row (same id, local_path set).
        conn.execute(
            "UPDATE repository SET local_path = '/clones/widget' WHERE id = ?1",
            params![r1.id],
        )
        .unwrap();
        let r3 = get_or_create_remote_repository(&conn, "acme", "widget", &[]).unwrap();
        assert_eq!(r3.id, r1.id, "should reuse the same row");
        assert_eq!(r3.local_path.as_deref(), Some("/clones/widget"));
        assert!(matches!(gh_ctx_for_repo(&conn, r3.id).unwrap(), GhRepo::Local(_)));
    }

    #[test]
    fn deleting_review_cascades_to_comments() {
        let conn = open_memory();
        let repo = seed_repo(&conn, None, None);
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
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
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
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
                base_sha: None,
                head_sha: Some("headsha".into()),
                since_base_sha: None,
                since_head_sha: None,
                published_at: None,
                github_review_id: None,
                last_exported_at: None,
                created_at: "now".into(),
                updated_at: "now".into(),
                sidebar_collapsed: false,
                chat_collapsed: true,
                pr_details_collapsed: false,
                left_panel_width: 248,
                right_panel_width: 360,
            },
            target: Target {
                id: 1,
                repo_id: 1,
                kind: "github_pr".into(),
                github_pr_number: Some(1),
                gh_pr_id: None,
                title: "t".into(),
                base_ref: "main".into(),
                head_ref: "feature".into(),
                three_dot: true,
                created_at: "now".into(),
            },
            local_path: Some("/repo".into()),
            remote_owner: Some("owner".into()),
            remote_name: Some("name".into()),
            friendly_name: None,
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
            anchored_base_sha: None,
            github_comment_id: None,
            resolved_at: None,
            github_thread_root_id: None,
            tag: None,
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
    fn since_review_posts_right_comments_inline_and_folds_left_ones_into_the_body() {
        let mut d = detail_with(
            None,
            "",
            vec![payload_comment(5, None, "RIGHT"), payload_comment(7, None, "LEFT")],
        );
        d.review.since_base_sha = Some("oldbase".into());
        d.review.since_head_sha = Some("oldhead".into());
        let p = build_publish_payload(&d);
        assert_eq!(p["comments"].as_array().unwrap().len(), 1);
        assert_eq!(p["comments"][0]["side"], "RIGHT");
        let body = p["body"].as_str().unwrap();
        assert!(body.contains("could not be posted inline") && body.contains("L7"), "{body}");
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
        d.review.head_sha = None;
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
        d.review.head_sha = Some("freshsha".into());

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
        d.review.head_sha = Some("freshsha".into());

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

    #[test]
    fn publish_gate_is_side_aware() {
        // base_sha = "freshbase", head_sha = "freshhead".
        let mut stale_left = payload_comment(7, None, "LEFT");
        stale_left.id = 1;
        stale_left.anchored_base_sha = Some("oldbase".into());

        let mut fresh_left = payload_comment(9, None, "LEFT");
        fresh_left.id = 2;
        fresh_left.anchored_base_sha = Some("freshbase".into());

        // A LEFT comment whose head pin is stale but whose base pin is current:
        // it must still post inline (the behaviour change — head is irrelevant to
        // LEFT line numbers).
        let mut left_stale_head = payload_comment(11, None, "LEFT");
        left_stale_head.id = 3;
        left_stale_head.anchored_base_sha = Some("freshbase".into());
        left_stale_head.anchored_head_sha = Some("stalehead".into());

        let mut d = detail_with(None, "", vec![stale_left, fresh_left, left_stale_head]);
        d.review.base_sha = Some("freshbase".into());
        d.review.head_sha = Some("freshhead".into());

        let p = build_publish_payload(&d);
        let comments = p["comments"].as_array().unwrap();
        // The stale-LEFT comment is excluded; the two current-base LEFT comments post inline.
        assert_eq!(comments.len(), 2);
        let lines: Vec<i64> = comments.iter().map(|c| c["line"].as_i64().unwrap()).collect();
        assert!(lines.contains(&9));
        assert!(lines.contains(&11), "stale-head/current-base LEFT still posts inline");
        assert!(!lines.contains(&7), "stale-base LEFT must not post inline");
        for c in comments {
            assert_eq!(c["side"], "LEFT");
        }

        let body = p["body"].as_str().unwrap();
        assert!(body.contains("## Comments that could not be re-anchored"));
        assert!(body.contains("**src/lib.rs** L7: comment body"));
        assert!(!body.contains("L9"), "fresh LEFT comment is not in the lost section");
        assert!(!body.contains("L11"), "current-base LEFT comment is not in the lost section");
    }

    // ---- comment-id capture (Spec 17): inline selection, matcher, store ----

    #[test]
    fn inline_publish_comments_matches_payload_filter() {
        let detail = detail_with(
            None,
            "",
            vec![
                payload_comment(5, None, "RIGHT"),
                file_comment("src/lib.rs", "whole-file note"),
                file_view_comment("src/lib.rs", 12, None, "pane note"),
                anchored_payload_comment(7, None, Some("stalesha")),
            ],
        );
        // The single inline-eligible comment is the anchored line comment; file,
        // file-view, and not-anchored-to-head rows are excluded.
        let inline = inline_publish_comments(&detail);
        assert_eq!(inline.len(), 1);
        let (c, body) = &inline[0];
        assert_eq!(c.line, 5);
        assert_eq!(body, "comment body");
        assert_eq!(*body, c.body, "posted body equals c.body verbatim");
    }

    fn remote_comment(
        id: i64,
        line: Option<i64>,
        start_line: Option<i64>,
        side: Option<&str>,
        body: &str,
    ) -> ReviewComment {
        ReviewComment {
            id,
            path: "src/lib.rs".into(),
            side: side.map(Into::into),
            line,
            start_line,
            body: body.into(),
        }
    }

    fn local_key(id: i64, line: i64, start_line: Option<i64>, body: &str) -> (i64, AnchorKey, String) {
        let c = Comment {
            line,
            start_line,
            body: body.into(),
            ..payload_comment(line, start_line, "RIGHT")
        };
        (id, AnchorKey::local(&c), body.into())
    }

    #[test]
    fn match_singleton_anchor_pairs_despite_body_drift() {
        let local = vec![(1, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "local body".into())];
        let remote = vec![remote_comment(9001, Some(5), None, Some("RIGHT"), "drifted body")];
        assert_eq!(match_review_comments(&local, &remote), vec![(1, 9001)]);
    }

    #[test]
    fn match_body_tiebreak_on_shared_anchor() {
        // Two locals on the same anchor with bodies A/B; two remotes B/A → each
        // pairs with its body twin regardless of order.
        let local = vec![local_key(1, 5, None, "A"), local_key(2, 5, None, "B")];
        let remote = vec![
            remote_comment(9002, Some(5), None, Some("RIGHT"), "B"),
            remote_comment(9001, Some(5), None, Some("RIGHT"), "A"),
        ];
        let mut got = match_review_comments(&local, &remote);
        got.sort();
        assert_eq!(got, vec![(1, 9001), (2, 9002)]);
    }

    #[test]
    fn match_identical_duplicates_zip_in_order() {
        // Two locals, same anchor + same body; two identical remotes → paired in
        // order, distinct gh ids, each remote consumed once.
        let local = vec![
            (1, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "dup".into()),
            (2, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "dup".into()),
        ];
        let remote = vec![
            remote_comment(9001, Some(5), None, Some("RIGHT"), "dup"),
            remote_comment(9002, Some(5), None, Some("RIGHT"), "dup"),
        ];
        assert_eq!(match_review_comments(&local, &remote), vec![(1, 9001), (2, 9002)]);
    }

    #[test]
    fn match_ambiguous_bucket_left_unmatched() {
        // Two locals same anchor, two remotes, no body equality, not a singleton
        // bucket → nothing paired.
        let local = vec![
            (1, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "A".into()),
            (2, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "B".into()),
        ];
        let remote = vec![
            remote_comment(9001, Some(5), None, Some("RIGHT"), "X"),
            remote_comment(9002, Some(5), None, Some("RIGHT"), "Y"),
        ];
        assert!(match_review_comments(&local, &remote).is_empty());
    }

    #[test]
    fn match_tolerates_count_mismatch() {
        // Two locals on distinct anchors, one remote matching the first → exactly
        // one pair (folded-comment scenario: the second local has no remote).
        let local = vec![
            (1, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "first".into()),
            (2, AnchorKey::local(&payload_comment(9, None, "RIGHT")), "second".into()),
        ];
        let remote = vec![remote_comment(9001, Some(5), None, Some("RIGHT"), "first")];
        assert_eq!(match_review_comments(&local, &remote), vec![(1, 9001)]);
    }

    #[test]
    fn match_normalizes_single_line_start() {
        // Local with start_line == line normalizes to None and matches a remote
        // single-line comment (start_line: None).
        let local = vec![(1, AnchorKey::local(&payload_comment(5, Some(5), "RIGHT")), "comment body".into())];
        let remote = vec![remote_comment(9001, Some(5), None, Some("RIGHT"), "comment body")];
        assert_eq!(match_review_comments(&local, &remote), vec![(1, 9001)]);
    }

    #[test]
    fn match_multiline_range_key() {
        // Local range (start=3, line=5) matches the range remote, never the
        // single-line remote at line 5.
        let local = vec![(1, AnchorKey::local(&payload_comment(5, Some(3), "RIGHT")), "ranged".into())];
        let remote = vec![
            remote_comment(9001, Some(5), None, Some("RIGHT"), "single"),
            remote_comment(9002, Some(5), Some(3), Some("RIGHT"), "ranged"),
        ];
        assert_eq!(match_review_comments(&local, &remote), vec![(1, 9002)]);
    }

    #[test]
    fn match_skips_remote_without_anchor() {
        let local = vec![(1, AnchorKey::local(&payload_comment(5, None, "RIGHT")), "comment body".into())];
        // Remote missing side and line has no AnchorKey → never paired.
        let remote = vec![remote_comment(9001, None, None, None, "comment body")];
        assert!(match_review_comments(&local, &remote).is_empty());
    }

    #[test]
    fn store_comment_id_matches_writes_only_matched_rows() {
        let conn = open_memory();
        let repo = seed_repo(&conn, Some("owner"), Some("repo"));
        let target =
            get_or_create_local_target(&conn, repo, "/nope", "main", "feature", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();

        let c1 = seed_comment(&conn, review.id, "a.rs", "RIGHT", 5, None, "one");
        let c2 = seed_comment(&conn, review.id, "a.rs", "RIGHT", 9, None, "two");

        let before: String = conn
            .query_row("SELECT updated_at FROM comment WHERE id = ?1", params![c2], |r| r.get(0))
            .unwrap();

        let n = store_comment_id_matches(&conn, &[(c1, 9001)]).unwrap();
        assert_eq!(n, 1);

        let id1: Option<i64> = conn
            .query_row("SELECT github_comment_id FROM comment WHERE id = ?1", params![c1], |r| r.get(0))
            .unwrap();
        let id2: Option<i64> = conn
            .query_row("SELECT github_comment_id FROM comment WHERE id = ?1", params![c2], |r| r.get(0))
            .unwrap();
        assert_eq!(id1, Some(9001));
        assert_eq!(id2, None, "the unmatched row stays NULL");

        let after: String = conn
            .query_row("SELECT updated_at FROM comment WHERE id = ?1", params![c2], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "store must not touch updated_at");
    }

    // ---- threaded replies (Spec 11): publish folds replies under their root ----

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
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES (?1, 'main', 'now')",
            params![path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn refresh_local_review_reports_new_commits_without_moving_the_pin() {
        let (dir, h1, h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        // The review was pinned at H1; the branch has since moved to H2.
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &pin(Some(&h1), Some(&h1))).unwrap();

        let db = Db(Mutex::new(conn));
        let fresh = refresh_review_impl(review.id, &db).unwrap();
        assert!(fresh.moved);
        assert_eq!(fresh.pinned_head_sha.as_deref(), Some(h1.as_str()));
        assert_eq!(fresh.live_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(fresh.live_base_sha.as_deref(), Some(h1.as_str()));
        assert_eq!(fresh.new_commits, Some(1));

        let stored = get_review_row(&db.0.lock().unwrap(), review.id).unwrap();
        assert_eq!(stored.head_sha.as_deref(), Some(h1.as_str()), "refresh never moves the pin");
        assert_eq!(stored.updated_at, review.updated_at);
    }

    #[test]
    fn update_review_pin_moves_a_draft_to_the_live_head() {
        let (dir, h1, h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &pin(Some(&h1), Some(&h1))).unwrap();
        let other = new_review_for_target(&conn, target.id, &pin(Some(&h1), Some(&h1))).unwrap();
        // beta is line 2 at H1; H2 inserts a line above it.
        let c = seed_comment_anchored(&conn, review.id, "file.txt", "RIGHT", 2, None, "on beta", Some(&h1));

        let db = Db(Mutex::new(conn));
        let result = update_review_pin_impl(review.id, &db).unwrap();
        assert!(result.moved);
        assert_eq!(result.reanchor.reanchored, 1);

        let conn = db.0.lock().unwrap();
        let stored = get_review_row(&conn, review.id).unwrap();
        assert_eq!(stored.head_sha.as_deref(), Some(h2.as_str()));
        let moved = get_comment(&conn, c).unwrap();
        assert_eq!(moved.line, 3);
        assert_eq!(moved.anchored_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(
            get_review_row(&conn, other.id).unwrap().head_sha.as_deref(),
            Some(h1.as_str()),
            "only the updated review moves"
        );
        drop(conn);

        let again = refresh_review_impl(review.id, &db).unwrap();
        assert!(!again.moved);
        assert_eq!(again.new_commits, Some(0));
    }

    #[test]
    fn update_review_pin_refuses_published_reviews() {
        let (dir, h1, _h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", true).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &pin(Some(&h1), Some(&h1))).unwrap();
        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![review.id])
            .unwrap();

        let db = Db(Mutex::new(conn));
        assert!(update_review_pin_impl(review.id, &db).is_err());
        let stored = get_review_row(&db.0.lock().unwrap(), review.id).unwrap();
        assert_eq!(stored.head_sha.as_deref(), Some(h1.as_str()));
    }

    /// A repo whose `main` and `feature` branches diverge from a common fork
    /// point: each adds a distinct line. Returns `(dir, fork_sha)` where
    /// `fork_sha` is the merge-base of the two tips (and the LEFT side a three-dot
    /// `main...feature` diff shows). The base tip (`main`) differs from it.
    fn fixture_diverged_branches() -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "shared\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "fork point"]);
        let fork = git::rev_parse(p, "HEAD").unwrap();

        git(p, &["checkout", "-q", "-b", "feature"]);
        fs::write(p.join("file.txt"), "shared\nfeature\n").unwrap();
        git(p, &["commit", "-q", "-am", "feature work"]);

        git(p, &["checkout", "-q", "main"]);
        fs::write(p.join("file.txt"), "shared\nmain\n").unwrap();
        git(p, &["commit", "-q", "-am", "main work"]);
        (dir, fork)
    }

    #[test]
    fn local_target_base_sha_is_merge_base_when_three_dot() {
        let (dir, fork) = fixture_diverged_branches();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let base_tip = git::rev_parse(dir.path(), "main").unwrap();
        assert_ne!(fork, base_tip, "fixture must actually diverge");

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);

        // Three-dot: the LEFT side is the merge-base (fork), not the base branch tip.
        let (three, live3) =
            get_or_create_local_target(&conn, repo, &repo_path, "main", "feature", true).unwrap();
        assert_eq!(live3.base_sha.as_deref(), Some(fork.as_str()));

        // Two-dot: the LEFT side is the literal base tip.
        let (_, live2) =
            get_or_create_local_target(&conn, repo, &repo_path, "main", "feature", false).unwrap();
        assert_eq!(live2.base_sha.as_deref(), Some(base_tip.as_str()));

        // resolve_live reproduces the same coordinate choice from three_dot.
        conn.execute("UPDATE target SET three_dot = 1 WHERE id = ?1", params![three.id]).unwrap();
        let db = Db(Mutex::new(conn));
        let target = get_target(&db.0.lock().unwrap(), three.id).unwrap();
        let live = resolve_live(&db, &target).unwrap();
        assert_eq!(live.base_sha.as_deref(), Some(fork.as_str()));
    }

    #[test]
    fn refresh_local_review_no_move_reports_false() {
        let (dir, _h1, h2) = fixture_repo_two_heads();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        // A new review pins at the live head, so refresh finds nothing new.
        let review =
            create_review_impl(&conn, repo, &repo_path, "main~1", "main", true, false).unwrap();
        assert_eq!(review.head_sha.as_deref(), Some(h2.as_str()));

        let db = Db(Mutex::new(conn));
        let fresh = refresh_review_impl(review.id, &db).unwrap();
        assert!(!fresh.moved);
        assert_eq!(fresh.pinned_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(fresh.live_head_sha.as_deref(), Some(h2.as_str()));
        assert_eq!(fresh.new_commits, Some(0));
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
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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
    fn reanchor_relocates_moved_line_by_content() {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "one\ntwo\nneedle_call(42)\nthree\nfour\nfive\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "h1"]);
        let h1 = git::rev_parse(p, "HEAD").unwrap();
        // The commented line moves to the end: the diff deletes it, content finds it.
        fs::write(p.join("file.txt"), "one\ntwo\nthree\nfour\nfive\nneedle_call(42)\n").unwrap();
        git(p, &["commit", "-q", "-am", "h2"]);
        let h2 = git::rev_parse(p, "HEAD").unwrap();
        let repo_path = p.to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute("UPDATE review SET head_sha = ?1 WHERE id = ?2", params![h2, review.id])
            .unwrap();
        let id = seed_comment_anchored(
            &conn, review.id, "file.txt", "RIGHT", 3, None, "on call", Some(&h1),
        );

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!((result.reanchored, result.relocated, result.lost), (1, 1, 0));
        let c = comment_row(&conn, id);
        assert_eq!(c.line, 6);
        assert_eq!(c.anchored_head_sha.as_deref(), Some(h2.as_str()));
    }

    #[test]
    fn reanchor_leaves_replaced_comment_lost_with_sha_untouched() {
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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
    fn head_pass_never_touches_left_side_comments() {
        // A LEFT comment pinned only on the head column is wrong-side for the head
        // pass and has a NULL base pin, so neither pass moves it; the base pass
        // counts the NULL pin as skipped_no_change.
        let (dir, h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let (target, live) =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap();
        let review = new_review_for_target(&conn, target.id, &live).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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
        assert_eq!(result.skipped_no_change, 1, "base pass skips the NULL base pin");

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 2);
        assert_eq!(c.anchored_head_sha.as_deref(), Some(h1.as_str()));
        assert_eq!(c.anchored_base_sha, None);
    }

    #[test]
    fn reanchor_skips_comment_already_on_current_head() {
        let (dir, _h1, h2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();

        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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
            get_or_create_local_target(&conn, repo, &repo_path, "main~1", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET head_sha = ?1 WHERE id = ?2",
            params![h2, review.id],
        )
        .unwrap();

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

    // ---- LEFT/base re-anchor pass (spec 16) ----

    /// Set up a two-dot local review where the base advanced B1 -> B2 (the
    /// insert+replace fixture). LEFT comments pinned to B1 should remap through the
    /// B1 -> B2 diff onto B2. Returns `(dir, conn, review_id, b1, b2)`.
    fn left_base_review() -> (TempDir, Connection, i64, String, String) {
        let (dir, b1, b2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        // Two-dot local target; pin base_sha to the *current* base B2 (head is
        // irrelevant to the LEFT pass) while comments still point at B1.
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main", "main", false).unwrap().0;
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET base_sha = ?1, head_sha = ?2 WHERE id = ?3",
            params![b2, b2, review.id],
        )
        .unwrap();
        (dir, conn, review.id, b1, b2)
    }

    #[test]
    fn left_reanchor_shifts_comment_and_advances_base_sha() {
        let (_dir, conn, review_id, b1, b2) = left_base_review();
        // beta (B1 line 2) shifts to line 3 in B2; head pin left NULL.
        let id = seed_comment_anchored_base(&conn, review_id, "file.txt", 2, None, "on beta", Some(&b1));

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1);
        assert_eq!(result.lost, 0);
        assert_eq!(result.skipped_no_change, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 3);
        assert_eq!(c.anchored_base_sha.as_deref(), Some(b2.as_str()));
        assert_eq!(c.anchored_head_sha, None, "head pin untouched");
    }

    #[test]
    fn left_reanchor_replaced_line_is_lost() {
        let (_dir, conn, review_id, b1, _b2) = left_base_review();
        // gamma (B1 line 3) is replaced in B2 -> Lost.
        let id = seed_comment_anchored_base(&conn, review_id, "file.txt", 3, None, "on gamma", Some(&b1));

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.lost, 1);
        assert_eq!(result.reanchored, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 3, "lost comment's line is left untouched");
        assert_eq!(c.anchored_base_sha.as_deref(), Some(b1.as_str()), "pin stays at B1");
    }

    #[test]
    fn left_reanchor_range_moves_both_endpoints() {
        let (_dir, conn, review_id, b1, b2) = left_base_review();
        // Range alpha..beta (B1 lines 1-2) -> 1-3 (beta shifts, alpha unmoved).
        let moved =
            seed_comment_anchored_base(&conn, review_id, "file.txt", 2, Some(1), "range", Some(&b1));
        // Range with one endpoint on gamma (replaced) -> whole comment Lost.
        let lost =
            seed_comment_anchored_base(&conn, review_id, "file.txt", 3, Some(1), "spans gamma", Some(&b1));

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1);
        assert_eq!(result.lost, 1);

        let m = comment_row(&conn, moved);
        assert_eq!(m.start_line, Some(1));
        assert_eq!(m.line, 3);
        assert_eq!(m.anchored_base_sha.as_deref(), Some(b2.as_str()));

        let l = comment_row(&conn, lost);
        assert_eq!(l.start_line, Some(1));
        assert_eq!(l.line, 3, "lost range untouched");
        assert_eq!(l.anchored_base_sha.as_deref(), Some(b1.as_str()));
    }

    #[test]
    fn left_pass_skips_null_pin_and_current_pin() {
        let (_dir, conn, review_id, _b1, b2) = left_base_review();
        // NULL base pin and a pin already at B2 must both be skipped, untouched.
        let null_pin =
            seed_comment_anchored_base(&conn, review_id, "file.txt", 2, None, "null pin", None);
        let current_pin =
            seed_comment_anchored_base(&conn, review_id, "file.txt", 2, None, "already current", Some(&b2));

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.skipped_no_change, 2);
        assert_eq!(result.reanchored, 0);
        assert_eq!(result.lost, 0);

        assert_eq!(comment_row(&conn, null_pin).line, 2);
        assert_eq!(comment_row(&conn, null_pin).anchored_base_sha, None);
        assert_eq!(comment_row(&conn, current_pin).line, 2);
        assert_eq!(comment_row(&conn, current_pin).anchored_base_sha.as_deref(), Some(b2.as_str()));
    }

    #[test]
    fn left_pass_noop_when_target_base_sha_null() {
        let (_dir, conn, review_id, b1, _b2) = left_base_review();
        // Wipe the current base SHA: the pass must not error, just skip.
        conn.execute(
            "UPDATE review SET base_sha = NULL WHERE id = ?1",
            params![review_id],
        )
        .unwrap();
        let id = seed_comment_anchored_base(&conn, review_id, "file.txt", 2, None, "on beta", Some(&b1));

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.skipped_no_change, 1);
        assert_eq!(result.reanchored, 0);
        assert_eq!(result.lost, 0);

        let c = comment_row(&conn, id);
        assert_eq!(c.line, 2);
        assert_eq!(c.anchored_base_sha.as_deref(), Some(b1.as_str()));
    }

    #[test]
    fn passes_do_not_cross_sides() {
        // One stale RIGHT comment (head moved) and one fresh LEFT comment in the
        // same review: only the RIGHT row moves; counts aggregate across passes.
        let (dir, b1, b2) = fixture_repo_insert_and_replace();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let conn = open_memory();
        let repo = seed_repo_at(&conn, &repo_path);
        let target =
            get_or_create_local_target(&conn, repo, &repo_path, "main", "main", false).unwrap().0;
        // head advanced B1 -> B2; base is already current (B2).
        let review = new_review_for_target(&conn, target.id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET base_sha = ?1, head_sha = ?2 WHERE id = ?3",
            params![b2, b2, review.id],
        )
        .unwrap();

        let right = seed_comment_anchored(
            &conn, review.id, "file.txt", "RIGHT", 2, None, "right beta", Some(&b1),
        );
        let left = seed_comment_anchored_base(&conn, review.id, "file.txt", 1, None, "left alpha", Some(&b2));

        let detail = load_detail(&conn, review.id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1, "only the RIGHT row moves");
        assert_eq!(result.lost, 0);
        assert_eq!(result.skipped_no_change, 1, "the current LEFT pin is skipped");

        let r = comment_row(&conn, right);
        assert_eq!(r.line, 3);
        assert_eq!(r.anchored_head_sha.as_deref(), Some(b2.as_str()));
        let l = comment_row(&conn, left);
        assert_eq!(l.line, 1, "current LEFT comment untouched");
    }

    #[test]
    fn left_reanchor_moves_replies_with_root() {
        let (_dir, conn, review_id, b1, b2) = left_base_review();
        let root_id =
            seed_comment_anchored_base(&conn, review_id, "file.txt", 2, None, "on beta", Some(&b1));
        let reply_id = seed_reply(&conn, review_id, root_id, "agreed");

        let detail = load_detail(&conn, review_id).unwrap();
        let result = reanchor_review_comments(&conn, &detail).unwrap();
        assert_eq!(result.reanchored, 1, "only the root counts");
        assert_eq!(result.lost, 0);

        let root = comment_row(&conn, root_id);
        let reply = comment_row(&conn, reply_id);
        assert_eq!(root.line, 3);
        assert_eq!(reply.line, 3);
        assert_eq!(root.anchored_base_sha.as_deref(), Some(b2.as_str()));
        assert_eq!(reply.anchored_base_sha.as_deref(), Some(b2.as_str()));
    }

    #[test]
    fn closed_pr_candidates_includes_only_eligible_github_reviews() {
        let conn = open_memory();
        let repo_remote = seed_repo(&conn, Some("o"), Some("n"));
        let repo_local = seed_repo_at(&conn, "/repo-local");

        conn.execute(
            "INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, created_at)
             VALUES (?1, 'github_pr', 1, 'pr target', 'main', 'feat', 'now')",
            params![repo_remote],
        )
        .unwrap();
        let pr_target_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, created_at)
             VALUES (?1, 'local', NULL, 'local target', 'main', 'feat', 'now')",
            params![repo_remote],
        )
        .unwrap();
        let local_target_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, created_at)
             VALUES (?1, 'github_pr', 2, 'no remote', 'main', 'feat', 'now')",
            params![repo_local],
        )
        .unwrap();
        let no_remote_target_id = conn.last_insert_rowid();

        // Eligible: draft review on a github_pr target with a known remote.
        let draft = new_review_for_target(&conn, pr_target_id, &LiveShas::default()).unwrap();
        // Excluded: pending-on-GitHub review (delete_review_impl refuses these).
        let pending = new_review_for_target(&conn, pr_target_id, &LiveShas::default()).unwrap();
        conn.execute(
            "UPDATE review SET status = 'published_pending' WHERE id = ?1",
            params![pending.id],
        )
        .unwrap();
        // Excluded: local virtual-PR target.
        new_review_for_target(&conn, local_target_id, &LiveShas::default()).unwrap();
        // Excluded: github_pr target whose repo has no resolved remote.
        new_review_for_target(&conn, no_remote_target_id, &LiveShas::default()).unwrap();

        let candidates = closed_pr_candidates_impl(&conn).unwrap();
        assert_eq!(candidates.len(), 1);
        let (review_id, owner, name, number) = &candidates[0];
        assert_eq!(*review_id, draft.id);
        assert_eq!(owner, "o");
        assert_eq!(name, "n");
        assert_eq!(*number, 1);
    }

    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git").current_dir(dir).args(args).status().unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// A deleted local branch can't touch a review's pin: the diff keeps working
    /// from the pinned SHAs, and Update review refuses rather than wiping it.
    #[test]
    fn deleted_local_branch_keeps_the_pin_and_the_diff() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        // Pin the initial branch name explicitly — CI runners' git doesn't all
        // default `init.defaultBranch` to "main" the way local dev setups do.
        git_in(path, &["init", "-q", "-b", "main"]);
        git_in(path, &["config", "user.email", "t@t.com"]);
        git_in(path, &["config", "user.name", "t"]);
        fs::write(path.join("a.txt"), "1").unwrap();
        git_in(path, &["add", "."]);
        git_in(path, &["commit", "-q", "-m", "base"]);
        git_in(path, &["checkout", "-q", "-b", "feature"]);
        fs::write(path.join("a.txt"), "2").unwrap();
        git_in(path, &["commit", "-q", "-am", "feature"]);
        git_in(path, &["checkout", "-q", "main"]);

        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at)
             VALUES (?1, 'main', 'now')",
            params![path.to_string_lossy()],
        )
        .unwrap();
        let repo_id = conn.last_insert_rowid();
        let review =
            create_review_impl(&conn, repo_id, &path.to_string_lossy(), "main", "feature", true, false).unwrap();
        let pinned_head = review.head_sha.clone().expect("sha resolves while the branch exists");
        let pinned_base = review.base_sha.clone().expect("merge-base resolves");

        git_in(path, &["branch", "-D", "feature"]);

        assert!(review_diff_impl(&conn, review.id).unwrap().contains("+2"));
        let db = Db(Mutex::new(conn));
        assert!(update_review_pin_impl(review.id, &db).is_err());

        let stored = get_review_row(&db.0.lock().unwrap(), review.id).unwrap();
        assert_eq!(stored.head_sha, Some(pinned_head), "head pin preserved");
        assert_eq!(stored.base_sha, Some(pinned_base), "base pin preserved");
    }

    fn seed_mirror_pr(conn: &Connection, repo_id: i64, number: i64, head_sha: &str) {
        conn.execute(
            "INSERT INTO gh_pr (node_id, repo_id, number, title, url, base_ref, head_ref, head_sha, fetched_at)
             VALUES ('PR_node', ?1, ?2, 'A title', 'https://example.test/pr', 'main', 'feat', ?3, 'now')",
            params![repo_id, number, head_sha],
        )
        .unwrap();
    }

    #[test]
    fn mirror_pr_info_reads_refs_and_head_from_the_mirror() {
        let conn = open_memory();
        let repo_id = seed_repo(&conn, Some("o"), Some("r"));
        assert!(mirror_pr_info(&conn, repo_id, 7).unwrap().is_none(), "no mirror row yet");
        seed_mirror_pr(&conn, repo_id, 7, "abc123");
        let (info, node_id) = mirror_pr_info(&conn, repo_id, 7).unwrap().unwrap();
        assert_eq!((info.base_ref.as_str(), info.head_ref.as_str(), info.head_sha.as_str()), ("main", "feat", "abc123"));
        assert_eq!(node_id, "PR_node");
    }

    #[test]
    fn mirror_pr_info_needs_refs_to_be_usable() {
        let conn = open_memory();
        let repo_id = seed_repo(&conn, Some("o"), Some("r"));
        seed_mirror_pr(&conn, repo_id, 7, "abc123");
        conn.execute("UPDATE gh_pr SET head_ref = NULL", []).unwrap();
        assert!(mirror_pr_info(&conn, repo_id, 7).unwrap().is_none(), "falls back to a live fetch");
    }

    #[test]
    fn reopening_a_pr_draft_needs_no_github_call() {
        let conn = open_memory();
        let repo_id = seed_repo(&conn, Some("o"), Some("r"));
        // A friendly name changes the repo's display label, not how its drafts are found.
        conn.execute("UPDATE repository SET friendly_name = 'Widget' WHERE id = ?1", params![repo_id]).unwrap();
        seed_mirror_pr(&conn, repo_id, 7, "abc123");
        let info = PrInfo { title: "A title".into(), base_ref: "main".into(), head_ref: "feat".into(), head_sha: "abc123".into() };
        let target = get_or_create_pr_target(&conn, repo_id, 7, &info, Some("PR_node")).unwrap();
        let live = LiveShas { base_sha: Some("base0".into()), head_sha: Some("abc123".into()) };
        let draft = new_review_for_target(&conn, target.id, &live).unwrap();
        let db = Db(Mutex::new(conn));
        // The test has no network: a GitHub lookup here would fail the call.
        let reopened = create_review_for_pr_impl(&db, "o", "r", 7, &[], false).unwrap();
        assert_eq!(reopened.id, draft.id);
        assert_eq!(reopened.head_sha.as_deref(), Some("abc123"), "pin untouched");
    }

    #[test]
    fn existing_pr_review_is_none_without_an_open_review() {
        let conn = open_memory();
        assert!(existing_pr_review(&conn, "o", "r", 7, false).unwrap().is_none(), "unknown repo");
        let repo_id = seed_repo(&conn, Some("o"), Some("r"));
        assert!(existing_pr_review(&conn, "o", "r", 7, false).unwrap().is_none(), "no target");
        let info = PrInfo { title: "t".into(), base_ref: "main".into(), head_ref: "feat".into(), head_sha: "abc".into() };
        let target = get_or_create_pr_target(&conn, repo_id, 7, &info, None).unwrap();
        assert!(existing_pr_review(&conn, "o", "r", 7, false).unwrap().is_none(), "no review");
        let live = LiveShas { base_sha: None, head_sha: Some("abc".into()) };
        let draft = new_review_for_target(&conn, target.id, &live).unwrap();
        assert_eq!(existing_pr_review(&conn, "o", "r", 7, false).unwrap().unwrap().id, draft.id);
        conn.execute("UPDATE review SET status = 'published' WHERE id = ?1", params![draft.id]).unwrap();
        assert!(existing_pr_review(&conn, "o", "r", 7, false).unwrap().is_none(), "only published");
        assert_eq!(
            existing_pr_review(&conn, "o", "r", 7, true).unwrap().unwrap().id,
            draft.id,
            "a published review is reopened when asked"
        );
    }
}
