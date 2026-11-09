//! Read-only remote sessions (S10): the live sessions of every enabled host, from `GET /v1/sessions`,
//! shaped like the local ones plus the host they belong to. Only metadata crosses the wire (no
//! transcripts, no summaries); nothing here launches, focuses or stops a remote session.

use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::client::RemoteClient;
use super::error::{RemoteError, RemoteResult, HOST_DISABLED, HOST_UNREACHABLE, PROTOCOL_MISMATCH};
use super::host::Host;
use super::runtime::RemoteRuntime;
use super::tunnel::ConnState;
use crate::ai::AgentSession;
use crate::claude_sessions::{FocusState, TmuxLoc};

/// One read of a host's answer may take this long...
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// ...and the whole answer this long (a host dripping bytes is cut off).
pub const SESSIONS_TIMEOUT: Duration = Duration::from_secs(10);
/// The hard limit for a whole lookup, tunnel connect included. Past it a host is "timed out" and
/// the others' rows are returned without it.
pub const DEADLINE: Duration = Duration::from_secs(12);
/// A host's answer is never trusted to be small.
pub const MAX_BODY: usize = 4 * 1024 * 1024;
pub const MAX_SESSIONS: usize = 500;
const MAX_FIELD: usize = 512;
const MAX_CWD: usize = 2048;
const MAX_ERROR: usize = 300;

