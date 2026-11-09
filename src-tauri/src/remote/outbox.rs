//! Applying a remote run's outbox locally.
//!
//! `cr` inside a remote run cannot touch the app's database; it appends structured ops (board
//! metadata, links, focus, notifications) to the run's outbox on the host. While mirroring, and on
//! reconnect, the app fetches the unacknowledged ops, applies each one at most once (the op id is the
//! key of `remote_applied_op`) and acknowledges what it settled. If the acknowledgement is lost, the
//! host sends the same ops again and the table makes the second application a no-op.

use std::sync::MutexGuard;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::client::{OutboxOp, RemoteClient};
use crate::error::{AppError, AppResult};

/// How many times a failing op is tried before it is given up on.
pub const MAX_ATTEMPTS: i64 = 3;

/// A failed op is tried again at most this often. Syncs run about once a second while events flow, so
/// counting every one would burn the attempts inside a short network blip (a `gh` outage of a few seconds).
pub const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What applying an op needs from the app.
pub trait OpApplier {
    fn conn(&self) -> MutexGuard<'_, Connection>;
    /// Apply one op locally. Called at most once per successful op.
    fn apply(&self, run_id: i64, op: &OutboxOp) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// Never tried.
    New,
    Applied,
    /// Failed, and it is time to try again (or it is not yet time: `wait`).
    Failing { due: bool },
    GivenUp,
}

fn progress(conn: &Connection, run_id: i64, op_id: &str, retry_after: Duration) -> Progress {
    let row = conn
        .query_row(
            "SELECT error IS NULL, attempts, applied_at FROM remote_applied_op WHERE run_id = ?1 AND op_id = ?2",
            params![run_id, op_id],
            |r| Ok((r.get::<_, bool>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
        )
        .optional();
    match row {
        Ok(None) => Progress::New,
        Ok(Some((true, _, _))) => Progress::Applied,
        Ok(Some((false, attempts, _))) if attempts >= MAX_ATTEMPTS => Progress::GivenUp,
        Ok(Some((false, _, at))) => {
            let last = chrono::DateTime::parse_from_rfc3339(&at).map(|t| t.with_timezone(&chrono::Utc)).unwrap_or_default();
            let due = (chrono::Utc::now() - last).to_std().map_or(true, |age| age >= retry_after);
            Progress::Failing { due }
        }
        Err(e) => {
            // Treating an unreadable row as "new" would apply the op again: say so loudly and wait.
            log::error!(target: "remote", "run {run_id}: cannot read the state of op {op_id}: {e}");
            Progress::Failing { due: false }
        }
    }
}

fn record(conn: &Connection, run_id: i64, op_id: &str, result: &AppResult<()>) {
    let now = chrono::Utc::now().to_rfc3339();
    let written = match result {
        Ok(()) => conn.execute(
            "INSERT INTO remote_applied_op (run_id, op_id, applied_at, error, attempts) VALUES (?1, ?2, ?3, NULL, 1)
             ON CONFLICT(run_id, op_id) DO UPDATE SET error = NULL, applied_at = excluded.applied_at",
            params![run_id, op_id, now],
        ),
        Err(e) => conn.execute(
            "INSERT INTO remote_applied_op (run_id, op_id, applied_at, error, attempts) VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT(run_id, op_id) DO UPDATE SET error = excluded.error, applied_at = excluded.applied_at, attempts = attempts + 1",
            params![run_id, op_id, now, e.to_string()],
        ),
    };
    if let Err(e) = written {
        // Applied but not recorded: the op may be applied again on the next sync (at-least-once).
        log::error!(target: "remote", "run {run_id}: cannot record op {op_id}: {e}");
    }
}

/// Fetch, apply and acknowledge the pending ops of a run. Never fails the run: a host that cannot be
/// reached just leaves the ops for the next sync. Returns the ops given up on (after [`MAX_ATTEMPTS`]
/// failures spread over at least [`RETRY_AFTER`] each); they are acknowledged only when `terminal`
/// (the run is over), so the host keeps them while there is still a chance to apply them.
pub fn sync(applier: &dyn OpApplier, run_id: i64, remote_run_id: i64, client: &RemoteClient, terminal: bool) -> Vec<String> {
    sync_with(applier, run_id, remote_run_id, client, terminal, RETRY_AFTER)
}

pub fn sync_with(applier: &dyn OpApplier, run_id: i64, remote_run_id: i64, client: &RemoteClient, terminal: bool, retry_after: Duration) -> Vec<String> {
    let ops = match client.outbox(remote_run_id, 0) {
        Ok(ops) => ops,
        Err(e) => {
            log::debug!(target: "remote", "run {run_id}: cannot read the outbox: {e}");
            return Vec::new();
        }
    };
    // The highest seq up to which every op is settled; the host forgets those on the ack.
    let mut ack_upto = 0;
    let mut contiguous = true;
    let mut dropped = Vec::new();
    for op in &ops {
        let mut state = progress(&applier.conn(), run_id, &op.op_id, retry_after);
        if matches!(state, Progress::New | Progress::Failing { due: true }) {
            let result = applier.apply(run_id, op);
            if let Err(e) = &result {
                log::warn!(target: "remote", "run {run_id}: op {} ({}) failed: {e}", op.op_id, op.op_type);
            }
            record(&applier.conn(), run_id, &op.op_id, &result);
            state = progress(&applier.conn(), run_id, &op.op_id, retry_after);
        }
        if state == Progress::GivenUp {
            let why = format!("{} ({})", op.op_type, op.op_id);
            log::error!(target: "remote", "run {run_id}: giving up on op {why}");
            dropped.push(why);
        }
        let settled = state == Progress::Applied || (terminal && state == Progress::GivenUp);
        if contiguous && settled {
            ack_upto = op.seq;
        } else {
            contiguous = false;
        }
    }
    if ack_upto > 0 {
        if let Err(e) = client.ack(remote_run_id, ack_upto) {
            log::debug!(target: "remote", "run {run_id}: cannot acknowledge the outbox: {e}");
        }
    }
    dropped
}

// ---- the ops themselves ----------------------------------------------------------------------

/// Arguments of an op, with readable errors.
pub struct Args<'a>(pub &'a Value);

impl Args<'_> {
    pub fn str(&self, key: &str) -> AppResult<&str> {
        self.0.get(key).and_then(Value::as_str).ok_or_else(|| AppError::Other(format!("op argument `{key}` is missing")))
    }

    pub fn opt_str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
    }

    pub fn int(&self, key: &str) -> AppResult<i64> {
        self.0.get(key).and_then(Value::as_i64).ok_or_else(|| AppError::Other(format!("op argument `{key}` is missing")))
    }

    pub fn flag(&self, key: &str) -> bool {
        self.0.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
}

