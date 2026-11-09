//! opencode (`opencode` CLI) behind the [`Harness`] trait. The only place that builds an `opencode`
//! command line or parses its output. Verified against opencode 2.2.5 (fixtures in
//! `ai/fixtures/opencode_*`).
//!
//! What the CLI does, as observed:
//! - `opencode run --format json` prints one JSON object per line: `step_start`, `text`,
//!   `tool_use` (a finished tool call: its input and output arrive together), `step_finish`
//!   (`reason`, `cost`, `tokens`) and `error`. There is **no final result line**: the run ends with
//!   a `step_finish` whose reason is not `tool-calls`, and the answer is the last `text`. Usage is
//!   per step, so the run engine folds it ([`Harness::accumulates`]).
//! - The prompt is read from stdin. There is no system prompt flag: it is put in front of the message.
//! - `-s <id>` continues the session or creates it under that id; ids must start with `ses`, so the
//!   app's UUIDs are mapped ([`session_id`]).
//! - Permissions come from the agent (`--agent build|plan`); `--auto` approves what the agent would ask.
//!
//! Auth stays in opencode (`opencode auth login`); the app stores nothing.

use std::io::Write as _;
use std::process::{Child, Command, Stdio};

use serde_json::Value;

use super::{
    Access, Caps, Event, FinalResult, Harness, Interactive, OneShot, RunAcc, SessionMode, StreamRun, SystemPrompt,
    ToolPolicy, TurnResult,
};
use crate::ai_usage::{self, RunUsage};
use crate::error::{AppError, AppResult};

pub struct OpenCode;

