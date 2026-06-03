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
const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

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
