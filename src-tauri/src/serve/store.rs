//! The serve DB: runs, their events and the outbox of `cr` ops. A SQLite file of its own, separate
//! from the interactive app DB. `ServeStore` plugs it into the run engine (one engine for every harness).
//!
//! `cr` inside a remote run opens the same file to append outbox ops (see [`super::remote`]), so
//! the connection uses WAL and a busy timeout.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::AppResult;
use crate::run_engine::{Finished, RunStore, RunningRun};
use crate::run_usage::{now_stamp, RunUsage};

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE serve_run (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    idem_key     TEXT NOT NULL UNIQUE,
    harness      TEXT NOT NULL,
    model        TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'preparing',
    request      TEXT NOT NULL,
    cwd          TEXT NOT NULL,
    timeout_secs INTEGER NOT NULL,
    pid          INTEGER NOT NULL DEFAULT 0,
    session_id   TEXT,
    cleanup      TEXT NOT NULL DEFAULT '[]',
    text         TEXT NOT NULL DEFAULT '',
    error        TEXT,
    usage        TEXT,
    created_at   TEXT NOT NULL,
    started_at   TEXT,
    finished_at  TEXT,
    updated_at   TEXT NOT NULL
);
CREATE INDEX serve_run_status ON serve_run(status);
CREATE INDEX serve_run_updated ON serve_run(updated_at);

CREATE TABLE serve_event (
    run_id INTEGER NOT NULL REFERENCES serve_run(id) ON DELETE CASCADE,
    seq    INTEGER NOT NULL,
    kind   TEXT NOT NULL,
    json   TEXT NOT NULL,
    norm   TEXT NOT NULL DEFAULT '',
    ts     TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);

CREATE TABLE serve_outbox (
    run_id     INTEGER NOT NULL REFERENCES serve_run(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    op_id      TEXT NOT NULL UNIQUE,
    type       TEXT NOT NULL,
    args       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    acked      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, seq)
);
"#];

/// Create `dir` (and missing parents) readable by the owner only. An existing directory is left alone.
#[cfg(unix)]
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

#[cfg(not(unix))]
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Make `dir` owner-only even when it already exists (the daemon's own directories).
#[cfg(unix)]
pub fn restrict_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    create_private_dir(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub fn restrict_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Open (and migrate) the serve DB at `path`. The file is created owner-only, so the `-wal` and
/// `-shm` files SQLite derives from it are too (prompts and answers are stored in it).
pub fn open(path: &Path) -> AppResult<Connection> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_private_dir(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if !path.exists() {
            std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(path)?;
        } else if std::fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
    let mut version = conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    while version < MIGRATIONS.len() {
        conn.execute_batch(MIGRATIONS[version])?;
        version += 1;
        conn.pragma_update(None, "user_version", version as i64)?;
    }
    Ok(conn)
}

/// `preparing`: inserted but its attachments are not on disk yet, so the engine must not pick it up.
pub fn is_terminal(status: &str) -> bool {
    !matches!(status, "preparing" | "queued" | "running")
}

/// A run row as the API reports it.
#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub id: i64,
    pub idempotency_key: String,
    pub harness: String,
    pub model: String,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub updated_at: String,
    pub text: String,
    pub error: Option<String>,
    pub usage: Option<Value>,
    pub last_seq: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub seq: i64,
    pub kind: String,
    /// The raw line, parsed when it is JSON.
    pub json: Value,
    /// The harness-neutral form (`{"v":1,"events":[..]}`).
    pub norm: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboxOp {
    pub seq: i64,
    pub op_id: String,
    #[serde(rename = "type")]
    pub op_type: String,
    pub args: Value,
    pub created_at: String,
}

fn parse_or_string(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

const RUN_COLS: &str = "id, idem_key, harness, model, status, created_at, started_at, finished_at, updated_at, text, error, usage,
    COALESCE((SELECT MAX(seq) FROM serve_event WHERE run_id = serve_run.id), 0)";

fn run_from_row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        idempotency_key: r.get(1)?,
        harness: r.get(2)?,
        model: r.get(3)?,
        status: r.get(4)?,
        created_at: r.get(5)?,
        started_at: r.get(6)?,
        finished_at: r.get(7)?,
        updated_at: r.get(8)?,
        text: r.get(9)?,
        error: r.get(10)?,
        usage: r.get::<_, Option<String>>(11)?.map(|s| parse_or_string(&s)),
        last_seq: r.get(12)?,
    })
}

