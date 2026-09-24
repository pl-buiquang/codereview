use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::db::models::Repository;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::git;

fn get_repository(conn: &Connection, id: i64) -> AppResult<Repository> {
    conn.query_row(
        "SELECT * FROM repository WHERE id = ?1",
        params![id],
        Repository::from_row,
    )
    .map_err(Into::into)
}

fn get_repository_by_local_path(conn: &Connection, local_path: &str) -> AppResult<Repository> {
    conn.query_row(
        "SELECT * FROM repository WHERE local_path = ?1",
        params![local_path],
        Repository::from_row,
    )
    .map_err(Into::into)
}

/// Insert a repository, or refresh its remote/default-branch metadata if the
/// local_path is already tracked. Returns the resulting row.
fn upsert_repository(
    conn: &Connection,
    local_path: &str,
    remote_owner: Option<String>,
    remote_name: Option<String>,
    default_branch: Option<String>,
) -> AppResult<Repository> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO repository (local_path, remote_owner, remote_name, default_branch, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(local_path) DO UPDATE SET
             remote_owner   = excluded.remote_owner,
             remote_name    = excluded.remote_name,
             default_branch = excluded.default_branch",
        params![local_path, remote_owner, remote_name, default_branch, now],
    )?;
    get_repository_by_local_path(conn, local_path)
}

pub fn add_repository_impl(conn: &Connection, path: &str) -> AppResult<Repository> {
    let p = Path::new(path);
    if !git::is_git_repo(p) {
        return Err(AppError::NotARepo(path.to_string()));
    }
    let remote = git::remote_info(p);
    let default_branch = git::default_branch(p);

    // If a remote-only row (local_path IS NULL) exists for this remote, merge:
    // set its local_path so all existing targets/reviews gain the local clone.
    if let (Some(ref owner), Some(ref name)) = (&remote.owner, &remote.name) {
        let existing: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT id, local_path FROM repository WHERE remote_owner = ?1 AND remote_name = ?2",
                params![owner, name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, None)) = existing {
            conn.execute(
                "UPDATE repository SET local_path = ?1, default_branch = ?2 WHERE id = ?3",
                params![path, default_branch, id],
            )?;
            return get_repository(conn, id);
        }
    }

    upsert_repository(conn, path, remote.owner, remote.name, default_branch)
}

#[tauri::command]
pub fn add_repository(path: String, db: State<Db>) -> AppResult<Repository> {
    let conn = db.0.lock().unwrap();
    add_repository_impl(&conn, &path)
}

