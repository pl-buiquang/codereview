//! The engine proper: queue, workers, stop / timeout, settle, shutdown and re-adoption.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::outcome::{
    file_has_result, final_status, outcome_from, read_stderr, Outcome, PROMPT_FILE, STDERR_FILE, STDOUT_FILE,
};
use super::proc::{process_alive, signal_group, terminate, SIGKILL, SIGTERM};
use super::tail::{event_json, tail_events, Tail};
use super::{EventSink, Finished, Prepared, RunStore, RunningRun};
use crate::ai::{Harness, StreamRun};
use crate::ai_usage;
use crate::error::{AppError, AppResult};

pub const DEFAULT_CONCURRENCY: i64 = 4;
/// Finished runs kept in the store.
pub const KEEP_RUNS: i64 = 200;
/// How often the ticker looks for runs past their deadline.
pub const TICK: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Inner {
    live: HashMap<i64, Live>,
    /// Stop requests (`killed` / `timeout`), consumed when the run finishes.
    stops: HashMap<i64, &'static str>,
    shutting_down: bool,
    /// Exiting with the runs left alive: workers stop tailing without finalizing and nothing is killed.
    detaching: bool,
}

struct Live {
    pgid: i32,
    deadline: Instant,
}

pub struct RunEngine {
    store: Arc<dyn RunStore>,
    sink: Arc<dyn EventSink>,
    harness: &'static dyn Harness,
    inner: Mutex<Inner>,
}

impl RunEngine {
    pub fn new(store: Arc<dyn RunStore>, sink: Arc<dyn EventSink>, harness: &'static dyn Harness) -> Arc<Self> {
        Arc::new(Self { store, sink, harness, inner: Mutex::new(Inner::default()) })
    }

