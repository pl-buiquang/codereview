pub mod models;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::AppResult;

/// Tauri-managed database handle. rusqlite is synchronous, so a Mutex around a
/// single connection is sufficient for a desktop single-user app.
pub struct Db(pub Mutex<Connection>);

/// Ordered list of migration SQL scripts. The current schema version is stored
/// in SQLite's `user_version` pragma; on open we apply any scripts beyond it.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_file_view_state.sql"),
    include_str!("migrations/0003_comment_subject_type.sql"),
    include_str!("migrations/0004_comment_origin.sql"),
    include_str!("migrations/0005_repo_remote_index.sql"),
    include_str!("migrations/0006_inbox.sql"),
    include_str!("migrations/0007_comment_resolved.sql"),
    include_str!("migrations/0008_comment_anchored_base_sha.sql"),
    include_str!("migrations/0009_review_status_pending.sql"),
];

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    // execute_batch tolerates the result row that `journal_mode` returns.
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> AppResult<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut version = current as usize;
    while version < MIGRATIONS.len() {
        conn.execute_batch(MIGRATIONS[version])?;
        version += 1;
    }
    conn.pragma_update(None, "user_version", version as i64)?;
    Ok(())
}

/// Open an in-memory, fully-migrated database for tests. Foreign keys are
/// enabled (so cascade-delete behaviour matches production); WAL is skipped as
/// it is meaningless for `:memory:`.
#[cfg(test)]
pub(crate) fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    migrate(&conn).expect("run migrations");
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_sets_user_version_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        let before: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(before, 0);

        migrate(&conn).unwrap();
        let after: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(after, MIGRATIONS.len() as i64);

        // Re-running migrate must be a no-op (no "table already exists" error).
        migrate(&conn).unwrap();
    }

    #[test]
    fn schema_creates_expected_tables() {
        let conn = open_memory();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for expected in ["comment", "repository", "review", "target"] {
            assert!(tables.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn migration_0007_upgrades_an_existing_0006_database() {
        // Apply every migration except the last (simulating a DB created at 0006),
        // insert a pre-0007 comment, then run the full migrate and confirm the new
        // column applies cleanly with the old row defaulting to NULL.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (1, 'a.rs', 'RIGHT', 1, 'old', 'now', 'now')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let resolved: Option<String> = conn
            .query_row("SELECT resolved_at FROM comment WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(resolved.is_none(), "old comment defaults to unresolved");
    }

    #[test]
    fn migration_0008_upgrades_an_existing_0007_database() {
        // Apply every migration except the last (simulating a DB created at 0007),
        // insert a pre-0008 comment, then run the full migrate and confirm the new
        // anchored_base_sha column applies cleanly with the old row defaulting to NULL.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (1, 'a.rs', 'LEFT', 1, 'old', 'now', 'now')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let base_sha: Option<String> = conn
            .query_row("SELECT anchored_base_sha FROM comment WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(base_sha.is_none(), "old comment defaults to NULL base pin");
    }

    #[test]
    fn migration_0009_rebuilds_review_preserving_rows() {
        // Apply migrations through 0008 (a DB created at schema version 8), seed
        // a review + comment + file_view_state, then apply the 0009 table rebuild
        // and confirm the rebuild preserved every row and broke no FK.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..8] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", 8_i64).unwrap();

        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (id, target_id, body, status, created_at, updated_at)
             VALUES (42, 1, 'b', 'published', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (42, 'a.rs', 'RIGHT', 1, 'c', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_view_state (review_id, file_path, viewed, updated_at)
             VALUES (42, 'a.rs', 1, 'now')",
            [],
        )
        .unwrap();

        conn.execute_batch(MIGRATIONS[8]).unwrap();

        let (id, status): (i64, String) = conn
            .query_row("SELECT id, status FROM review", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(id, 42, "review row keeps its id through the rebuild");
        assert_eq!(status, "published", "review status survives the rebuild");

        let comments: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(comments, 1, "comment rows are NOT cascade-deleted");
        let views: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_view_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(views, 1, "file_view_state rows are NOT cascade-deleted");

        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0, "no dangling foreign keys after the rebuild");
    }

    #[test]
    fn migration_0009_check_accepts_pending_rejects_bogus() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'published_pending', 'now', 'now')",
            [],
        )
        .expect("published_pending passes the rebuilt CHECK");
        let bogus = conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'bogus', 'now', 'now')",
            [],
        );
        assert!(bogus.is_err(), "an unknown status fails the CHECK");
    }

    #[test]
    fn migration_0009_cascade_still_works() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (id, target_id, body, status, created_at, updated_at)
             VALUES (7, 1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (7, 'a.rs', 'RIGHT', 1, 'c', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_view_state (review_id, file_path, viewed, updated_at)
             VALUES (7, 'a.rs', 1, 'now')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM review WHERE id = 7", []).unwrap();

        let comments: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(comments, 0, "deleting a review cascades to its comments");
        let views: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_view_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(views, 0, "deleting a review cascades to file_view_state");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let conn = open_memory();
        // Inserting a target for a non-existent repo must fail the FK constraint.
        let err = conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (999, 'local', 't', 'a', 'b', 'now')",
            [],
        );
        assert!(err.is_err(), "expected foreign key violation");
    }
}