impl Harness for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }

    fn name(&self) -> &'static str {
        "opencode"
    }

    fn caps(&self) -> Caps {
        Caps {
            sessions: true,
            resume: true,
            fork: false,
            tools: true,
            // Tool permissions belong to opencode's agents.
            tool_allowlist: false,
            streaming: true,
            interactive_tui: true,
            reports_cost: true,
            replaces_system_prompt: false,
            // No ReadOnly: opencode has no flag that denies bash (its `plan` agent only denies edits),
            // so a read-only routine can't be enforced and is refused at launch.
            access_levels: vec![Access::Default, Access::Edit, Access::Full],
            pins_session_id: true,
        }
    }

    fn available(&self) -> bool {
        crate::tools::opencode_path().is_some()
    }

    fn version(&self) -> Option<String> {
        let out = Command::new(crate::tools::opencode_bin()).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    }

    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
        let message = message(req)?;
        let mut cmd = command(req, false)?;
        let started_at = ai_usage::now_stamp();
        let started = std::time::Instant::now();
        let (result, usage) = execute(&mut cmd, &message);
        let mut tag = req.usage_tag.clone();
        tag.model = req.model.to_string();
        let error = result.as_ref().err().map(|e| e.to_string());
        ai_usage::record(&tag, &started_at, started.elapsed().as_millis() as i64, &usage, error.as_deref());
        result
    }

    fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
        // A private server per run (`--standalone`): stopping the run's process group then stops the
        // work too, which the shared background service would carry on with.
        let mut cmd = command(&run.req, true)?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::from(run.stdout)).stderr(Stdio::from(run.stderr));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().map_err(|e| AppError::Other(format!("failed to spawn opencode: {e}")))
    }

    fn parse_line(&self, line: &str) -> (String, Option<Value>) {
        match serde_json::from_str::<Value>(line) {
            Ok(v) => (v.get("type").and_then(Value::as_str).unwrap_or("unknown").to_string(), Some(v)),
            Err(_) => ("raw".to_string(), None),
        }
    }

    fn normalize(&self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return vec![Event::Other { kind: "raw".into(), raw: Value::String(line.trim_end().to_string()) }];
        };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("unknown");
        let mut events = Vec::new();
        match kind {
            "text" => {
                let text = v.pointer("/part/text").and_then(Value::as_str).unwrap_or_default();
                if !text.trim().is_empty() {
                    events.push(Event::Text(text.to_string()));
                }
            }
            "tool_use" => {
                let id = v.pointer("/part/id").and_then(Value::as_str).unwrap_or_default().to_string();
                let state = v.pointer("/part/state");
                events.push(Event::ToolCall {
                    id: id.clone(),
                    name: v.pointer("/part/tool").and_then(Value::as_str).unwrap_or_default().to_string(),
                    input: state
                        .and_then(|s| s.get("input"))
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Default::default())),
                });
                // The call and its outcome arrive together; a call still pending has no result yet.
                match state.and_then(|s| s.get("status")).and_then(Value::as_str) {
                    Some("completed") => events.push(Event::ToolResult {
                        id,
                        ok: true,
                        text: state.and_then(|s| s.get("output")).map(value_text).unwrap_or_default(),
                    }),
                    Some("error") => events.push(Event::ToolResult {
                        id,
                        ok: false,
                        text: state
                            .and_then(|s| s.get("error").or_else(|| s.get("output")))
                            .map(value_text)
                            .unwrap_or_default(),
                    }),
                    _ => {}
                }
            }
            "step_finish" => {
                let reason = v.pointer("/part/reason").and_then(Value::as_str).unwrap_or_default();
                // A step that ends in tool calls is followed by another one; any other reason ends the run.
                if reason != "tool-calls" {
                    events.push(Event::Final {
                        ok: reason == "stop",
                        text: if reason == "stop" { String::new() } else { format!("opencode stopped: {reason}") },
                        usage: step_usage(&v).unwrap_or_default(),
                    });
                }
            }
            "error" => events.push(Event::Final { ok: false, text: error_text(&v), usage: RunUsage::default() }),
            _ => {}
        }
        // Everything else (step starts, reasoning, unknown types) stays as `other`.
        if events.is_empty() {
            events.push(Event::Other { kind: kind.to_string(), raw: v });
        }
        events
    }

    fn accumulates(&self) -> bool {
        true
    }

    fn step_usage(&self, line: &str) -> Option<RunUsage> {
        let v = serde_json::from_str::<Value>(line).ok()?;
        step_usage(&v)
    }

    #[cfg(unix)]
    fn is_own_process(&self, pid: i32, session_id: Option<&str>) -> bool {
        let Ok(out) = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output() else {
            return false;
        };
        let cmd = String::from_utf8_lossy(&out.stdout);
        cmd.contains("opencode") && cmd.contains(" run") && session_id.is_none_or(|s| cmd.contains(&session_id_for(s)))
    }

    #[cfg(not(unix))]
    fn is_own_process(&self, _pid: i32, _session_id: Option<&str>) -> bool {
        false
    }

    fn interactive_argv(&self, opts: &Interactive<'_>) -> Option<Vec<String>> {
        let mut argv = vec!["opencode".to_string()];
        match opts.session {
            SessionMode::Ephemeral => {}
            SessionMode::New(id) | SessionMode::Resume(id) => argv.extend(["--session".to_string(), session_id_for(id)]),
            SessionMode::Fork(_) => return None,
        }
        // No system prompt flag: the instructions go in as the opening prompt of a new session.
        if let (Some(file), SessionMode::New(_) | SessionMode::Ephemeral) = (opts.system_prompt_file, opts.session) {
            if let Ok(text) = std::fs::read_to_string(file) {
                if !text.trim().is_empty() {
                    argv.extend(["--prompt".to_string(), text]);
                }
            }
        }
        Some(argv)
    }

    fn models(&self) -> AppResult<Vec<String>> {
        let out = Command::new(crate::tools::opencode_bin())
            .arg("models")
            .output()
            .map_err(|e| AppError::Other(format!("failed to run opencode models: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Other(format!(
                "opencode models failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(parse_models(&String::from_utf8_lossy(&out.stdout)))
    }

    fn settings_args(&self, settings: &Value) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(agent) = settings.get("agent").and_then(Value::as_str).map(str::trim).filter(|a| !a.is_empty()) {
            args.extend(["--agent".to_string(), agent.to_string()]);
        }
        for a in settings.get("extra_args").and_then(Value::as_array).into_iter().flatten() {
            if let Some(a) = a.as_str() {
                args.push(a.to_string());
            }
        }
        args
    }
}

/// The ids `opencode models` prints, one `provider/model` per line.
pub(crate) fn parse_models(stdout: &str) -> Vec<String> {
    stdout.lines().map(str::trim).filter(|l| l.contains('/') && !l.contains(char::is_whitespace)).map(String::from).collect()
}

/// opencode wants session ids starting with `ses`; the app's own ids (UUIDs) become `ses_<hex>`.
/// Deterministic, so a session started under one id resumes under it.
pub(crate) fn session_id_for(id: &str) -> String {
    if id.starts_with("ses") {
        id.to_string()
    } else {
        format!("ses_{}", id.replace('-', ""))
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The usage of one `step_finish` line.
fn step_usage(v: &Value) -> Option<RunUsage> {
    if v.get("type").and_then(Value::as_str) != Some("step_finish") {
        return None;
    }
    let n = |p: &str| v.pointer(p).and_then(Value::as_i64).unwrap_or(0);
    Some(RunUsage {
        num_turns: 1,
        input_tokens: n("/part/tokens/input"),
        output_tokens: n("/part/tokens/output") + n("/part/tokens/reasoning"),
        cache_read_tokens: n("/part/tokens/cache/read"),
        cache_creation_tokens: n("/part/tokens/cache/write"),
        cost_usd: v.pointer("/part/cost").and_then(Value::as_f64).unwrap_or(0.0),
        ..RunUsage::default()
    })
}

/// The message of an `error` line.
fn error_text(v: &Value) -> String {
    let message = v.pointer("/error/message").and_then(Value::as_str).filter(|m| !m.is_empty());
    match (message, v.pointer("/error/type").and_then(Value::as_str)) {
        (Some(m), _) => m.to_string(),
        (None, Some(t)) => t.to_string(),
        (None, None) => "opencode reported an error".to_string(),
    }
}

/// What goes to opencode's stdin: the system prompt (it has no flag for one) above the prompt.
fn message(req: &OneShot<'_>) -> AppResult<String> {
    let system = match req.system_prompt {
        Some(SystemPrompt::Text(t)) => t.to_string(),
        Some(SystemPrompt::File(path)) => std::fs::read_to_string(path)
            .map_err(|e| AppError::Other(format!("failed to read the system prompt: {e}")))?,
        None => String::new(),
    };
    Ok(if system.trim().is_empty() { req.prompt.to_string() } else { format!("{}\n\n---\n\n{}", system.trim_end(), req.prompt) })
}

/// The `--agent` / `--auto` flags of `access`. An explicit `--agent` in the extra args (a profile's
/// `agent` setting included) wins, so a `build` agent lets a run edit whatever the access says.
/// `plan` only denies edits: bash stays allowed, which is why opencode does not offer read-only
/// access. It is still used for runs that ask for no tools (summaries, theme), where it is strictly
/// less capable than the default agent.
fn access_flags(access: &Access, tools: &ToolPolicy, extra_args: &[String]) -> Vec<&'static str> {
    let named = extra_args.iter().any(|a| a == "--agent" || a.starts_with("--agent="));
    let mut flags = Vec::new();
    match access {
        Access::ReadOnly if !named => flags.extend(["--agent", "plan"]),
        Access::Edit if !named => flags.extend(["--agent", "build"]),
        Access::Full => {
            if !named {
                flags.extend(["--agent", "build"]);
            }
            flags.push("--auto");
        }
        Access::Default if !named && *tools == ToolPolicy::None => flags.extend(["--agent", "plan"]),
        _ => {}
    }
    flags
}

/// The `opencode run` command of `req` (the prompt goes to stdin).
pub(crate) fn command(req: &OneShot<'_>, stream: bool) -> AppResult<Command> {
    let mut cmd = Command::new(crate::tools::opencode_bin());
    cmd.current_dir(req.cwd);
    cmd.args(["run", "--format", "json"]);
    if stream {
        cmd.arg("--standalone");
    }
    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }
    cmd.args(access_flags(&req.access, &req.tools, &req.extra_args));
    match req.session {
        SessionMode::New(id) | SessionMode::Resume(id) => {
            cmd.arg("--session").arg(session_id_for(id));
        }
        SessionMode::Ephemeral => {}
        SessionMode::Fork(_) => return Err(AppError::Other("opencode can't fork a session".into())),
    }
    // The tool policy has no opencode equivalent: the agent decides which tools exist.
    cmd.args(&req.extra_args);
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    Ok(cmd)
}

fn execute(cmd: &mut Command, message: &str) -> (AppResult<TurnResult>, RunUsage) {
    let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return (Err(AppError::Chat(format!("failed to spawn opencode: {e}"))), RunUsage::default()),
    };
    if let Some(mut stdin) = child.stdin.take() {
        // A process that exited early closes the pipe: its output and stderr say why, so go on and
        // collect them (this also reaps the child).
        if let Err(e) = stdin.write_all(message.as_bytes()) {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                let _ = child.kill();
                let _ = child.wait();
                return (Err(AppError::Chat(format!("failed to write prompt: {e}"))), RunUsage::default());
            }
        }
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return (Err(AppError::Chat(format!("opencode process error: {e}"))), RunUsage::default()),
    };
    interpret(&String::from_utf8_lossy(&output.stdout), &String::from_utf8_lossy(&output.stderr), output.status.code().unwrap_or(-1))
}

