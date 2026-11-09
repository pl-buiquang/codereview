//! Claude Code (`claude` CLI) behind the [`Harness`] trait. This is the only place that builds a
//! `claude` command line or parses Claude's output formats (the JSON envelope of `-p`, the
//! `stream-json` lines of routine runs). Session discovery stays in `claude_sessions.rs`.

use std::io::Write as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use super::{
    Access, Caps, Event, Harness, Interactive, OneShot, SessionMode, StreamRun, SystemPrompt, SystemPromptMode,
    ToolPolicy, TurnResult,
};
use crate::ai_usage::{self, ModelTokens, RunTag, RunUsage};
use crate::error::{AppError, AppResult};

pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn caps(&self) -> Caps {
        Caps {
            sessions: true,
            resume: true,
            fork: true,
            tools: true,
            tool_allowlist: true,
            streaming: true,
            interactive_tui: true,
            reports_cost: true,
            replaces_system_prompt: true,
            access_levels: vec![Access::Default, Access::ReadOnly, Access::Edit, Access::Full],
            pins_session_id: true,
        }
    }

    fn available(&self) -> bool {
        crate::tools::claude_path().is_some()
    }

    fn version(&self) -> Option<String> {
        let out = Command::new(crate::tools::claude_bin()).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    }

    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
        run_one_shot(req)
    }

    fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
        let prompt_path = match run.req.system_prompt {
            Some(SystemPrompt::File(path)) => Some(path),
            Some(SystemPrompt::Text(_)) => {
                return Err(AppError::Other("a streaming run needs its system prompt as a file".into()))
            }
            None => None,
        };
        let mut cmd = command(&run.req, true, prompt_path);
        // Files, not pipes: the process must outlive us when the app quits with runs left alive.
        cmd.stdin(Stdio::piped()).stdout(Stdio::from(run.stdout)).stderr(Stdio::from(run.stderr));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().map_err(|e| AppError::Other(format!("failed to spawn claude: {e}")))
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
            "assistant" => {
                for block in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or_default();
                            if !text.trim().is_empty() {
                                events.push(Event::Text(text.to_string()));
                            }
                        }
                        Some("tool_use") => events.push(Event::ToolCall {
                            id: str_field(block, "id"),
                            name: str_field(block, "name"),
                            input: block.get("input").cloned().unwrap_or_else(|| Value::Object(Default::default())),
                        }),
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        events.push(Event::ToolResult {
                            id: str_field(block, "tool_use_id"),
                            ok: !block.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                            text: tool_result_text(block.get("content")),
                        });
                    }
                }
            }
            "result" => {
                let usage = serde_json::from_value::<ClaudeEnvelope>(v.clone()).map(|e| e.run_usage()).unwrap_or_default();
                events.push(Event::Final {
                    ok: !v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    text: match v.get("result") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(other) => other.to_string(),
                    },
                    usage,
                });
            }
            _ => {}
        }
        // Lines with nothing to show (system, thinking-only messages, unknown types) stay as `other`.
        if events.is_empty() {
            events.push(Event::Other { kind: kind.to_string(), raw: v });
        }
        events
    }

    fn models(&self) -> AppResult<Vec<String>> {
        // The CLI has no model listing: aliases it understands.
        Ok(["opus", "sonnet", "haiku"].map(String::from).to_vec())
    }

    #[cfg(unix)]
    fn is_own_process(&self, pid: i32, session_id: Option<&str>) -> bool {
        let Ok(out) = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output() else {
            return false;
        };
        let cmd = String::from_utf8_lossy(&out.stdout);
        cmd.contains("claude") && cmd.contains(" -p") && session_id.is_none_or(|s| cmd.contains(s))
    }

    #[cfg(not(unix))]
    fn is_own_process(&self, _pid: i32, _session_id: Option<&str>) -> bool {
        false
    }

    fn interactive_argv(&self, opts: &Interactive<'_>) -> Option<Vec<String>> {
        // "claude" is resolved by the terminal's own PATH, as it always was.
        let mut argv = vec!["claude".to_string()];
        match opts.session {
            SessionMode::Ephemeral => {}
            SessionMode::New(id) => argv.extend(["--session-id".to_string(), id.to_string()]),
            SessionMode::Resume(id) => argv.extend(["--resume".to_string(), id.to_string()]),
            SessionMode::Fork(_) => return None,
        }
        if let Some(file) = opts.system_prompt_file {
            argv.extend([system_prompt_flag(opts.system_prompt_mode).to_string(), file.to_string_lossy().into_owned()]);
        }
        Some(argv)
    }
}

