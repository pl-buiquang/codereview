//! The SSH tunnel to a host's `cr serve` daemon and its connection state.
//!
//! The app opens the tunnel with the system `ssh -N -L ...` (no embedded SSH, no stored credential:
//! `BatchMode` keeps it from ever prompting), on demand, and keeps it alive while a host is in use:
//! a monitor thread health-checks `/v1/info` and restarts a failed tunnel with a backoff. A remote
//! unix socket is forwarded to a local unix socket; where that is not possible (a port endpoint, a
//! platform without unix sockets) a loopback port is used.
//!
//! The HTTP side is behind [`Health`], so the state machine is tested with a fake `ssh` script and a
//! fake health check, without any network.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::error::{RemoteError, RemoteResult};
use super::host::{Host, RemoteEndpoint};
use crate::error::AppResult;
use crate::serve::PROTOCOL;

/// Where the local end of a tunnel listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Addr {
    Unix(PathBuf),
    /// A loopback port.
    Tcp(u16),
}

/// What a host reports on `/v1/info` that the tunnel cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthInfo {
    pub protocol: i64,
    pub version: String,
}

/// The health check of a tunnel: `GET /v1/info` in production.
pub trait Health: Send + Sync {
    fn check(&self, addr: &Addr) -> RemoteResult<HealthInfo>;
}

/// The connection state of a host, as shown in Settings and the run header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnState {
    /// Not used (yet) in this session.
    Disconnected,
    Connecting,
    Connected { protocol: i64, version: String },
    /// The last attempt or health check failed; the monitor retries while the host is in use.
    Unreachable { error: String },
    /// The host speaks another protocol. Sticky until the host is tested again.
    ProtocolMismatch { remote_protocol: i64, remote_version: String, local_protocol: u32 },
}

impl ConnState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnState::Connected { .. })
    }
}

/// What the tunnel needs to know about a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    pub id: i64,
    pub ssh_target: String,
    pub remote: RemoteEndpoint,
}

impl HostSpec {
    pub fn from_host(host: &Host) -> AppResult<Self> {
        Ok(Self { id: host.id, ssh_target: host.ssh_target.clone(), remote: super::host::parse_endpoint(&host.remote_endpoint)? })
    }
}

/// How the tunnel process is started.
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// The `ssh` binary (a fake script in tests).
    pub program: OsString,
    /// A private directory for local sockets and ssh's stderr.
    pub dir: PathBuf,
    /// How long a fresh tunnel may take to answer `/v1/info`.
    pub ready_timeout: Duration,
    /// First retry delay of the monitor; doubles per failure up to [`SshConfig::backoff_max`].
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl SshConfig {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            program: "ssh".into(),
            dir,
            ready_timeout: Duration::from_secs(15),
            backoff_base: Duration::from_secs(2),
            backoff_max: Duration::from_secs(60),
        }
    }
}

/// The delay before retry number `failures` (1-based): `base * 2^(failures-1)`, capped.
pub fn backoff(failures: u32, base: Duration, max: Duration) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    base.saturating_mul(1u32 << shift).min(max)
}