pub fn list_repositories_impl(conn: &Connection) -> AppResult<Vec<Repository>> {
    let mut stmt = conn.prepare("SELECT * FROM repository ORDER BY added_at DESC")?;
    let rows = stmt
        .query_map([], Repository::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[tauri::command]
pub fn list_repositories(db: State<Db>) -> AppResult<Vec<Repository>> {
    let conn = db.0.lock().unwrap();
    list_repositories_impl(&conn)
}

pub fn remove_repository_impl(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM repository WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn remove_repository(id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    remove_repository_impl(&conn, id)
}

#[tauri::command]
pub fn list_branches(repo_id: i64, db: State<Db>) -> AppResult<Vec<git::Branch>> {
    let conn = db.0.lock().unwrap();
    let local_path: Option<String> = conn
        .query_row(
            "SELECT local_path FROM repository WHERE id = ?1",
            params![repo_id],
            |r| r.get(0),
        )
        .map_err(AppError::from)?;
    let path =
        local_path.ok_or_else(|| AppError::Other("listing branches requires a local clone".into()))?;
    git::list_branches(Path::new(&path))
}

/// Unified diff between two refs for a "virtual PR" comparison.
#[tauri::command]
pub fn diff_refs(
    repo_id: i64,
    base: String,
    head: String,
    three_dot: bool,
    db: State<Db>,
) -> AppResult<String> {
    let conn = db.0.lock().unwrap();
    let local_path: Option<String> = conn
        .query_row(
            "SELECT local_path FROM repository WHERE id = ?1",
            params![repo_id],
            |r| r.get(0),
        )
        .map_err(AppError::from)?;
    let path =
        local_path.ok_or_else(|| AppError::Other("diff requires a local clone".into()))?;
    git::diff(Path::new(&path), &base, &head, three_dot)
}

/// Manually link a local git clone to an existing remote-only repository row.
#[tauri::command]
pub fn link_local_path(repo_id: i64, path: String, db: State<Db>) -> AppResult<Repository> {
    let p = Path::new(&path);
    if !git::is_git_repo(p) {
        return Err(AppError::NotARepo(path));
    }
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE repository SET local_path = ?1 WHERE id = ?2",
        params![path, repo_id],
    )?;
    get_repository(&conn, repo_id)
}

/// Scan all remote-only repos and try to find local clones under the configured
/// base paths. Returns the count of repos that were newly linked.
#[tauri::command]
pub fn auto_link_repos(base_paths: Vec<String>, db: State<Db>) -> AppResult<i64> {
    let conn = db.0.lock().unwrap();
    let remote_only: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, remote_owner, remote_name FROM repository
             WHERE local_path IS NULL AND remote_owner IS NOT NULL AND remote_name IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    let mut count = 0i64;
    for (id, owner, name) in remote_only {
        if let Some(path) = resolve_local_path(&owner, &name, &base_paths) {
            let default_branch = git::default_branch(Path::new(&path));
            conn.execute(
                "UPDATE repository SET local_path = ?1, default_branch = COALESCE(default_branch, ?2) WHERE id = ?3",
                params![path, default_branch, id],
            )?;
            count += 1;
        }
    }
    Ok(count)
}

/// Expand a leading `~/` to the real home directory.
fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// Try to find a local git clone of `owner/name` under the configured base paths.
/// Searches up to 2 levels: direct child, owner-namespaced child, and a scan
/// of immediate subdirectories (to catch renamed clones).
pub fn resolve_local_path(owner: &str, name: &str, base_paths: &[String]) -> Option<String> {
    for base_raw in base_paths {
        let base_str = expand_home(base_raw.trim());
        let base = Path::new(&base_str);

        // Priority 1: {base}/{name}
        let candidate = base.join(name);
        if is_matching_clone(&candidate, owner, name) {
            return candidate.to_str().map(String::from);
        }
        // Priority 2: {base}/{owner}/{name}
        let candidate = base.join(owner).join(name);
        if is_matching_clone(&candidate, owner, name) {
            return candidate.to_str().map(String::from);
        }
        // Priority 3: scan immediate subdirectories (catches renamed clones)
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let candidate = entry.path();
                    if is_matching_clone(&candidate, owner, name) {
                        return candidate.to_str().map(String::from);
                    }
                }
            }
        }
    }
    None
}

fn is_matching_clone(path: &Path, owner: &str, name: &str) -> bool {
    if !git::is_git_repo(path) {
        return false;
    }
    let remote = git::remote_info(path);
    remote.owner.as_deref() == Some(owner) && remote.name.as_deref() == Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    #[test]
    fn upsert_inserts_new_repository() {
        let conn = open_memory();
        let repo = upsert_repository(
            &conn,
            "/path/to/repo",
            Some("owner".into()),
            Some("name".into()),
            Some("main".into()),
        )
        .unwrap();
        assert_eq!(repo.local_path.as_deref(), Some("/path/to/repo"));
        assert_eq!(repo.remote_owner.as_deref(), Some("owner"));
        assert_eq!(repo.remote_name.as_deref(), Some("name"));
        assert_eq!(repo.default_branch.as_deref(), Some("main"));
    }

    #[test]
    fn upsert_refreshes_metadata_without_duplicating() {
        let conn = open_memory();
        let first = upsert_repository(&conn, "/repo", None, None, Some("main".into())).unwrap();

        // Re-adding the same path updates remote info in place.
        let second = upsert_repository(
            &conn,
            "/repo",
            Some("acme".into()),
            Some("widget".into()),
            Some("develop".into()),
        )
        .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.remote_owner.as_deref(), Some("acme"));
        assert_eq!(second.default_branch.as_deref(), Some("develop"));
        // added_at of the original row is preserved across the upsert.
        assert_eq!(first.added_at, second.added_at);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM repository", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_repository_by_local_path_errors_when_missing() {
        let conn = open_memory();
        assert!(get_repository_by_local_path(&conn, "/absent").is_err());
    }
}