#[cfg(all(test, feature = "serve", unix))]
mod tests {
    use super::*;
    use crate::serve::store::append_outbox;
    use std::cell::RefCell;
    use std::sync::Mutex;

    type ApplyHook = Box<dyn Fn(&OutboxOp)>;

    struct Fake {
        conn: Mutex<Connection>,
        applied: RefCell<Vec<String>>,
        /// Ops whose application fails, by op type.
        fail: RefCell<Vec<String>>,
        on_apply: RefCell<Option<ApplyHook>>,
    }

    impl Fake {
        fn new() -> (Self, i64) {
            let conn = crate::db::open_memory();
            crate::routines::seed(&conn).unwrap();
            let rid = crate::routines::list(&conn).unwrap()[0].id;
            conn.execute(
                "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at) VALUES (?1, 'pr', 'a', '{}', 'running', 'n')",
                params![rid],
            )
            .unwrap();
            let run = conn.last_insert_rowid();
            (Self { conn: Mutex::new(conn), applied: RefCell::default(), fail: RefCell::default(), on_apply: RefCell::default() }, run)
        }
    }

    impl OpApplier for Fake {
        fn conn(&self) -> MutexGuard<'_, Connection> {
            self.conn.lock().unwrap()
        }
        fn apply(&self, _: i64, op: &OutboxOp) -> AppResult<()> {
            if let Some(f) = self.on_apply.borrow().as_ref() {
                f(op);
            }
            if self.fail.borrow().contains(&op.op_type) {
                return Err(AppError::Other("boom".into()));
            }
            self.applied.borrow_mut().push(op.op_id.clone());
            Ok(())
        }
    }

    use crate::remote::test_server::{Proxy, TestServer};

    fn remote_run(srv: &TestServer, ops: &[(&str, Value)]) -> i64 {
        let req = serde_json::from_value(serde_json::json!({
            "idempotency_key": "k", "harness": "fake", "model": "sleep 30", "prompt": "go",
            "workspace": { "repo": "acme/widgets", "kind": "main-clone" }
        }))
        .unwrap();
        let id = RemoteClient::new(crate::remote::tunnel::Addr::Unix(match &srv.addr {
            crate::remote::tunnel::Addr::Unix(p) => p.clone(),
            _ => unreachable!(),
        }))
        .create_run(&req)
        .unwrap()
        .id;
        let conn = crate::serve::store::open(&srv.state.serve_db_path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        for (t, a) in ops {
            append_outbox(&conn, id, t, a).unwrap();
        }
        id
    }

    fn pending(c: &RemoteClient, id: i64) -> usize {
        c.outbox(id, 0).unwrap().len()
    }

    #[test]
    fn ops_are_applied_once_in_order_and_acknowledged() {
        let srv = TestServer::start(1);
        let proxy = Proxy::start(&srv);
        let c = proxy.client().unwrap();
        let id = remote_run(&srv, &[("notify", serde_json::json!({"message": "a"})), ("board_track", serde_json::json!({"reference": "o/n#1"})), ("notify", serde_json::json!({"message": "b"}))]);
        let (fake, run) = Fake::new();
        sync(&fake, run, id, &c, false);
        assert_eq!(*fake.applied.borrow(), [format!("{id}:1"), format!("{id}:2"), format!("{id}:3")]);
        assert_eq!(pending(&c, id), 0, "acknowledged");
        // a second sync finds nothing and applies nothing
        sync(&fake, run, id, &c, false);
        assert_eq!(fake.applied.borrow().len(), 3);
    }

    #[test]
    fn ops_sent_again_after_a_lost_ack_are_not_applied_twice() {
        let srv = TestServer::start(1);
        let proxy = Proxy::start(&srv);
        let c = proxy.client().unwrap();
        let id = remote_run(&srv, &[("notify", serde_json::json!({"message": "a"})), ("notify", serde_json::json!({"message": "b"}))]);
        let (fake, run) = Fake::new();
        // the connection dies right after the last op is applied, so the acknowledgement is lost
        let p = proxy.clone();
        *fake.on_apply.borrow_mut() = Some(Box::new(move |op| {
            if op.seq == 2 {
                p.set_down(true);
                p.cut();
            }
        }));
        sync(&fake, run, id, &c, false);
        assert_eq!(fake.applied.borrow().len(), 2);
        proxy.set_down(false);
        assert_eq!(pending(&proxy.client().unwrap(), id), 2, "the host still has them");
        // reconnect: the same ops come again, none is applied again, then they are acknowledged
        *fake.on_apply.borrow_mut() = None;
        sync(&fake, run, id, &proxy.client().unwrap(), false);
        assert_eq!(fake.applied.borrow().len(), 2);
        assert_eq!(pending(&proxy.client().unwrap(), id), 0);
    }

    #[test]
    fn a_failing_op_is_retried_on_a_schedule_and_only_dropped_when_the_run_is_over() {
        let srv = TestServer::start(1);
        let proxy = Proxy::start(&srv);
        let c = proxy.client().unwrap();
        let id = remote_run(&srv, &[("board_link", serde_json::json!({})), ("notify", serde_json::json!({"message": "ok"}))]);
        let (fake, run) = Fake::new();
        *fake.fail.borrow_mut() = vec!["board_link".into()];
        let now = Duration::ZERO;
        sync_with(&fake, run, id, &c, false, now);
        assert_eq!(*fake.applied.borrow(), [format!("{id}:2")], "the later op still goes through");
        assert_eq!(pending(&c, id), 2, "nothing is acknowledged past the failed op");
        sync_with(&fake, run, id, &c, false, now);
        // third attempt: given up, but a running run keeps it on the host
        let dropped = sync_with(&fake, run, id, &c, false, now);
        assert_eq!(dropped, [format!("board_link ({id}:1)")]);
        assert_eq!(pending(&c, id), 2, "a given-up op is not acknowledged while the run goes on");
        // the run ends: it is acknowledged and reported
        let dropped = sync_with(&fake, run, id, &c, true, now);
        assert_eq!(dropped.len(), 1);
        assert_eq!(pending(&c, id), 0);
        let (err, attempts): (String, i64) = fake
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT error, attempts FROM remote_applied_op WHERE op_id = ?1", params![format!("{id}:1")], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((err.as_str(), attempts), ("boom", 3));
        assert_eq!(fake.applied.borrow().len(), 1, "the good op was applied once");
    }

    #[test]
    fn syncs_inside_the_retry_window_do_not_burn_attempts() {
        let srv = TestServer::start(1);
        let proxy = Proxy::start(&srv);
        let c = proxy.client().unwrap();
        let id = remote_run(&srv, &[("board_link", serde_json::json!({}))]);
        let (fake, run) = Fake::new();
        *fake.fail.borrow_mut() = vec!["board_link".into()];
        // a network blip: syncs every moment, but the op is retried at most once per window
        for _ in 0..10 {
            sync(&fake, run, id, &c, false);
        }
        let attempts: i64 = fake.conn.lock().unwrap().query_row("SELECT attempts FROM remote_applied_op", [], |r| r.get(0)).unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(pending(&c, id), 1);
        // when the blip is over the op goes through
        fake.fail.borrow_mut().clear();
        fake.conn.lock().unwrap().execute("UPDATE remote_applied_op SET applied_at = '2000-01-01T00:00:00Z'", []).unwrap();
        sync(&fake, run, id, &c, false);
        assert_eq!(fake.applied.borrow().len(), 1);
        assert_eq!(pending(&c, id), 0);
    }

    #[test]
    fn an_unreachable_host_leaves_the_ops_alone() {
        let (fake, run) = Fake::new();
        let c = RemoteClient::new(crate::remote::tunnel::Addr::Unix("/nonexistent/s.sock".into()));
        sync(&fake, run, 1, &c, false);
        assert!(fake.applied.borrow().is_empty());
    }

    #[test]
    fn args_report_what_is_missing() {
        let v = serde_json::json!({"a": "x", "n": 3, "empty": "", "f": true});
        let a = Args(&v);
        assert_eq!((a.str("a").unwrap(), a.int("n").unwrap(), a.opt_str("empty"), a.flag("f"), a.flag("zz")), ("x", 3, None, true, false));
        assert!(a.str("zz").unwrap_err().to_string().contains("`zz` is missing"));
    }
}
