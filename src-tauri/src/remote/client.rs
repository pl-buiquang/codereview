//! A typed, blocking client of the `cr serve` `/v1` API (see the API reference in the docs). Wire types
//! shared with the daemon (`serve::request`) are reused so the two cannot drift apart; the response
//! types are lenient about fields the client does not use.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{RemoteError, RemoteResult};
use super::http::{self, SseReader};
use super::tunnel::{Addr, Health, HealthInfo};
use crate::ai::Caps;
use crate::serve::request::{OneShotRequest, OneShotResponse, RunRequest};

/// Calls that only read or write a row.
const QUICK: Duration = Duration::from_secs(30);
/// Creating a run may resolve a worktree (git and gh calls of up to 60 s each).
const CREATE: Duration = Duration::from_secs(150);
/// The daemon answers a one-shot after at most 10 minutes.
const ONESHOT: Duration = Duration::from_secs(11 * 60);
/// The daemon sends a keep-alive every 15 s; three missed ones mean a dead connection.
const STREAM_SILENCE: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Deserialize)]
pub struct HarnessInfo {
    pub id: String,
    pub name: String,
    pub caps: Caps,
    pub available: bool,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Load {
    pub running: i64,
    pub queued: i64,
}

/// `GET /v1/info`
#[derive(Debug, Clone, Deserialize)]
pub struct Info {
    pub protocol: i64,
    pub version: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub harnesses: Vec<HarnessInfo>,
    #[serde(default)]
    pub max_concurrency: i64,
    #[serde(default)]
    pub load: Load,
}

/// A run as the daemon reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct RunView {
    pub id: i64,
    #[serde(default)]
    pub idempotency_key: String,
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub model: String,
    pub status: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: Option<Value>,
    #[serde(default)]
    pub last_seq: i64,
}

/// The remote statuses that end a run.
pub fn is_terminal(status: &str) -> bool {
    !matches!(status, "preparing" | "queued" | "running")
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatedRun {
    pub id: i64,
    pub created: bool,
}

/// One stored event of a run.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub seq: i64,
    pub kind: String,
    pub json: Value,
    #[serde(default)]
    pub norm: Value,
}

/// The terminal `status` event of a stream.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StatusEvent {
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub usage: Option<Value>,
    #[serde(default)]
    pub last_seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Event(StreamEvent),
    Status(StatusEvent),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OutboxOp {
    pub seq: i64,
    pub op_id: String,
    #[serde(rename = "type")]
    pub op_type: String,
    #[serde(default)]
    pub args: Value,
}

/// An open event stream of a run.
pub struct EventStream {
    reader: SseReader,
}

