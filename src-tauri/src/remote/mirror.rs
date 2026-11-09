//! The mirror: keeps the local copy of a remote routine run in step with the host.
//!
//! The remote store is authoritative while a run is live. The app owns a `routine_run` row (with
//! `host_id` and `remote_run_id`) and copies the remote events into `routine_run_event` under the same
//! `(run, seq)` key; `last_remote_seq` records how far it got, so after a dropped connection (or an
//! app restart, or a run that finished while the app was closed) the stream resumes `after_seq`.
//! Duplicates are harmless: an event already stored is ignored and not announced twice.
//!
//! This module has no Tauri dependency: the app plugs in [`MirrorHooks`] (events to the webview, the
//! finish handling, the outbox) and the tests plug in a fake.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::client::{is_terminal, RemoteClient, StatusEvent, StreamEvent, StreamItem};
use super::error::RemoteError;
use crate::run_engine::Finished;
use crate::run_usage::{now_stamp, RunUsage};

/// How the mirror talks to the rest of the app.
pub trait MirrorHooks: Send + Sync {
    /// The app database.
    fn db(&self) -> MutexGuard<'_, Connection>;
    /// A client for the host (opening its tunnel when needed).
    fn client(&self, host_id: i64) -> Result<RemoteClient, RemoteError>;
    /// An event was stored locally for the first time.
    fn event(&self, _run_id: i64, _seq: i64, _kind: &str, _json: &str) {}
    fn status_changed(&self, _run_id: i64, _status: &str) {}
    /// The run ended and its row is settled.
    fn finished(&self, _run_id: i64, _fin: &Finished) {}
    /// Apply the run's pending outbox ops locally and acknowledge them (see [`super::outbox`]). `terminal`:
    /// the run is over. Returns the ops that were given up on.
    fn sync_outbox(&self, _run_id: i64, _remote_run_id: i64, _client: &RemoteClient, _terminal: bool) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct MirrorConfig {
    /// First wait after a lost connection; doubles up to `retry_max`.
    pub retry_base: Duration,
    pub retry_max: Duration,
    /// At most one outbox sync per this while events flow (one more at the end).
    pub outbox_every: Duration,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self { retry_base: Duration::from_secs(2), retry_max: Duration::from_secs(30), outbox_every: Duration::from_secs(1) }
    }
}

/// The local row of a mirrored run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorRun {
    pub host_id: i64,
    pub remote_run_id: Option<i64>,
    pub last_seq: i64,
    pub status: String,
    pub started_at: Option<String>,
}

pub fn load(conn: &Connection, run_id: i64) -> Option<MirrorRun> {
    conn.query_row(
        "SELECT host_id, remote_run_id, last_remote_seq, status, started_at FROM routine_run WHERE id = ?1 AND host_id IS NOT NULL",
        params![run_id],
        |r| Ok(MirrorRun { host_id: r.get(0)?, remote_run_id: r.get(1)?, last_seq: r.get(2)?, status: r.get(3)?, started_at: r.get(4)? }),
    )
    .optional()
    .ok()
    .flatten()
}

/// Store one remote event; false when it was already there.
pub fn store_event(conn: &Connection, run_id: i64, e: &StreamEvent) -> bool {
    // A line that is not JSON arrives as a JSON string: keep the raw text.
    let json = match &e.json {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let norm = (!e.norm.is_null()).then(|| e.norm.to_string());
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO routine_run_event (run_id, seq, ts, kind, json, norm) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run_id, e.seq, chrono::Utc::now().to_rfc3339(), e.kind, json, norm],
        )
        .unwrap_or(0)
        > 0;
    let _ = conn.execute(
        "UPDATE routine_run SET last_remote_seq = MAX(last_remote_seq, ?2) WHERE id = ?1",
        params![run_id, e.seq],
    );
    inserted
}

/// The usage a remote status reports (`usage` is the daemon's `usage_json`).
pub fn usage_of(v: &Option<Value>) -> RunUsage {
    let Some(v) = v.as_ref().filter(|v| v.is_object()) else { return RunUsage::default() };
    let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    RunUsage {
        num_turns: n("num_turns"),
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
        cache_read_tokens: n("cache_read_tokens"),
        cache_creation_tokens: n("cache_creation_tokens"),
        cost_usd: v.get("cost_usd").and_then(Value::as_f64).unwrap_or(0.0),
        session_id: v.get("session_id").and_then(Value::as_str).map(str::to_string),
        models: Vec::new(),
    }
}