/// A remote-supplied string: control characters dropped (they would reach the clipboard and a
/// terminal) and the length capped.
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn clean_opt(s: Option<String>) -> Option<String> {
    s.map(|s| clean(&s, MAX_FIELD))
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireTmux {
    pub session: String,
    pub window: String,
    pub pane: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// One entry of `GET /v1/sessions`, lenient about the fields the app does not use.
#[derive(Debug, Clone, Deserialize)]
pub struct WireSession {
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub attached: bool,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub waiting_for: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub tmux: Option<WireTmux>,
    #[serde(default)]
    pub updated_at: i64,
}

/// A remote session as the Sessions view gets it: an [`AgentSession`] (with the fields that need
/// local access at their "nothing here" values) and the host. Identity is `(host_id, harness, session_id)`.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSession {
    #[serde(flatten)]
    pub session: AgentSession,
    pub host_id: i64,
    pub host_name: String,
    /// What `ssh` takes to reach the host (for the copy-ssh action).
    pub host_ssh_target: String,
}

impl RemoteSession {
    pub fn from_wire(w: WireSession, host: &Host) -> Self {
        let session = AgentSession {
            harness: clean(&w.harness, MAX_FIELD),
            session_id: clean(&w.session_id, MAX_FIELD),
            pid: None,
            live: w.live,
            status: clean_opt(w.status),
            kind: clean_opt(w.kind),
            attached: w.attached,
            job_id: clean_opt(w.job_id),
            waiting_for: clean_opt(w.waiting_for),
            agent_view: None,
            name: clean_opt(w.name),
            cwd: clean(&w.cwd, MAX_CWD),
            git_branch: clean_opt(w.git_branch),
            repo: clean_opt(w.repo),
            tmux: w.tmux.map(|t| TmuxLoc {
                session: clean(&t.session, MAX_FIELD),
                window: clean(&t.window, MAX_FIELD),
                pane: clean(&t.pane, MAX_FIELD),
                label: clean_opt(t.label),
            }),
            tty: None,
            updated_at: w.updated_at,
            // No terminal of ours shows a remote session.
            focus: FocusState::NoTerminal,
            summaries: Vec::new(),
            original_name: None,
        };
        Self { session, host_id: host.id, host_name: host.name.clone(), host_ssh_target: host.ssh_target.clone() }
    }
}

impl WireSession {
    /// Drop what a bounded list cannot hold: sessions without an id.
    fn usable(&self) -> bool {
        !self.session_id.is_empty()
    }
}

/// Why a host has no sessions to show. Serialized as the row's state in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSessionsStatus {
    Ok,
    Unreachable,
    ProtocolMismatch,
    Disabled,
    /// The host answered, but not with a session list.
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostSessions {
    pub host_id: i64,
    pub host_name: String,
    pub status: HostSessionsStatus,
    pub error: Option<String>,
    pub sessions: Vec<RemoteSession>,
}

fn status_of(e: &RemoteError) -> HostSessionsStatus {
    match e.code.as_str() {
        HOST_UNREACHABLE => HostSessionsStatus::Unreachable,
        PROTOCOL_MISMATCH => HostSessionsStatus::ProtocolMismatch,
        HOST_DISABLED => HostSessionsStatus::Disabled,
        _ => HostSessionsStatus::Error,
    }
}

/// The sessions of every host in `hosts`, in host-name order. Hosts are asked in parallel, each on
/// its own thread, and the lookup is abandoned at `deadline`: a host that has not answered by then
/// is a "timed out" row and nothing waits for it (its thread ends on its own timeouts). A host that
/// fails gives a row with its status, never an error.
pub fn collect<F>(mut hosts: Vec<Host>, deadline: Duration, fetch: Arc<F>) -> Vec<HostSessions>
where
    F: Fn(&Host) -> RemoteResult<Vec<WireSession>> + Send + Sync + 'static,
{
    hosts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    let (tx, rx) = mpsc::channel::<(usize, RemoteResult<Vec<WireSession>>)>();
    for (i, host) in hosts.iter().cloned().enumerate() {
        let (tx, fetch) = (tx.clone(), fetch.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fetch(&host)))
                .unwrap_or_else(|_| Err(RemoteError::protocol("the session lookup panicked")));
            let _ = tx.send((i, r));
        });
    }
    drop(tx);
    let end = Instant::now() + deadline;
    let mut results: Vec<Option<RemoteResult<Vec<WireSession>>>> = hosts.iter().map(|_| None).collect();
    let mut pending = hosts.len();
    while pending > 0 {
        let Some(left) = end.checked_duration_since(Instant::now()) else { break };
        match rx.recv_timeout(left) {
            Ok((i, r)) => {
                results[i] = Some(r);
                pending -= 1;
            }
            Err(_) => break,
        }
    }
    hosts
        .iter()
        .zip(results)
        .map(|(host, result)| {
            let result = result.unwrap_or_else(|| Err(RemoteError::unreachable("timed out")));
            let (status, error, mut sessions) = match result {
                Ok(list) => {
                    let sessions = list.into_iter().filter(WireSession::usable).take(MAX_SESSIONS).map(|w| RemoteSession::from_wire(w, host)).collect();
                    (HostSessionsStatus::Ok, None, sessions)
                }
                Err(e) => (status_of(&e), Some(clean(&e.message, MAX_ERROR)), Vec::new()),
            };
            sessions.sort_by_key(|s: &RemoteSession| std::cmp::Reverse(s.session.updated_at));
            HostSessions { host_id: host.id, host_name: host.name.clone(), status, error, sessions }
        })
        .collect()
}

/// Ask one host. An unreachable host is the tunnel monitor's to retry (it backs off), so a poll
/// reports that state instead of starting an ssh attempt every few seconds.
pub fn fetch(rt: &RemoteRuntime, host: &Host) -> RemoteResult<Vec<WireSession>> {
    match rt.tunnels.state(host.id) {
        ConnState::Unreachable { error } => return Err(RemoteError::unreachable(error)),
        // Another lookup is dialling it: say so instead of queueing behind its lock.
        ConnState::Connecting => return Err(RemoteError::unreachable("still connecting")),
        _ => {}
    }
    rt.client(host.id)?.sessions()
}

/// Hosts whose previous lookup has not ended (a stalled host): not asked again meanwhile, so a bad
/// host cannot pile up threads poll after poll.
fn in_flight() -> &'static Mutex<HashSet<i64>> {
    static SET: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

