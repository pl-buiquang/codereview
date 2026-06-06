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