/// The `ssh` arguments of a tunnel from `local` to the host's daemon. Nothing here can prompt, and
/// the user's `ControlMaster` setup is bypassed so the forward belongs to (and dies with) this process.
pub fn ssh_args(spec: &HostSpec, local: &Addr) -> Vec<String> {
    let forward = match (local, &spec.remote) {
        (Addr::Unix(l), RemoteEndpoint::Socket(r)) => format!("{}:{r}", l.display()),
        (Addr::Unix(l), RemoteEndpoint::Port(p)) => format!("{}:127.0.0.1:{p}", l.display()),
        (Addr::Tcp(lp), RemoteEndpoint::Socket(r)) => format!("127.0.0.1:{lp}:{r}"),
        (Addr::Tcp(lp), RemoteEndpoint::Port(p)) => format!("127.0.0.1:{lp}:127.0.0.1:{p}"),
    };
    let mut args: Vec<String> = [
        "-N",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
        "-o",
        "StreamLocalBindUnlink=yes",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push("-L".into());
    args.push(forward);
    args.push("--".into());
    args.push(spec.ssh_target.clone());
    args
}

/// A private directory for the local ends of tunnels: owner-only and owned by us, as a predictable
/// path under a shared temp dir must be.
pub fn private_dir() -> AppResult<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let uid = unsafe { libc::geteuid() };
    let base = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => std::env::temp_dir(),
    };
    let dir = base.join(format!("codereview-tunnels-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(crate::error::AppError::Other(format!("{} is not a directory of yours; refusing to put sockets there", dir.display())));
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

struct Entry {
    spec: HostSpec,
    child: Option<Child>,
    addr: Option<Addr>,
    state: ConnState,
    failures: u32,
    retry_at: Option<Instant>,
    /// A host that was asked for since the app started: the monitor keeps it connected.
    in_use: bool,
}

type StateFn = dyn Fn(i64, &ConnState) + Send + Sync;

struct Inner {
    cfg: SshConfig,
    health: Arc<dyn Health>,
    hosts: Mutex<HashMap<i64, Arc<Mutex<Entry>>>>,
    on_state: Box<StateFn>,
    stop: AtomicBool,
}

/// Opens, watches and closes the tunnels of all hosts.
#[derive(Clone)]
pub struct TunnelManager {
    inner: Arc<Inner>,
}

impl TunnelManager {
    pub fn new(cfg: SshConfig, health: Arc<dyn Health>, on_state: Box<StateFn>) -> Self {
        Self { inner: Arc::new(Inner { cfg, health, hosts: Mutex::new(HashMap::new()), on_state, stop: AtomicBool::new(false) }) }
    }

    fn entry(&self, spec: &HostSpec) -> Arc<Mutex<Entry>> {
        let mut hosts = self.inner.hosts.lock().unwrap();
        let entry = hosts.entry(spec.id).or_insert_with(|| {
            Arc::new(Mutex::new(Entry {
                spec: spec.clone(),
                child: None,
                addr: None,
                state: ConnState::Disconnected,
                failures: 0,
                retry_at: None,
                in_use: false,
            }))
        });
        entry.clone()
    }

    fn set_state(&self, id: i64, e: &mut Entry, state: ConnState) {
        if e.state != state {
            e.state = state.clone();
            (self.inner.on_state)(id, &state);
        }
    }

    /// The connection state of a host (`Disconnected` when it was never used).
    pub fn state(&self, host_id: i64) -> ConnState {
        let entry = self.inner.hosts.lock().unwrap().get(&host_id).cloned();
        entry.map_or(ConnState::Disconnected, |e| e.lock().unwrap().state.clone())
    }

    /// The local address of a connected host, if any (no connection attempt).
    pub fn addr_if_connected(&self, host_id: i64) -> Option<Addr> {
        let entry = self.inner.hosts.lock().unwrap().get(&host_id).cloned()?;
        let e = entry.lock().unwrap();
        if e.state.is_connected() { e.addr.clone() } else { None }
    }

    /// The local address to talk to `spec`, connecting first when needed. Fails fast with
    /// `host_unreachable` (nothing is retried here: the monitor and the user's relaunch do that), and
    /// with `protocol_mismatch` while the host is known to speak another protocol.
    pub fn ensure(&self, spec: &HostSpec) -> RemoteResult<Addr> {
        let entry = self.entry(spec);
        let mut e = entry.lock().unwrap();
        e.in_use = true;
        if e.spec != *spec {
            // The host was edited: the old tunnel points at the old place.
            self.close(&mut e);
            e.spec = spec.clone();
            e.failures = 0;
            e.retry_at = None;
            self.set_state(spec.id, &mut e, ConnState::Disconnected);
        }
        if let ConnState::ProtocolMismatch { remote_protocol, remote_version, local_protocol } = &e.state {
            return Err(mismatch_error(*remote_protocol, remote_version, *local_protocol));
        }
        if e.state.is_connected() && self.alive(&mut e) {
            return Ok(e.addr.clone().expect("a connected tunnel has an address"));
        }
        self.connect(spec.id, &mut e)
    }

    /// Like [`ensure`](Self::ensure) but also leaves a protocol mismatch behind: the "Test
    /// connection" button, after the user updated the remote.
    pub fn retest(&self, spec: &HostSpec) -> RemoteResult<Addr> {
        let entry = self.entry(spec);
        {
            let mut e = entry.lock().unwrap();
            if matches!(e.state, ConnState::ProtocolMismatch { .. }) {
                self.set_state(spec.id, &mut e, ConnState::Disconnected);
            }
        }
        self.ensure(spec)
    }

    /// Whether the tunnel process is still running.
    fn alive(&self, e: &mut Entry) -> bool {
        matches!(e.child.as_mut().map(Child::try_wait), Some(Ok(None)))
    }

    fn connect(&self, id: i64, e: &mut Entry) -> RemoteResult<Addr> {
        self.close(e);
        self.set_state(id, e, ConnState::Connecting);
        match self.open(e) {
            Ok((child, addr, info)) => {
                e.child = Some(child);
                e.addr = Some(addr.clone());
                e.failures = 0;
                e.retry_at = None;
                self.set_state(id, e, ConnState::Connected { protocol: info.protocol, version: info.version });
                Ok(addr)
            }
            Err(Failed::Mismatch(info)) => {
                self.close(e);
                self.set_state(
                    id,
                    e,
                    ConnState::ProtocolMismatch { remote_protocol: info.protocol, remote_version: info.version.clone(), local_protocol: PROTOCOL },
                );
                Err(mismatch_error(info.protocol, &info.version, PROTOCOL))
            }
            Err(Failed::Unreachable(msg)) => {
                self.close(e);
                self.fail(id, e, msg.clone(), Instant::now());
                Err(RemoteError::unreachable(msg))
            }
        }
    }

    fn fail(&self, id: i64, e: &mut Entry, error: String, now: Instant) {
        e.failures = e.failures.saturating_add(1);
        e.retry_at = Some(now + backoff(e.failures, self.inner.cfg.backoff_base, self.inner.cfg.backoff_max));
        self.set_state(id, e, ConnState::Unreachable { error });
    }

    /// Start ssh and wait until the daemon answers through it.
    fn open(&self, e: &mut Entry) -> Result<(Child, Addr, HealthInfo), Failed> {
        let cfg = &self.inner.cfg;
        let addr = local_addr(&cfg.dir, &e.spec).map_err(|m| Failed::Unreachable(m.to_string()))?;
        let stderr_path = cfg.dir.join(format!("ssh-{}.log", e.spec.id));
        let stderr = std::fs::File::create(&stderr_path).map_err(|m| Failed::Unreachable(m.to_string()))?;
        let mut spawned = None;
        for attempt in 0..5 {
            // ETXTBSY (26): a just-written executable (a script) is briefly busy while another thread forks.
            match Command::new(&cfg.program)
                .args(ssh_args(&e.spec, &addr))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(stderr.try_clone().map_err(|m| Failed::Unreachable(m.to_string()))?)
                .spawn()
            {
                Err(err) if err.raw_os_error() == Some(26) && attempt < 4 => std::thread::sleep(Duration::from_millis(20)),
                other => {
                    spawned = Some(other);
                    break;
                }
            }
        }
        let mut child = spawned
            .expect("the loop always ends with a result")
            .map_err(|err| Failed::Unreachable(format!("cannot run `{}`: {err}", cfg.program.to_string_lossy())))?;
        let deadline = Instant::now() + cfg.ready_timeout;
        let mut last;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let tail = last_lines(&stderr_path, 3);
                return Err(Failed::Unreachable(if tail.is_empty() { format!("ssh exited with {status}") } else { tail }));
            }
            match self.inner.health.check(&addr) {
                Ok(info) if info.protocol == i64::from(PROTOCOL) => return Ok((child, addr, info)),
                Ok(info) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failed::Mismatch(info));
                }
                Err(err) => last = err.message,
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let tail = last_lines(&stderr_path, 3);
                return Err(Failed::Unreachable(if tail.is_empty() { format!("timed out waiting for the daemon through ssh ({last})") } else { tail }));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn close(&self, e: &mut Entry) {
        if let Some(mut child) = e.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(Addr::Unix(p)) = e.addr.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Close the tunnel of a host (it was deleted, edited or disabled) and forget its state.
    pub fn forget(&self, host_id: i64) {
        let entry = self.inner.hosts.lock().unwrap().remove(&host_id);
        if let Some(entry) = entry {
            let mut e = entry.lock().unwrap();
            self.close(&mut e);
            self.set_state(host_id, &mut e, ConnState::Disconnected);
        }
    }

    /// One monitor pass at `now`: a connected host whose tunnel died or stopped answering becomes
    /// unreachable; an unreachable one whose backoff elapsed is reconnected.
    pub fn tick_at(&self, now: Instant) {
        let entries: Vec<(i64, Arc<Mutex<Entry>>)> =
            self.inner.hosts.lock().unwrap().iter().map(|(id, e)| (*id, e.clone())).collect();
        for (id, entry) in entries {
            let mut e = entry.lock().unwrap();
            if !e.in_use {
                continue;
            }
            match e.state.clone() {
                ConnState::Connected { .. } => {
                    let died = !self.alive(&mut e);
                    let unhealthy = !died && e.addr.as_ref().is_none_or(|a| self.inner.health.check(a).is_err());
                    if died || unhealthy {
                        let why = if died { last_lines(&self.inner.cfg.dir.join(format!("ssh-{id}.log")), 3) } else { String::new() };
                        let error = if !why.is_empty() { why } else if died { "the ssh tunnel exited".into() } else { "the daemon stopped answering".into() };
                        self.close(&mut e);
                        self.fail(id, &mut e, error, now);
                    }
                }
                ConnState::Unreachable { .. } if e.retry_at.is_some_and(|t| t <= now) => {
                    let _ = self.connect(id, &mut e);
                }
                _ => {}
            }
        }
    }

    pub fn tick(&self) {
        self.tick_at(Instant::now());
    }

    /// Run [`tick`](Self::tick) every `interval` on a background thread until [`shutdown`](Self::shutdown).
    pub fn start_monitor(&self, interval: Duration) {
        let me = self.clone();
        std::thread::spawn(move || {
            let step = Duration::from_millis(200).min(interval);
            let mut waited = Duration::ZERO;
            while !me.inner.stop.load(Ordering::Relaxed) {
                std::thread::sleep(step);
                waited += step;
                if waited >= interval {
                    waited = Duration::ZERO;
                    me.tick();
                }
            }
        });
    }

    /// Close every tunnel and stop the monitor (app exit).
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        let ids: Vec<i64> = self.inner.hosts.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.forget(id);
        }
    }
}