fn static_status(s: &str) -> &'static str {
    match s {
        "ok" => "ok",
        "needs_attention" => "needs_attention",
        "timeout" => "timeout",
        "killed" => "killed",
        "interrupted" => "interrupted",
        _ => "error",
    }
}

/// Settle the local row (only a `queued` / `running` one) and tell the hooks. Returns whether this
/// call settled it.
pub fn finish_local(h: &dyn MirrorHooks, run_id: i64, status: &str, text: &str, error: Option<&str>, usage: RunUsage) -> bool {
    let status = static_status(status);
    let started_at = {
        let conn = h.db();
        let changed = conn
            .execute(
                "UPDATE routine_run SET status = ?2, ended_at = ?3, result_text = ?4, error = ?5, cost_usd = ?6, num_turns = ?7,
                        session_id = COALESCE(?8, session_id)
                 WHERE id = ?1 AND status IN ('queued', 'running')",
                params![run_id, status, chrono::Utc::now().to_rfc3339(), (!text.is_empty()).then_some(text), error, usage.cost_usd, usage.num_turns, usage.session_id],
            )
            .unwrap_or(0);
        if changed == 0 {
            return false;
        }
        conn.query_row("SELECT started_at FROM routine_run WHERE id = ?1", params![run_id], |r| r.get::<_, Option<String>>(0)).ok().flatten()
    };
    let elapsed = started_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).to_std().unwrap_or_default())
        .unwrap_or_default();
    let fin = Finished {
        status,
        text: text.to_string(),
        error: error.map(str::to_string),
        usage,
        usage_error: (status != "ok" && status != "needs_attention").then(|| error.unwrap_or(status).to_string()),
        shutting_down: false,
        started_at: started_at.unwrap_or_else(now_stamp),
        elapsed,
    };
    h.finished(run_id, &fin);
    true
}

fn finish_status(h: &dyn MirrorHooks, run_id: i64, s: &StatusEvent) -> bool {
    finish_local(h, run_id, &s.status, s.text.as_deref().unwrap_or(""), s.error.as_deref(), usage_of(&s.usage))
}

enum Step {
    /// The run is settled (or was already).
    Done,
    /// Lost the connection or the stream: try again. `progress` says whether anything arrived.
    Retry { why: String, progress: bool },
}

/// What [`follow`] ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The run reached a terminal state.
    Settled,
    /// `stop` was raised: the run stays as it is (the app is going away or the run was given up).
    Abandoned,
}

fn sleep_unless_stopped(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10).min(d));
    }
}

/// Follow run `run_id` until it is over: resume from `last_remote_seq`, reconnect with a backoff when
/// the connection drops, finalize when the host says it is terminal (or that it never heard of it).
pub fn follow(h: &dyn MirrorHooks, cfg: &MirrorConfig, run_id: i64, stop: &AtomicBool) -> Followed {
    let mut failures = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Followed::Abandoned;
        }
        let Some(run) = load(&h.db(), run_id) else { return Followed::Settled };
        if !matches!(run.status.as_str(), "queued" | "running") {
            return Followed::Settled;
        }
        let Some(remote_id) = run.remote_run_id else {
            finish_local(h, run_id, "error", "", Some("the run was never created on the host"), RunUsage::default());
            return Followed::Settled;
        };
        match step(h, cfg, run_id, &run, remote_id, stop) {
            Step::Done => return Followed::Settled,
            Step::Retry { why, progress } => {
                failures = if progress { 1 } else { failures.saturating_add(1) };
                let wait = super::tunnel::backoff(failures, cfg.retry_base, cfg.retry_max);
                log::info!(target: "remote", "run {run_id}: {why}; retrying in {}ms", wait.as_millis());
                sleep_unless_stopped(stop, wait);
            }
        }
    }
}

