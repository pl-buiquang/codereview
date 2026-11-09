//! [`RemoteHarness`]: a harness of a remote host, seen through the [`Harness`] trait. It wraps the
//! local implementation of the same harness id (for its static name and line parsing) and talks to the
//! host's `cr worker` instead of starting a process.
//!
//! What it supports on protocol 1: one-shot calls without a session (`POST /v1/oneshot`). Routine
//! runs do not go through `spawn`: the routine launcher posts them and a mirror follows them
//! ([`super::mirror`]). Multi-turn sessions and interactive sessions stay local (no endpoint for them).

use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use super::runtime::{self, RemoteRuntime};
use crate::ai::{
    harness_by_id, Access, Caps, Event, Harness, Interactive, OneShot, SessionMode, StreamRun, SystemPrompt, SystemPromptMode,
    ToolPolicy, TurnResult,
};
use crate::error::{AppError, AppResult};
use crate::run_usage::{now_stamp, RunUsage};
use crate::worker::request::{AccessWire, OneShotRequest, ToolsWire};

pub struct RemoteHarness {
    host_id: i64,
    /// The local implementation of the same harness id: static name, line parsing.
    inner: &'static dyn Harness,
    /// Where the runtime comes from; `None` is the app's global one.
    runtime: Option<Arc<RemoteRuntime>>,
}

impl RemoteHarness {
    pub fn new(host_id: i64, inner: &'static dyn Harness) -> Self {
        Self { host_id, inner, runtime: None }
    }

    pub fn with_runtime(host_id: i64, inner: &'static dyn Harness, runtime: Arc<RemoteRuntime>) -> Self {
        Self { host_id, inner, runtime: Some(runtime) }
    }

    pub fn host_id(&self) -> i64 {
        self.host_id
    }

    fn rt(&self) -> AppResult<Arc<RemoteRuntime>> {
        self.runtime.clone().or_else(runtime::global).ok_or_else(|| AppError::Other("remote hosts are not available here".into()))
    }

    fn host_label(&self) -> String {
        self.rt().ok().and_then(|rt| rt.host(self.host_id).ok()).map_or_else(|| format!("host #{}", self.host_id), |h| h.name)
    }
}

/// The remote harness `harness_id` of `host_id`, one instance per pair for the life of the process
/// (profiles resolve to `&'static dyn Harness`).
pub fn for_host(host_id: i64, harness_id: &str) -> Option<&'static dyn Harness> {
    static CACHE: Mutex<Option<HashMap<(i64, &'static str), &'static RemoteHarness>>> = Mutex::new(None);
    let inner = harness_by_id(harness_id)?;
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    let h: &'static RemoteHarness = cache.entry((host_id, inner.id())).or_insert_with(|| Box::leak(Box::new(RemoteHarness::new(host_id, inner))));
    Some(h)
}

/// The wire form of a tool policy.
fn tools_wire(policy: &ToolPolicy) -> (ToolsWire, Vec<String>) {
    match policy {
        ToolPolicy::None => (ToolsWire::None, Vec::new()),
        ToolPolicy::ReadOnly => (ToolsWire::ReadOnly, Vec::new()),
        ToolPolicy::Edit => (ToolsWire::Edit, Vec::new()),
        ToolPolicy::Full => (ToolsWire::Full, Vec::new()),
        ToolPolicy::Allowlist(rules) => (ToolsWire::Allowlist, rules.clone()),
    }
}

fn access_wire(access: &Access) -> AppResult<AccessWire> {
    Ok(match access {
        Access::Default => AccessWire::Default,
        Access::ReadOnly => AccessWire::ReadOnly,
        Access::Edit => AccessWire::Edit,
        Access::Full => AccessWire::Full,
        Access::Native(mode) => {
            return Err(AppError::Other(format!("a remote host takes a neutral access level, not the native mode `{mode}`")))
        }
    })
}

/// The `/v1/oneshot` request of a one-shot call.
pub fn oneshot_request(harness_id: &str, req: &OneShot<'_>) -> AppResult<OneShotRequest> {
    if req.session != SessionMode::Ephemeral {
        return Err(AppError::Other(
            "a remote host runs one-shot calls without a session: multi-turn chats and session forks stay on this machine".into(),
        ));
    }
    if !req.extra_args.is_empty() {
        return Err(AppError::Other("harness-native extra arguments cannot be sent to a remote host".into()));
    }
    let system_prompt = match req.system_prompt {
        None => String::new(),
        Some(SystemPrompt::Text(t)) => t.to_string(),
        Some(SystemPrompt::File(p)) => std::fs::read_to_string(p)?,
    };
    let (tools, allowed_tools) = tools_wire(&req.tools);
    Ok(OneShotRequest {
        harness: harness_id.to_string(),
        model: req.model.to_string(),
        prompt: req.prompt.to_string(),
        system_prompt,
        system_prompt_replace: req.system_prompt_mode == SystemPromptMode::Replace,
        access: access_wire(&req.access)?,
        tools,
        allowed_tools,
        max_turns: req.max_turns,
        workspace: None,
    })
}

