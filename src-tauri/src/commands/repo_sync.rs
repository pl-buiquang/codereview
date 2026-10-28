use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::Db;
use crate::error::AppResult;
use crate::repo_sync::{self, FetchResult, RepoSyncState};
use crate::watch::{ChangeKind, Watchers};

/// Absorbs double clicks on the Fetch button.
const MANUAL_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(5);

pub const GIT_CHANGED_EVENT: &str = "git-changed";

#[derive(Clone, Serialize)]
struct GitChanged {
    repo_id: i64,
    kinds: Vec<ChangeKind>,
}

/// `git fetch --prune origin`, skipped when a successful fetch is younger than
/// `min_interval_secs` (default: a few seconds).
#[tauri::command]
pub async fn fetch_repo(repo_id: i64, min_interval_secs: Option<u64>, db: State<'_, Db>) -> AppResult<FetchResult> {
    let min = min_interval_secs.map_or(MANUAL_FETCH_MIN_INTERVAL, Duration::from_secs);
    repo_sync::fetch_repo(&db, repo_id, min)
}

#[tauri::command]
pub fn repo_sync_state(repo_id: i64, db: State<Db>) -> AppResult<RepoSyncState> {
    repo_sync::state(&db.0.lock().unwrap(), repo_id)
}

/// Holds a watcher on the repo's git dir. Returns false when the repo can't be
/// watched (no local clone, or the watcher failed); then don't release.
#[tauri::command(async)]
pub fn watch_acquire(app: AppHandle, repo_id: i64, db: State<Db>, watchers: State<Watchers>) -> AppResult<bool> {
    let local_path: Option<String> = db.0.lock().unwrap().query_row(
        "SELECT local_path FROM repository WHERE id = ?1",
        rusqlite::params![repo_id],
        |r| r.get(0),
    )?;
    let Some(local_path) = local_path else {
        log::debug!(target: "watch", "acquire repo {repo_id}: no local clone, not watching");
        return Ok(false);
    };
    let on_change = Arc::new(move |repo_id: i64, kinds: Vec<ChangeKind>| {
        crate::worktree_gc::note_change(repo_id);
        if let Err(e) = repo_sync::record_local_change(&app.state::<Db>().0.lock().unwrap(), repo_id) {
            crate::logging::warn(&format!("[watch] could not record change for repo {repo_id}: {e}"));
        }
        log::debug!(target: "watch", "emit {GIT_CHANGED_EVENT} repo {repo_id} {kinds:?}");
        let _ = app.emit(GIT_CHANGED_EVENT, GitChanged { repo_id, kinds });
    });
    match watchers.acquire(repo_id, Path::new(&local_path), on_change) {
        Ok(()) => Ok(true),
        Err(e) => {
            crate::logging::warn(&format!("[watch] repo {repo_id}: {e}"));
            Ok(false)
        }
    }
}

#[tauri::command]
pub fn watch_release(repo_id: i64, watchers: State<Watchers>) {
    log::debug!(target: "watch", "release repo {repo_id}");
    watchers.release(repo_id);
}