enum Failed {
    Unreachable(String),
    Mismatch(HealthInfo),
}

fn mismatch_error(remote_protocol: i64, remote_version: &str, local_protocol: u32) -> RemoteError {
    RemoteError::new(
        super::error::PROTOCOL_MISMATCH,
        format!(
            "the host runs cr {remote_version} (protocol {remote_protocol}) but this app speaks protocol {local_protocol}: update `cr` on the host to the same version as the app, restart its daemon, then test the connection again"
        ),
    )
}

/// The local end for `spec`: a unix socket in `dir`, or a free loopback port.
fn local_addr(dir: &Path, spec: &HostSpec) -> std::io::Result<Addr> {
    if matches!(spec.remote, RemoteEndpoint::Socket(_)) || cfg!(unix) {
        // One socket per host and process: a stale one is replaced by ssh (`StreamLocalBindUnlink`).
        let path = dir.join(format!("h{}-{}.sock", spec.id, std::process::id()));
        if path.as_os_str().len() < 100 {
            return Ok(Addr::Unix(path));
        }
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(Addr::Tcp(listener.local_addr()?.port()))
}

/// The last `n` non-empty lines of a file, joined (ssh's complaint when it fails).
fn last_lines(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join(" / ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn spec(remote: RemoteEndpoint) -> HostSpec {
        HostSpec { id: 1, ssh_target: "koza".into(), remote }
    }

    #[test]
    fn a_remote_socket_is_forwarded_to_a_local_socket() {
        let args = ssh_args(&spec(RemoteEndpoint::Socket("/run/user/1000/codereview/serve.sock".into())), &Addr::Unix("/tmp/t/h1.sock".into()));
        let l = args.iter().position(|a| a == "-L").unwrap();
        assert_eq!(args[l + 1], "/tmp/t/h1.sock:/run/user/1000/codereview/serve.sock");
        // the target is last, after `--`, so it can never be read as an option
        assert_eq!(&args[args.len() - 2..], ["--", "koza"]);
        assert_eq!(&args[..2], ["-N", "-T"]);
        for opt in ["BatchMode=yes", "ExitOnForwardFailure=yes", "ControlPath=none", "ControlMaster=no", "StreamLocalBindUnlink=yes"] {
            assert!(args.contains(&opt.to_string()), "{opt}");
        }
    }

    #[test]
    fn ports_and_the_loopback_fallback_build_the_other_forwards() {
        let at = |remote, local| {
            let args = ssh_args(&spec(remote), &local);
            args[args.iter().position(|a| a == "-L").unwrap() + 1].clone()
        };
        assert_eq!(at(RemoteEndpoint::Port(7777), Addr::Unix("/t/s.sock".into())), "/t/s.sock:127.0.0.1:7777");
        assert_eq!(at(RemoteEndpoint::Port(7777), Addr::Tcp(5000)), "127.0.0.1:5000:127.0.0.1:7777");
        assert_eq!(at(RemoteEndpoint::Socket("/r/s.sock".into()), Addr::Tcp(5000)), "127.0.0.1:5000:/r/s.sock");
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let (b, m) = (Duration::from_secs(2), Duration::from_secs(60));
        let secs: Vec<u64> = (0..8).map(|n| backoff(n, b, m).as_secs()).collect();
        assert_eq!(secs, [2, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(backoff(u32::MAX, b, m), m);
    }

    // ---- state machine with a fake ssh ------------------------------------------------------

    struct FakeHealth {
        up: AtomicBool,
        protocol: Mutex<i64>,
        checks: AtomicUsize,
    }

    impl FakeHealth {
        fn new() -> Arc<Self> {
            Arc::new(Self { up: AtomicBool::new(true), protocol: Mutex::new(PROTOCOL as i64), checks: AtomicUsize::new(0) })
        }
    }

    impl Health for FakeHealth {
        fn check(&self, _: &Addr) -> RemoteResult<HealthInfo> {
            self.checks.fetch_add(1, Ordering::Relaxed);
            if self.up.load(Ordering::Relaxed) {
                Ok(HealthInfo { protocol: *self.protocol.lock().unwrap(), version: "9.9.9".into() })
            } else {
                Err(RemoteError::unreachable("connection refused"))
            }
        }
    }

    struct Rig {
        dir: tempfile::TempDir,
        mgr: TunnelManager,
        health: Arc<FakeHealth>,
        states: Arc<Mutex<Vec<ConnState>>>,
    }

    impl Rig {
        /// The fake ssh logs asynchronously; wait until it has logged `n` starts.
        fn started(&self, n: usize) {
            for _ in 0..200 {
                if self.log().lines().count() >= n {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("fake ssh did not log {n} start(s): {:?}", self.log());
        }

        fn log(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("ssh.log")).unwrap_or_default()
        }
    }

    /// A fake `ssh`: logs its arguments, fails for a target named `bad`, otherwise stays up.
    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-ssh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" >> {log}\nfor a in \"$@\"; do last=$a; done\nif [ \"$last\" = bad ]; then echo 'ssh: Could not resolve hostname bad' >&2; exit 255; fi\nexec sleep 300\n",
                log = dir.path().join("ssh.log").display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = SshConfig::new(dir.path().to_path_buf());
        cfg.program = script.into_os_string();
        cfg.ready_timeout = Duration::from_millis(500);
        let health = FakeHealth::new();
        let states = Arc::new(Mutex::new(Vec::new()));
        let sink = states.clone();
        let mgr = TunnelManager::new(cfg, health.clone(), Box::new(move |_, s| sink.lock().unwrap().push(s.clone())));
        Rig { dir, mgr, health, states }
    }

    fn connected() -> ConnState {
        ConnState::Connected { protocol: PROTOCOL as i64, version: "9.9.9".into() }
    }

    #[test]
    fn ensure_connects_once_and_reuses_the_tunnel() {
        let r = rig();
        let s = spec(RemoteEndpoint::Socket("/r/serve.sock".into()));
        let a = r.mgr.ensure(&s).unwrap();
        let b = r.mgr.ensure(&s).unwrap();
        assert_eq!(a, b);
        r.started(1);
        assert!(matches!(a, Addr::Unix(_)));
        assert_eq!(r.log().lines().count(), 1, "ssh is started once");
        assert!(r.log().contains("-L") && r.log().contains(":/r/serve.sock"));
        assert_eq!(*r.states.lock().unwrap(), vec![ConnState::Connecting, connected()]);
        assert_eq!(r.mgr.state(1), connected());
        r.mgr.shutdown();
        assert_eq!(r.mgr.state(1), ConnState::Disconnected);
    }

    #[test]
    fn a_failing_ssh_is_unreachable_with_its_own_message() {
        let r = rig();
        r.health.up.store(false, Ordering::Relaxed);
        let mut s = spec(RemoteEndpoint::Port(7777));
        s.ssh_target = "bad".into();
        let err = r.mgr.ensure(&s).unwrap_err();
        assert_eq!(err.code, "host_unreachable");
        assert!(err.message.contains("Could not resolve hostname bad"), "{err}");
        assert!(matches!(r.mgr.state(1), ConnState::Unreachable { .. }));
    }

    #[test]
    fn a_daemon_that_never_answers_times_out_as_unreachable() {
        let r = rig();
        r.health.up.store(false, Ordering::Relaxed);
        let err = r.mgr.ensure(&spec(RemoteEndpoint::Port(1))).unwrap_err();
        assert!(err.is_connection() && err.message.contains("timed out"), "{err}");
    }

    #[test]
    fn a_protocol_mismatch_disables_the_host_until_it_is_tested_again() {
        let r = rig();
        let s = spec(RemoteEndpoint::Port(7777));
        *r.health.protocol.lock().unwrap() = 2;
        let err = r.mgr.ensure(&s).unwrap_err();
        assert_eq!(err.code, "protocol_mismatch");
        assert!(err.message.contains("protocol 2") && err.message.contains("9.9.9") && err.message.contains("update"), "{err}");
        assert_eq!(
            r.mgr.state(1),
            ConnState::ProtocolMismatch { remote_protocol: 2, remote_version: "9.9.9".into(), local_protocol: PROTOCOL }
        );
        // sticky: no new ssh while it is known to be wrong
        let started = r.log().lines().count();
        assert_eq!(r.mgr.ensure(&s).unwrap_err().code, "protocol_mismatch");
        assert_eq!(r.log().lines().count(), started);
        // the user updated the remote and tests again
        *r.health.protocol.lock().unwrap() = PROTOCOL as i64;
        r.mgr.retest(&s).unwrap();
        assert_eq!(r.mgr.state(1), connected());
        r.mgr.shutdown();
    }

    #[test]
    fn the_monitor_notices_a_dead_daemon_and_reconnects_after_the_backoff() {
        let r = rig();
        let s = spec(RemoteEndpoint::Port(7777));
        r.mgr.ensure(&s).unwrap();
        r.started(1);
        r.health.up.store(false, Ordering::Relaxed);
        let t0 = Instant::now();
        r.mgr.tick_at(t0);
        assert_eq!(r.mgr.state(1), ConnState::Unreachable { error: "the daemon stopped answering".into() });
        // still inside the backoff: nothing happens, no new ssh
        let started = r.log().lines().count();
        r.mgr.tick_at(t0 + Duration::from_millis(500));
        assert_eq!(r.log().lines().count(), started);
        // after the backoff, with the daemon still down: one more attempt, a longer wait
        r.mgr.tick_at(t0 + Duration::from_secs(3));
        r.started(started + 1);
        assert!(matches!(r.mgr.state(1), ConnState::Unreachable { .. }));
        // the daemon is back: the next elapsed backoff reconnects
        r.health.up.store(true, Ordering::Relaxed);
        r.mgr.tick_at(Instant::now() + Duration::from_secs(60));
        assert_eq!(r.mgr.state(1), connected());
        r.mgr.shutdown();
    }

    #[test]
    fn the_monitor_notices_an_ssh_that_exited() {
        let r = rig();
        let s = spec(RemoteEndpoint::Port(7777));
        r.mgr.ensure(&s).unwrap();
        // kill the fake ssh behind the manager's back
        {
            let entry = r.mgr.inner.hosts.lock().unwrap().get(&1).cloned().unwrap();
            let mut e = entry.lock().unwrap();
            let child = e.child.as_mut().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
        }
        r.mgr.tick_at(Instant::now());
        assert_eq!(r.mgr.state(1), ConnState::Unreachable { error: "the ssh tunnel exited".into() });
        r.mgr.tick_at(Instant::now() + Duration::from_secs(5));
        assert_eq!(r.mgr.state(1), connected());
        r.mgr.shutdown();
    }

    #[test]
    fn hosts_that_were_never_asked_for_are_left_alone_and_edits_reopen_the_tunnel() {
        let r = rig();
        r.mgr.tick();
        assert_eq!(r.log(), "");
        let s = spec(RemoteEndpoint::Port(7777));
        r.mgr.ensure(&s).unwrap();
        r.started(1);
        let mut moved = s.clone();
        moved.remote = RemoteEndpoint::Port(8888);
        r.mgr.ensure(&moved).unwrap();
        r.started(2);
        assert!(r.log().lines().nth(1).unwrap().contains("8888"));
        r.mgr.forget(1);
        assert_eq!(r.mgr.addr_if_connected(1), None);
        r.mgr.shutdown();
    }

    #[test]
    fn private_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = private_dir().unwrap();
        assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