pub fn usage_json(u: &RunUsage) -> Value {
    json!({
        "num_turns": u.num_turns,
        "input_tokens": u.input_tokens,
        "output_tokens": u.output_tokens,
        "cache_read_tokens": u.cache_read_tokens,
        "cache_creation_tokens": u.cache_creation_tokens,
        "cost_usd": u.cost_usd,
        "session_id": u.session_id,
    })
}

/// The shared connection of the daemon.
pub struct ServeDb(pub Mutex<Connection>);

impl ServeDb {
    pub fn open(path: &Path) -> AppResult<Arc<Self>> {
        Ok(Arc::new(Self(Mutex::new(open(path)?))))
    }

    /// Insert a `preparing` run (the engine ignores it until [`mark_queued`](Self::mark_queued)), or
    /// return the one that already has this idempotency key. `true`: created.
    pub fn insert_run(
        &self,
        idem_key: &str,
        harness: &str,
        model: &str,
        request_json: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> AppResult<(i64, bool)> {
        let conn = self.0.lock().unwrap();
        let now = now_stamp();
        let n = conn.execute(
            "INSERT INTO serve_run (idem_key, harness, model, request, cwd, timeout_secs, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) ON CONFLICT(idem_key) DO NOTHING",
            params![idem_key, harness, model, request_json, cwd.to_string_lossy(), timeout_secs as i64, now],
        )?;
        let id = conn.query_row("SELECT id FROM serve_run WHERE idem_key = ?1", params![idem_key], |r| r.get(0))?;
        Ok((id, n == 1))
    }

    pub fn run_by_key(&self, idem_key: &str) -> Option<i64> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT id FROM serve_run WHERE idem_key = ?1", params![idem_key], |r| r.get(0)).ok()
    }

    /// The run is fully prepared (attachments written): the engine may start it. False when it is no
    /// longer `preparing`.
    pub fn mark_queued(&self, id: i64) -> bool {
        let n = self
            .0
            .lock()
            .unwrap()
            .execute("UPDATE serve_run SET status = 'queued', updated_at = ?2 WHERE id = ?1 AND status = 'preparing'", params![id, now_stamp()])
            .unwrap_or(0);
        n == 1
    }

