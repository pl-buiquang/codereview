use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::error::AppResult;
use crate::provider::provider_for;
use crate::sync::{MirrorUpdated, Scope, MIRROR_UPDATED_EVENT};
use crate::{gh, mirror, tools};

#[tauri::command]
pub fn gh_auth_status() -> bool {
    provider_for().auth_status()
}

#[tauri::command]
pub fn list_prs(repo_id: i64, db: State<Db>) -> AppResult<Vec<gh::PrSummary>> {
    let conn = db.0.lock().unwrap();
    let ctx = crate::commands::review::gh_ctx_for_repo(&conn, repo_id)?;
    provider_for().list_prs(&ctx)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The Review's live fetches land in the mirror too, so Inbox and Board follow.
pub(crate) fn notify_mirror(app: &AppHandle, id: &str) {
    let update = MirrorUpdated { scopes: vec![Scope::Inbox, Scope::Board], ids: vec![id.to_string()] };
    log::debug!(target: "mirror", "emit {MIRROR_UPDATED_EVENT} from a review fetch: {update:?}");
    let _ = app.emit(MIRROR_UPDATED_EVENT, update);
}

/// Sync a PR's header metadata into the mirror and return it.
///
/// `async` so Tauri runs the slow `gh` GraphQL call off the main (UI) thread,
/// letting it overlap the diff/threads fetches instead of freezing the webview
/// (same rationale as the inbox commands). The DB lock is only taken to write.
#[tauri::command]
pub async fn pr_meta(
    owner: String,
    name: String,
    number: i64,
    app: AppHandle,
    db: State<'_, Db>,
) -> AppResult<gh::PrMeta> {
    let detail = provider_for().pr_detail(&owner, &name, number)?;
    mirror::store::upsert_pr_detail(&db.0.lock().unwrap(), &detail, &now())?;
    notify_mirror(&app, &detail.node.id);
    Ok(detail.meta)
}

/// The PR header metadata as last mirrored, shown while `pr_meta` syncs.
#[tauri::command]
pub fn mirror_pr_meta(owner: String, name: String, number: i64, db: State<Db>) -> AppResult<Option<gh::PrMeta>> {
    mirror::store::read_pr_meta(&db.0.lock().unwrap(), &owner, &name, number)
}

/// `async` for the same reason as `pr_meta`.
#[tauri::command]
pub async fn set_pr_draft(owner: String, name: String, number: i64, draft: bool) -> AppResult<()> {
    provider_for().set_pr_draft(&owner, &name, number, draft)
}

/// Per-check detail of the PR's head commit, for the CI badge popover. `async`
/// for the same reason as `pr_meta`.
#[tauri::command]
pub async fn pr_checks(owner: String, name: String, number: i64) -> AppResult<gh::PrChecks> {
    gh::pr_checks(&owner, &name, number)
}

/// Rerun workflow runs (all their jobs, or only the failed ones) from the checks popover.
#[tauri::command]
pub async fn rerun_ci_runs(owner: String, name: String, run_ids: Vec<i64>, failed_only: bool) -> AppResult<()> {
    let repo = format!("{owner}/{name}");
    run_ids.into_iter().try_for_each(|id| gh::rerun_run(&repo, id, failed_only))
}

/// Rerun one Actions job from the checks popover.
#[tauri::command]
pub async fn rerun_ci_job(owner: String, name: String, job_id: i64) -> AppResult<()> {
    gh::rerun_job(&format!("{owner}/{name}"), job_id)
}

#[tauri::command]
pub async fn set_pr_auto_merge(owner: String, name: String, number: i64, enable: bool) -> AppResult<()> {
    provider_for().set_pr_auto_merge(&owner, &name, number, enable)
}

#[tauri::command]
pub async fn set_pr_label(owner: String, name: String, number: i64, label: String, on: bool) -> AppResult<()> {
    provider_for().set_pr_label(&owner, &name, number, &label, on)
}

#[tauri::command]
pub async fn set_pr_reviewer(
    owner: String,
    name: String,
    number: i64,
    reviewer: String,
    team: bool,
    on: bool,
) -> AppResult<()> {
    provider_for().set_pr_reviewer(&owner, &name, number, &reviewer, team, on)
}

#[tauri::command]
pub async fn repo_labels(owner: String, name: String) -> AppResult<Vec<gh::PrLabel>> {
    provider_for().repo_labels(&owner, &name)
}

#[tauri::command]
pub async fn repo_assignable_users(owner: String, name: String, query: String) -> AppResult<Vec<gh::PrActor>> {
    provider_for().repo_assignable_users(&owner, &name, &query)
}

#[tauri::command]
pub async fn update_pr_branch(owner: String, name: String, number: i64, rebase: bool) -> AppResult<()> {
    provider_for().update_pr_branch(&owner, &name, number, rebase)
}

/// `async` for the same reason as `pr_meta` — keeps the (paginated) review-thread
/// fetch off the UI thread so it loads concurrently with the diff and metadata.
#[tauri::command]
pub async fn pr_review_threads(
    owner: String,
    name: String,
    number: i64,
    app: AppHandle,
    db: State<'_, Db>,
) -> AppResult<Vec<gh::PrThread>> {
    let threads = provider_for().pr_review_threads(&owner, &name, number)?;
    let conn = db.0.lock().unwrap();
    if let Some(id) = mirror::store::pr_node_id(&conn, &owner, &name, number)? {
        mirror::store::upsert_threads(&conn, &id, &threads, &now())?;
        notify_mirror(&app, &id);
    }
    Ok(threads)
}

/// A PR's review threads as last mirrored, shown while `pr_review_threads` syncs.
#[tauri::command]
pub fn mirror_pr_threads(
    owner: String,
    name: String,
    number: i64,
    db: State<Db>,
) -> AppResult<Option<Vec<gh::PrThread>>> {
    mirror::store::read_threads(&db.0.lock().unwrap(), &owner, &name, number)
}

/// Reply to an existing GitHub review thread. `comment_id` is the databaseId of
/// the thread's first comment. Acts on GitHub directly; nothing touches SQLite.
#[tauri::command]
pub async fn reply_to_thread(
    owner: String,
    name: String,
    number: i64,
    comment_id: i64,
    body: String,
) -> AppResult<i64> {
    provider_for().reply_to_thread(&owner, &name, number, comment_id, &body)
}

/// Resolve/unresolve a GitHub review thread by node id. Returns new isResolved.
#[tauri::command]
pub async fn set_pr_thread_resolved(thread_id: String, resolved: bool) -> AppResult<bool> {
    provider_for().set_thread_resolved(&thread_id, resolved)
}

/// Detected external-tool environment, for the Settings diagnostics panel.
/// `git`/`gh` are the resolved absolute paths (or `null` if not found).
#[derive(Debug, Serialize)]
pub struct ToolEnv {
    pub git: Option<String>,
    pub gh: Option<String>,
    pub gh_authed: bool,
    pub claude: Option<String>,
}

/// Fetch a GitHub image URL (e.g. user-attachments) with auth, return as data URL.
#[tauri::command]
pub async fn fetch_github_image(url: String) -> AppResult<String> {
    let bytes = gh::fetch_authenticated_url(&url).await?;
    Ok(gh::image_data_url(&bytes))
}

#[tauri::command]
pub fn check_environment() -> ToolEnv {
    ToolEnv {
        git: tools::git_path(),
        gh: tools::gh_path(),
        gh_authed: provider_for().auth_status(),
        claude: tools::claude_path(),
    }
}
