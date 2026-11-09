//! An in-process `cr serve` on a unix socket for the client tests: a fake harness (`sh -c <model>`
//! for runs, an echo for one-shots) and a registered `acme/widgets` repo. No network, no real remote.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use std::sync::{Arc, Mutex};

use super::host::{save, HostInput};
use super::runtime::RemoteRuntime;
use super::tunnel::{Addr, SshConfig, TunnelManager};
use crate::ai::{Access, Caps, Event as AiEvent, Harness, Interactive, OneShot, StreamRun, TurnResult};
use crate::error::{AppError, AppResult};
use crate::serve::http::{build, SharedState, Settings, ONESHOT_TIMEOUT};

pub struct Fake;

impl Harness for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn name(&self) -> &'static str {
        "Fake"
    }
    fn caps(&self) -> Caps {
        Caps {
            sessions: false,
            resume: false,
            fork: false,
            tools: true,
            tool_allowlist: false,
            streaming: true,
            interactive_tui: false,
            reports_cost: false,
            replaces_system_prompt: false,
            access_levels: vec![Access::ReadOnly, Access::Edit],
            pins_session_id: false,
        }
    }
    fn available(&self) -> bool {
        true
    }
    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
        if req.prompt == "fail" {
            return Err(AppError::Other("the harness failed".into()));
        }
        Ok(TurnResult { text: format!("echo:{}@{}", req.prompt, req.cwd.display()), input_tokens: 3, output_tokens: 4, cost_usd: 0.5, session_id: None })
    }
    fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(run.req.model).current_dir(run.req.cwd);
        for (k, v) in &run.req.env {
            cmd.env(k, v);
        }
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
    fn normalize(&self, line: &str) -> Vec<AiEvent> {
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v["type"] == "result" => {
                vec![AiEvent::Final { ok: true, text: v["result"].as_str().unwrap_or_default().to_string(), usage: Default::default() }]
            }
            Ok(v) => vec![AiEvent::Other { kind: "x".into(), raw: v }],
            Err(_) => vec![AiEvent::Other { kind: "raw".into(), raw: Value::Null }],
        }
    }
    fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
        false
    }
    fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
        None
    }
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

pub struct TestServer {
    pub addr: Addr,
    pub state: SharedState,
    #[allow(dead_code)]
    pub repo: PathBuf,
    _dir: tempfile::TempDir,
}

impl TestServer {
    pub fn start(max_concurrency: i64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("widgets");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["remote", "add", "origin", "https://github.com/acme/widgets.git"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let app_db = dir.path().join("app.db");
        let conn = crate::db::open(&app_db).unwrap();
        crate::commands::repo::add_repository_impl(&conn, repo.to_str().unwrap()).unwrap();
        drop(conn);
        let harness: &'static dyn Harness = Box::leak(Box::new(Fake));
        let state = build(
            Settings {
                app_db,
                serve_db_path: dir.path().join("serve.db"),
                data_dir: dir.path().to_path_buf(),
                max_concurrency,
                oneshot_timeout: ONESHOT_TIMEOUT,
                worktree_base: dir.path().join("wt"),
                pr_head: Box::new(|_, _| Err(AppError::Other("no gh in tests".into()))),
            },
            vec![harness],
        )
        .unwrap();
        let sock = dir.path().join("s.sock");
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let bound = rt.block_on(crate::serve::daemon::Bound::bind(&crate::serve::Listen::Unix(sock.clone()))).unwrap();
        crate::serve::daemon::start_engine(&state);
        let st = state.clone();
        std::thread::spawn(move || {
            rt.block_on(crate::serve::daemon::serve(st, bound, std::future::pending::<()>())).ok();
        });
        std::thread::sleep(Duration::from_millis(50));
        TestServer { addr: Addr::Unix(sock), state, repo, _dir: dir }
    }
}

impl TestServer {
    /// A runtime with host 1 (`koza`) whose tunnel is a fake `ssh` that only sleeps; the tunnel's local
    /// socket is a link to this server's, so the whole client path runs for real except the network.
    pub fn runtime(&self) -> Arc<RemoteRuntime> {
        use std::os::unix::fs::PermissionsExt;
        let dir = self._dir.path().join("tun");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-ssh");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 300\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = SshConfig::new(dir.clone());
        cfg.program = script.into_os_string();
        cfg.ready_timeout = Duration::from_secs(3);
        let Addr::Unix(sock) = self.addr.clone() else { unreachable!() };
        std::os::unix::fs::symlink(sock, dir.join(format!("h1-{}.sock", std::process::id()))).unwrap();
        let tunnels = TunnelManager::new(cfg, Arc::new(super::client::InfoHealth), Box::new(|_, _| {}));
        let conn = crate::db::open_memory();
        save(&conn, HostInput { id: None, name: "koza".into(), ssh_target: "koza".into(), remote_endpoint: "/run/x.sock".into(), enabled: true }).unwrap();
        let conn = Mutex::new(conn);
        RemoteRuntime::new(tunnels, Box::new(move |id| super::host::require(&conn.lock().unwrap(), id)))
    }
}
