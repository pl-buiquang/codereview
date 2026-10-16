//! Fetch: the one shared `git fetch --prune origin` per repository, with a
//! per-repo lock and a minimum interval, recorded in `repo_sync`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::git::run_git;

/// `origin/HEAD` is re-resolved from the remote at most this often.
const DEFAULT_BRANCH_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RepoSyncState {
    pub last_fetch_at: Option<String>,
    pub last_fetch_error: Option<String>,
    pub last_local_change_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FetchResult {
    /// false when skipped because the last successful fetch is recent enough.
    pub fetched: bool,
    pub state: RepoSyncState,
}

static LOCKS: Mutex<Option<HashMap<i64, Arc<Mutex<()>>>>> = Mutex::new(None);

fn repo_lock(repo_id: i64) -> Arc<Mutex<()>> {
    LOCKS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .entry(repo_id)
        .or_default()
        .clone()
}

pub fn state(conn: &Connection, repo_id: i64) -> AppResult<RepoSyncState> {
    Ok(conn
        .query_row(
            "SELECT last_fetch_at, last_fetch_error, last_local_change_at FROM repo_sync WHERE repo_id = ?1",
            params![repo_id],
            |r| {
                Ok(RepoSyncState {
                    last_fetch_at: r.get(0)?,
                    last_fetch_error: r.get(1)?,
                    last_local_change_at: r.get(2)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

fn older_than(ts: Option<&str>, age: Duration, now: DateTime<Utc>) -> bool {
    let Some(at) = ts.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else {
        return true;
    };
    (now - at.with_timezone(&Utc)).to_std().is_ok_and(|elapsed| elapsed >= age)
}

/// A fetch can be skipped when the last one succeeded less than `min_interval` ago.
fn is_fresh(state: &RepoSyncState, min_interval: Duration, now: DateTime<Utc>) -> bool {
    state.last_fetch_error.is_none() && !older_than(state.last_fetch_at.as_deref(), min_interval, now)
}

/// Fetches `origin` for the repo unless a successful fetch is younger than
/// `min_interval`. Concurrent calls for the same repo serialize, so the second
/// one usually skips. Also refreshes `repository.default_branch` from
/// `origin/HEAD` once a day. A failed fetch is recorded and returned as `Err`.
pub fn fetch_repo(db: &Db, repo_id: i64, min_interval: Duration) -> AppResult<FetchResult> {
    let lock = repo_lock(repo_id);
    let _guard = lock.lock().unwrap();

    let (path, before, branch_checked_at) = {
        let conn = db.0.lock().unwrap();
        let path: Option<String> = conn.query_row(
            "SELECT local_path FROM repository WHERE id = ?1",
            params![repo_id],
            |r| r.get(0),
        )?;
        let checked: Option<String> = conn
            .query_row(
                "SELECT default_branch_checked_at FROM repo_sync WHERE repo_id = ?1",
                params![repo_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        (path, state(&conn, repo_id)?, checked)
    };
    let path = PathBuf::from(path.ok_or_else(|| AppError::Other("fetch requires a local clone".into()))?);
    let started = Utc::now();
    if is_fresh(&before, min_interval, started) {
        return Ok(FetchResult { fetched: false, state: before });
    }

    let result = run_git(&path, &["fetch", "--quiet", "--prune", "origin"]);
    let check_branch = result.is_ok() && older_than(branch_checked_at.as_deref(), DEFAULT_BRANCH_MAX_AGE, started);
    let default_branch = check_branch.then(|| remote_default_branch(&path)).flatten();

    let now = Utc::now().to_rfc3339();
    let error = result.as_ref().err().map(|e| e.to_string());
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO repo_sync (repo_id, last_fetch_at, last_fetch_error) VALUES (?1, ?2, ?3)
         ON CONFLICT(repo_id) DO UPDATE SET last_fetch_at = excluded.last_fetch_at,
            last_fetch_error = excluded.last_fetch_error",
        params![repo_id, now, error],
    )?;
    if check_branch {
        conn.execute(
            "UPDATE repo_sync SET default_branch_checked_at = ?2 WHERE repo_id = ?1",
            params![repo_id, now],
        )?;
    }
    if let Some(branch) = &default_branch {
        conn.execute(
            "UPDATE repository SET default_branch = ?2 WHERE id = ?1",
            params![repo_id, branch],
        )?;
    }
    let after = state(&conn, repo_id)?;
    match result {
        Ok(_) => Ok(FetchResult { fetched: true, state: after }),
        Err(e) => Err(e),
    }
}

/// `origin/HEAD` as the remote currently reports it. Only that ref, never the
/// checked-out branch: a feature branch must not become the default.
fn remote_default_branch(path: &Path) -> Option<String> {
    let _ = run_git(path, &["remote", "set-head", "origin", "--auto"]);
    run_git(path, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .map(|s| s.trim().trim_start_matches("origin/").to_string())
        .filter(|s| !s.is_empty())
}

/// Stamps a local change seen by a watcher.
pub fn record_local_change(conn: &Connection, repo_id: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO repo_sync (repo_id, last_local_change_at) VALUES (?1, ?2)
         ON CONFLICT(repo_id) DO UPDATE SET last_local_change_at = excluded.last_local_change_at",
        params![repo_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A bare origin on `trunk` and a clone of it registered as repo 1.
    fn setup() -> (TempDir, TempDir, Db) {
        let origin = TempDir::new().unwrap();
        git(origin.path(), &["init", "-q", "--bare", "-b", "trunk"]);
        let seed = TempDir::new().unwrap();
        git(seed.path(), &["init", "-q", "-b", "trunk"]);
        git(seed.path(), &["-c", "user.email=t@e", "-c", "user.name=T", "commit", "-q", "--allow-empty", "-m", "c"]);
        git(seed.path(), &["remote", "add", "origin", origin.path().to_str().unwrap()]);
        git(seed.path(), &["push", "-q", "origin", "trunk"]);
        let clone = TempDir::new().unwrap();
        git(clone.path(), &["clone", "-q", origin.path().to_str().unwrap(), "."]);

        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (id, local_path, default_branch, added_at) VALUES (1, ?1, 'stale', 'now')",
            params![clone.path().to_str().unwrap()],
        )
        .unwrap();
        (origin, clone, Db(Mutex::new(conn)))
    }

    #[test]
    fn freshness_requires_recent_success() {
        let now = Utc::now();
        let recent = Some((now - chrono::Duration::seconds(10)).to_rfc3339());
        let min = Duration::from_secs(60);
        let ok = RepoSyncState { last_fetch_at: recent.clone(), ..Default::default() };
        assert!(is_fresh(&ok, min, now));
        assert!(!is_fresh(&ok, Duration::from_secs(5), now));
        let failed = RepoSyncState { last_fetch_at: recent, last_fetch_error: Some("x".into()), ..Default::default() };
        assert!(!is_fresh(&failed, min, now), "a failed fetch is retried");
        assert!(!is_fresh(&RepoSyncState::default(), min, now), "never fetched");
    }

    #[test]
    fn fetch_records_state_skips_when_fresh_and_refreshes_default_branch() {
        let (_origin, _clone, db) = setup();
        let first = fetch_repo(&db, 1, Duration::from_secs(60)).unwrap();
        assert!(first.fetched);
        assert!(first.state.last_fetch_at.is_some());
        assert!(first.state.last_fetch_error.is_none());
        let branch: String = db.0.lock().unwrap()
            .query_row("SELECT default_branch FROM repository WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(branch, "trunk");

        let second = fetch_repo(&db, 1, Duration::from_secs(60)).unwrap();
        assert!(!second.fetched);
        assert!(fetch_repo(&db, 1, Duration::ZERO).unwrap().fetched);
    }

    #[test]
    fn failed_fetch_is_recorded_and_returned() {
        let (origin, _clone, db) = setup();
        drop(origin);
        assert!(fetch_repo(&db, 1, Duration::from_secs(60)).is_err());
        let st = state(&db.0.lock().unwrap(), 1).unwrap();
        assert!(st.last_fetch_error.is_some());
        assert!(st.last_fetch_at.is_some());
    }
}