    /// The harness used for what names none (the default of the host).
    pub fn harness(&self) -> &'static dyn Harness {
        self.harness
    }

    /// The harness a stored run was started on; an id this build doesn't know falls back to the default.
    fn harness_by_id(&self, id: &str) -> &'static dyn Harness {
        if id == self.harness.id() {
            return self.harness;
        }
        crate::ai::harness_by_id(id).unwrap_or(self.harness)
    }

    /// Spawn the timeout watcher. Call once.
    pub fn start_ticker(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(TICK);
            engine.expire_overdue();
        });
    }

    /// Stop every live run that is past its deadline.
    pub fn expire_overdue(&self) {
        let expired: Vec<i64> = {
            let inner = self.inner.lock().unwrap();
            inner
                .live
                .iter()
                .filter(|(id, l)| Instant::now() > l.deadline && !inner.stops.contains_key(*id))
                .map(|(id, _)| *id)
                .collect()
        };
        for id in expired {
            self.request_stop(id, "timeout");
        }
    }

    /// A run row was just queued by the host: announce it and start what fits.
    pub fn submit(self: &Arc<Self>, id: i64) {
        self.sink.queued(id);
        self.pump();
    }

    /// Start queued runs while below the concurrency cap.
    pub fn pump(self: &Arc<Self>) {
        let inner = self.inner.lock().unwrap();
        // Queued runs wait while AI is off; the host pumps again when it is turned on.
        if inner.shutting_down || !self.store.ai_enabled() {
            return;
        }
        loop {
            let next: AppResult<Option<(i64, bool)>> = (|| {
                if self.store.running_count()? >= self.store.max_concurrency() {
                    return Ok(None);
                }
                let Some(id) = self.store.next_queued()? else { return Ok(None) };
                Ok(Some((id, self.store.mark_running(id)?)))
            })();
            match next {
                Ok(Some((id, true))) => {
                    self.sink.started(id);
                    let engine = Arc::clone(self);
                    std::thread::spawn(move || engine.execute(id));
                }
                // Stopped between the two reads: look at the next one.
                Ok(Some((_, false))) => {}
                Ok(None) => break,
                Err(e) => {
                    log::warn!(target: "routines", "pump failed: {e}");
                    break;
                }
            }
        }
    }

    /// Stop a run: a queued one is dropped, a running one is terminated.
    pub fn stop(&self, run_id: i64) -> AppResult<()> {
        if self.store.drop_queued(run_id)? {
            self.sink.status_changed(run_id, "killed");
            self.sink.dropped(run_id);
            return Ok(());
        }
        if !self.store.is_running(run_id) {
            return Err(AppError::Other("this run is not active".into()));
        }
        self.request_stop(run_id, "killed");
        Ok(())
    }

    fn request_stop(&self, run_id: i64, reason: &'static str) {
        let pgid = {
            let mut inner = self.inner.lock().unwrap();
            inner.stops.entry(run_id).or_insert(reason);
            inner.live.get(&run_id).map(|l| l.pgid)
        };
        if let Some(pgid) = pgid {
            terminate(pgid);
        }
    }

    fn is_stopped(&self, run_id: i64) -> bool {
        self.inner.lock().unwrap().stops.contains_key(&run_id)
    }

    /// Runs a quit would interrupt: started ones plus those still queued.
    pub fn active_count(&self) -> usize {
        self.store.active_count().unwrap_or_else(|_| self.inner.lock().unwrap().live.len())
    }

    /// Exiting but the runs go on: workers stop tailing (leaving their rows `running` for the next
    /// start to adopt) and nothing is signalled. Runs that never started are dropped.
    pub fn detach_all(&self) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            inner.detaching = true;
        }
        let _ = self.store.kill_queued();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !self.inner.lock().unwrap().live.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Exiting: terminate every process group and settle the rows ourselves (the worker threads
    /// won't get to).
    pub fn kill_all(&self) {
        let pgids: Vec<i32> = {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            if inner.detaching {
                return;
            }
            inner.live.values().map(|l| l.pgid).collect()
        };
        let _ = self.store.kill_active();
        for pgid in &pgids {
            signal_group(*pgid, SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !self.inner.lock().unwrap().live.is_empty() {
            std::thread::sleep(Duration::from_millis(100));
        }
        for pgid in pgids {
            signal_group(pgid, SIGKILL);
        }
    }

    /// Startup: pick up the runs a previous session left alive and sweep the files of every other run.
    pub fn adopt_detached(self: &Arc<Self>) {
        let Ok(running) = self.store.running_runs() else { return };
        if let Ok(entries) = std::fs::read_dir(self.store.runs_root()) {
            for entry in entries.flatten() {
                let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<i64>().ok()) else { continue };
                if !running.iter().any(|r| r.id == id) {
                    self.discard_run_files(id);
                }
            }
        }
        for run in running {
            let engine = Arc::clone(self);
            std::thread::spawn(move || engine.adopt(run));
        }
    }

    fn run_dir(&self, run_id: i64) -> PathBuf {
        self.store.runs_root().join(run_id.to_string())
    }

    // -----------------------------------------------------------------------------------------
    // One run
    // -----------------------------------------------------------------------------------------

    fn execute(self: Arc<Self>, run_id: i64) {
        let started = Instant::now();
        let started_at = ai_usage::now_stamp();
        let outcome = self.run_process(run_id);
        if outcome.detached {
            return;
        }
        self.settle(run_id, &outcome, started, &started_at);
    }

    /// Finalize a finished run and start the next queued one.
    fn settle(self: &Arc<Self>, run_id: i64, outcome: &Outcome, started: Instant, started_at: &str) {
        self.finalize(run_id, outcome, started, started_at);
        self.pump();
    }

    fn run_process(&self, run_id: i64) -> Outcome {
        let prepared = match self.sink.prepare(run_id, &|| self.is_stopped(run_id)) {
            Ok(Some(p)) => p,
            Ok(None) => return Outcome::default(),
            Err(e) => return Outcome::failed(e),
        };
        self.launch(run_id, prepared)
    }

    fn launch(&self, run_id: i64, p: Prepared) -> Outcome {
        if !p.cleanup.is_empty() {
            self.store.set_cleanup(run_id, &p.cleanup);
        }
        let cleanup = || p.cleanup.iter().for_each(|d| drop(std::fs::remove_dir_all(d)));
        let dir = self.run_dir(run_id);
        let prompt_path = dir.join(PROMPT_FILE);
        let files = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&prompt_path, &p.system_prompt))
            .and_then(|_| Ok((File::create(dir.join(STDOUT_FILE))?, File::create(dir.join(STDERR_FILE))?)));
        let (stdout_file, stderr_file) = match files {
            Ok(f) => f,
            Err(e) => {
                cleanup();
                return Outcome::failed(format!("failed to prepare the run directory: {e}"));
            }
        };
        let harness = p.harness;
        let spawned = harness.spawn(StreamRun {
            req: p.request(&prompt_path),
            stdout: stdout_file,
            stderr: stderr_file,
            timeout: p.timeout,
        });
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&prompt_path);
                cleanup();
                return Outcome::failed(e.to_string());
            }
        };
        let pid = child.id();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.live.insert(run_id, Live { pgid: pid as i32, deadline: Instant::now() + p.timeout });
            if inner.stops.contains_key(&run_id) {
                drop(inner);
                terminate(pid as i32);
            }
        }
        self.store.set_process(run_id, pid, &p.cwd, &p.session_id);

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(p.prompt.as_bytes());
        }
        let tail = self.tail(harness, run_id, &dir.join(STDOUT_FILE), 0, &mut || matches!(child.try_wait(), Ok(None)));
        let result = match tail {
            Tail::Detached => {
                // Dropping `child` neither kills nor reaps it; the next start adopts the run.
                self.inner.lock().unwrap().live.remove(&run_id);
                return Outcome { detached: true, ..Outcome::default() };
            }
            Tail::Done(result) => result,
        };
        let status = child.wait().map(|s| s.to_string()).map_err(|e| e.to_string());
        self.inner.lock().unwrap().live.remove(&run_id);
        let _ = std::fs::remove_file(&prompt_path);
        cleanup();
        outcome_from(harness, result, read_stderr(&dir), status)
    }

    fn tail(&self, harness: &dyn Harness, run_id: i64, path: &std::path::Path, skip: i64, alive: &mut dyn FnMut() -> bool) -> Tail {
        tail_events(
            harness,
            path,
            skip,
            alive,
            &|| self.inner.lock().unwrap().detaching,
            &mut |seq, kind, line, norm| {
                let json = event_json(kind, line);
                self.store.append_event(run_id, seq, kind, &json, norm);
                self.sink.event(run_id, seq, kind, &json);
            },
        )
    }

    // -----------------------------------------------------------------------------------------
    // Re-adoption
    // -----------------------------------------------------------------------------------------

    /// Take over a run a previous session left alive: keep tailing it, then settle it like any other.
    fn adopt(self: Arc<Self>, run: RunningRun) {
        let started = Instant::now();
        let started_at = ai_usage::now_stamp();
        let id = run.id;
        let Some(timeout) = self.sink.adopt(id) else { return self.settle_lost(id) };
        let dir = self.run_dir(id);
        let out = dir.join(STDOUT_FILE);
        let pid = run.pid;
        let harness = self.harness_by_id(&run.harness);
        let alive = pid > 0 && harness.is_own_process(pid, run.session_id.as_deref());
        // Gone without a result: it died with the app, or was killed meanwhile.
        if !alive && !file_has_result(harness, &out) {
            return self.settle_lost(id);
        }
        log::info!(target: "routines", "run {id}: adopted (process {})", if alive { "alive" } else { "finished" });
        if alive {
            let spent = run
                .started_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .and_then(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).to_std().ok())
                .unwrap_or_default();
            let left = timeout.saturating_sub(spent);
            self.inner.lock().unwrap().live.insert(id, Live { pgid: pid, deadline: Instant::now() + left });
        }
        let skip = self.store.last_seq(id);
        let tail = self.tail(harness, id, &out, skip, &mut || alive && process_alive(pid));
        self.inner.lock().unwrap().live.remove(&id);
        let Tail::Done(result) = tail else { return };
        let outcome = outcome_from(harness, result, read_stderr(&dir), Ok("an unknown status".into()));
        self.settle(id, &outcome, started, &started_at);
    }

    /// A run that can't be picked up again: it ended with the app.
    fn settle_lost(&self, run_id: i64) {
        self.store.mark_lost(run_id);
        self.discard_run_files(run_id);
        self.sink.status_changed(run_id, "interrupted");
    }

    /// Remove a run's directory and the scratch dirs it created.
    fn discard_run_files(&self, run_id: i64) {
        for d in self.store.cleanup_dirs(run_id) {
            let _ = std::fs::remove_dir_all(d);
        }
        let _ = std::fs::remove_dir_all(self.run_dir(run_id));
    }

    /// Write the final state, prune old runs, tell the sink. Returns the status it settled on.
    fn finalize(&self, run_id: i64, outcome: &Outcome, started: Instant, started_at: &str) -> &'static str {
        let (stop, shutting_down) = {
            let mut inner = self.inner.lock().unwrap();
            (inner.stops.remove(&run_id), inner.shutting_down)
        };
        self.discard_run_files(run_id);
        let result = outcome.result.clone();
        let text = result.as_ref().and_then(|r| r.text.clone()).unwrap_or_default();
        let status = final_status(stop, result.as_ref(), &text);
        let error = match status {
            "error" => Some(outcome.failure.clone().unwrap_or_else(|| {
                if text.is_empty() {
                    "the run reported an error".to_string()
                } else {
                    text.clone()
                }
            })),
            _ => None,
        };
        let usage_error = match status {
            "ok" | "needs_attention" => None,
            "error" => Some(outcome.failure.clone().unwrap_or_else(|| "error".into())),
            other => Some(other.to_string()),
        };
        let fin = Finished {
            status,
            text,
            error,
            usage: outcome.usage.clone(),
            usage_error,
            shutting_down,
            started_at: started_at.to_string(),
            elapsed: started.elapsed(),
        };
        // Guarded in the store: on exit `kill_all` already settled the row.
        self.store.finish(run_id, &fin);
        self.store.prune(KEEP_RUNS);
        log::info!(target: "routines", "run {run_id}: {status}");
        self.sink.finished(run_id, &fin);
        status
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::ai::{Caps, Event, Interactive, OneShot, TurnResult};
    use crate::ai::{Access, SystemPromptMode, ToolPolicy};
    use crate::ai_usage::RunTag;
    use serde_json::Value;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    /// Spawns `sh -c <prompt-independent script>` taken from the model field, as a group leader.
    struct FakeHarness {
        own: bool,
        /// Put in front of the final text, to tell harnesses apart.
        prefix: &'static str,
    }

    impl Harness for FakeHarness {
        fn id(&self) -> &'static str {
            if self.prefix.is_empty() { "fake" } else { "alt" }
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn caps(&self) -> Caps {
            Caps {
                sessions: false,
                resume: false,
                fork: false,
                tools: false,
                tool_allowlist: false,
                streaming: true,
                interactive_tui: false,
                reports_cost: false,
                replaces_system_prompt: false,
                access_levels: vec![],
                pins_session_id: false,
            }
        }
        fn available(&self) -> bool {
            true
        }
        fn one_shot(&self, _: &OneShot<'_>) -> AppResult<TurnResult> {
            unimplemented!()
        }
        fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
            use std::os::unix::process::CommandExt;
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(run.req.model);
            cmd.stdin(Stdio::piped()).stdout(Stdio::from(run.stdout)).stderr(Stdio::from(run.stderr));
            cmd.process_group(0);
            cmd.spawn().map_err(|e| AppError::Other(e.to_string()))
        }
        fn parse_line(&self, line: &str) -> (String, Option<Value>) {
            match serde_json::from_str::<Value>(line) {
                Ok(v) => (v.get("type").and_then(Value::as_str).unwrap_or("unknown").to_string(), Some(v)),
                Err(_) => ("raw".to_string(), None),
            }
        }
        fn normalize(&self, line: &str) -> Vec<Event> {
            match serde_json::from_str::<Value>(line) {
                Ok(v) if v.get("type").and_then(Value::as_str) == Some("result") => vec![Event::Final {
                    ok: !v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    text: format!("{}{}", self.prefix, v.get("result").and_then(Value::as_str).unwrap_or_default()),
                    usage: Default::default(),
                }],
                Ok(v) => vec![Event::Other { kind: "x".into(), raw: v }],
                Err(_) => vec![Event::Other { kind: "raw".into(), raw: Value::Null }],
            }
        }
        fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
            self.own
        }
        fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
            None
        }
    }

    #[derive(Clone, Debug, Default)]
    struct Row {
        status: String,
        /// Id of the harness the run was started on (empty: the default).
        harness: String,
        pid: i32,
        session_id: Option<String>,
        started_at: Option<String>,
        events: Vec<(i64, String)>,
        norms: Vec<(i64, String)>,
        text: String,
        error: Option<String>,
    }

    struct FakeStore {
        root: PathBuf,
        cap: i64,
        enabled: Mutex<bool>,
        rows: Mutex<std::collections::BTreeMap<i64, Row>>,
    }

    impl FakeStore {
        fn new(name: &str, cap: i64) -> Arc<Self> {
            let root = std::env::temp_dir().join(format!("cr_engine_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Arc::new(Self { root, cap, enabled: Mutex::new(true), rows: Mutex::new(Default::default()) })
        }
        fn add(&self, id: i64, status: &str) {
            self.rows.lock().unwrap().insert(id, Row { status: status.into(), ..Row::default() });
        }
        fn row(&self, id: i64) -> Row {
            self.rows.lock().unwrap()[&id].clone()
        }
        fn status(&self, id: i64) -> String {
            self.row(id).status
        }
    }

    impl RunStore for FakeStore {
        fn ai_enabled(&self) -> bool {
            *self.enabled.lock().unwrap()
        }
        fn max_concurrency(&self) -> i64 {
            self.cap
        }
        fn running_count(&self) -> AppResult<i64> {
            Ok(self.rows.lock().unwrap().values().filter(|r| r.status == "running").count() as i64)
        }
        fn next_queued(&self) -> AppResult<Option<i64>> {
            Ok(self.rows.lock().unwrap().iter().find(|(_, r)| r.status == "queued").map(|(id, _)| *id))
        }
        fn mark_running(&self, id: i64) -> AppResult<bool> {
            let mut rows = self.rows.lock().unwrap();
            let r = rows.get_mut(&id).unwrap();
            if r.status != "queued" {
                return Ok(false);
            }
            r.status = "running".into();
            Ok(true)
        }
        fn is_running(&self, id: i64) -> bool {
            self.rows.lock().unwrap().get(&id).is_some_and(|r| r.status == "running")
        }
        fn drop_queued(&self, id: i64) -> AppResult<bool> {
            let mut rows = self.rows.lock().unwrap();
            match rows.get_mut(&id) {
                Some(r) if r.status == "queued" => {
                    r.status = "killed".into();
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
        fn active_count(&self) -> AppResult<usize> {
            Ok(self.rows.lock().unwrap().values().filter(|r| r.status == "queued" || r.status == "running").count())
        }
        fn kill_queued(&self) -> AppResult<()> {
            for r in self.rows.lock().unwrap().values_mut().filter(|r| r.status == "queued") {
                r.status = "killed".into();
            }
            Ok(())
        }
        fn kill_active(&self) -> AppResult<()> {
            for r in self.rows.lock().unwrap().values_mut().filter(|r| r.status == "queued" || r.status == "running") {
                r.status = "killed".into();
            }
            Ok(())
        }
        fn set_process(&self, id: i64, pid: u32, _: &Path, session_id: &str) {
            let mut rows = self.rows.lock().unwrap();
            let r = rows.get_mut(&id).unwrap();
            r.pid = pid as i32;
            r.session_id = Some(session_id.into());
        }
        fn set_cleanup(&self, _: i64, _: &[PathBuf]) {}
        fn cleanup_dirs(&self, _: i64) -> Vec<PathBuf> {
            Vec::new()
        }
        fn append_event(&self, id: i64, seq: i64, kind: &str, _: &str, norm: &str) {
            let mut rows = self.rows.lock().unwrap();
            let row = rows.get_mut(&id).unwrap();
            row.events.retain(|(s, _)| *s != seq);
            row.events.push((seq, kind.into()));
            row.norms.retain(|(s, _)| *s != seq);
            row.norms.push((seq, norm.into()));
        }
        fn last_seq(&self, id: i64) -> i64 {
            self.rows.lock().unwrap()[&id].events.iter().map(|(s, _)| *s).max().unwrap_or(0)
        }
        fn running_runs(&self) -> AppResult<Vec<RunningRun>> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, r)| r.status == "running")
                .map(|(id, r)| RunningRun {
                    id: *id,
                    pid: r.pid,
                    harness: r.harness.clone(),
                    session_id: r.session_id.clone(),
                    started_at: r.started_at.clone(),
                })
                .collect())
        }
        fn finish(&self, id: i64, fin: &Finished) {
            let mut rows = self.rows.lock().unwrap();
            let r = rows.get_mut(&id).unwrap();
            if r.status == "running" {
                r.status = fin.status.into();
                r.text = fin.text.clone();
                r.error = fin.error.clone();
            }
        }
        fn prune(&self, _: i64) {}
        fn mark_lost(&self, id: i64) {
            let mut rows = self.rows.lock().unwrap();
            let r = rows.get_mut(&id).unwrap();
            if r.status == "running" {
                r.status = "interrupted".into();
            }
        }
        fn runs_root(&self) -> PathBuf {
            self.root.clone()
        }
    }

    /// `script` per run id is what the fake harness executes.
    struct FakeSink {
        scripts: Mutex<HashMap<i64, String>>,
        timeout: Duration,
        log: Mutex<Vec<String>>,
        finished: Mutex<Vec<(i64, Finished)>>,
        adopt_timeout: Option<Duration>,
        /// What runs are prepared with (set by `engine`), and per run id an alternative.
        harness: std::sync::OnceLock<&'static dyn Harness>,
        alt: Mutex<HashMap<i64, &'static dyn Harness>>,
    }

    impl FakeSink {
        fn new(timeout: Duration) -> Arc<Self> {
            Arc::new(Self {
                scripts: Default::default(),
                timeout,
                log: Default::default(),
                finished: Default::default(),
                adopt_timeout: Some(Duration::from_secs(60)),
                harness: Default::default(),
                alt: Default::default(),
            })
        }
        fn script(&self, id: i64, s: &str) {
            self.scripts.lock().unwrap().insert(id, s.into());
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl EventSink for FakeSink {
        fn queued(&self, id: i64) {
            self.log.lock().unwrap().push(format!("queued {id}"));
        }
        fn started(&self, id: i64) {
            self.log.lock().unwrap().push(format!("started {id}"));
        }
        fn status_changed(&self, id: i64, status: &str) {
            self.log.lock().unwrap().push(format!("status {id} {status}"));
        }
        fn event(&self, id: i64, seq: i64, kind: &str, _: &str) {
            self.log.lock().unwrap().push(format!("event {id} {seq} {kind}"));
        }
        fn dropped(&self, id: i64) {
            self.log.lock().unwrap().push(format!("dropped {id}"));
        }
        fn prepare(&self, id: i64, stopped: &dyn Fn() -> bool) -> Result<Option<Prepared>, String> {
            let Some(script) = self.scripts.lock().unwrap().get(&id).cloned() else {
                return Err(format!("run {id} not found"));
            };
            if stopped() {
                return Ok(None);
            }
            let harness = self.alt.lock().unwrap().get(&id).copied().unwrap_or_else(|| *self.harness.get().unwrap());
            Ok(Some(Prepared {
                harness,
                model: script,
                system_prompt: "sys".into(),
                system_prompt_mode: SystemPromptMode::Append,
                prompt: "hi".into(),
                cwd: std::env::temp_dir(),
                extra_dirs: vec![],
                tools: ToolPolicy::None,
                access: Access::Default,
                session_id: "sid".into(),
                env: vec![],
                extra_args: vec![],
                usage_tag: RunTag::new("t"),
                timeout: self.timeout,
                cleanup: vec![],
            }))
        }
        fn adopt(&self, _: i64) -> Option<Duration> {
            self.adopt_timeout
        }
        fn finished(&self, id: i64, fin: &Finished) {
            self.finished.lock().unwrap().push((id, fin.clone()));
        }
    }

    fn wait_for(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    const RESULT_OK: &str = r#"printf '%s\n' '{"type":"assistant"}' '{"type":"result","is_error":false,"result":"done"}'"#;

    fn engine(store: &Arc<FakeStore>, sink: &Arc<FakeSink>, own: bool) -> Arc<RunEngine> {
        let harness: &'static FakeHarness = Box::leak(Box::new(FakeHarness { own, prefix: "" }));
        let _ = sink.harness.set(harness);
        RunEngine::new(store.clone(), sink.clone(), harness)
    }

    #[test]
    fn each_run_goes_through_the_harness_it_was_prepared_with() {
        let store = FakeStore::new("per_run", 2);
        let sink = FakeSink::new(Duration::from_secs(60));
        let alt: &'static FakeHarness = Box::leak(Box::new(FakeHarness { own: false, prefix: "alt:" }));
        for id in 1..=2 {
            store.add(id, "queued");
            sink.script(id, RESULT_OK);
        }
        sink.alt.lock().unwrap().insert(2, alt);
        let e = engine(&store, &sink, false);
        e.pump();
        wait_for("both done", || store.status(1) == "ok" && store.status(2) == "ok");
        assert_eq!(store.row(1).text, "done");
        assert_eq!(store.row(2).text, "alt:done");
    }

    #[test]
    fn queue_is_fifo_under_the_cap() {
        let store = FakeStore::new("fifo", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        for id in 1..=3 {
            store.add(id, "queued");
            sink.script(id, &format!("sleep 0.3; {RESULT_OK}"));
        }
        let e = engine(&store, &sink, false);
        e.pump();
        // cap 1: only the oldest runs, the others wait
        assert_eq!(
            (store.status(1), store.status(2), store.status(3)),
            ("running".into(), "queued".into(), "queued".into())
        );
        wait_for("all done", || (1..=3).all(|i| store.status(i) == "ok"));
        let started: Vec<String> = sink.log().into_iter().filter(|l| l.starts_with("started")).collect();
        assert_eq!(started, ["started 1", "started 2", "started 3"]);
        let ev: Vec<_> = store.row(1).events.iter().map(|(_, k)| k.clone()).collect();
        assert_eq!(ev, ["assistant", "result"]);
        assert_eq!(store.row(1).text, "done");
    }

    #[test]
    fn pump_waits_while_ai_is_off() {
        let store = FakeStore::new("gate", 4);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "queued");
        sink.script(1, RESULT_OK);
        *store.enabled.lock().unwrap() = false;
        let e = engine(&store, &sink, false);
        e.pump();
        assert_eq!(store.status(1), "queued");
        *store.enabled.lock().unwrap() = true;
        e.pump();
        wait_for("run", || store.status(1) == "ok");
    }

    #[test]
    fn stopping_a_queued_run_drops_it() {
        let store = FakeStore::new("dropq", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "queued");
        let e = engine(&store, &sink, false);
        e.stop(1).unwrap();
        assert_eq!(store.status(1), "killed");
        assert_eq!(sink.log(), ["status 1 killed", "dropped 1"]);
        assert!(e.stop(1).is_err());
    }

    #[test]
    fn stop_wins_over_the_reported_result() {
        let store = FakeStore::new("stop", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "queued");
        // reports a successful result, then lingers; SIGTERM ends it
        sink.script(1, &format!("{RESULT_OK}; sleep 30"));
        let e = engine(&store, &sink, false);
        e.pump();
        wait_for("result event", || store.row(1).events.len() == 2);
        e.stop(1).unwrap();
        wait_for("killed", || store.status(1) == "killed");
        let fins = sink.finished.lock().unwrap();
        assert_eq!(fins[0].1.status, "killed");
        assert_eq!(fins[0].1.usage_error.as_deref(), Some("killed"));
    }

    #[test]
    fn overdue_runs_time_out() {
        let store = FakeStore::new("timeout", 1);
        let sink = FakeSink::new(Duration::from_millis(1));
        store.add(1, "queued");
        sink.script(1, "sleep 30");
        let e = engine(&store, &sink, false);
        e.pump();
        wait_for("live", || e.inner.lock().unwrap().live.contains_key(&1));
        std::thread::sleep(Duration::from_millis(20));
        e.expire_overdue();
        wait_for("timeout", || store.status(1) == "timeout");
    }

    #[test]
    fn stop_before_launch_skips_the_process() {
        let store = FakeStore::new("prestop", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running");
        sink.script(1, "touch should_not_exist");
        let e = engine(&store, &sink, false);
        e.inner.lock().unwrap().stops.insert(1, "killed");
        Arc::clone(&e).execute(1);
        assert_eq!(store.status(1), "killed");
        assert!(store.row(1).events.is_empty());
    }

    #[test]
    fn a_failing_prepare_fails_the_run() {
        let store = FakeStore::new("prep", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running"); // no script: prepare errors
        let e = engine(&store, &sink, false);
        Arc::clone(&e).execute(1);
        assert_eq!(store.status(1), "error");
        assert_eq!(store.row(1).error.as_deref(), Some("run 1 not found"));
    }

    #[test]
    fn a_run_without_result_reports_the_exit() {
        let store = FakeStore::new("noresult", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running");
        sink.script(1, "echo oops >&2; exit 3");
        let e = engine(&store, &sink, false);
        Arc::clone(&e).execute(1);
        assert_eq!(store.status(1), "error");
        assert!(store.row(1).error.unwrap().starts_with("fake exited with exit status: 3 without a result. oops"));
    }

    fn leave_files(store: &FakeStore, id: i64, stdout: &str) {
        let dir = store.root.join(id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STDOUT_FILE), stdout).unwrap();
    }

    #[test]
    fn adopting_a_finished_process_settles_from_its_output() {
        let store = FakeStore::new("adopt_done", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running");
        store.append_event(1, 1, "assistant", "{}", "{}");
        leave_files(&store, 1, "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"result\":\"late\"}\n");
        let e = engine(&store, &sink, false);
        e.adopt_detached();
        wait_for("ok", || store.status(1) == "ok");
        // seq 1 was recorded before: only the result is new
        assert_eq!(store.row(1).events, vec![(1, "assistant".to_string()), (2, "result".to_string())]);
        assert_eq!(store.row(1).text, "late");
        // the new line was stored with its normalized form next to it; the old one is untouched
        let norms = store.row(1).norms;
        assert_eq!(norms[0], (1, "{}".to_string()));
        let last: serde_json::Value = serde_json::from_str(&norms[1].1).unwrap();
        assert_eq!((last["v"].clone(), last["events"][0]["type"].clone()), (1.into(), "final".into()));
        assert_eq!(last["events"][0]["text"], "late");
        assert!(!store.root.join("1").exists());
    }

    #[test]
    fn adopting_a_missing_process_without_result_is_interrupted() {
        let store = FakeStore::new("adopt_lost", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running");
        leave_files(&store, 1, "{\"type\":\"assistant\"}\n");
        let e = engine(&store, &sink, false);
        e.adopt_detached();
        wait_for("interrupted", || store.status(1) == "interrupted");
        assert!(sink.log().contains(&"status 1 interrupted".to_string()));
        assert!(!store.root.join("1").exists());
    }

    #[test]
    fn adopting_an_alive_process_keeps_tailing_it() {
        let store = FakeStore::new("adopt_alive", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "running");
        leave_files(&store, 1, "");
        let out = store.root.join("1").join(STDOUT_FILE);
        // stands in for a process that outlived the previous session
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 0.6; printf '%s\\n' '{{\"type\":\"result\",\"result\":\"alive\"}}' > {}", out.display()))
            .spawn()
            .unwrap();
        store.rows.lock().unwrap().get_mut(&1).unwrap().pid = child.id() as i32;
        // reaped by its parent, as the real orphan is by init (a zombie would still look alive)
        let reaper = std::thread::spawn(move || child.wait());
        let e = engine(&store, &sink, true);
        e.adopt_detached();
        wait_for("ok", || store.status(1) == "ok");
        let _ = reaper.join();
        assert_eq!(store.row(1).text, "alive");
    }

    #[test]
    fn adopt_sweeps_directories_of_other_runs() {
        let store = FakeStore::new("sweep", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        leave_files(&store, 7, "x");
        std::fs::create_dir_all(store.root.join("notarun")).unwrap();
        let e = engine(&store, &sink, false);
        e.adopt_detached();
        assert!(!store.root.join("7").exists());
        assert!(store.root.join("notarun").exists());
    }

    #[test]
    fn detach_drops_queued_and_leaves_running_alone() {
        let store = FakeStore::new("detach", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "queued");
        store.add(2, "queued");
        sink.script(1, "sleep 1");
        let e = engine(&store, &sink, false);
        e.pump();
        wait_for("live", || e.inner.lock().unwrap().live.contains_key(&1));
        let pid = store.row(1).pid;
        e.detach_all();
        // the running row stays `running` for the next start; the queued one is dropped
        assert_eq!((store.status(1), store.status(2)), ("running".into(), "killed".into()));
        assert!(process_alive(pid));
        e.kill_all(); // detaching: a no-op
        assert_eq!(store.status(1), "running");
        signal_group(pid, SIGKILL);
    }

    #[test]
    fn kill_all_terminates_groups_and_settles_rows() {
        let store = FakeStore::new("killall", 1);
        let sink = FakeSink::new(Duration::from_secs(60));
        store.add(1, "queued");
        store.add(2, "queued");
        sink.script(1, "sleep 30");
        let e = engine(&store, &sink, false);
        e.pump();
        wait_for("live", || e.inner.lock().unwrap().live.contains_key(&1));
        e.kill_all();
        assert_eq!((store.status(1), store.status(2)), ("killed".into(), "killed".into()));
        wait_for("finished", || !sink.finished.lock().unwrap().is_empty());
        assert!(sink.finished.lock().unwrap()[0].1.shutting_down);
    }
}