fn step(h: &dyn MirrorHooks, cfg: &MirrorConfig, run_id: i64, run: &MirrorRun, remote_id: i64, stop: &AtomicBool) -> Step {
    let retry = |e: RemoteError, progress| Step::Retry { why: e.to_string(), progress };
    let client = match h.client(run.host_id) {
        Ok(c) => c,
        Err(e) => return retry(e, false),
    };
    // Reconcile first: is the run still there, and where is it?
    match client.get_run(remote_id) {
        Ok(None) => {
            finish_local(
                h,
                run_id,
                "error",
                "",
                Some("run_not_found: the host does not know this run any more (it was pruned, or the host's data was reset)"),
                RunUsage::default(),
            );
            return Step::Done;
        }
        Ok(Some(rv)) => log::debug!(target: "remote", "run {run_id}: remote run {remote_id} is {}", rv.status),
        Err(e) => return retry(e, false),
    }
    // A connection-level resume point: what is stored locally, not what the row says (the two can
    // differ after a crash between the two writes; the insert ignores duplicates either way).
    let after = {
        let conn = h.db();
        let stored: i64 = conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM routine_run_event WHERE run_id = ?1", params![run_id], |r| r.get(0))
            .unwrap_or(0);
        stored.max(run.last_seq)
    };
    let mut stream = match client.events(remote_id, after) {
        Ok(s) => s,
        Err(e) if e.code == "not_found" => {
            finish_local(h, run_id, "error", "", Some("run_not_found: the host does not know this run any more"), RunUsage::default());
            return Step::Done;
        }
        Err(e) => return retry(e, false),
    };
    let mut progress = false;
    let mut last_sync = Instant::now() - cfg.outbox_every;
    // Ops left from before a restart or a drop.
    h.sync_outbox(run_id, remote_id, &client, false);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Step::Retry { why: "stopped".into(), progress };
        }
        match stream.next_item() {
            Ok(Some(StreamItem::Event(e))) => {
                progress = true;
                let (stored, kind, json) = {
                    let conn = h.db();
                    let json = match &e.json {
                        Value::String(s) => s.clone(),
                        v => v.to_string(),
                    };
                    (store_event(&conn, run_id, &e), e.kind.clone(), json)
                };
                if stored {
                    h.event(run_id, e.seq, &kind, &json);
                }
                if last_sync.elapsed() >= cfg.outbox_every {
                    last_sync = Instant::now();
                    h.sync_outbox(run_id, remote_id, &client, false);
                }
            }
            Ok(Some(StreamItem::Status(s))) => {
                // Ops first, so a `notify` fires before the "finished" notification and a board link
                // exists when the run is reported done.
                let terminal = is_terminal(&s.status);
                let dropped = h.sync_outbox(run_id, remote_id, &client, terminal);
                if terminal {
                    let mut s = s;
                    if !dropped.is_empty() {
                        // Never silent: the run's error says which board/focus writes did not make it.
                        let note = format!("{} op(s) from the remote run could not be applied here: {}", dropped.len(), dropped.join(", "));
                        s.error = Some(match s.error.take() {
                            Some(e) if !e.is_empty() => format!("{e}\n{note}"),
                            _ => note,
                        });
                    }
                    if finish_status(h, run_id, &s) {
                        h.status_changed(run_id, static_status(&s.status));
                    }
                    return Step::Done;
                }
            }
            Ok(None) => {
                // Closed without a terminal status: the daemon restarted or the tunnel flapped.
                return Step::Retry { why: "the event stream ended before the run did".into(), progress };
            }
            Err(e) => return retry(e, progress),
        }
    }
}

#[cfg(all(test, feature = "serve", unix))]
mod tests {
    use super::*;
    use crate::remote::test_server::{Proxy, TestServer};
    use crate::serve::request::RunRequest;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    pub struct TestHooks {
        pub conn: Mutex<Connection>,
        pub proxy: Arc<Proxy>,
        pub events: Mutex<Vec<(i64, i64)>>,
        pub finished: Mutex<Vec<(String, String, Option<String>)>>,
        pub syncs: AtomicUsize,
        pub dropped: Mutex<Vec<String>>,
    }