    /// Remove a run whose preparation failed. Only a `preparing` row goes: once queued the engine owns it.
    pub fn delete_run(&self, id: i64) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM serve_run WHERE id = ?1 AND status = 'preparing'", params![id]);
    }

    /// Startup: rows a crashed daemon left `preparing` never became runs. Returns their ids.
    pub fn purge_preparing(&self) -> Vec<i64> {
        let conn = self.0.lock().unwrap();
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM serve_run WHERE status = 'preparing'")
            .and_then(|mut st| st.query_map([], |r| r.get(0))?.collect())
            .unwrap_or_default();
        let _ = conn.execute("DELETE FROM serve_run WHERE status = 'preparing'", []);
        ids
    }

    pub fn get_run(&self, id: i64) -> AppResult<Option<RunRow>> {
        let conn = self.0.lock().unwrap();
        Ok(conn.query_row(&format!("SELECT {RUN_COLS} FROM serve_run WHERE id = ?1"), params![id], run_from_row).optional()?)
    }

    /// Runs updated after `since` (an RFC 3339 stamp as reported in `updated_at`), oldest first.
    pub fn list_runs(&self, since: Option<&str>, limit: i64) -> AppResult<Vec<RunRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLS} FROM serve_run WHERE (?1 IS NULL OR updated_at > ?1) ORDER BY updated_at, id LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![since, limit], run_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The stored request (attachment bytes are never kept).
    pub fn request_json(&self, id: i64) -> AppResult<Option<String>> {
        let conn = self.0.lock().unwrap();
        Ok(conn.query_row("SELECT request FROM serve_run WHERE id = ?1", params![id], |r| r.get(0)).optional()?)
    }

    pub fn cwd(&self, id: i64) -> AppResult<Option<PathBuf>> {
        let conn = self.0.lock().unwrap();
        Ok(conn
            .query_row("SELECT cwd FROM serve_run WHERE id = ?1", params![id], |r| r.get::<_, String>(0))
            .optional()?
            .map(PathBuf::from))
    }

    pub fn status(&self, id: i64) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT status FROM serve_run WHERE id = ?1", params![id], |r| r.get(0)).ok()
    }

    pub fn events_after(&self, id: i64, after_seq: i64, limit: i64) -> AppResult<Vec<EventRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT seq, kind, json, norm FROM serve_event WHERE run_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3")?;
        let rows = stmt
            .query_map(params![id, after_seq, limit], |r| {
                Ok(EventRow {
                    seq: r.get(0)?,
                    kind: r.get(1)?,
                    json: parse_or_string(&r.get::<_, String>(2)?),
                    norm: parse_or_string(&r.get::<_, String>(3)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn counts(&self) -> (i64, i64) {
        let conn = self.0.lock().unwrap();
        let count = |status: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM serve_run WHERE status = ?1",
                params![status],
                |r| r.get(0),
            )
            .unwrap_or(0)
        };
        (count("running"), count("queued"))
    }

    pub fn outbox_after(&self, id: i64, after: i64) -> AppResult<Vec<OutboxOp>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT seq, op_id, type, args, created_at FROM serve_outbox WHERE run_id = ?1 AND seq > ?2 AND acked = 0 ORDER BY seq",
        )?;
        let rows = stmt
            .query_map(params![id, after], |r| {
                Ok(OutboxOp {
                    seq: r.get(0)?,
                    op_id: r.get(1)?,
                    op_type: r.get(2)?,
                    args: parse_or_string(&r.get::<_, String>(3)?),
                    created_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Mark every op up to `upto` as applied by the client, so the run can be pruned. Returns how many.
    pub fn ack_outbox(&self, id: i64, upto: i64) -> AppResult<usize> {
        let conn = self.0.lock().unwrap();
        Ok(conn.execute("UPDATE serve_outbox SET acked = 1 WHERE run_id = ?1 AND seq <= ?2 AND acked = 0", params![id, upto])?)
    }
}

/// Append an op to a run's outbox on an open connection. The op id (`<run>:<seq>`) is unique, so a
/// client re-applying after a reconnect can recognise ops it already applied.
pub fn append_outbox(conn: &Connection, run_id: i64, op_type: &str, args: &Value) -> AppResult<String> {
    let tx = conn.unchecked_transaction()?;
    let seq: i64 =
        tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM serve_outbox WHERE run_id = ?1", params![run_id], |r| r.get(0))?;
    let op_id = format!("{run_id}:{seq}");
    tx.execute(
        "INSERT INTO serve_outbox (run_id, seq, op_id, type, args, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![run_id, seq, op_id, op_type, args.to_string(), now_stamp()],
    )?;
    tx.commit()?;
    Ok(op_id)
}

/// The run engine's view of the serve DB. One engine serves every harness (each run names its own).
pub struct ServeStore {
    pub db: Arc<ServeDb>,
    cap: Arc<AtomicI64>,
    root: PathBuf,
}

impl ServeStore {
    pub fn new(db: Arc<ServeDb>, cap: Arc<AtomicI64>, root: PathBuf) -> Arc<Self> {
        Arc::new(Self { db, cap, root })
    }

    fn exec(&self, sql: &str, p: impl rusqlite::Params) -> AppResult<usize> {
        Ok(self.db.0.lock().unwrap().execute(sql, p)?)
    }

    fn count(&self, statuses: &[&str]) -> AppResult<i64> {
        let conn = self.db.0.lock().unwrap();
        let mut n = 0;
        for s in statuses {
            n += conn.query_row(
                "SELECT COUNT(*) FROM serve_run WHERE status = ?1",
                params![s],
                |r| r.get::<_, i64>(0),
            )?;
        }
        Ok(n)
    }
}

impl RunStore for ServeStore {
    fn ai_enabled(&self) -> bool {
        true
    }

    fn max_concurrency(&self) -> i64 {
        self.cap.load(Ordering::Relaxed).max(1)
    }

    fn running_count(&self) -> AppResult<i64> {
        self.count(&["running"])
    }

    fn next_queued(&self) -> AppResult<Option<i64>> {
        let conn = self.db.0.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT id FROM serve_run WHERE status = 'queued' ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn mark_running(&self, id: i64) -> AppResult<bool> {
        let now = now_stamp();
        Ok(self.exec(
            "UPDATE serve_run SET status = 'running', started_at = ?2, updated_at = ?2 WHERE id = ?1 AND status = 'queued'",
            params![id, now],
        )? == 1)
    }

    fn is_running(&self, id: i64) -> bool {
        self.db.status(id).as_deref() == Some("running")
    }

    fn drop_queued(&self, id: i64) -> AppResult<bool> {
        let now = now_stamp();
        Ok(self.exec(
            "UPDATE serve_run SET status = 'killed', finished_at = ?2, updated_at = ?2 WHERE id = ?1 AND status = 'queued'",
            params![id, now],
        )? == 1)
    }

    fn active_count(&self) -> AppResult<usize> {
        Ok(self.count(&["preparing", "queued", "running"])? as usize)
    }

    fn kill_queued(&self) -> AppResult<()> {
        let now = now_stamp();
        self.exec(
            "UPDATE serve_run SET status = 'killed', finished_at = ?1, updated_at = ?1 WHERE status = 'queued'",
            params![now],
        )?;
        Ok(())
    }

    fn kill_active(&self) -> AppResult<()> {
        let now = now_stamp();
        self.exec(
            "UPDATE serve_run SET status = 'killed', finished_at = ?2, updated_at = ?2
             WHERE status IN ('queued', 'running')",
            params![now],
        )?;
        Ok(())
    }

    fn set_process(&self, id: i64, pid: u32, _cwd: &Path, session_id: &str) {
        let _ = self.exec("UPDATE serve_run SET pid = ?2, session_id = ?3 WHERE id = ?1", params![id, pid as i64, session_id]);
    }

    fn set_cleanup(&self, id: i64, dirs: &[PathBuf]) {
        let list: Vec<String> = dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect();
        let _ = self.exec("UPDATE serve_run SET cleanup = ?2 WHERE id = ?1", params![id, json!(list).to_string()]);
    }

    fn cleanup_dirs(&self, id: i64) -> Vec<PathBuf> {
        let conn = self.db.0.lock().unwrap();
        let raw: Option<String> =
            conn.query_row("SELECT cleanup FROM serve_run WHERE id = ?1", params![id], |r| r.get(0)).ok();
        raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()).unwrap_or_default().into_iter().map(PathBuf::from).collect()
    }

    fn append_event(&self, id: i64, seq: i64, kind: &str, json: &str, norm: &str) {
        let _ = self.exec(
            "INSERT OR IGNORE INTO serve_event (run_id, seq, kind, json, norm, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, seq, kind, json, norm, now_stamp()],
        );
    }

    fn last_seq(&self, id: i64) -> i64 {
        let conn = self.db.0.lock().unwrap();
        conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM serve_event WHERE run_id = ?1", params![id], |r| r.get(0)).unwrap_or(0)
    }

    fn running_runs(&self) -> AppResult<Vec<RunningRun>> {
        let conn = self.db.0.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, harness, pid, session_id, started_at FROM serve_run WHERE status = 'running'")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RunningRun {
                    id: r.get(0)?,
                    harness: r.get(1)?,
                    pid: r.get::<_, i64>(2)? as i32,
                    session_id: r.get(3)?,
                    started_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn finish(&self, id: i64, fin: &Finished) {
        let now = now_stamp();
        let _ = self.exec(
            "UPDATE serve_run SET status = ?2, text = ?3, error = ?4, usage = ?5, finished_at = ?6, updated_at = ?6
             WHERE id = ?1 AND status = 'running'",
            params![id, fin.status, fin.text, fin.error, usage_json(&fin.usage).to_string(), now],
        );
    }

    fn prune(&self, keep: i64) {
        // Runs whose outbox the client has not acknowledged yet stay, so no op is lost.
        let _ = self.exec(
            "DELETE FROM serve_run WHERE status NOT IN ('queued', 'running')
               AND id NOT IN (SELECT id FROM serve_run ORDER BY id DESC LIMIT ?1)
               AND NOT EXISTS (SELECT 1 FROM serve_outbox o WHERE o.run_id = serve_run.id AND o.acked = 0)",
            params![keep],
        );
    }

    fn mark_lost(&self, id: i64) {
        let now = now_stamp();
        let _ = self.exec(
            "UPDATE serve_run SET status = 'interrupted', finished_at = ?2, updated_at = ?2 WHERE id = ?1 AND status = 'running'",
            params![id, now],
        );
    }

    fn runs_root(&self) -> PathBuf {
        self.root.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_db(name: &str) -> (tempfile::TempDir, Arc<ServeDb>) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServeDb::open(&dir.path().join(format!("{name}.db"))).unwrap();
        (dir, db)
    }

    #[test]
    #[cfg(unix)]
    fn the_db_and_its_wal_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("fresh").join("data");
        let path = data.join("serve.db");
        let db = ServeDb::open(&path).unwrap();
        db.insert_run("k", "claude", "", "{}", Path::new("/tmp"), 60).unwrap();
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&path), 0o600);
        let wal = data.join("serve.db-wal");
        assert!(wal.exists());
        assert_eq!(mode(&wal), 0o600);
        // a DB that an older version left world-readable is tightened
        drop(db);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        ServeDb::open(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        // restrict_dir tightens a directory that already exists
        let shared = dir.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        restrict_dir(&shared).unwrap();
        assert_eq!(mode(&shared), 0o700);
    }

    #[test]
    fn insert_is_idempotent_on_the_key() {
        let (_d, db) = temp_db("idem");
        let (a, created) = db.insert_run("k1", "claude", "m", "{}", Path::new("/tmp"), 60).unwrap();
        assert!(created);
        let (b, created) = db.insert_run("k1", "claude", "m", "{}", Path::new("/tmp"), 60).unwrap();
        assert!(!created);
        assert_eq!(a, b);
        let (c, _) = db.insert_run("k2", "claude", "m", "{}", Path::new("/tmp"), 60).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn events_replay_after_a_seq_and_are_idempotent() {
        let (d, db) = temp_db("ev");
        let (id, _) = db.insert_run("k", "claude", "", "{}", Path::new("/tmp"), 60).unwrap();
        let store = ServeStore::new(db.clone(), Arc::new(AtomicI64::new(2)), d.path().join("runs"));
        for seq in 1..=3 {
            store.append_event(id, seq, "assistant", &format!(r#"{{"n":{seq}}}"#), r#"{"v":1,"events":[]}"#);
        }
        store.append_event(id, 2, "assistant", r#"{"n":99}"#, "");
        let after = db.events_after(id, 1, 10).unwrap();
        assert_eq!(after.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(after[0].json["n"], 2);
        assert_eq!(after[0].norm["v"], 1);
        assert_eq!(store.last_seq(id), 3);
        assert_eq!(db.get_run(id).unwrap().unwrap().last_seq, 3);
    }

    #[test]
    fn lifecycle_transitions_are_guarded() {
        let (d, db) = temp_db("life");
        let (id, _) = db.insert_run("k", "claude", "", "{}", Path::new("/tmp"), 60).unwrap();
        let store = ServeStore::new(db.clone(), Arc::new(AtomicI64::new(1)), d.path().join("runs"));
        assert_eq!(store.next_queued().unwrap(), None, "a preparing run is invisible to the engine");
        assert_eq!(store.active_count().unwrap(), 1, "but it counts against the queue limit");
        assert!(db.mark_queued(id));
        assert!(!db.mark_queued(id));
        assert_eq!(store.next_queued().unwrap(), Some(id));
        assert!(store.mark_running(id).unwrap());
        assert!(!store.mark_running(id).unwrap());
        assert!(!store.drop_queued(id).unwrap());
        assert_eq!(store.running_count().unwrap(), 1);
        store.mark_lost(id);
        assert_eq!(db.status(id).as_deref(), Some("interrupted"));
        // a settled row is left alone
        store.finish(id, &Finished {
            status: "ok", text: "x".into(), error: None, usage: Default::default(), usage_error: None,
            shutting_down: false, started_at: now_stamp(), elapsed: Default::default(),
        });
        assert_eq!(db.status(id).as_deref(), Some("interrupted"));
    }

    #[test]
    fn outbox_ops_are_numbered_acked_and_keep_the_run_from_pruning() {
        let (d, db) = temp_db("out");
        let (id, _) = db.insert_run("k", "claude", "", "{}", Path::new("/tmp"), 60).unwrap();
        let store = ServeStore::new(db.clone(), Arc::new(AtomicI64::new(1)), d.path().join("runs"));
        db.mark_queued(id);
        {
            let conn = db.0.lock().unwrap();
            assert_eq!(append_outbox(&conn, id, "board_link", &json!({"a": 1})).unwrap(), format!("{id}:1"));
            assert_eq!(append_outbox(&conn, id, "notify", &json!({"message": "m"})).unwrap(), format!("{id}:2"));
        }
        assert!(store.mark_running(id).unwrap());
        store.finish(id, &Finished {
            status: "ok", text: String::new(), error: None, usage: Default::default(), usage_error: None,
            shutting_down: false, started_at: now_stamp(), elapsed: Default::default(),
        });
        store.prune(0);
        assert!(db.get_run(id).unwrap().is_some(), "unacked ops keep the run");
        let ops = db.outbox_after(id, 0).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[1].op_type, "notify");
        assert_eq!(db.outbox_after(id, 1).unwrap().len(), 1);
        assert_eq!(db.ack_outbox(id, 1).unwrap(), 1);
        assert_eq!(db.outbox_after(id, 0).unwrap().len(), 1);
        assert_eq!(db.ack_outbox(id, 2).unwrap(), 1);
        store.prune(0);
        assert!(db.get_run(id).unwrap().is_none(), "acked runs are pruned");
    }
}