impl Harness for RemoteHarness {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// What the host last reported for this harness, minus what protocol 1 cannot carry (sessions,
    /// interactive use). Before the host was ever reached: the local implementation's caps.
    fn caps(&self) -> Caps {
        let known = self
            .rt()
            .ok()
            .and_then(|rt| rt.cached_info(self.host_id))
            .and_then(|info| info.harnesses.into_iter().find(|h| h.id == self.inner.id()))
            .map(|h| h.caps);
        Caps {
            sessions: false,
            resume: false,
            fork: false,
            interactive_tui: false,
            pins_session_id: false,
            ..known.unwrap_or_else(|| self.inner.caps())
        }
    }

    fn available(&self) -> bool {
        self.unavailable_reason().is_none()
    }

    fn unavailable_reason(&self) -> Option<String> {
        let rt = match self.rt() {
            Ok(rt) => rt,
            Err(e) => return Some(e.to_string()),
        };
        match rt.info(self.host_id) {
            Err(e) => Some(e.to_string()),
            Ok(info) => match info.harnesses.iter().find(|h| h.id == self.inner.id()) {
                Some(h) if h.available => None,
                Some(_) => Some(format!("`{}` is not installed on {}", self.inner.id(), self.host_label())),
                None => Some(format!("{} does not offer the `{}` harness", self.host_label(), self.inner.id())),
            },
        }
    }

    fn version(&self) -> Option<String> {
        self.rt().ok()?.cached_info(self.host_id)?.harnesses.into_iter().find(|h| h.id == self.inner.id())?.version
    }

    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
        let wire = oneshot_request(self.inner.id(), req)?;
        let started = now_stamp();
        let t0 = Instant::now();
        let tag = req.usage_tag.clone().host(Some(self.host_id));
        let outcome = self.rt().and_then(|rt| Ok(rt.client(self.host_id)?.oneshot(&wire)?));
        let record = |usage: &RunUsage, error: Option<&str>| {
            crate::ai_usage::record(&tag, &started, t0.elapsed().as_millis() as i64, usage, error);
        };
        match outcome {
            Err(e) => {
                record(&RunUsage::default(), Some(&e.to_string()));
                Err(e)
            }
            Ok(resp) => {
                let usage = resp.usage.as_ref().map_or_else(RunUsage::default, |u| RunUsage {
                    num_turns: 1,
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                    ..RunUsage::default()
                });
                if let Some(error) = resp.error {
                    record(&usage, Some(&error));
                    return Err(AppError::Other(error));
                }
                record(&usage, None);
                Ok(TurnResult {
                    session_id: None,
                    text: resp.text,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cost_usd: usage.cost_usd,
                })
            }
        }
    }

    fn spawn(&self, _: StreamRun<'_>) -> AppResult<Child> {
        Err(AppError::Other("a remote run is posted to the host and mirrored, it does not start a local process".into()))
    }

    fn parse_line(&self, line: &str) -> (String, Option<Value>) {
        self.inner.parse_line(line)
    }

    fn normalize(&self, line: &str) -> Vec<Event> {
        self.inner.normalize(line)
    }

    fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
        false
    }

    fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
        None
    }

    fn models(&self) -> AppResult<Vec<String>> {
        Ok(self.rt()?.client(self.host_id)?.models(self.inner.id())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ClaudeCode;
    use crate::run_usage::RunTag;
    use std::path::Path;

    fn req<'a>(prompt: &'a str, cwd: &'a Path) -> OneShot<'a> {
        OneShot {
            model: "haiku",
            system_prompt: Some(SystemPrompt::Text("be brief")),
            system_prompt_mode: SystemPromptMode::Replace,
            file_tag: "t",
            prompt,
            cwd,
            extra_dirs: Vec::new(),
            tools: ToolPolicy::Allowlist(vec!["Read".into()]),
            access: Access::ReadOnly,
            session: SessionMode::Ephemeral,
            max_turns: Some(2),
            effort: Some("low"),
            env: vec![("X".into(), "1".into())],
            extra_args: Vec::new(),
            usage_tag: RunTag::new("theme_suggest"),
        }
    }

    #[test]
    fn a_one_shot_becomes_the_neutral_wire_request() {
        let w = oneshot_request("claude", &req("hello", Path::new("/local/only"))).unwrap();
        assert_eq!((w.harness.as_str(), w.model.as_str(), w.prompt.as_str()), ("claude", "haiku", "hello"));
        assert_eq!((w.system_prompt.as_str(), w.system_prompt_replace), ("be brief", true));
        assert_eq!((w.access, w.tools, w.allowed_tools.clone(), w.max_turns), (AccessWire::ReadOnly, ToolsWire::Allowlist, vec!["Read".to_string()], Some(2)));
        assert!(w.workspace.is_none(), "a local path never goes over the wire");
        let json = serde_json::to_string(&w).unwrap();
        assert!(!json.contains("/local/only"));
    }

    #[test]
    fn what_the_wire_cannot_carry_is_refused_not_dropped() {
        let cwd = Path::new("/x");
        let mut r = req("p", cwd);
        r.session = SessionMode::Resume("s");
        assert!(oneshot_request("claude", &r).unwrap_err().to_string().contains("without a session"));
        let mut r = req("p", cwd);
        r.extra_args = vec!["--effort".into()];
        assert!(oneshot_request("claude", &r).unwrap_err().to_string().contains("extra arguments"));
        let mut r = req("p", cwd);
        r.access = Access::Native("plan".into());
        assert!(oneshot_request("claude", &r).unwrap_err().to_string().contains("neutral access"));
    }

    #[test]
    fn a_system_prompt_file_is_sent_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sys.txt");
        std::fs::write(&file, "from a file").unwrap();
        let mut r = req("p", dir.path());
        r.system_prompt = Some(SystemPrompt::File(&file));
        assert_eq!(oneshot_request("claude", &r).unwrap().system_prompt, "from a file");
    }

    #[test]
    fn caps_drop_sessions_and_the_instances_are_shared() {
        let h = for_host(7, "claude").unwrap();
        let caps = h.caps();
        assert!(!caps.sessions && !caps.resume && !caps.fork && !caps.interactive_tui && !caps.pins_session_id);
        assert_eq!(caps.access_levels, ClaudeCode.caps().access_levels);
        assert_eq!((h.id(), h.name()), ("claude", "Claude Code"));
        assert!(std::ptr::eq(h as *const _ as *const u8, for_host(7, "claude").unwrap() as *const _ as *const u8));
        assert!(for_host(7, "zed").is_none());
        assert!(h.interactive_argv(&Interactive { session: SessionMode::Ephemeral, system_prompt_file: None, system_prompt_mode: SystemPromptMode::Append }).is_none());
    }

    #[test]
    fn without_a_runtime_the_harness_says_so() {
        let h = RemoteHarness::new(1, &ClaudeCode);
        assert!(h.unavailable_reason().unwrap().contains("not available here"));
        assert!(!h.available());
    }
}

