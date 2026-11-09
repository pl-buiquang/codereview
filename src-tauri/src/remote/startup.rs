//! Wires the remote runtime into the app: the tunnel manager with its state events and the monitor.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::client::InfoHealth;
use super::host;
use super::runtime::{self, RemoteRuntime};
use super::tunnel::{private_dir, ConnState, SshConfig, TunnelManager};
use crate::db::Db;

/// The monitor's pass interval.
const MONITOR_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Serialize, Clone)]
struct HostStateEvent {
    host_id: i64,
    state: ConnState,
}

/// Create and install the runtime. Call once, after the database is managed.
pub fn init(app: &AppHandle) {
    let dir = match private_dir() {
        Ok(d) => d,
        Err(e) => {
            log::warn!(target: "remote", "remote hosts are off: {e}");
            return;
        }
    };
    let on_state = {
        let app = app.clone();
        Box::new(move |host_id: i64, state: &ConnState| {
            match state {
                ConnState::Connected { protocol, version } => {
                    if let Some(db) = app.try_state::<Db>() {
                        host::record_seen(&db.0.lock().unwrap(), host_id, *protocol, version);
                    }
                }
                ConnState::ProtocolMismatch { remote_protocol, remote_version, .. } => {
                    if let Some(db) = app.try_state::<Db>() {
                        host::record_mismatch(&db.0.lock().unwrap(), host_id, *remote_protocol, remote_version);
                    }
                }
                _ => {}
            }
            log::info!(target: "remote", "host {host_id}: {state:?}");
            let _ = app.emit("host-state", HostStateEvent { host_id, state: state.clone() });
        })
    };
    let tunnels = TunnelManager::new(SshConfig::new(dir), Arc::new(InfoHealth), on_state);
    let hosts = {
        let app = app.clone();
        Box::new(move |id: i64| {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            host::require(&conn, id)
        })
    };
    tunnels.start_monitor(MONITOR_INTERVAL);
    runtime::init(RemoteRuntime::new(tunnels, hosts));
}