/// Turn the finished output of `opencode run --format json` into the run result and its usage.
pub(crate) fn interpret(stdout: &str, stderr: &str, exit_code: i32) -> (AppResult<TurnResult>, RunUsage) {
    let h = OpenCode;
    let mut acc = RunAcc::default();
    let mut fin: Option<FinalResult> = None;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let events = h.normalize(line);
        acc.push(&h, line, &events);
        if let Some(f) = events.iter().find_map(Event::as_final) {
            fin = Some(acc.complete(f));
        }
    }
    let Some(fin) = fin else {
        let stderr = stderr.trim();
        let message = if stderr.is_empty() { format!("opencode exited {exit_code} without a result") } else { stderr.to_string() };
        return (Err(AppError::Chat(message)), RunUsage::default());
    };
    let text = fin.text.clone().unwrap_or_default();
    if fin.is_error {
        return (Err(AppError::Chat(text)), fin.usage);
    }
    let turn = TurnResult {
        text,
        input_tokens: fin.usage.input_tokens,
        output_tokens: fin.usage.output_tokens,
        cost_usd: fin.usage.cost_usd,
    };
    (Ok(turn), fin.usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::norm_json;
    use crate::ai_usage::RunTag;
    use std::path::Path;

    const STREAM: &str = include_str!("fixtures/opencode_stream.jsonl");
    const RESUME: &str = include_str!("fixtures/opencode_resume.jsonl");
    const ERROR_AUTH: &str = include_str!("fixtures/opencode_error_auth.jsonl");
    const ERROR_ROUTE: &str = include_str!("fixtures/opencode_error_route.jsonl");
    const MODELS: &str = include_str!("fixtures/opencode_models.txt");

    fn norm(line: &str) -> Value {
        serde_json::from_str(&norm_json(&OpenCode.normalize(line), 100_000)).unwrap()
    }

    fn request<'a>(cwd: &'a Path) -> OneShot<'a> {
        OneShot {
            model: "openrouter/anthropic/claude-haiku-4.5",
            system_prompt: None,
            system_prompt_mode: super::super::SystemPromptMode::Append,
            file_tag: "t",
            prompt: "hi",
            cwd,
            extra_dirs: Vec::new(),
            tools: ToolPolicy::Full,
            access: Access::Default,
            session: SessionMode::Ephemeral,
            max_turns: None,
            effort: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            usage_tag: RunTag::new("chat"),
        }
    }

    fn argv(cmd: &Command) -> Vec<String> {
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn strs(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalizes_a_run_with_a_tool_call() {
        let lines: Vec<&str> = STREAM.lines().collect();
        let events: Vec<Value> = lines.iter().map(|l| norm(l)).collect();
        // step_start, text, tool_use (call + result), step_finish (tool-calls), step_start, text, step_finish (stop)
        assert_eq!(events[0]["events"][0]["type"], "other");
        assert_eq!(events[0]["events"][0]["kind"], "step_start");
        assert_eq!(events[1]["events"][0]["text"], "I'll read the file note.txt for you.");
        let call = &events[2]["events"][0];
        assert_eq!((call["type"].as_str(), call["name"].as_str()), (Some("tool_call"), Some("read")));
        assert_eq!(call["input"]["path"], "note.txt");
        let result = &events[2]["events"][1];
        assert_eq!((result["type"].as_str(), result["ok"].as_bool()), (Some("tool_result"), Some(true)));
        assert_eq!(result["id"], call["id"]);
        assert!(result["text"].as_str().unwrap().contains("hello-from-file"));
        // a step that ends in tool calls is not the end of the run
        assert_eq!(events[3]["events"][0]["type"], "other");
        let fin = &events[6]["events"][0];
        assert_eq!((fin["type"].as_str(), fin["ok"].as_bool()), (Some("final"), Some(true)));
    }

    #[test]
    fn the_final_text_and_usage_are_folded_over_all_steps() {
        let (turn, usage) = interpret(STREAM, "", 0);
        let turn = turn.unwrap();
        assert_eq!(turn.text, "The file note.txt contains a single line: \"hello-from-file\".");
        assert_eq!(usage.num_turns, 2);
        assert_eq!(usage.input_tokens, 3 + 6);
        assert_eq!(usage.output_tokens, 65 + 20);
        assert_eq!(usage.cache_read_tokens, 11874);
        assert_eq!(usage.cache_creation_tokens, 11874 + 93);
        assert!((usage.cost_usd - (0.0151705 + 0.00140965)).abs() < 1e-9, "{}", usage.cost_usd);
        assert!((turn.cost_usd - usage.cost_usd).abs() < 1e-12);
    }

    #[test]
    fn a_resumed_turn_normalizes_the_same_way() {
        let (turn, usage) = interpret(RESUME, "", 0);
        assert_eq!(turn.unwrap().text, "PONG");
        assert_eq!(usage.num_turns, 1);
    }

    #[test]
    fn an_error_line_is_a_failed_final() {
        for (raw, needle) in [(ERROR_AUTH, "Reconnect OpenCode Console"), (ERROR_ROUTE, "Model unavailable: nope/none")] {
            let fin = OpenCode.normalize(raw.lines().next().unwrap()).iter().find_map(Event::as_final).unwrap();
            assert!(fin.is_error);
            assert!(fin.text.unwrap().contains(needle));
        }
        let (res, _) = interpret(ERROR_ROUTE, "", 1);
        assert!(res.unwrap_err().to_string().contains("Model unavailable"));
        // an error after a step start is still the end result
        let (res, _) = interpret(ERROR_AUTH, "", 1);
        assert!(res.is_err());
    }

    #[test]
    fn unknown_and_malformed_lines_degrade_to_other() {
        for line in [r#"{"type":"brand_new","x":1}"#, r#"{"no_type":true}"#, "not json at all", r#"{"type":"text","part":{}}"#] {
            let events = OpenCode.normalize(line);
            assert_eq!(events.len(), 1, "{line}");
            assert!(matches!(events[0], Event::Other { .. }), "{line}");
        }
        assert_eq!(OpenCode.parse_line("garbage").0, "raw");
        assert_eq!(OpenCode.parse_line(r#"{"type":"text"}"#).0, "text");
    }

    #[test]
    fn a_run_without_a_final_fails_with_stderr() {
        let (res, _) = interpret("", "boom\n", 2);
        assert!(res.unwrap_err().to_string().contains("boom"));
        let (res, _) = interpret(STREAM.lines().next().unwrap(), "", 0);
        assert!(res.unwrap_err().to_string().contains("without a result"));
    }

    #[test]
    fn a_step_cut_short_is_a_failed_final() {
        let line = r#"{"type":"step_finish","part":{"reason":"length","cost":0,"tokens":{"input":1,"output":2}}}"#;
        let fin = OpenCode.normalize(line).iter().find_map(Event::as_final).unwrap();
        assert!(fin.is_error);
        assert!(fin.text.unwrap().contains("length"));
    }

    #[test]
    fn a_failed_tool_call_yields_a_failed_result() {
        let line = r#"{"type":"tool_use","part":{"id":"t1","tool":"shell","state":{"status":"error","input":{"command":"x"},"error":"nope"}}}"#;
        let events = OpenCode.normalize(line);
        assert!(matches!(&events[1], Event::ToolResult { ok: false, text, .. } if text == "nope"));
        let pending = r#"{"type":"tool_use","part":{"id":"t1","tool":"shell","state":{"status":"running","input":{}}}}"#;
        assert_eq!(OpenCode.normalize(pending).len(), 1);
    }

    #[test]
    fn models_are_the_provider_slash_model_lines() {
        let models = parse_models(MODELS);
        assert!(models.contains(&"opencode/beam-free".to_string()));
        assert!(models.iter().any(|m| m.starts_with("openrouter/")));
        assert_eq!(models.len(), MODELS.lines().count());
        assert_eq!(parse_models("\nnote: something\nopenai/gpt-5\n  \n"), ["openai/gpt-5"]);
    }

    #[test]
    fn session_ids_are_mapped_into_opencodes_namespace() {
        assert_eq!(session_id_for("3f2a1b0c-aaaa-4bbb-8ccc-0123456789ab"), "ses_3f2a1b0caaaa4bbb8ccc0123456789ab");
        assert_eq!(session_id_for("ses_abc"), "ses_abc");
        assert_eq!(session_id_for("x"), session_id_for("x"));
    }

    #[test]
    fn argv_snapshot_per_access_level() {
        let cwd = Path::new("/w");
        let base = ["run", "--format", "json", "--model", "openrouter/anthropic/claude-haiku-4.5"];
        let mut req = request(cwd);
        assert_eq!(argv(&command(&req, false).unwrap()), strs(&base));
        req.access = Access::ReadOnly;
        assert_eq!(&argv(&command(&req, false).unwrap())[5..], &strs(&["--agent", "plan"])[..]);
        req.access = Access::Edit;
        assert_eq!(&argv(&command(&req, false).unwrap())[5..], &strs(&["--agent", "build"])[..]);
        req.access = Access::Full;
        assert_eq!(&argv(&command(&req, false).unwrap())[5..], &strs(&["--agent", "build", "--auto"])[..]);
        // no tools at all: the plan agent
        req.access = Access::Default;
        req.tools = ToolPolicy::None;
        assert_eq!(&argv(&command(&req, false).unwrap())[5..], &strs(&["--agent", "plan"])[..]);
        req.tools = ToolPolicy::Full;
        req.access = Access::Full;
        // a profile's agent wins over the access level, which still auto-approves
        req.extra_args = strs(&["--agent", "review"]);
        assert_eq!(&argv(&command(&req, false).unwrap())[5..], &strs(&["--auto", "--agent", "review"])[..]);
    }

    #[test]
    fn argv_snapshot_for_sessions_streaming_and_model() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.model = "";
        req.session = SessionMode::New("3f2a1b0c-aaaa-4bbb-8ccc-0123456789ab");
        let cmd = command(&req, true).unwrap();
        assert_eq!(argv(&cmd), strs(&["run", "--format", "json", "--standalone", "--session", "ses_3f2a1b0caaaa4bbb8ccc0123456789ab"]));
        assert_eq!(cmd.get_current_dir(), Some(cwd));
        req.session = SessionMode::Resume("ses_x");
        assert_eq!(&argv(&command(&req, false).unwrap())[3..], &strs(&["--session", "ses_x"])[..]);
        req.session = SessionMode::Fork("ses_x");
        assert!(command(&req, false).is_err());
    }

    #[test]
    fn the_system_prompt_goes_in_front_of_the_message() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        assert_eq!(message(&req).unwrap(), "hi");
        req.system_prompt = Some(SystemPrompt::Text("be brief\n"));
        assert_eq!(message(&req).unwrap(), "be brief\n\n---\n\nhi");
        let path = std::env::temp_dir().join(format!("cr_oc_sys_{}.txt", std::process::id()));
        std::fs::write(&path, "from file").unwrap();
        req.system_prompt = Some(SystemPrompt::File(&path));
        assert_eq!(message(&req).unwrap(), "from file\n\n---\n\nhi");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn profile_settings_become_flags() {
        let h = OpenCode;
        assert!(h.settings_args(&serde_json::json!({})).is_empty());
        assert_eq!(
            h.settings_args(&serde_json::json!({ "agent": " review ", "extra_args": ["--thinking", 3] })),
            strs(&["--agent", "review", "--thinking"])
        );
    }

    #[test]
    fn interactive_argv_opens_the_tui_on_a_session() {
        let opts = |session| Interactive { session, system_prompt_file: None, system_prompt_mode: super::super::SystemPromptMode::Append };
        assert_eq!(OpenCode.interactive_argv(&opts(SessionMode::Ephemeral)), Some(strs(&["opencode"])));
        assert_eq!(
            OpenCode.interactive_argv(&opts(SessionMode::Resume("a-b"))),
            Some(strs(&["opencode", "--session", "ses_ab"]))
        );
        assert_eq!(OpenCode.interactive_argv(&opts(SessionMode::Fork("x"))), None);
        let path = std::env::temp_dir().join(format!("cr_oc_int_{}.txt", std::process::id()));
        std::fs::write(&path, "instructions").unwrap();
        let with_file = Interactive { session: SessionMode::New("a"), system_prompt_file: Some(&path), ..opts(SessionMode::New("a")) };
        assert_eq!(OpenCode.interactive_argv(&with_file), Some(strs(&["opencode", "--session", "ses_a", "--prompt", "instructions"])));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn capabilities_are_refused_when_a_routine_needs_more() {
        use crate::ai::{check_caps, Needs};
        let needs = |tool_allowlist| Needs { access: Access::Edit, tools: true, tool_allowlist };
        check_caps(&OpenCode, "r", &needs(false)).unwrap();
        let err = check_caps(&OpenCode, "“Babysit”", &needs(true)).unwrap_err().to_string();
        assert!(err.contains("a tool allowlist") && err.contains("opencode"), "{err}");
        assert!(!OpenCode.caps().fork);
    }

    #[test]
    fn read_only_routines_are_refused_because_bash_cannot_be_denied() {
        use crate::ai::{check_caps, Needs};
        let err = check_caps(&OpenCode, "“Babysit”", &Needs { access: Access::ReadOnly, tools: false, tool_allowlist: false })
            .unwrap_err()
            .to_string();
        assert!(err.contains("“Babysit”") && err.contains("read-only") && err.contains("opencode"), "{err}");
        assert!(err.contains("Edit or Full"), "{err}");
        assert!(!OpenCode.caps().access_levels.contains(&Access::ReadOnly));
        check_caps(&OpenCode, "r", &Needs { access: Access::Edit, tools: true, tool_allowlist: false }).unwrap();
    }
}