struct Flight(i64);

impl Flight {
    fn enter(id: i64) -> Option<Flight> {
        in_flight().lock().unwrap().insert(id).then_some(Flight(id))
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        in_flight().lock().unwrap().remove(&self.0);
    }
}

/// Every enabled host's sessions, via the app's runtime. Disabled hosts are left out.
pub fn collect_all(rt: Arc<RemoteRuntime>, hosts: Vec<Host>) -> Vec<HostSessions> {
    collect(enabled(hosts), DEADLINE, Arc::new(move |h: &Host| {
        let Some(_flight) = Flight::enter(h.id) else {
            return Err(RemoteError::unreachable("the previous lookup is still running"));
        };
        fetch(&rt, h)
    }))
}

fn enabled(hosts: Vec<Host>) -> Vec<Host> {
    hosts.into_iter().filter(|h| h.enabled).collect()
}

impl RemoteClient {
    /// `GET /v1/sessions`: the live sessions of the host's harnesses.
    pub fn sessions(&self) -> RemoteResult<Vec<WireSession>> {
        #[derive(Deserialize)]
        struct Sessions {
            sessions: Vec<WireSession>,
        }
        let limits = super::http::Limits { max_body: MAX_BODY, total: Some(SESSIONS_TIMEOUT) };
        Ok(self.get_limited::<Sessions>("/v1/sessions", READ_TIMEOUT, limits)?.sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: i64, name: &str) -> Host {
        Host {
            id,
            name: name.into(),
            kind: "ssh".into(),
            ssh_target: format!("{name}-alias"),
            remote_endpoint: "/run/x.sock".into(),
            enabled: true,
            last_seen: None,
            last_protocol: None,
            last_version: None,
        }
    }

    fn wire(id: &str, updated_at: i64) -> WireSession {
        serde_json::from_value(serde_json::json!({
            "harness": "claude", "session_id": id, "live": true, "cwd": "/home/x/app", "updated_at": updated_at,
            "tmux": {"session": "0", "window": "@1", "pane": "%3", "label": "0:1"},
            "pid": 4, "summaries": [], "something_new": true
        }))
        .unwrap()
    }

    #[test]
    fn one_reachable_and_one_down_host_give_rows_in_name_order_without_failing_the_list() {
        let hosts = vec![host(2, "nija"), host(1, "koza")];
        let out = collect(hosts, DEADLINE, Arc::new(|h: &Host| {
            if h.name == "koza" {
                Ok(vec![wire("old", 1), wire("new", 9)])
            } else {
                Err(RemoteError::unreachable("ssh: connection refused"))
            }
        }));
        assert_eq!(out.iter().map(|h| (h.host_name.as_str(), h.status)).collect::<Vec<_>>(), [("koza", HostSessionsStatus::Ok), ("nija", HostSessionsStatus::Unreachable)]);
        assert_eq!(out[0].sessions.iter().map(|s| s.session.session_id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
        assert_eq!(out[1].error.as_deref(), Some("ssh: connection refused"));
        assert!(out[1].sessions.is_empty());
        let s = &out[0].sessions[0];
        assert_eq!((s.host_id, s.host_name.as_str(), s.host_ssh_target.as_str()), (1, "koza", "koza-alias"));
        assert_eq!(s.session.tmux.as_ref().map(|t| t.pane.as_str()), Some("%3"));
    }

    #[test]
    fn errors_map_to_a_status_and_the_dto_carries_the_host_next_to_the_session() {
        let out = collect(vec![host(1, "a"), host(2, "b"), host(3, "c")], DEADLINE, Arc::new(|h: &Host| match h.id {
            1 => Err(RemoteError::new(PROTOCOL_MISMATCH, "host runs protocol 2")),
            2 => Err(RemoteError::disabled("off")),
            _ => Err(RemoteError::protocol("bad json")),
        }));
        assert_eq!(out.iter().map(|h| h.status).collect::<Vec<_>>(), [HostSessionsStatus::ProtocolMismatch, HostSessionsStatus::Disabled, HostSessionsStatus::Error]);
        let ok = collect(vec![host(1, "a")], DEADLINE, Arc::new(|_: &Host| Ok(vec![wire("s", 1)])));
        let v = serde_json::to_value(&ok[0].sessions[0]).unwrap();
        assert_eq!((v["session_id"].as_str(), v["host_id"].as_i64(), v["focus"].as_str(), v["summaries"].as_array().map(Vec::len)), (Some("s"), Some(1), Some("no_terminal"), Some(0)));
        assert_eq!(v["pid"], serde_json::Value::Null);
    }

    #[test]
    fn a_stalled_host_times_out_without_delaying_the_good_one() {
        let started = Instant::now();
        let out = collect(vec![host(1, "good"), host(2, "stuck")], Duration::from_millis(300), Arc::new(|h: &Host| {
            if h.name == "stuck" {
                std::thread::sleep(Duration::from_secs(3));
            }
            Ok(vec![wire("s", 1)])
        }));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!((out[0].status, out[0].sessions.len()), (HostSessionsStatus::Ok, 1));
        assert_eq!((out[1].status, out[1].error.as_deref(), out[1].sessions.len()), (HostSessionsStatus::Unreachable, Some("timed out"), 0));
    }

    #[test]
    fn remote_text_is_cleaned_and_bounded() {
        let mut hostile = wire("id\u{1b}[200~x\nrm", 1);
        hostile.cwd = format!("/a\r\n{}", "d".repeat(5000));
        let many: Vec<WireSession> = std::iter::once(hostile).chain((0..800).map(|i| wire(&format!("s{i}"), i))).collect();
        let out = collect(vec![host(1, "a"), host(2, "b")], DEADLINE, Arc::new(move |h: &Host| {
            if h.id == 1 {
                Ok(many.clone())
            } else {
                Err(RemoteError::unreachable(format!("bad\u{7}\n{}", "e".repeat(1000))))
            }
        }));
        assert_eq!(out[0].sessions.len(), MAX_SESSIONS);
        let h = out[0].sessions.iter().find(|s| s.session.session_id.starts_with("id")).unwrap();
        assert_eq!(h.session.session_id, "id[200~xrm");
        assert!(h.session.cwd.len() <= MAX_CWD && !h.session.cwd.contains('\n'));
        let e = out[1].error.as_deref().unwrap();
        assert!(e.chars().count() <= MAX_ERROR && !e.chars().any(char::is_control));
    }

    #[test]
    fn disabled_hosts_are_not_asked() {
        let mut off = host(1, "off");
        off.enabled = false;
        let on = enabled(vec![off, host(2, "on")]);
        assert_eq!(on.iter().map(|h| h.id).collect::<Vec<_>>(), [2]);
    }
}

#[cfg(all(test, feature = "worker", unix))]
mod server_tests {
    use super::*;
    use crate::remote::test_server::TestServer;

    #[test]
    fn the_sessions_of_a_worker_come_through_the_tunnel() {
        let srv = TestServer::start(1);
        let rt = srv.runtime();
        let h = rt.host(1).unwrap();
        let out = collect_all(rt.clone(), vec![h]);
        assert_eq!(out[0].status, HostSessionsStatus::Ok, "{:?}", out[0].error);
        let s = &out[0].sessions[0];
        assert_eq!((s.session.harness.as_str(), s.session.session_id.as_str(), s.host_name.as_str()), ("fake", "fake-1", "koza"));
        assert_eq!(s.session.repo.as_deref(), Some("acme/widgets"));
        assert_eq!(s.session.tmux.as_ref().map(|t| t.pane.as_str()), Some("%7"));
        rt.shutdown();
    }
}
