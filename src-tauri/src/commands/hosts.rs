//! Tauri commands for remote hosts (Settings → AI → Hosts).

use serde::Serialize;
use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::remote::client::HarnessInfo;
use crate::remote::host::{self, Host, HostInput};
use crate::remote::runtime::{self, RemoteRuntime};
use crate::remote::sessions::{self, HostSessions};
use crate::remote::tunnel::ConnState;

fn runtime() -> AppResult<std::sync::Arc<RemoteRuntime>> {
    runtime::global().ok_or_else(|| AppError::Other("remote hosts are not ready yet".into()))
}

#[tauri::command]
pub fn host_list(db: State<Db>) -> AppResult<Vec<Host>> {
    host::list(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn host_save(host: HostInput, db: State<Db>) -> AppResult<Host> {
    let saved = host::save(&db.0.lock().unwrap(), host)?;
    // An edit may move the host: the old tunnel and what it reported are stale.
    if let Some(rt) = runtime::global() {
        rt.forget(saved.id);
    }
    Ok(saved)
}

#[tauri::command]
pub fn host_delete(id: i64, db: State<Db>) -> AppResult<()> {
    host::delete(&db.0.lock().unwrap(), id)?;
    if let Some(rt) = runtime::global() {
        rt.forget(id);
    }
    Ok(())
}

/// The connection state of every host (hosts not used yet are `disconnected`).
#[tauri::command]
pub fn host_states(db: State<Db>) -> AppResult<Vec<HostState>> {
    let hosts = host::list(&db.0.lock().unwrap())?;
    let rt = runtime::global();
    Ok(hosts
        .into_iter()
        .map(|h| HostState { host_id: h.id, state: rt.as_ref().map_or(ConnState::Disconnected, |rt| rt.tunnels.state(h.id)) })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct HostState {
    pub host_id: i64,
    pub state: ConnState,
}

/// What "Test connection" shows.
#[derive(Debug, Serialize)]
pub struct HostTest {
    pub protocol: i64,
    pub version: String,
    pub host: String,
    pub harnesses: Vec<String>,
    pub running: i64,
    pub queued: i64,
    pub max_concurrency: i64,
}

/// Open the tunnel (looking again at a protocol mismatch) and ask the host for `/v1/info`. Errors
/// carry the stable code (`host_unreachable`, `protocol_mismatch`, ...) for the UI to explain.
#[tauri::command]
pub async fn host_test(id: i64) -> AppResult<HostTest> {
    let rt = runtime()?;
    let info = tauri::async_runtime::spawn_blocking(move || rt.test(id))
        .await
        .map_err(|e| AppError::Other(format!("connection test failed: {e}")))??;
    Ok(HostTest {
        protocol: info.protocol,
        version: info.version,
        host: info.host,
        harnesses: info.harnesses.iter().filter(|h| h.available).map(|h| h.name.clone()).collect(),
        running: info.load.running,
        queued: info.load.queued,
        max_concurrency: info.max_concurrency,
    })
}

/// The harnesses of a host with their capabilities (for the profile editor of a remote profile).
pub fn remote_harness_infos(host_id: i64) -> AppResult<Vec<HarnessInfo>> {
    Ok(runtime()?.info(host_id)?.harnesses)
}

/// The live sessions of every enabled host, read-only, for the Sessions view (S10). One entry per
/// host; a host that is down is an entry with its status, never an error. Kept apart from
/// `list_claude_sessions` so that a slow host cannot delay the local list and the local counts
/// (sidebar, board cards) stay local.
#[tauri::command]
pub async fn list_remote_sessions(db: State<'_, Db>) -> AppResult<Vec<HostSessions>> {
    let hosts = host::list(&db.0.lock().unwrap())?;
    let Some(rt) = runtime::global() else { return Ok(Vec::new()) };
    if hosts.iter().all(|h| !h.enabled) {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(move || sessions::collect_all(rt, hosts))
        .await
        .map_err(|e| AppError::Other(format!("remote session lookup failed: {e}")))
}
