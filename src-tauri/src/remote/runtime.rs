//! The app-wide handle on remote hosts: the tunnel manager plus a cache of what each host last
//! reported on `/v1/info` (its harnesses and capabilities), which the synchronous
//! [`Harness`](crate::ai::Harness) trait needs without a round trip.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::client::{Info, RemoteClient};
use super::error::{RemoteError, RemoteResult};
use super::host::Host;
use super::tunnel::{HostSpec, TunnelManager};
use crate::error::AppResult;

type HostFn = dyn Fn(i64) -> AppResult<Host> + Send + Sync;

pub struct RemoteRuntime {
    pub tunnels: TunnelManager,
    hosts: Box<HostFn>,
    infos: Mutex<HashMap<i64, Info>>,
}

static GLOBAL: OnceLock<Arc<RemoteRuntime>> = OnceLock::new();

/// Install the runtime of the app (once, at startup).
pub fn init(rt: Arc<RemoteRuntime>) {
    let _ = GLOBAL.set(rt);
}

/// The app's runtime; `None` outside the app (the `cr` binary, tests).
pub fn global() -> Option<Arc<RemoteRuntime>> {
    GLOBAL.get().cloned()
}

impl RemoteRuntime {
    pub fn new(tunnels: TunnelManager, hosts: Box<HostFn>) -> Arc<Self> {
        Arc::new(Self { tunnels, hosts, infos: Mutex::new(HashMap::new()) })
    }

    /// The host row, refusing a missing or disabled one.
    pub fn host(&self, host_id: i64) -> RemoteResult<Host> {
        let host = (self.hosts)(host_id).map_err(|e| RemoteError::disabled(e.to_string()))?;
        if !host.enabled {
            return Err(RemoteError::disabled(format!("host “{}” is disabled in Settings → AI → Hosts", host.name)));
        }
        Ok(host)
    }

    fn spec(&self, host: &Host) -> RemoteResult<HostSpec> {
        HostSpec::from_host(host).map_err(|e| RemoteError::disabled(e.to_string()))
    }

    /// A client for the host, opening the tunnel when needed. Fails fast with `host_unreachable`.
    pub fn client(&self, host_id: i64) -> RemoteResult<RemoteClient> {
        let host = self.host(host_id)?;
        let addr = self.tunnels.ensure(&self.spec(&host)?)?;
        Ok(RemoteClient::new(addr))
    }

    /// `/v1/info` of the host (connecting first), remembered for [`cached_info`](Self::cached_info).
    pub fn info(&self, host_id: i64) -> RemoteResult<Info> {
        let info = self.client(host_id)?.info()?;
        self.infos.lock().unwrap().insert(host_id, info.clone());
        Ok(info)
    }

    /// "Test connection": like [`info`](Self::info), and a protocol mismatch is looked at again.
    pub fn test(&self, host_id: i64) -> RemoteResult<Info> {
        let host = self.host(host_id)?;
        let addr = self.tunnels.retest(&self.spec(&host)?)?;
        let info = RemoteClient::new(addr).info()?;
        self.infos.lock().unwrap().insert(host_id, info.clone());
        Ok(info)
    }

    pub fn cached_info(&self, host_id: i64) -> Option<Info> {
        self.infos.lock().unwrap().get(&host_id).cloned()
    }

    /// Drop the tunnel and the cache of a host (it was edited or removed).
    pub fn forget(&self, host_id: i64) {
        self.tunnels.forget(host_id);
        self.infos.lock().unwrap().remove(&host_id);
    }

    pub fn shutdown(&self) {
        self.tunnels.shutdown();
    }
}