fn str_field(block: &Value, key: &str) -> String {
    block.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A `tool_result` block's content: a string, or the `text` of each part.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|p| p.get("text").map(|t| t.as_str().map_or_else(|| t.to_string(), str::to_string)).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn system_prompt_flag(mode: SystemPromptMode) -> &'static str {
    match mode {
        SystemPromptMode::Replace => "--system-prompt-file",
        SystemPromptMode::Append => "--append-system-prompt-file",
    }
}

/// The value of `--permission-mode` for `access`.
fn permission_mode(access: &Access) -> &str {
    match access {
        Access::Default => "default",
        Access::ReadOnly => "plan",
        Access::Edit => "acceptEdits",
        Access::Full => "bypassPermissions",
        Access::Native(mode) => mode,
    }
}

/// The `claude -p` command of `req`. `prompt_path` is the system prompt file, when there is one.
/// A fork request builds the minimal fork command instead: only the session, the turn cap, the
/// env and the cwd of `req` apply to it.
pub(crate) fn command(req: &OneShot<'_>, stream: bool, prompt_path: Option<&Path>) -> Command {
    let mut cmd = Command::new(crate::tools::claude_bin());
    // Run from the reviewed project so CLAUDE.md, git context, etc. come from it, not the Tauri
    // app's directory.
    cmd.current_dir(req.cwd);
    if let SessionMode::Fork(id) = req.session {
        cmd.args(["-p", "--output-format", "json", "--resume", id, "--fork-session", "--no-session-persistence"]);
        if let Some(n) = req.max_turns {
            cmd.arg("--max-turns").arg(n.to_string());
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        return cmd;
    }
    cmd.arg("-p").arg("--output-format").arg(if stream { "stream-json" } else { "json" });
    if stream {
        cmd.arg("--verbose");
    }
    // --strict-mcp-config: don't load MCP servers (they add latency/noise).
    // Note: we deliberately omit --setting-sources "" so claude can read its own auth and Bedrock
    // configuration from ~/.claude/. The permission mode is always explicit, so the user's own
    // `~/.claude/settings.json` (e.g. a `defaultMode: plan`) never leaks into a non-interactive
    // session.
    cmd.arg("--strict-mcp-config")
        .arg("--permission-mode")
        .arg(permission_mode(&req.access))
        .arg("--add-dir")
        .arg(req.cwd);
    for dir in &req.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    match &req.tools {
        ToolPolicy::Allowlist(tools) if !tools.is_empty() => {
            cmd.arg("--allowedTools").args(tools);
        }
        ToolPolicy::ReadOnly => {
            cmd.arg("--allowedTools").args(["Read", "Glob", "Grep"]);
        }
        ToolPolicy::Edit => {
            cmd.arg("--allowedTools").args(["Read", "Glob", "Grep", "Edit", "Write"]);
        }
        ToolPolicy::None | ToolPolicy::Full | ToolPolicy::Allowlist(_) => {}
    }
    if stream {
        // A self-paced /loop or background wait must not hit the 10-minute idle cap of `-p`.
        cmd.env("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS", "0");
    }
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }
    if req.tools == ToolPolicy::None {
        cmd.arg("--tools").arg("");
    }
    if let Some(effort) = req.effort {
        cmd.arg("--effort").arg(effort);
    }
    if let Some(n) = req.max_turns {
        cmd.arg("--max-turns").arg(n.to_string());
    }
    cmd.args(&req.extra_args);
    match req.session {
        SessionMode::New(id) => {
            cmd.arg("--session-id").arg(id);
        }
        SessionMode::Resume(id) => {
            cmd.arg("--resume").arg(id);
        }
        SessionMode::Ephemeral | SessionMode::Fork(_) => {
            cmd.arg("--no-session-persistence");
        }
    }
    if let Some(path) = prompt_path {
        cmd.arg(system_prompt_flag(req.system_prompt_mode)).arg(path);
    }
    cmd
}

fn run_one_shot(req: &OneShot<'_>) -> AppResult<TurnResult> {
    // The file name is deterministic per tag to avoid leaking temp files on crash.
    let mut temp_prompt = None;
    let prompt_path = match req.system_prompt {
        Some(SystemPrompt::File(path)) => Some(path.to_path_buf()),
        Some(SystemPrompt::Text(text)) if !matches!(req.session, SessionMode::Fork(_)) => {
            let path = std::env::temp_dir().join(format!("codereview_{}.txt", req.file_tag));
            std::fs::write(&path, text).map_err(|e| AppError::Chat(format!("failed to write system prompt: {e}")))?;
            temp_prompt = Some(path.clone());
            Some(path)
        }
        _ => None,
    };
    let cmd = command(req, false, prompt_path.as_deref());
    let tag = req.file_tag;
    log::info!(
        target: "claude",
        "{tag}: starting claude -p (model {:?}, cwd {})",
        req.model,
        req.cwd.display()
    );
    let started = std::time::Instant::now();
    let mut usage_tag = req.usage_tag.clone();
    usage_tag.model = req.model.to_string();
    let result = run_command(cmd, req.prompt, &usage_tag);
    if let Some(p) = temp_prompt {
        let _ = std::fs::remove_file(p);
    }
    let secs = started.elapsed().as_secs();
    match &result {
        Ok(t) => {
            log::info!(target: "claude", "{tag}: finished in {secs}s");
            log::debug!(
                target: "claude",
                "{tag}: {} in / {} out tokens, ${:.2}",
                t.input_tokens,
                t.output_tokens,
                t.cost_usd
            );
        }
        Err(e) => log::warn!(target: "claude", "{tag}: failed after {secs}s: {e}"),
    }
    result
}

/// Spawn a prepared `claude -p --output-format json` command, feed it `text` on stdin, and parse
/// the JSON envelope. Every run, failed ones included, lands in the usage log under `tag`.
fn run_command(cmd: Command, text: &str, tag: &RunTag) -> AppResult<TurnResult> {
    let started_at = ai_usage::now_stamp();
    let started = std::time::Instant::now();
    let (result, usage) = execute(cmd, text);
    let error = result.as_ref().err().map(|e| e.to_string());
    ai_usage::record(tag, &started_at, started.elapsed().as_millis() as i64, &usage, error.as_deref());
    result
}

fn execute(mut cmd: Command, text: &str) -> (AppResult<TurnResult>, RunUsage) {
    let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => return (Err(AppError::Chat(format!("failed to spawn claude: {e}"))), RunUsage::default()),
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(text.as_bytes()) {
            return (Err(AppError::Chat(format!("failed to write prompt: {e}"))), RunUsage::default());
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(e) => return (Err(AppError::Chat(format!("claude process error: {e}"))), RunUsage::default()),
    };
    interpret(
        &output.stdout,
        &String::from_utf8_lossy(&output.stderr),
        output.status.success(),
        output.status.code().unwrap_or(-1),
    )
}

/// Turn a finished process' output into the run result plus whatever usage it reported.
fn interpret(stdout: &[u8], stderr: &str, success: bool, exit_code: i32) -> (AppResult<TurnResult>, RunUsage) {
    // Even on non-zero exit, claude may have written a JSON envelope with a
    // human-readable `result` — parse it first for a clean error message.
    let raw = String::from_utf8_lossy(stdout);
    let parsed = serde_json::from_str::<ClaudeEnvelope>(&raw);
    let usage = parsed.as_ref().map(ClaudeEnvelope::run_usage).unwrap_or_default();

    if let Ok(envelope) = &parsed {
        if envelope.is_error {
            let detail = envelope
                .result
                .clone()
                .map(|v| match v {
                    Value::String(s) => s,
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "unknown error".to_string());
            return (Err(AppError::Chat(detail)), usage);
        }
    }

    if !success || stdout.is_empty() {
        let stderr = stderr.trim();
        let message = if stderr.is_empty() { format!("claude exited {exit_code}") } else { stderr.to_string() };
        return (Err(AppError::Chat(message)), usage);
    }

    let envelope = match parsed {
        Ok(envelope) => envelope,
        Err(e) => return (Err(AppError::Chat(format!("failed to parse claude response: {e}"))), usage),
    };

    let text_out = envelope
        .result
        .map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default();

    let turn = TurnResult {
        text: text_out,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd,
    };
    (Ok(turn), usage)
}

/// JSON envelope returned by `claude -p --output-format json`, and the shape of the final
/// `result` line of `stream-json`.
#[derive(Deserialize)]
struct ClaudeEnvelope {
    result: Option<Value>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    total_cost_usd: f64,
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    num_turns: i64,
    session_id: Option<String>,
    #[serde(default, rename = "modelUsage")]
    model_usage: std::collections::HashMap<String, ClaudeModelUsage>,
}

#[derive(Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ClaudeModelUsage {
    input_tokens: i64,
    output_tokens: i64,
    cache_read_input_tokens: i64,
    cache_creation_input_tokens: i64,
    #[serde(rename = "costUSD")]
    cost_usd: f64,
}

impl ClaudeEnvelope {
    fn run_usage(&self) -> RunUsage {
        let usage = self.usage.as_ref();
        let mut models: Vec<ModelTokens> = self
            .model_usage
            .iter()
            .map(|(model, m)| ModelTokens {
                model: model.clone(),
                input_tokens: m.input_tokens,
                output_tokens: m.output_tokens,
                cache_read_tokens: m.cache_read_input_tokens,
                cache_creation_tokens: m.cache_creation_input_tokens,
                cost_usd: m.cost_usd,
            })
            .collect();
        models.sort_by(|a, b| a.model.cmp(&b.model));
        RunUsage {
            num_turns: self.num_turns,
            input_tokens: usage.map_or(0, |u| u.input_tokens),
            output_tokens: usage.map_or(0, |u| u.output_tokens),
            cache_read_tokens: usage.map_or(0, |u| u.cache_read_input_tokens),
            cache_creation_tokens: usage.map_or(0, |u| u.cache_creation_input_tokens),
            cost_usd: self.total_cost_usd,
            session_id: self.session_id.clone(),
            models,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"{"type":"result","is_error":false,"result":"hi","num_turns":2,"session_id":"s1",
        "total_cost_usd":1.5,
        "usage":{"input_tokens":3,"output_tokens":4,"cache_read_input_tokens":5,"cache_creation_input_tokens":6},
        "modelUsage":{"opus":{"inputTokens":2,"outputTokens":3,"cacheReadInputTokens":5,"cacheCreationInputTokens":6,"costUSD":1.4},
                      "haiku":{"inputTokens":1,"outputTokens":1,"costUSD":0.1}}}"#;

    #[test]
    fn success_reports_text_and_per_model_usage() {
        let (result, usage) = interpret(OK.as_bytes(), "", true, 0);
        let turn = result.unwrap();
        assert_eq!((turn.text.as_str(), turn.input_tokens, turn.output_tokens, turn.cost_usd), ("hi", 3, 4, 1.5));
        assert_eq!((usage.num_turns, usage.cache_read_tokens, usage.cache_creation_tokens), (2, 5, 6));
        assert_eq!(usage.session_id.as_deref(), Some("s1"));
        let models: Vec<_> = usage.models.iter().map(|m| (m.model.as_str(), m.cost_usd)).collect();
        assert_eq!(models, vec![("haiku", 0.1), ("opus", 1.4)]);
    }

    #[test]
    fn error_envelope_still_reports_usage() {
        let raw = r#"{"is_error":true,"result":"rate limited","total_cost_usd":0.25,"usage":{"input_tokens":9}}"#;
        let (result, usage) = interpret(raw.as_bytes(), "", false, 1);
        assert_eq!(result.unwrap_err().to_string(), AppError::Chat("rate limited".into()).to_string());
        assert_eq!((usage.cost_usd, usage.input_tokens), (0.25, 9));
    }

    #[test]
    fn crash_and_garbage_have_no_usage() {
        let (result, usage) = interpret(b"", "boom\n", false, 2);
        assert_eq!(result.unwrap_err().to_string(), AppError::Chat("boom".into()).to_string());
        assert_eq!(usage.cost_usd, 0.0);

        let (result, usage) = interpret(b"not json", "", true, 0);
        assert!(result.is_err());
        assert!(usage.models.is_empty());
    }

    #[test]
    fn interactive_argv_per_session_mode() {
        let argv = |session, file: Option<&Path>, mode| {
            ClaudeCode.interactive_argv(&Interactive { session, system_prompt_file: file, system_prompt_mode: mode })
        };
        let append = SystemPromptMode::Append;
        assert_eq!(argv(SessionMode::Ephemeral, None, append), Some(vec!["claude".to_string()]));
        assert_eq!(
            argv(SessionMode::Resume("s1"), None, append),
            Some(vec!["claude".into(), "--resume".into(), "s1".into()])
        );
        assert_eq!(
            argv(SessionMode::New("n1"), Some(Path::new("/ctx.md")), append),
            Some(vec![
                "claude".into(),
                "--session-id".into(),
                "n1".into(),
                "--append-system-prompt-file".into(),
                "/ctx.md".into()
            ])
        );
        assert_eq!(
            argv(SessionMode::Resume("s1"), Some(Path::new("/c")), SystemPromptMode::Replace),
            Some(vec!["claude".into(), "--resume".into(), "s1".into(), "--system-prompt-file".into(), "/c".into()])
        );
        assert_eq!(argv(SessionMode::Fork("s1"), None, append), None);
    }

    const STREAM: &str = include_str!("fixtures/claude_stream.jsonl");
    const RESULT_ERROR: &str = include_str!("fixtures/claude_result_error.jsonl");

    /// The stored `norm` of every line of `fixture`.
    fn norms(fixture: &str) -> Vec<Value> {
        fixture
            .lines()
            .map(|l| serde_json::from_str(&crate::ai::norm_json(&ClaudeCode.normalize(l), 100_000)).unwrap())
            .collect()
    }

    #[test]
    fn normalizes_a_claude_stream() {
        use serde_json::json;
        let n = norms(STREAM);
        let ev = |i: usize| n[i]["events"].clone();
        assert!(n.iter().all(|v| v["v"] == 1));
        // system init: kept as other
        assert_eq!(ev(0)[0]["type"], "other");
        assert_eq!(ev(0)[0]["kind"], "system");
        assert_eq!(ev(0)[0]["raw"]["session_id"], "s-1");
        // assistant text + tool_use
        assert_eq!(
            ev(1),
            json!([
                { "type": "text", "text": "Checking CI." },
                { "type": "tool_call", "id": "toolu_1", "name": "Bash", "input": { "command": "gh pr checks 1" } }
            ])
        );
        // tool_result, string content
        assert_eq!(ev(2), json!([{ "type": "tool_result", "id": "toolu_1", "ok": true, "text": "all green" }]));
        // tool_result, block content, is_error
        assert_eq!(ev(4), json!([{ "type": "tool_result", "id": "toolu_2", "ok": false, "text": "File does not exist.\nTry again." }]));
        // thinking-only message: nothing to show, kept as other
        assert_eq!(ev(5)[0]["type"], "other");
        assert_eq!(ev(5)[0]["kind"], "assistant");
        // malformed line
        assert_eq!(ev(6), json!([{ "type": "other", "kind": "raw", "raw": "not json at all" }]));
        // result ok, with usage
        assert_eq!(
            ev(7),
            json!([{
                "type": "final", "ok": true, "text": "All checks pass.",
                "usage": { "num_turns": 3, "input_tokens": 10, "output_tokens": 20, "cache_read_tokens": 30,
                           "cache_creation_tokens": 40, "cost_usd": 0.5 }
            }])
        );
    }

    #[test]
    fn normalizes_an_error_result() {
        let events = ClaudeCode.normalize(RESULT_ERROR.trim());
        let fin = events[0].as_final().expect("final event");
        assert!(fin.is_error);
        assert_eq!(fin.text.as_deref(), Some("Reached the turn limit"));
        assert_eq!((fin.usage.num_turns, fin.usage.cost_usd), (9, 0.25));
    }

    #[test]
    fn final_event_keeps_the_full_text_in_memory_and_truncates_when_stored() {
        let big = "é".repeat(60);
        let line = serde_json::json!({ "type": "result", "is_error": false, "result": big }).to_string();
        let events = ClaudeCode.normalize(&line);
        assert_eq!(events[0].as_final().unwrap().text.unwrap().len(), 120);
        let stored = events[0].to_json(11);
        assert_eq!(stored["text"].as_str().unwrap().len(), 10); // cut on a char boundary
        assert_eq!(stored["truncated"], true);
        // an oversized tool input is stored as a clipped string
        let call = Event::ToolCall { id: "i".into(), name: "Bash".into(), input: serde_json::json!({ "c": "x".repeat(50) }) };
        let j = call.to_json(20);
        assert!(j["input"].is_string() && j["truncated"] == true);
    }

    #[test]
    fn unknown_and_untyped_lines_never_fail() {
        for line in [r#"{"x":1}"#, r#"{"type":"stream_event"}"#, "", "{", r#"{"type":"assistant"}"#, r#"{"type":"user","message":{"content":"hi"}}"#] {
            let events = ClaudeCode.normalize(line);
            assert_eq!(events.len(), 1, "{line}");
            assert!(matches!(events[0], Event::Other { .. }), "{line}");
        }
    }

    #[test]
    fn spawn_refuses_an_inline_system_prompt() {
        let dir = std::env::temp_dir();
        let req = OneShot {
            model: "",
            system_prompt: Some(SystemPrompt::Text("x")),
            system_prompt_mode: SystemPromptMode::Append,
            file_tag: "t",
            prompt: "",
            cwd: &dir,
            extra_dirs: Vec::new(),
            tools: ToolPolicy::Allowlist(Vec::new()),
            access: Access::Default,
            session: SessionMode::Ephemeral,
            max_turns: None,
            effort: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            usage_tag: RunTag::new("t"),
        };
        let file = || std::fs::File::create(dir.join("codereview_spawn_test.out")).unwrap();
        let run = StreamRun { req, stdout: file(), stderr: file(), timeout: std::time::Duration::from_secs(1) };
        assert!(ClaudeCode.spawn(run).is_err());
        let _ = std::fs::remove_file(dir.join("codereview_spawn_test.out"));
    }
}