#[cfg(all(test, feature = "worker", unix))]
mod server_tests {
    use super::*;
    use crate::remote::test_server::{TestServer, Fake};
    use crate::run_usage::RunTag;

    fn fake_harness(rt: Arc<RemoteRuntime>) -> RemoteHarness {
        RemoteHarness::with_runtime(1, Box::leak(Box::new(Fake)), rt)
    }

    fn request(prompt: &str) -> OneShot<'_> {
        OneShot {
            model: "",
            system_prompt: None,
            system_prompt_mode: SystemPromptMode::Append,
            file_tag: "t",
            prompt,
            cwd: std::path::Path::new("/ignored"),
            extra_dirs: Vec::new(),
            tools: ToolPolicy::None,
            access: Access::Default,
            session: SessionMode::Ephemeral,
            max_turns: None,
            effort: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            usage_tag: RunTag::new("theme_suggest"),
        }
    }

    #[test]
    fn a_one_shot_runs_on_the_host_and_reports_its_usage() {
        let srv = TestServer::start(2);
        let rt = srv.runtime();
        let h = fake_harness(rt.clone());
        let turn = h.one_shot(&request("hi")).unwrap();
        assert!(turn.text.starts_with("echo:hi@"), "{}", turn.text);
        assert_eq!((turn.input_tokens, turn.output_tokens, turn.cost_usd, turn.session_id), (3, 4, 0.5, None));
        // the host's failure comes back as an error, not as empty text
        assert!(h.one_shot(&request("fail")).unwrap_err().to_string().contains("the harness failed"));
        // capabilities and availability come from the host
        assert!(h.available());
        assert!(h.caps().streaming && !h.caps().sessions);
        assert_eq!(h.version(), None);
        assert!(h.models().unwrap().is_empty());
        rt.shutdown();
    }

    #[test]
    fn a_harness_the_host_lacks_is_unavailable_with_a_reason() {
        let srv = TestServer::start(1);
        let rt = srv.runtime();
        let h = RemoteHarness::with_runtime(1, &crate::ai::ClaudeCode, rt.clone());
        assert!(!h.available());
        assert!(h.unavailable_reason().unwrap().contains("does not offer the `claude` harness"));
        let err = h.one_shot(&request("hi")).unwrap_err().to_string();
        assert!(err.contains("harness_unavailable"), "{err}");
        rt.shutdown();
    }

    #[test]
    fn a_missing_or_disabled_host_is_reported_with_its_code() {
        let srv = TestServer::start(1);
        let rt = srv.runtime();
        let h = RemoteHarness::with_runtime(42, &crate::ai::ClaudeCode, rt);
        assert!(h.unavailable_reason().unwrap().starts_with("host_disabled"));
        assert!(h.models().is_err());
    }
}