    impl MirrorHooks for TestHooks {
        fn db(&self) -> MutexGuard<'_, Connection> {
            self.conn.lock().unwrap()
        }
        fn client(&self, _: i64) -> Result<RemoteClient, RemoteError> {
            self.proxy.client()
        }
        fn event(&self, run_id: i64, seq: i64, _: &str, _: &str) {
            self.events.lock().unwrap().push((run_id, seq));
        }
        fn finished(&self, _: i64, fin: &Finished) {
            self.finished.lock().unwrap().push((fin.status.to_string(), fin.text.clone(), fin.error.clone()));
        }
        fn sync_outbox(&self, _: i64, _: i64, _: &RemoteClient, _: bool) -> Vec<String> {
            self.syncs.fetch_add(1, Ordering::Relaxed);
            self.dropped.lock().unwrap().clone()
        }
    }

    fn fast() -> MirrorConfig {
        MirrorConfig { retry_base: Duration::from_millis(20), retry_max: Duration::from_millis(80), outbox_every: Duration::from_millis(0) }
    }

    fn request(key: &str, script: &str) -> RunRequest {
        serde_json::from_value(serde_json::json!({
            "idempotency_key": key, "harness": "fake", "model": script, "prompt": "go",
            "workspace": { "repo": "acme/widgets", "kind": "main-clone" }
        }))
        .unwrap()
    }

    /// A local run row mirroring a fresh remote run made from `script`.
    fn setup(script: &str) -> (TestServer, Arc<TestHooks>, i64, i64) {
        let srv = TestServer::start(2);
        let proxy = Proxy::start(&srv);
        let conn = crate::db::open_memory();
        crate::routines::seed(&conn).unwrap();
        let rid = crate::routines::list(&conn).unwrap()[0].id;
        conn.execute("INSERT INTO host (name, ssh_target, remote_endpoint, created_at, updated_at) VALUES ('h', 'h', '1', 'n', 'n')", []).unwrap();
        let created = proxy.client().unwrap().create_run(&request("k", script)).unwrap();
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at, started_at, host_id, remote_run_id)
             VALUES (?1, 'pr', 'a', '{}', 'running', 'n', ?2, 1, ?3)",
            params![rid, chrono::Utc::now().to_rfc3339(), created.id],
        )
        .unwrap();
        let run_id = conn.last_insert_rowid();
        let hooks = Arc::new(TestHooks { conn: Mutex::new(conn), proxy, events: Mutex::new(Vec::new()), finished: Mutex::new(Vec::new()), syncs: AtomicUsize::new(0), dropped: Mutex::new(Vec::new()) });
        (srv, hooks, run_id, created.id)
    }

    fn stored_seqs(h: &TestHooks, run_id: i64) -> Vec<i64> {
        let conn = h.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT seq FROM routine_run_event WHERE run_id = ?1 ORDER BY seq").unwrap();
        stmt.query_map(params![run_id], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    fn status_of(h: &TestHooks, run_id: i64) -> (String, Option<String>, Option<String>) {
        h.conn
            .lock()
            .unwrap()
            .query_row("SELECT status, result_text, error FROM routine_run WHERE id = ?1", params![run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
    }

    const QUICK: &str = r#"echo '{"type":"a"}'; echo '{"type":"b"}'; echo '{"type":"result","result":"all done"}'"#;
    const SLOW: &str = r#"for i in 1 2 3 4 5 6 7 8; do echo "{\"type\":\"a\",\"n\":$i}"; sleep 0.15; done; echo '{"type":"result","result":"slow done"}'"#;

    #[test]
    fn events_arrive_in_order_and_the_terminal_status_is_applied() {
        let (_srv, h, run_id, _) = setup(QUICK);
        let stop = AtomicBool::new(false);
        assert_eq!(follow(&*h, &fast(), run_id, &stop), Followed::Settled);
        assert_eq!(stored_seqs(&h, run_id), [1, 2, 3]);
        assert_eq!(h.events.lock().unwrap().iter().map(|e| e.1).collect::<Vec<_>>(), [1, 2, 3]);
        let (status, text, error) = status_of(&h, run_id);
        assert_eq!((status.as_str(), text.as_deref(), error), ("ok", Some("all done"), None));
        assert_eq!(h.finished.lock().unwrap().len(), 1);
        assert!(h.syncs.load(Ordering::Relaxed) >= 2, "the outbox is synced while mirroring and at the end");
        // the normalized form came from the remote
        let norm: Option<String> = h.conn.lock().unwrap().query_row("SELECT norm FROM routine_run_event WHERE seq = 3", [], |r| r.get(0)).unwrap();
        assert!(norm.unwrap().contains("\"final\""));
        let last: i64 = h.conn.lock().unwrap().query_row("SELECT last_remote_seq FROM routine_run", [], |r| r.get(0)).unwrap();
        assert_eq!(last, 3);
    }

    #[test]
    fn a_dropped_connection_resumes_after_the_last_seq_without_gaps_or_duplicates() {
        let (_srv, h, run_id, _) = setup(SLOW);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (h, stop) = (h.clone(), stop.clone());
            std::thread::spawn(move || follow(&*h, &fast(), run_id, &stop))
        };
        // let a few events through, then cut the stream and refuse connections for a while
        for _ in 0..200 {
            if h.events.lock().unwrap().len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        h.proxy.set_down(true);
        h.proxy.cut();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(status_of(&h, run_id).0, "running", "a lost connection does not end the run");
        h.proxy.set_down(false);
        assert_eq!(worker.join().unwrap(), Followed::Settled);
        assert_eq!(stored_seqs(&h, run_id), (1..=9).collect::<Vec<i64>>());
        let announced: Vec<i64> = h.events.lock().unwrap().iter().map(|e| e.1).collect();
        assert_eq!(announced, (1..=9).collect::<Vec<i64>>(), "each event is announced once, in order");
        assert_eq!(status_of(&h, run_id).1.as_deref(), Some("slow done"));
    }

    #[test]
    fn events_the_row_forgot_are_replayed_and_ignored() {
        let (_srv, h, run_id, _) = setup(QUICK);
        let stop = AtomicBool::new(false);
        follow(&*h, &fast(), run_id, &stop);
        // pretend the app crashed before it recorded progress: the row is rewound, the events stay
        h.conn.lock().unwrap().execute("UPDATE routine_run SET status = 'running', last_remote_seq = 0, ended_at = NULL", []).unwrap();
        h.events.lock().unwrap().clear();
        h.finished.lock().unwrap().clear();
        follow(&*h, &fast(), run_id, &stop);
        assert_eq!(stored_seqs(&h, run_id), [1, 2, 3], "no duplicate rows");
        assert!(h.events.lock().unwrap().is_empty(), "nothing announced twice");
        assert_eq!(status_of(&h, run_id).0, "ok");
    }

    #[test]
    fn a_run_that_finished_while_the_app_was_closed_is_caught_up_and_finalized() {
        let (srv, h, run_id, remote_id) = setup(QUICK);
        // the run completes on the host with nobody watching
        for _ in 0..200 {
            if h.proxy.client().unwrap().get_run(remote_id).unwrap().unwrap().status == "ok" {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(srv.state.clone());
        assert!(stored_seqs(&h, run_id).is_empty());
        follow(&*h, &fast(), run_id, &AtomicBool::new(false));
        assert_eq!(stored_seqs(&h, run_id), [1, 2, 3]);
        assert_eq!(status_of(&h, run_id).0, "ok");
    }

    #[test]
    fn a_run_the_host_never_heard_of_becomes_an_explicit_error() {
        let (srv, h, run_id, remote_id) = setup("sleep 30");
        // the host pruned it (or its data was wiped)
        let conn = crate::serve::store::open(&srv.state.serve_db_path).unwrap();
        conn.execute("DELETE FROM serve_run WHERE id = ?1", params![remote_id]).unwrap();
        assert_eq!(follow(&*h, &fast(), run_id, &AtomicBool::new(false)), Followed::Settled);
        let (status, _, error) = status_of(&h, run_id);
        assert_eq!(status, "error");
        assert!(error.unwrap().contains("does not know this run"), "explicit message");
        assert_eq!(h.finished.lock().unwrap().len(), 1);
    }

    #[test]
    fn ops_given_up_on_are_reported_in_the_runs_error() {
        let (_srv, h, run_id, _) = setup(QUICK);
        *h.dropped.lock().unwrap() = vec!["board_link (1:1)".into()];
        follow(&*h, &fast(), run_id, &AtomicBool::new(false));
        let (status, _, error) = status_of(&h, run_id);
        assert_eq!(status, "ok");
        assert!(error.unwrap().contains("1 op(s) from the remote run could not be applied here: board_link (1:1)"));
    }

    #[test]
    fn a_stopped_remote_run_ends_killed() {
        let (_srv, h, run_id, remote_id) = setup("sleep 30");
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (h, stop) = (h.clone(), stop.clone());
            std::thread::spawn(move || follow(&*h, &fast(), run_id, &stop))
        };
        std::thread::sleep(Duration::from_millis(300));
        h.proxy.client().unwrap().stop(remote_id).unwrap();
        assert_eq!(worker.join().unwrap(), Followed::Settled);
        assert_eq!(status_of(&h, run_id).0, "killed");
    }

    #[test]
    fn an_unreachable_host_leaves_the_run_running_until_it_is_abandoned() {
        let (_srv, h, run_id, _) = setup("sleep 30");
        h.proxy.set_down(true);
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (h, stop) = (h.clone(), stop.clone());
            std::thread::spawn(move || follow(&*h, &fast(), run_id, &stop))
        };
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(status_of(&h, run_id).0, "running");
        stop.store(true, Ordering::Relaxed);
        assert_eq!(worker.join().unwrap(), Followed::Abandoned);
        assert_eq!(status_of(&h, run_id).0, "running", "abandoning does not settle the row");
    }

    #[test]
    fn usage_is_read_from_the_remote_status() {
        let u = usage_of(&Some(serde_json::json!({"num_turns": 3, "input_tokens": 10, "output_tokens": 20, "cost_usd": 0.25, "session_id": "s"})));
        assert_eq!((u.num_turns, u.input_tokens, u.output_tokens, u.cost_usd, u.session_id.as_deref()), (3, 10, 20, 0.25, Some("s")));
        assert_eq!(usage_of(&None).cost_usd, 0.0);
    }
}
