use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};
use tauri::State;

use crate::db::models::Repository;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::git;

fn repo_by_path(conn: &Connection, path: &str) -> AppResult<Repository> {
    conn.query_row(
        "SELECT * FROM repository WHERE path = ?1",
        params![path],
        Repository::from_row,
    )
    .map_err(Into::into)
}

/// Insert a repository, or refresh its remote/default-branch metadata if the
/// path is already tracked. Returns the resulting row. The `added_at` of an
/// existing repo is intentionally preserved.
fn upsert_repository(
    conn: &Connection,
    path: &str,
    remote_owner: Option<String>,
    remote_name: Option<String>,
    default_branch: Option<String>,
) -> AppResult<Repository> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO repository (path, remote_owner, remote_name, default_branch, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET
             remote_owner   = excluded.remote_owner,
             remote_name    = excluded.remote_name,
             default_branch = excluded.default_branch",
        params![path, remote_owner, remote_name, default_branch, now],
    )?;
    // last_insert_rowid is unreliable after an upsert, so look it up by path.
    repo_by_path(conn, path)
}

#[tauri::command]
pub fn add_repository(path: String, db: State<Db>) -> AppResult<Repository> {
    let p = Path::new(&path);
    if !git::is_git_repo(p) {
        return Err(AppError::NotARepo(path));
    }
    let remote = git::remote_info(p);
    let default_branch = git::default_branch(p);

    let conn = db.0.lock().unwrap();
    upsert_repository(&conn, &path, remote.owner, remote.name, default_branch)
}

#[tauri::command]
pub fn list_repositories(db: State<Db>) -> AppResult<Vec<Repository>> {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare("SELECT * FROM repository ORDER BY added_at DESC")?;
    let rows = stmt
        .query_map([], Repository::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[tauri::command]
pub fn remove_repository(id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM repository WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn list_branches(repo_path: String) -> AppResult<Vec<git::Branch>> {
    git::list_branches(Path::new(&repo_path))
}

/// Unified diff between two refs for a "virtual PR" comparison.
#[tauri::command]
pub fn diff_refs(
    repo_path: String,
    base: String,
    head: String,
    three_dot: bool,
) -> AppResult<String> {
    git::diff(Path::new(&repo_path), &base, &head, three_dot)
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
        assert_eq!(repo.path, "/path/to/repo");
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
    fn repo_by_path_errors_when_missing() {
        let conn = open_memory();
        assert!(repo_by_path(&conn, "/absent").is_err());
    }
}
