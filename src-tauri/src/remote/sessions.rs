//! Read-only remote sessions (S10): the live sessions of every enabled host, from `GET /v1/sessions`,
//! shaped like the local ones plus the host they belong to. Only metadata crosses the wire (no
//! transcripts, no summaries); nothing here launches, focuses or stops a remote session.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::client::RemoteClient;
use super::error::{RemoteError, RemoteResult, HOST_DISABLED, HOST_UNREACHABLE, PROTOCOL_MISMATCH};
use super::host::Host;
use super::runtime::RemoteRuntime;
use super::tunnel::ConnState;
use crate::ai::AgentSession;
use crate::claude_sessions::{FocusState, TmuxLoc};

/// A host's answer must come back in this long, or its row says so; the local list never waits for it.
pub const SESSIONS_TIMEOUT: Duration = Duration::from_secs(10);

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
            harness: w.harness,
            session_id: w.session_id,
            pid: None,
            live: w.live,
            status: w.status,
            kind: w.kind,
            attached: w.attached,
            job_id: w.job_id,
            waiting_for: w.waiting_for,
            agent_view: None,
            name: w.name,
            cwd: w.cwd,
            git_branch: w.git_branch,
            repo: w.repo,
            tmux: w.tmux.map(|t| TmuxLoc { session: t.session, window: t.window, pane: t.pane, label: t.label }),
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

/// Why a host has no sessions to show. Serialized as the row's state in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSessionsStatus {
    Ok,
    Connecting,
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

/// The sessions of every host in `hosts` (fetched in parallel, so a slow host does not hold the
/// others up), in host-name order. A host that fails gives a row with its status, never an error.
pub fn collect<F>(mut hosts: Vec<Host>, fetch: F) -> Vec<HostSessions>
where
    F: Fn(&Host) -> RemoteResult<Vec<WireSession>> + Sync,
{
    hosts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    let fetch = &fetch;
    std::thread::scope(|scope| {
        let handles: Vec<_> = hosts.iter().map(|h| scope.spawn(move || fetch(h))).collect();
        hosts
            .iter()
            .zip(handles)
            .map(|(host, handle)| {
                let result = handle.join().unwrap_or_else(|_| Err(RemoteError::protocol("the session lookup panicked")));
                let (status, error, mut sessions) = match result {
                    Ok(list) => (HostSessionsStatus::Ok, None, list.into_iter().map(|w| RemoteSession::from_wire(w, host)).collect()),
                    Err(e) => (status_of(&e), Some(e.message), Vec::new()),
                };
                sessions.sort_by_key(|s: &RemoteSession| std::cmp::Reverse(s.session.updated_at));
                HostSessions { host_id: host.id, host_name: host.name.clone(), status, error, sessions }
            })
            .collect()
    })
}

/// Ask one host. An unreachable host is the tunnel monitor's to retry (it backs off), so a poll
/// reports that state instead of starting an ssh attempt every few seconds.
pub fn fetch(rt: &RemoteRuntime, host: &Host) -> RemoteResult<Vec<WireSession>> {
    match rt.tunnels.state(host.id) {
        ConnState::Unreachable { error } => return Err(RemoteError::unreachable(error)),
        ConnState::Connecting => return Err(RemoteError::unreachable("connecting…")),
        _ => {}
    }
    rt.client(host.id)?.sessions()
}

/// Every enabled host's sessions, via the app's runtime. Disabled hosts are left out.
pub fn collect_all(rt: &RemoteRuntime, hosts: Vec<Host>) -> Vec<HostSessions> {
    collect(enabled(hosts), |h| fetch(rt, h))
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
        Ok(self.get_within::<Sessions>("/v1/sessions", SESSIONS_TIMEOUT)?.sessions)
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
        let out = collect(hosts, |h| {
            if h.name == "koza" {
                Ok(vec![wire("old", 1), wire("new", 9)])
            } else {
                Err(RemoteError::unreachable("ssh: connection refused"))
            }
        });
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
        let out = collect(vec![host(1, "a"), host(2, "b"), host(3, "c")], |h| match h.id {
            1 => Err(RemoteError::new(PROTOCOL_MISMATCH, "host runs protocol 2")),
            2 => Err(RemoteError::disabled("off")),
            _ => Err(RemoteError::protocol("bad json")),
        });
        assert_eq!(out.iter().map(|h| h.status).collect::<Vec<_>>(), [HostSessionsStatus::ProtocolMismatch, HostSessionsStatus::Disabled, HostSessionsStatus::Error]);
        let ok = collect(vec![host(1, "a")], |_| Ok(vec![wire("s", 1)]));
        let v = serde_json::to_value(&ok[0].sessions[0]).unwrap();
        assert_eq!((v["session_id"].as_str(), v["host_id"].as_i64(), v["focus"].as_str(), v["summaries"].as_array().map(Vec::len)), (Some("s"), Some(1), Some("no_terminal"), Some(0)));
        assert_eq!(v["pid"], serde_json::Value::Null);
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
        let out = collect_all(&rt, vec![h]);
        assert_eq!(out[0].status, HostSessionsStatus::Ok, "{:?}", out[0].error);
        let s = &out[0].sessions[0];
        assert_eq!((s.session.harness.as_str(), s.session.session_id.as_str(), s.host_name.as_str()), ("fake", "fake-1", "koza"));
        assert_eq!(s.session.repo.as_deref(), Some("acme/widgets"));
        assert_eq!(s.session.tmux.as_ref().map(|t| t.pane.as_str()), Some("%7"));
        rt.shutdown();
    }
}