impl EventStream {
    /// The next event or status; `None` when the daemon closed the stream (after the status).
    pub fn next_item(&mut self) -> RemoteResult<Option<StreamItem>> {
        loop {
            let Some(e) = self.reader.next_event()? else { return Ok(None) };
            let bad = |what: &str, err: serde_json::Error| RemoteError::protocol(format!("bad {what} event: {err}"));
            match e.event.as_str() {
                "event" => return Ok(Some(StreamItem::Event(serde_json::from_str(&e.data).map_err(|x| bad("run", x))?))),
                "status" => return Ok(Some(StreamItem::Status(serde_json::from_str(&e.data).map_err(|x| bad("status", x))?))),
                _ => continue,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RemoteClient {
    addr: Addr,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// An error answer: `{"error": {"code", "message"}}`, else the raw status.
fn api_error(status: u16, body: &[u8]) -> RemoteError {
    #[derive(Deserialize)]
    struct Body {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        code: String,
        message: String,
    }
    match serde_json::from_slice::<Body>(body) {
        Ok(b) => RemoteError::new(b.error.code, b.error.message),
        Err(_) => RemoteError::protocol(format!("unexpected HTTP {status}: {}", String::from_utf8_lossy(body).chars().take(200).collect::<String>())),
    }
}

impl RemoteClient {
    pub fn new(addr: Addr) -> Self {
        Self { addr }
    }

    fn call<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<&impl Serialize>, timeout: Duration) -> RemoteResult<T> {
        let bytes = body.map(serde_json::to_vec).transpose().map_err(|e| RemoteError::protocol(e.to_string()))?;
        let resp = http::request(&self.addr, method, path, bytes.as_deref(), timeout)?;
        if !(200..300).contains(&resp.status) {
            return Err(api_error(resp.status, &resp.body));
        }
        serde_json::from_slice(&resp.body).map_err(|e| RemoteError::protocol(format!("bad answer from {path}: {e}")))
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> RemoteResult<T> {
        self.call::<T>("GET", path, None::<&()>, QUICK)
    }

    pub fn info(&self) -> RemoteResult<Info> {
        self.get("/v1/info")
    }

    pub fn models(&self, harness: &str) -> RemoteResult<Vec<String>> {
        #[derive(Deserialize)]
        struct Models {
            models: Vec<String>,
        }
        Ok(self.get::<Models>(&format!("/v1/harnesses/{}/models", encode(harness)))?.models)
    }

    pub fn oneshot(&self, req: &OneShotRequest) -> RemoteResult<OneShotResponse> {
        self.call("POST", "/v1/oneshot", Some(req), ONESHOT)
    }

    /// Create a run (or find the one the same idempotency key made before).
    pub fn create_run(&self, req: &RunRequest) -> RemoteResult<CreatedRun> {
        self.call("POST", "/v1/runs", Some(req), CREATE)
    }

    /// The run, or `None` when the daemon does not know it.
    pub fn get_run(&self, id: i64) -> RemoteResult<Option<RunView>> {
        match self.get(&format!("/v1/runs/{id}")) {
            Ok(r) => Ok(Some(r)),
            Err(e) if e.code == "not_found" => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn list_runs(&self, since: Option<&str>, limit: i64) -> RemoteResult<Vec<RunView>> {
        #[derive(Deserialize)]
        struct Runs {
            runs: Vec<RunView>,
        }
        let mut path = format!("/v1/runs?limit={limit}");
        if let Some(s) = since {
            path.push_str(&format!("&since={}", encode(s)));
        }
        Ok(self.get::<Runs>(&path)?.runs)
    }

    /// Follow a run: replay events after `after_seq`, then live ones, then the terminal status.
    pub fn events(&self, id: i64, after_seq: i64) -> RemoteResult<EventStream> {
        match http::open_sse(&self.addr, &format!("/v1/runs/{id}/events?after_seq={after_seq}"), STREAM_SILENCE)? {
            Ok(reader) => Ok(EventStream { reader }),
            Err(resp) => Err(api_error(resp.status, &resp.body)),
        }
    }

    pub fn stop(&self, id: i64) -> RemoteResult<()> {
        self.call::<Value>("POST", &format!("/v1/runs/{id}/stop"), Some(&Value::Null), QUICK).map(|_| ())
    }

    /// The unacknowledged outbox ops after `after`.
    pub fn outbox(&self, id: i64, after: i64) -> RemoteResult<Vec<OutboxOp>> {
        #[derive(Deserialize)]
        struct Ops {
            ops: Vec<OutboxOp>,
        }
        Ok(self.get::<Ops>(&format!("/v1/runs/{id}/outbox?after={after}"))?.ops)
    }

    pub fn ack(&self, id: i64, upto: i64) -> RemoteResult<()> {
        self.call::<Value>("POST", &format!("/v1/runs/{id}/outbox/ack"), Some(&serde_json::json!({ "upto": upto })), QUICK).map(|_| ())
    }
}

/// The production health check of a tunnel: `GET /v1/info`.
pub struct InfoHealth;

impl Health for InfoHealth {
    fn check(&self, addr: &Addr) -> RemoteResult<HealthInfo> {
        // A short timeout: the monitor and the connect loop retry.
        let resp = http::request(addr, "GET", "/v1/info", None, Duration::from_secs(5))?;
        if !(200..300).contains(&resp.status) {
            return Err(api_error(resp.status, &resp.body));
        }
        let info: Info = serde_json::from_slice(&resp.body).map_err(|e| RemoteError::protocol(format!("bad /v1/info: {e}")))?;
        Ok(HealthInfo { protocol: info.protocol, version: info.version })
    }
}

#[cfg(all(test, feature = "serve", unix))]
mod tests {
    use super::*;
    use crate::remote::test_server::TestServer;
    use crate::serve::PROTOCOL;

    fn client(srv: &TestServer) -> RemoteClient {
        RemoteClient::new(srv.addr.clone())
    }

    fn run_request(key: &str, script: &str) -> RunRequest {
        serde_json::from_value(serde_json::json!({
            "idempotency_key": key, "harness": "fake", "model": script, "prompt": "go",
            "workspace": { "repo": "acme/widgets", "kind": "main-clone" }
        }))
        .unwrap()
    }

    fn wait_terminal(c: &RemoteClient, id: i64) -> RunView {
        for _ in 0..300 {
            let r = c.get_run(id).unwrap().unwrap();
            if is_terminal(&r.status) {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("run {id} did not finish");
    }

    #[test]
    fn info_models_and_the_health_check() {
        let srv = TestServer::start(2);
        let c = client(&srv);
        let info = c.info().unwrap();
        assert_eq!(info.protocol, i64::from(PROTOCOL));
        assert_eq!((info.max_concurrency, info.load.clone()), (2, Load::default()));
        assert_eq!(info.harnesses[0].id, "fake");
        assert!(info.harnesses[0].available && info.harnesses[0].caps.streaming);
        assert!(c.models("fake").unwrap().is_empty());
        let h = InfoHealth.check(&srv.addr).unwrap();
        assert_eq!((h.protocol, h.version), (i64::from(PROTOCOL), info.version));
    }

    #[test]
    fn oneshot_returns_text_and_usage_and_api_errors_keep_their_code() {
        let srv = TestServer::start(2);
        let c = client(&srv);
        let mut req: OneShotRequest = serde_json::from_value(serde_json::json!({"harness": "fake", "prompt": "hi"})).unwrap();
        let r = c.oneshot(&req).unwrap();
        assert!(r.text.starts_with("echo:hi@"), "{}", r.text);
        assert_eq!(r.usage.unwrap().cost_usd, 0.5);
        req.harness = "nope".into();
        let err = c.oneshot(&req).unwrap_err();
        assert_eq!(err.code, "harness_unavailable");
        req.harness = "fake".into();
        req.workspace = serde_json::from_value(serde_json::json!({"repo": "acme/nothere", "kind": "main-clone"})).unwrap();
        assert_eq!(c.oneshot(&req).unwrap_err().code, "unknown_repo");
    }

    #[test]
    fn a_run_is_created_idempotently_streamed_and_finished() {
        let srv = TestServer::start(2);
        let c = client(&srv);
        let req = run_request("k1", r#"echo '{"type":"a"}'; echo '{"type":"result","result":"done"}'"#);
        let first = c.create_run(&req).unwrap();
        assert!(first.created);
        let again = c.create_run(&req).unwrap();
        assert_eq!((again.id, again.created), (first.id, false));
        let mut other = req.clone();
        other.prompt = "different".into();
        assert_eq!(c.create_run(&other).unwrap_err().code, "conflict");

        let mut stream = c.events(first.id, 0).unwrap();
        let mut seqs = Vec::new();
        let status = loop {
            match stream.next_item().unwrap().expect("a status ends the stream") {
                StreamItem::Event(e) => seqs.push(e.seq),
                StreamItem::Status(s) => break s,
            }
        };
        assert_eq!(seqs, [1, 2]);
        assert_eq!((status.status.as_str(), status.text.as_deref(), status.last_seq), ("ok", Some("done"), 2));
        assert_eq!(stream.next_item().unwrap(), None);

        // resuming after seq 1 replays only the second event
        let mut resumed = c.events(first.id, 1).unwrap();
        assert!(matches!(resumed.next_item().unwrap(), Some(StreamItem::Event(e)) if e.seq == 2));
        let run = wait_terminal(&c, first.id);
        assert_eq!(run.last_seq, 2);
        assert_eq!(c.list_runs(None, 10).unwrap().len(), 1);
        assert!(c.list_runs(Some(&run.updated_at), 10).unwrap().len() <= 1);
        assert!(c.get_run(9999).unwrap().is_none());
        assert_eq!(c.events(9999, 0).err().map(|e| e.code), Some("not_found".into()));
    }

    #[test]
    fn stop_and_the_outbox_round_trip() {
        let srv = TestServer::start(2);
        let c = client(&srv);
        let created = c.create_run(&run_request("k2", "sleep 30")).unwrap();
        for _ in 0..200 {
            if c.get_run(created.id).unwrap().unwrap().status == "running" {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // ops written by `cr` inside the run
        let sdb = srv.state.db.clone();
        let conn = crate::serve::store::open(&srv.state.serve_db_path).unwrap();
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        crate::serve::store::append_outbox(&conn, created.id, "notify", &serde_json::json!({"message": "m"})).unwrap();
        crate::serve::store::append_outbox(&conn, created.id, "board_track", &serde_json::json!({"reference": "a/b#1"})).unwrap();
        drop(sdb);
        let ops = c.outbox(created.id, 0).unwrap();
        assert_eq!(ops.iter().map(|o| (o.seq, o.op_type.as_str())).collect::<Vec<_>>(), [(1, "notify"), (2, "board_track")]);
        assert_eq!(ops[0].op_id, format!("{}:1", created.id));
        c.ack(created.id, 1).unwrap();
        assert_eq!(c.outbox(created.id, 0).unwrap().len(), 1);

        c.stop(created.id).unwrap();
        assert_eq!(wait_terminal(&c, created.id).status, "killed");
        assert_eq!(c.stop(created.id).unwrap_err().code, "bad_request");
    }

    #[test]
    fn a_dead_tunnel_is_host_unreachable() {
        let c = RemoteClient::new(Addr::Unix("/nonexistent/dir/serve.sock".into()));
        let err = c.info().unwrap_err();
        assert!(err.is_connection(), "{err}");
    }
}
