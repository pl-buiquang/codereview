//! Applying a remote run's outbox locally.
//!
//! `cr` inside a remote run cannot touch the app's database; it appends structured ops (board
//! metadata, links, focus, notifications) to the run's outbox on the host. While mirroring, and on
//! reconnect, the app fetches the unacknowledged ops, applies each one at most once (the op id is the
//! key of `remote_applied_op`) and acknowledges what it settled. If the acknowledgement is lost, the
//! host sends the same ops again and the table makes the second application a no-op.

use std::sync::MutexGuard;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::client::{OutboxOp, RemoteClient};
use crate::error::{AppError, AppResult};

/// How often a failing op is retried (on later syncs) before it is given up on and acknowledged.
pub const MAX_ATTEMPTS: i64 = 3;

/// What applying an op needs from the app.
pub trait OpApplier {
    fn conn(&self) -> MutexGuard<'_, Connection>;
    /// Apply one op locally. Called at most once per successful op.
    fn apply(&self, run_id: i64, op: &OutboxOp) -> AppResult<()>;
}

/// Whether the op needs no further attempt: applied, or failed too often.
fn settled(conn: &Connection, run_id: i64, op_id: &str) -> bool {
    conn.query_row(
        "SELECT error IS NULL OR attempts >= ?3 FROM remote_applied_op WHERE run_id = ?1 AND op_id = ?2",
        params![run_id, op_id, MAX_ATTEMPTS],
        |r| r.get::<_, bool>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(false)
}

fn record(conn: &Connection, run_id: i64, op_id: &str, result: &AppResult<()>) {
    let now = chrono::Utc::now().to_rfc3339();
    let _ = match result {
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
}

/// Fetch, apply and acknowledge the pending ops of a run. Never fails the run: a host that cannot be
/// reached just leaves the ops for the next sync.
pub fn sync(applier: &dyn OpApplier, run_id: i64, remote_run_id: i64, client: &RemoteClient) {
    let ops = match client.outbox(remote_run_id, 0) {
        Ok(ops) => ops,
        Err(e) => {
            log::debug!(target: "remote", "run {run_id}: cannot read the outbox: {e}");
            return;
        }
    };
    // The highest seq up to which every op is settled; the host forgets those on the ack.
    let mut ack_upto = 0;
    let mut contiguous = true;
    for op in &ops {
        let done = settled(&applier.conn(), run_id, &op.op_id);
        if !done {
            let result = applier.apply(run_id, op);
            if let Err(e) = &result {
                log::warn!(target: "remote", "run {run_id}: op {} ({}) failed: {e}", op.op_id, op.op_type);
            }
            record(&applier.conn(), run_id, &op.op_id, &result);
        }
        if contiguous && settled(&applier.conn(), run_id, &op.op_id) {
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
        sync(&fake, run, id, &c);
        assert_eq!(*fake.applied.borrow(), [format!("{id}:1"), format!("{id}:2"), format!("{id}:3")]);
        assert_eq!(pending(&c, id), 0, "acknowledged");
        // a second sync finds nothing and applies nothing
        sync(&fake, run, id, &c);
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
        sync(&fake, run, id, &c);
        assert_eq!(fake.applied.borrow().len(), 2);
        proxy.set_down(false);
        assert_eq!(pending(&proxy.client().unwrap(), id), 2, "the host still has them");
        // reconnect: the same ops come again, none is applied again, then they are acknowledged
        *fake.on_apply.borrow_mut() = None;
        sync(&fake, run, id, &proxy.client().unwrap());
        assert_eq!(fake.applied.borrow().len(), 2);
        assert_eq!(pending(&proxy.client().unwrap(), id), 0);
    }

    #[test]
    fn a_failing_op_is_retried_then_given_up_without_blocking_the_others() {
        let srv = TestServer::start(1);
        let proxy = Proxy::start(&srv);
        let c = proxy.client().unwrap();
        let id = remote_run(&srv, &[("board_link", serde_json::json!({})), ("notify", serde_json::json!({"message": "ok"}))]);
        let (fake, run) = Fake::new();
        *fake.fail.borrow_mut() = vec!["board_link".into()];
        sync(&fake, run, id, &c);
        assert_eq!(*fake.applied.borrow(), [format!("{id}:2")], "the later op still goes through");
        assert_eq!(pending(&c, id), 2, "nothing is acknowledged past the failed op");
        sync(&fake, run, id, &c);
        assert_eq!(pending(&c, id), 2);
        sync(&fake, run, id, &c); // third attempt: given up
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
    fn an_unreachable_host_leaves_the_ops_alone() {
        let (fake, run) = Fake::new();
        let c = RemoteClient::new(crate::remote::tunnel::Addr::Unix("/nonexistent/s.sock".into()));
        sync(&fake, run, 1, &c);
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
