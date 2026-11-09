//! OpenAI Codex (`codex` CLI) behind the [`Harness`] trait. The only place that builds a `codex`
//! command line or parses its output. Verified against codex-cli 0.167.0 (fixtures in
//! `ai/fixtures/codex_*`).
//!
//! What the CLI does, as observed:
//! - `codex exec --json` prints one JSON object per line: `thread.started` (the session id),
//!   `turn.started`, `item.started` / `item.completed` (`agent_message`, `command_execution`,
//!   `file_change`, and a non-fatal `error` item for warnings), `turn.completed` (`usage`) and
//!   `turn.failed` / `error`. There is **no result line**: the answer is the last `agent_message`,
//!   so the run engine folds the stream ([`Harness::accumulates`]). The prompt is read from stdin (`-`).
//! - Codex assigns the session id itself (`thread_id`); it cannot be pinned. `codex exec resume <id>`
//!   continues it. `exec resume` has no `-s`, `-C` or `--add-dir`: the sandbox and the writable
//!   roots go in as `-c` config overrides there (verified in the banner of a text run).
//! - `turn.completed.usage` is the total of the **whole thread**, not of the turn (a resumed turn
//!   reports the earlier turns too). `input_tokens` includes `cached_input_tokens`. No cost is reported.
//! - In `exec` mode nothing can be approved interactively: the approval policy is pinned to `never`.
//! - The sandbox is enforced by the OS (bubblewrap/landlock on Linux, seatbelt on macOS): a write
//!   in a `read-only` sandbox fails with a read-only file system. What it does **not** do: hide the
//!   rest of the file system (everything stays readable), or allow `git commit` in a linked
//!   worktree under `workspace-write` (the git dir is outside the writable roots).
//!
//! Auth stays in Codex (`codex login`); the app stores nothing.

use std::collections::HashMap;
use std::sync::Mutex;
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;

use super::opencode::{
    message, next_field, normalize_path, now_ms, parse_etime, proc_argv, proc_cwd, run_with_timeout, PROBE_TIMEOUT,
    START_SLACK_MS,
};
use super::{
    Access, AgentSession, Caps, Event, FinalResult, Harness, Interactive, OneShot, RunAcc, SessionMode, StreamRun, TurnResult,
};
use crate::ai_usage;
use crate::run_usage::{self, RunUsage};
use crate::error::{AppError, AppResult};

pub struct Codex;

/// Marker `settings_args` puts in the extra args for `allow_full`; [`command`] consumes it.
const MARK_FULL: &str = "--cr-allow-full=";
/// Marker prefix for the profile's `sandbox` setting; [`command`] consumes it.
const MARK_SANDBOX: &str = "--cr-sandbox=";
/// Weakest to strongest: a profile `sandbox` may only move down this list from the access-derived mode.
const SANDBOXES: [&str; 3] = ["read-only", "workspace-write", "danger-full-access"];

/// A per-process secret that goes after the markers, so a marker written by hand in `extra_args`
/// (`--cr-allow-full`) is not taken for the one `settings_args` made: `allow_full` only comes from
/// the settings key.
fn nonce() -> &'static str {
    static N: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    N.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        format!("{:016x}", h.finish())
    })
}

fn mark_full() -> String {
    format!("{MARK_FULL}{}", nonce())
}

fn mark_sandbox(mode: &str) -> String {
    format!("{MARK_SANDBOX}{}:{mode}", nonce())
}

impl Harness for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn caps(&self) -> Caps {
        Caps {
            sessions: true,
            resume: true,
            fork: false,
            tools: true,
            // Tool permissions are the sandbox's; Codex has no per-tool allowlist.
            tool_allowlist: false,
            streaming: true,
            interactive_tui: true,
            // Tokens only: Codex reports no cost.
            reports_cost: false,
            replaces_system_prompt: false,
            // Unlike opencode, a real read-only mode: the OS sandbox denies every write.
            access_levels: vec![Access::Default, Access::ReadOnly, Access::Edit, Access::Full],
            pins_session_id: false,
        }
    }

    fn available(&self) -> bool {
        crate::tools::codex_path().is_some()
    }

    fn version(&self) -> Option<String> {
        let out = Command::new(crate::tools::codex_bin()).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let text = text.strip_prefix("codex-cli ").unwrap_or(&text).to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    }

    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
        let message = message(req)?;
        let mut cmd = command(req)?;
        // The usage a resumed thread reports includes its earlier turns: take them off.
        let before = match req.session {
            SessionMode::Resume(id) => {
                let (usage, note) = thread_usage(id);
                if let Some(note) = note {
                    // The turn's usage will then include the earlier turns.
                    log::warn!(target: "ai", "codex {id}: {note}");
                }
                usage
            }
            _ => RunUsage::default(),
        };
        let started_at = run_usage::now_stamp();
        let started = std::time::Instant::now();
        let (result, usage) = execute(&mut cmd, &message, &before);
        let mut tag = req.usage_tag.clone();
        tag.model = req.model.to_string();
        let error = result.as_ref().err().map(|e| e.to_string());
        ai_usage::record(&tag, &started_at, started.elapsed().as_millis() as i64, &usage, error.as_deref());
        result
    }

    fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
        let mut cmd = command(&run.req)?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::from(run.stdout)).stderr(Stdio::from(run.stderr));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd.spawn().map_err(|e| AppError::Other(format!("failed to spawn codex: {e}")))
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
        let item = v.get("item");
        let item_kind = item.and_then(|i| i.get("type")).and_then(Value::as_str).unwrap_or_default();
        let mut events = Vec::new();
        match (kind, item_kind) {
            ("item.completed", "agent_message") => {
                let text = item.and_then(|i| i.get("text")).and_then(Value::as_str).unwrap_or_default();
                if !text.trim().is_empty() {
                    events.push(Event::Text(text.to_string()));
                }
            }
            ("item.started", "command_execution") => {
                events.push(Event::ToolCall {
                    id: item_str(item, "id"),
                    name: "command".into(),
                    input: serde_json::json!({ "command": item_str(item, "command") }),
                });
            }
            ("item.completed", "command_execution") => {
                let ok = item.and_then(|i| i.get("exit_code")).and_then(Value::as_i64) == Some(0)
                    && item_str(item, "status") != "failed";
                events.push(Event::ToolResult { id: item_str(item, "id"), ok, text: item_str(item, "aggregated_output") });
            }
            ("item.started", "file_change") => {
                events.push(Event::ToolCall {
                    id: item_str(item, "id"),
                    name: "file_change".into(),
                    input: serde_json::json!({ "changes": item.and_then(|i| i.get("changes")).cloned().unwrap_or(Value::Null) }),
                });
            }
            ("item.completed", "file_change") => {
                events.push(Event::ToolResult {
                    id: item_str(item, "id"),
                    ok: item_str(item, "status") == "completed",
                    text: changes_text(item.and_then(|i| i.get("changes"))),
                });
            }
            ("turn.completed", _) => {
                events.push(Event::Final { ok: true, text: String::new(), usage: usage_of(v.get("usage")) });
            }
            ("turn.failed", _) => {
                let message = v.pointer("/error/message").and_then(Value::as_str).filter(|m| !m.is_empty());
                events.push(Event::Final {
                    ok: false,
                    text: message.unwrap_or("codex reported an error").to_string(),
                    usage: RunUsage::default(),
                });
            }
            _ => {}
        }
        // Everything else (thread and turn starts, reasoning, warnings, the top-level `error` line
        // that `turn.failed` follows, unknown types) stays as `other`.
        if events.is_empty() {
            let kind = if item_kind.is_empty() { kind } else { item_kind };
            events.push(Event::Other { kind: kind.to_string(), raw: v });
        }
        events
    }

    fn accumulates(&self) -> bool {
        true
    }

    fn step_usage(&self, line: &str) -> Option<RunUsage> {
        let v = serde_json::from_str::<Value>(line).ok()?;
        (v.get("type").and_then(Value::as_str) == Some("turn.completed")).then(|| usage_of(v.get("usage")))
    }

    fn session_of_line(&self, line: &str) -> Option<String> {
        let v = serde_json::from_str::<Value>(line).ok()?;
        if v.get("type").and_then(Value::as_str) != Some("thread.started") {
            return None;
        }
        v.get("thread_id").and_then(Value::as_str).filter(|t| !t.is_empty()).map(String::from)
    }

    #[cfg(unix)]
    fn is_own_process(&self, pid: i32, _session_id: Option<&str>) -> bool {
        // The session id is Codex's own, not the one the run was started under: only the command is checked.
        let Ok(out) = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output() else {
            return false;
        };
        let cmd = String::from_utf8_lossy(&out.stdout);
        cmd.contains("codex") && cmd.contains(" exec")
    }

    #[cfg(not(unix))]
    fn is_own_process(&self, _pid: i32, _session_id: Option<&str>) -> bool {
        false
    }

    fn interactive_argv(&self, opts: &Interactive<'_>) -> Option<Vec<String>> {
        let mut argv = vec!["codex".to_string()];
        match opts.session {
            // Codex picks the id of a new session itself.
            SessionMode::Ephemeral | SessionMode::New(_) => {}
            SessionMode::Resume(id) => argv.extend(["resume".to_string(), id.to_string()]),
            SessionMode::Fork(_) => return None,
        }
        // No system prompt flag: the instructions go in as the opening prompt of a new session.
        if let Some(text) = opening_prompt(opts) {
            if text.starts_with('-') {
                argv.push("--".to_string());
            }
            argv.push(text);
        }
        Some(argv)
    }

    fn context_file_note(&self, opts: &Interactive<'_>) -> Option<String> {
        opts.system_prompt_file?;
        Some(if opening_prompt(opts).is_some() {
            "Codex has no system prompt flag: the context was passed as the opening prompt".to_string()
        } else if matches!(opts.session, SessionMode::New(_) | SessionMode::Ephemeral) {
            "context file skipped: it is empty or could not be read".to_string()
        } else {
            "context file skipped: Codex can only take it as the opening prompt of a new session".to_string()
        })
    }

    fn live_sessions(&self) -> Vec<AgentSession> {
        live_sessions_now()
    }

    fn models(&self) -> AppResult<Vec<String>> {
        // The catalog bundled with the binary: no network, no auth, no model call. Account-specific
        // models may be missing; the field takes free text.
        let out = Command::new(crate::tools::codex_bin())
            .args(["debug", "models", "--bundled"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| AppError::Other(format!("failed to run codex debug models: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Other("codex debug models failed".into()));
        }
        Ok(parse_models(&String::from_utf8_lossy(&out.stdout)))
    }

    fn validate_settings(&self, settings: &Value) -> AppResult<()> {
        let extra: Vec<String> = settings
            .get("extra_args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a.as_str().map(String::from))
            .collect();
        check_extra(&extra, ExtraCheck::Save)
    }

    fn settings_args(&self, settings: &Value) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(mode) = settings.get("sandbox").and_then(Value::as_str).map(str::trim).filter(|m| SANDBOXES.contains(m)) {
            args.push(mark_sandbox(mode));
        }
        if settings.get("allow_full").and_then(Value::as_bool) == Some(true) {
            args.push(mark_full());
        }
        for a in settings.get("extra_args").and_then(Value::as_array).into_iter().flatten() {
            if let Some(a) = a.as_str() {
                args.push(a.to_string());
            }
        }
        args
    }
}

/// The context file as the opening prompt: only for a new or ephemeral session, and only when the
/// file is readable and not blank. `interactive_argv` and `context_file_note` both go through it, so
/// the note always matches the argv.
fn opening_prompt(opts: &Interactive<'_>) -> Option<String> {
    let (file, SessionMode::New(_) | SessionMode::Ephemeral) = (opts.system_prompt_file?, opts.session) else { return None };
    std::fs::read_to_string(file).ok().filter(|t| !t.trim().is_empty())
}

/// The listed model ids of `codex debug models` (JSON `{"models":[{"slug","visibility","priority"}]}`), best first.
pub(crate) fn parse_models(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut models: Vec<(i64, String)> = v
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| m.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|m| Some((m.get("priority").and_then(Value::as_i64).unwrap_or(i64::MAX), m.get("slug")?.as_str()?.to_string())))
        .collect();
    models.sort();
    models.into_iter().map(|(_, slug)| slug).collect()
}

fn item_str(item: Option<&Value>, key: &str) -> String {
    item.and_then(|i| i.get(key)).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// `add path` lines of a `file_change` item.
fn changes_text(changes: Option<&Value>) -> String {
    changes
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| format!("{} {}", c.get("kind").and_then(Value::as_str).unwrap_or("change"), c.get("path").and_then(Value::as_str).unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The usage of a `usage` object (a `turn.completed` line, or a rollout's `total_token_usage`).
/// Codex counts cached tokens inside `input_tokens`; here they are apart, like Claude's.
fn usage_of(u: Option<&Value>) -> RunUsage {
    let n = |k: &str| u.and_then(|u| u.get(k)).and_then(Value::as_i64).unwrap_or(0);
    let cached = n("cached_input_tokens");
    RunUsage {
        num_turns: 1,
        input_tokens: (n("input_tokens") - cached).max(0),
        // `output_tokens` already counts the reasoning tokens.
        output_tokens: n("output_tokens"),
        cache_read_tokens: cached,
        cache_creation_tokens: n("cache_write_input_tokens"),
        cost_usd: 0.0,
        ..RunUsage::default()
    }
}

fn minus(mut a: RunUsage, b: &RunUsage) -> RunUsage {
    a.input_tokens = (a.input_tokens - b.input_tokens).max(0);
    a.output_tokens = (a.output_tokens - b.output_tokens).max(0);
    a.cache_read_tokens = (a.cache_read_tokens - b.cache_read_tokens).max(0);
    a.cache_creation_tokens = (a.cache_creation_tokens - b.cache_creation_tokens).max(0);
    a
}

/// The sandbox flags and config keys `extra_args` may not carry: the access level and the profile's
/// `sandbox` / `allow_full` settings decide the sandbox, nothing else.
const FORBIDDEN_FLAGS: &[(&str, &str)] = &[
    ("--dangerously-bypass-approvals-and-sandbox", "--dangerously-bypass-approvals-and-sandbox"),
    ("--yolo", "--yolo"),
    ("--full-auto", "--full-auto"),
    ("-s", "--sandbox"),
    ("--sandbox", "--sandbox"),
    ("-a", "--ask-for-approval"),
    ("--ask-for-approval", "--ask-for-approval"),
    ("-p", "--profile"),
    ("--profile", "--profile"),
    // `--cd` moves the workspace root (the writable root) anywhere; the app sets the directory.
    ("-C", "--cd"),
    ("--cd", "--cd"),
    // Feature flags can turn on approval and permission machinery; no harmless subset is known.
    ("--enable", "--enable"),
    ("--disable", "--disable"),
    // Approval and trust machinery, the working tree, and rule / user-config loading.
    ("--approve-for-me", "--approve-for-me"),
    ("--dangerously-bypass-hook-trust", "--dangerously-bypass-hook-trust"),
    ("--worktree", "--worktree"),
    ("--ignore-rules", "--ignore-rules"),
    ("--ignore-user-config", "--ignore-user-config"),
    // Makes codex write an arbitrary file, outside the sandbox.
    ("-o", "--output-last-message"),
    ("--output-last-message", "--output-last-message"),
];

/// The only `-c` keys `extra_args` may set: model tuning and display, none of which touch the
/// sandbox, the approvals, permissions, profiles or features. Everything else is refused (an
/// allowlist, because `codex -c` takes quoted, dotted and aliased keys: `"sandbox_mode"`,
/// `default_permissions`, `permissions.*`, `profiles.*`, `features.*`, ... all widen access).
pub(crate) const ALLOWED_CONFIG_KEYS: &[&str] = &[
    "model",
    "model_reasoning_effort",
    "model_reasoning_summary",
    "model_verbosity",
    "hide_agent_reasoning",
    "show_raw_agent_reasoning",
    "personality",
];

/// The dotted segments of the key of a `-c key=value` override, with quotes and whitespace
/// stripped from each (TOML allows `"a" . 'b'`). `None` when it has no `=` or a quote is open.
fn config_key(kv: &str) -> Option<Vec<String>> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut chars = kv.chars();
    while let Some(c) = chars.next() {
        match c {
            '=' => {
                segs.push(cur.trim().to_string());
                return Some(segs);
            }
            '.' => segs.push(std::mem::take(&mut cur).trim().to_string()),
            q @ ('"' | '\'') => loop {
                match chars.next()? {
                    '\\' if q == '"' => cur.push(chars.next()?),
                    c if c == q => break,
                    c => cur.push(c),
                }
            },
            c => cur.push(c),
        }
    }
    None
}

/// Whether a `-c` override is outside the allowlist: a single-segment, allowlisted key. The value
/// is never looked at, so inline tables and arrays cannot smuggle a key in.
fn forbidden_config(kv: &str) -> bool {
    match config_key(kv) {
        Some(segs) => !(segs.len() == 1 && ALLOWED_CONFIG_KEYS.contains(&segs[0].as_str())),
        None => true,
    }
}

/// The first flag of `extra_args` that could change the sandbox or the approvals, if any.
fn forbidden_extra(args: &[String]) -> Option<String> {
    let mut it = args.iter().map(String::as_str);
    while let Some(a) = it.next() {
        if a.starts_with("--cr-") {
            return Some(a.split('=').next().unwrap_or(a).to_string());
        }
        let (flag, attached) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (a, None),
        };
        if let Some((_, name)) = FORBIDDEN_FLAGS.iter().find(|(f, _)| *f == flag) {
            return Some(name.to_string());
        }
        // `-sread-only`, `-awhatever`: a short flag with its value attached.
        if !a.starts_with("--") {
            if let Some((_, name)) = FORBIDDEN_FLAGS.iter().find(|(f, _)| f.len() == 2 && a.len() > 2 && a.starts_with(f)) {
                return Some(name.to_string());
            }
        }
        if flag == "-c" || flag == "--config" || (!a.starts_with("--") && a.starts_with("-c") && a.len() > 2) {
            let kv = match (attached, a.strip_prefix("-c").filter(|r| !r.is_empty() && !a.starts_with("--"))) {
                (Some(v), _) => Some(v),
                (None, Some(r)) => Some(r.trim_start_matches('=')),
                _ => it.next(),
            };
            if kv.is_some_and(forbidden_config) {
                return Some(format!(
                    "{flag} {} (only these -c keys are allowed: {})",
                    kv.unwrap_or_default().split('=').next().unwrap_or("").trim(),
                    ALLOWED_CONFIG_KEYS.join(", ")
                ));
            }
        }
    }
    None
}

/// Where `extra_args` are checked: on profile save (no run yet, paths need not exist) or for a run
/// in a working directory (paths must be existing directories).
#[derive(Clone, Copy)]
enum ExtraCheck<'a> {
    Save,
    Run(&'a Path),
}

/// Credential and system roots `--add-dir` may not name or sit beneath. Relative to the home dir.
const HOME_DENY: &[&str] = &[".ssh", ".gnupg", ".aws", ".config", ".codex", ".claude"];
const SYSTEM_DENY: &[&str] = &["/etc", "/usr", "/bin", "/sbin", "/boot", "/proc", "/sys", "/dev"];

/// `p` with `.` and `..` resolved lexically; `None` when a `..` climbs above the root.
fn lexical_normalize(p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::from("/");
    for c in p.components() {
        match c {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(n) => out.push(n),
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// The canonical form of `p`: its deepest existing ancestor canonicalized (symlinks followed) plus
/// the missing rest.
fn canonical_or_normalized(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(c) = std::fs::canonicalize(cur) {
            return rest.iter().rev().fold(c, |acc, n| acc.join(n));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Why `--add-dir <raw>` is refused, if it is. A heuristic against widening the writable roots to
/// the home directory, the working directory's ancestors and well-known credential / system
/// directories; other sensitive directories pass.
fn add_dir_problem(raw: &str, check: ExtraCheck<'_>, home: Option<&Path>) -> Option<String> {
    if raw.is_empty() {
        return Some("the path is empty".into());
    }
    if raw.contains(['\0', '\n', '\r']) {
        return Some("the path contains a NUL or newline".into());
    }
    if raw == "~" || raw.starts_with("~/") {
        return Some("`~` is not expanded: give an absolute path".into());
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Some("the path must be absolute".into());
    }
    let Some(norm) = lexical_normalize(path) else {
        return Some("the path climbs above the root with `..`".into());
    };
    if let ExtraCheck::Run(_) = check {
        if !std::fs::metadata(&norm).is_ok_and(|m| m.is_dir()) {
            return Some("the directory does not exist".into());
        }
    }
    let canon = canonical_or_normalized(&norm);
    if canon == Path::new("/") {
        return Some("the file system root".into());
    }
    if let Some(home) = home {
        let home_canon = canonical_or_normalized(home);
        if home.starts_with(&canon) || home_canon.starts_with(&canon) {
            return Some("the home directory or one of its ancestors".into());
        }
        for d in HOME_DENY {
            for h in [home, home_canon.as_path()] {
                let deny = h.join(d);
                if canon.starts_with(&deny) || norm.starts_with(&deny) {
                    return Some(format!("it is under ~/{d}"));
                }
            }
        }
    }
    for d in SYSTEM_DENY {
        if canon.starts_with(d) || norm.starts_with(d) {
            return Some(format!("it is under {d}"));
        }
    }
    if let ExtraCheck::Run(cwd) = check {
        let cwd_norm = lexical_normalize(&std::path::absolute(cwd).unwrap_or_else(|_| cwd.to_path_buf()));
        let cwd_canon = canonical_or_normalized(&cwd_norm.clone().unwrap_or_else(|| cwd.to_path_buf()));
        if cwd_canon.starts_with(&canon) || cwd_norm.is_some_and(|c| c.starts_with(&canon)) {
            return Some("it is the working directory or one of its ancestors".into());
        }
    }
    None
}

/// The first `--add-dir` of `extra_args` that is refused, as a message.
fn add_dir_error(args: &[String], check: ExtraCheck<'_>) -> Option<String> {
    let home = dirs::home_dir();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let value = if a == "--add-dir" {
            match it.next() {
                Some(v) => v.as_str(),
                None => return Some("--add-dir needs a directory".into()),
            }
        } else if let Some(v) = a.strip_prefix("--add-dir=") {
            v
        } else {
            continue;
        };
        if let Some(why) = add_dir_problem(value, check, home.as_deref()) {
            return Some(format!("--add-dir {value:?} is refused: {why}"));
        }
    }
    None
}

/// Refuses `extra_args` that could change the sandbox or the approvals, or that add an unsafe
/// writable root (at run time and on save).
fn check_extra(args: &[String], check: ExtraCheck<'_>) -> AppResult<()> {
    if let Some(flag) = forbidden_extra(args) {
        return Err(AppError::Other(format!(
            "Codex extra_args may not contain {flag}: the sandbox and approvals come from the run's access level and the profile's \"sandbox\" and \"allow_full\" settings"
        )));
    }
    match add_dir_error(args, check) {
        Some(msg) => Err(AppError::Other(format!("Codex extra_args: {msg}"))),
        None => Ok(()),
    }
}

fn rank(mode: &str) -> usize {
    SANDBOXES.iter().position(|m| *m == mode).unwrap_or(0)
}

/// The sandbox mode of a request and its extra args with the markers taken out. The access level
/// sets the mode; a profile's `sandbox` setting can only lower it (read-only < workspace-write <
/// danger-full-access), never raise it. `danger-full-access` needs the profile's `allow_full`, and
/// `extra_args` may not carry any flag that changes the sandbox or the approvals.
fn plan(req: &OneShot<'_>) -> AppResult<(&'static str, Vec<String>)> {
    let (full, sandbox) = (mark_full(), format!("{MARK_SANDBOX}{}:", nonce()));
    let mut allow_full = false;
    let mut chosen: Option<&'static str> = None;
    let mut rest = Vec::new();
    for a in &req.extra_args {
        if *a == full {
            allow_full = true;
        } else if let Some(m) = a.strip_prefix(&sandbox) {
            chosen = SANDBOXES.iter().copied().find(|s| *s == m);
        } else {
            rest.push(a.clone());
        }
    }
    check_extra(&rest, ExtraCheck::Run(req.cwd))?;
    let derived = match &req.access {
        Access::Edit => "workspace-write",
        Access::Full => "danger-full-access",
        // The default is the strictest: nothing is written unless the request says so.
        Access::Default | Access::ReadOnly | Access::Native(_) => "read-only",
    };
    let mode = chosen.filter(|c| rank(c) < rank(derived)).unwrap_or(derived);
    if mode == "danger-full-access" && !allow_full {
        return Err(AppError::Other(
            "this Codex profile does not allow full access (no sandbox): set \"allow_full\": true in its settings, or use a lower access level".into(),
        ));
    }
    Ok((mode, rest))
}

/// A TOML basic string (JSON string escapes are valid TOML).
fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The `codex exec` command of `req` (the prompt goes to stdin).
pub(crate) fn command(req: &OneShot<'_>) -> AppResult<Command> {
    let (mode, rest) = plan(req)?;
    let resume = match req.session {
        SessionMode::Resume(id) => Some(id),
        SessionMode::Fork(_) => return Err(AppError::Other("Codex can't fork a session".into())),
        SessionMode::New(_) | SessionMode::Ephemeral => None,
    };
    let mut cmd = Command::new(crate::tools::codex_bin());
    cmd.current_dir(req.cwd);
    cmd.arg("exec");
    if resume.is_some() {
        cmd.arg("resume");
    }
    cmd.args(["--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\""]);
    if matches!(req.session, SessionMode::Ephemeral) {
        cmd.arg("--ephemeral");
    }
    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }
    if let Some(effort) = req.effort.filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
        cmd.arg("-c").arg(format!("model_reasoning_effort={}", toml_str(effort)));
    }
    // `exec resume` takes neither `-s` nor `--add-dir`: the same settings go in as config overrides.
    if resume.is_some() {
        cmd.arg("-c").arg(format!("sandbox_mode={}", toml_str(mode)));
    } else {
        cmd.args(["--sandbox", mode]);
    }
    // Extra directories only matter (and are only granted) when the sandbox lets the workspace be
    // written; in a read-only sandbox everything is readable already.
    if mode == "workspace-write" && !req.extra_dirs.is_empty() {
        if resume.is_some() {
            let roots: Vec<String> = req.extra_dirs.iter().map(|d| toml_str(&d.to_string_lossy())).collect();
            cmd.arg("-c").arg(format!("sandbox_workspace_write.writable_roots=[{}]", roots.join(",")));
        } else {
            for d in &req.extra_dirs {
                cmd.arg("--add-dir").arg(d);
            }
        }
    }
    // The tool policy has no Codex equivalent: the sandbox decides what a command may do.
    cmd.args(&rest);
    if let Some(id) = resume {
        cmd.arg(id);
    }
    cmd.arg("-");
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    Ok(cmd)
}

fn execute(cmd: &mut Command, message: &str, before: &RunUsage) -> (AppResult<TurnResult>, RunUsage) {
    let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return (Err(AppError::Chat(format!("failed to spawn codex: {e}"))), RunUsage::default()),
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
        Err(e) => return (Err(AppError::Chat(format!("codex process error: {e}"))), RunUsage::default()),
    };
    interpret(&String::from_utf8_lossy(&output.stdout), &String::from_utf8_lossy(&output.stderr), output.status.code().unwrap_or(-1), before)
}

/// The stderr of `codex exec` without its noise: the stdin notice and the MCP transport errors of
/// the user's configured servers.
pub(crate) fn clean_stderr(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("Reading additional input") && !l.contains("rmcp::"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Turn the finished output of `codex exec --json` into the run result and its usage. `before` is
/// what the thread had used already (a resumed thread reports its total).
pub(crate) fn interpret(stdout: &str, stderr: &str, exit_code: i32, before: &RunUsage) -> (AppResult<TurnResult>, RunUsage) {
    let h = Codex;
    let mut acc = RunAcc::default();
    let mut fin: Option<FinalResult> = None;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let events = h.normalize(line);
        acc.push(&h, line, &events);
        if let Some(f) = events.iter().find_map(Event::as_final) {
            fin = Some(acc.complete(f));
        }
    }
    let Some(mut fin) = fin else {
        let stderr = clean_stderr(stderr);
        let message = if stderr.is_empty() { format!("codex exited {exit_code} without a result") } else { stderr };
        return (Err(AppError::Chat(message)), RunUsage::default());
    };
    fin.usage = minus(fin.usage, before);
    let text = fin.text.clone().unwrap_or_default();
    if fin.is_error {
        return (Err(AppError::Chat(text)), fin.usage);
    }
    let turn = TurnResult {
        session_id: fin.usage.session_id.clone(),
        text,
        input_tokens: fin.usage.input_tokens + fin.usage.cache_read_tokens,
        output_tokens: fin.usage.output_tokens,
        cost_usd: 0.0,
    };
    (Ok(turn), fin.usage)
}

// ---- session logs on disk --------------------------------------------------------------------

/// `$CODEX_HOME` or `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// The header (first line) of a `rollout-<time>-<id>.jsonl` session log.
#[derive(Debug, Clone, PartialEq)]
struct Rollout {
    id: String,
    cwd: String,
    /// `codex_exec` for `codex exec` runs, `codex-tui` for the TUI, `Codex Desktop` for the app.
    originator: String,
    source: String,
    /// Last write to the log, ms since the epoch.
    updated: i64,
}

impl Rollout {
    /// A log some interactive Codex client wrote (not a `codex exec` run, which is no TUI's).
    fn interactive(&self) -> bool {
        self.originator != "codex_exec" && self.originator != "Codex Desktop" && self.source != "exec"
    }
}

/// The session header in the first line of a log; `updated` is filled by the caller.
fn parse_rollout_meta(first_line: &str) -> Option<Rollout> {
    let v: Value = serde_json::from_str(first_line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let p = v.get("payload")?;
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let id = p.get("id").or_else(|| p.get("session_id")).and_then(Value::as_str)?.to_string();
    Some(Rollout { id, cwd: s("cwd"), originator: s("originator"), source: s("source"), updated: 0 })
}

/// The header of the log at `path` (read up to 1 MiB of its first line), with its mtime.
fn read_rollout(path: &Path) -> Option<Rollout> {
    let file = std::fs::File::open(path).ok()?;
    let mtime = file.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    let mut line = String::new();
    std::io::BufReader::new(file.take(1 << 20)).read_line(&mut line).ok()?;
    parse_rollout_meta(&line).map(|r| Rollout { updated: mtime, ..r })
}

/// The newest `limit` `YYYY/MM/DD` directories under `sessions/`, newest first. Walks years, then
/// months, then days lazily, so an old archive is never listed past the cap.
fn day_dirs(sessions: &Path, limit: usize) -> Vec<PathBuf> {
    let sub = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        v.reverse();
        v
    };
    let mut out = Vec::new();
    'years: for y in sub(sessions) {
        for m in sub(&y) {
            for d in sub(&m) {
                out.push(d);
                if out.len() >= limit {
                    break 'years;
                }
            }
        }
    }
    out
}

/// How many day directories (newest first) a search for recently written logs looks at.
const RECENT_DAYS: usize = 7;

/// Logs under `sessions` written at or after `since_ms` (looking at the last [`RECENT_DAYS`] days).
fn recent_rollouts(sessions: &Path, since_ms: i64) -> Vec<Rollout> {
    day_dirs(sessions, RECENT_DAYS)
        .into_iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|p| read_rollout(&p))
        .filter(|r| r.updated >= since_ms)
        .collect()
}

/// How many day directories a search for one session's log walks (newest first).
const MAX_DAY_DIRS: usize = 400;

/// The path of the log of session `id`, looking at the newest [`MAX_DAY_DIRS`] day directories.
fn find_rollout_path(sessions: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("-{id}.jsonl");
    day_dirs(sessions, MAX_DAY_DIRS).into_iter().find_map(|d| {
        std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)))
    })
}

/// Rollout paths found so far, by session id, for the life of the process.
fn rollout_cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    static C: std::sync::OnceLock<Mutex<HashMap<String, PathBuf>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

fn cached_rollout_path(sessions: &Path, id: &str) -> Option<PathBuf> {
    if let Some(p) = rollout_cache().lock().unwrap().get(id).filter(|p| p.is_file()) {
        return Some(p.clone());
    }
    let found = find_rollout_path(sessions, id)?;
    rollout_cache().lock().unwrap().insert(id.to_string(), found.clone());
    Some(found)
}

/// How much of the end of a log is read to find its last `token_count`.
const TAIL_BYTES: u64 = 1 << 20;

/// The last [`TAIL_BYTES`] of the file at `path`, read lossily (a cut multi-byte char is harmless).
fn read_tail(path: &Path) -> std::io::Result<String> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// What the session `id` has used so far, from the last `token_count` of its log, and a note when
/// that could not be told (no log, unreadable, no `token_count`): zero usage would otherwise look real.
fn thread_usage(id: &str) -> (RunUsage, Option<String>) {
    let Some(path) = codex_home().and_then(|h| cached_rollout_path(&h.join("sessions"), id)) else {
        return (RunUsage::default(), Some("usage before this turn unknown: the session log was not found".into()));
    };
    match read_tail(&path) {
        Ok(t) => match last_total_usage(&t) {
            Some(u) => (u, None),
            None => (RunUsage::default(), Some("usage before this turn unknown: no token count in the session log".into())),
        },
        Err(e) => (RunUsage::default(), Some(format!("usage before this turn unknown: the session log is unreadable ({e})"))),
    }
}

/// The `total_token_usage` of the last `token_count` event of a log.
fn last_total_usage(log: &str) -> Option<RunUsage> {
    log.lines()
        .rev()
        .filter(|l| l.contains("total_token_usage"))
        .find_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            let total = v.pointer("/payload/info/total_token_usage")?;
            Some(usage_of(Some(total)))
        })
}

/// Titles by session id from `session_index.jsonl` (`{"id","thread_name"}` lines, the last one wins).
fn parse_titles(index: &str) -> HashMap<String, String> {
    index
        .lines()
        .filter_map(|l| {
            let v: Value = serde_json::from_str(l).ok()?;
            let name = v.get("thread_name")?.as_str()?.trim();
            let id = v.get("id")?.as_str()?.to_string();
            (!name.is_empty()).then(|| (id, name.to_string()))
        })
        .collect()
}

// ---- session discovery -----------------------------------------------------------------------

/// A running Codex TUI process.
#[derive(Debug, Clone, PartialEq)]
struct Tui {
    pid: i64,
    tty: Option<String>,
    cwd: String,
    /// The session it was opened on (`codex resume <uuid>`), when given.
    session: Option<String>,
    started_ms: i64,
}

/// Subcommands of the Codex CLI that are not the TUI (`codex --help` of 0.167.0 plus `mcp-server`),
/// so `codex exec …` or `codex app-server` is never taken for an interactive session. `resume` and
/// `fork` are TUIs and handled apart.
const SUBCOMMANDS: &[&str] = &[
    "agents", "exec", "e", "review", "login", "logout", "mcp", "mcp-server", "plugin", "app-server", "remote-control",
    "completion", "update", "doctor", "sandbox", "debug", "apply", "a", "queue", "archive", "delete", "migrate-rollouts",
    "migrate-nfs-db", "unarchive", "cloud", "exec-server", "features", "help",
];

/// Flags that take their value as the next argument. Anything else starting with `-` is a switch.
const VALUE_FLAGS: &[&str] = &[
    "-c", "--config", "--enable", "--disable", "--remote", "--remote-auth-token-env", "-m", "--model", "--local-provider",
    "-p", "--profile", "-s", "--sandbox", "-a", "--ask-for-approval", "-C", "--cd", "--add-dir", "--thread-source",
    "--cyber-access-program",
];

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// `(session id, --cd directory)` of a codex command line when it is an interactive TUI (no
/// subcommand but `resume` / `fork`); `None` for anything else. The session is only given by
/// `codex resume <uuid>`; a name, `--last` or the picker leave it unknown. The first word that is
/// not a flag or flag value is the opening prompt: nothing after it is read.
fn parse_tui_args(argv: &[String]) -> Option<(Option<String>, Option<String>)> {
    let bin = argv.first()?;
    if bin.rsplit('/').next() != Some("codex") {
        return None;
    }
    let (mut session, mut dir) = (None, None);
    let mut it = argv[1..].iter().map(String::as_str).peekable();
    let mut resume = false;
    // Past the opening prompt only `-C` / `--cd` (and the flags' values) are read.
    let mut prompted = false;
    while let Some(a) = it.next() {
        match a {
            "-C" | "--cd" => dir = it.next().map(String::from),
            // `-i` takes several files; with `resume` a uuid after them is still the session.
            "-i" | "--image" => {
                while it.next_if(|v| !(v.starts_with('-') || resume && looks_like_uuid(v))).is_some() {}
            }
            _ if VALUE_FLAGS.contains(&a) => {
                it.next();
            }
            _ if a.starts_with("--cd=") => dir = a.strip_prefix("--cd=").map(String::from),
            _ if a.starts_with('-') => {}
            _ if prompted => {}
            "resume" if !resume => resume = true,
            "fork" if !resume => prompted = true,
            _ if !resume && SUBCOMMANDS.contains(&a) => return None,
            _ => {
                if resume && looks_like_uuid(a) {
                    session = Some(a.to_string());
                }
                prompted = true;
            }
        }
    }
    Some((session, dir))
}

/// Parse `ps -eo pid=,tty=,etime=,args=` output into the running Codex TUIs (see
/// `opencode::parse_tuis`, which this mirrors).
fn parse_tuis(
    ps: &str,
    argv_of: impl Fn(i64) -> Option<Vec<String>>,
    cwd_of: impl Fn(i64) -> Option<String>,
    is_dir: impl Fn(&str) -> bool,
    now_ms: i64,
) -> Vec<Tui> {
    ps.lines()
        .filter_map(|line| {
            let (pid, rest) = next_field(line)?;
            let pid: i64 = pid.parse().ok()?;
            let (tty, rest) = next_field(rest)?;
            let (etime, args) = next_field(rest)?;
            let started_ms = now_ms - parse_etime(etime)? * 1000;
            let ps_argv: Vec<String> = args.split_whitespace().map(String::from).collect();
            if ps_argv.first()?.rsplit('/').next() != Some("codex") {
                return None;
            }
            let argv = argv_of(pid).filter(|a| !a.is_empty()).unwrap_or(ps_argv);
            let (session, dir) = parse_tui_args(&argv)?;
            let base = normalize_path(&cwd_of(pid)?);
            let cwd = dir
                .map(|d| if d.starts_with('/') { normalize_path(&d) } else { normalize_path(&format!("{base}/{d}")) })
                .filter(|d| is_dir(d))
                .unwrap_or(base);
            Some(Tui { pid, tty: (tty != "?" && tty != "??").then(|| tty.to_string()), cwd, session, started_ms })
        })
        .collect()
}

/// Pair each TUI with its session.
/// - `codex resume <uuid>` has that session: its log (if found) gives the directory and last write.
/// - Codex writes a session's log on the first message, not at startup. An unpinned TUI is paired
///   only when that is unambiguous: it is the one unpinned TUI of its directory and an interactive
///   log of that directory, not taken by a pinned TUI, was written since the TUI started. The newest
///   such log wins. Otherwise it is left out (it has no session yet).
fn match_tuis(
    tuis: &[Tui],
    recent: impl Fn(i64) -> Vec<Rollout>,
    by_id: impl Fn(&str) -> Option<Rollout>,
    titles: &HashMap<String, String>,
) -> Vec<AgentSession> {
    let session_of = |t: &Tui, id: &str, r: Option<&Rollout>| AgentSession {
        harness: "codex".into(),
        session_id: id.to_string(),
        pid: Some(t.pid),
        live: true,
        status: None,
        kind: Some("interactive".into()),
        attached: false,
        job_id: None,
        waiting_for: None,
        agent_view: None,
        name: titles.get(id).cloned(),
        cwd: r.map(|r| r.cwd.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| t.cwd.clone()),
        git_branch: None,
        repo: None,
        tmux: None,
        tty: t.tty.clone(),
        updated_at: r.map_or(t.started_ms, |r| r.updated.max(t.started_ms)),
        focus: crate::claude_sessions::FocusState::NoTerminal,
        summaries: Vec::new(),
        original_name: None,
    };
    let mut out: Vec<AgentSession> = Vec::new();
    for t in tuis.iter().filter(|t| t.session.is_some()) {
        let id = t.session.as_deref().unwrap_or_default();
        out.push(session_of(t, id, by_id(id).as_ref()));
    }
    let unpinned: Vec<&Tui> = tuis.iter().filter(|t| t.session.is_none()).collect();
    if let Some(earliest) = unpinned.iter().map(|t| t.started_ms).min() {
        let recent = recent(earliest - START_SLACK_MS);
        for t in &unpinned {
            if unpinned.iter().filter(|o| o.cwd == t.cwd).count() > 1 {
                continue;
            }
            let found = recent
                .iter()
                .filter(|r| r.interactive() && normalize_path(&r.cwd) == t.cwd && r.updated >= t.started_ms - START_SLACK_MS)
                .filter(|r| !out.iter().any(|o| o.session_id == r.id))
                .max_by_key(|r| r.updated);
            if let Some(r) = found {
                out.push(session_of(t, &r.id, Some(r)));
            }
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out
}

/// The running Codex TUIs (one `ps` call, plus a cwd lookup per TUI).
fn live_tuis() -> Vec<Tui> {
    let mut ps = Command::new("ps");
    ps.args(["-eo", "pid=,tty=,etime=,args="]);
    let Some(out) = run_with_timeout(&mut ps, PROBE_TIMEOUT) else { return Vec::new() };
    parse_tuis(&out, proc_argv, proc_cwd, |d| Path::new(d).is_dir(), now_ms())
}

/// The working directories of the running Codex TUIs, with no session lookup: what the worktree GC
/// needs to keep a worktree alive.
pub(crate) fn live_tui_cwds() -> Vec<String> {
    live_tuis().into_iter().map(|t| t.cwd).collect()
}

/// Live Codex TUIs and their sessions, from the process list and the session logs on disk (no
/// subprocess of codex itself, no model call). Sessions with no running TUI are not reported.
fn live_sessions_now() -> Vec<AgentSession> {
    let tuis = live_tuis();
    if tuis.is_empty() {
        return Vec::new();
    }
    let Some(home) = codex_home() else { return Vec::new() };
    let sessions = home.join("sessions");
    let titles = std::fs::read_to_string(home.join("session_index.jsonl")).map(|t| parse_titles(&t)).unwrap_or_default();
    match_tuis(
        &tuis,
        |since| recent_rollouts(&sessions, since),
        |id| find_rollout_path(&sessions, id).and_then(|p| read_rollout(&p)),
        &titles,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{check_caps, norm_json, Needs, SystemPromptMode, ToolPolicy};
    use crate::run_usage::RunTag;

    const OK: &str = include_str!("fixtures/codex_exec_ok.jsonl");
    const TOOL: &str = include_str!("fixtures/codex_exec_tool.jsonl");
    const RESUME: &str = include_str!("fixtures/codex_exec_resume.jsonl");
    const ERROR_MODEL: &str = include_str!("fixtures/codex_exec_error_model.jsonl");
    const FILE_CHANGE: &str = include_str!("fixtures/codex_exec_file_change.jsonl");
    const NOISE: &str = include_str!("fixtures/codex_stderr_noise.txt");
    const MODELS: &str = include_str!("fixtures/codex_models.json");
    const META_TUI: &str = include_str!("fixtures/codex_rollout_meta_tui.json");
    const META_EXEC: &str = include_str!("fixtures/codex_rollout_meta_exec.json");
    const TOKENS: &str = include_str!("fixtures/codex_rollout_tokens.jsonl");

    fn norm(line: &str) -> Value {
        serde_json::from_str(&norm_json(&Codex.normalize(line), 100_000)).unwrap()
    }

    fn request<'a>(cwd: &'a Path) -> OneShot<'a> {
        OneShot {
            model: "",
            system_prompt: None,
            system_prompt_mode: SystemPromptMode::Append,
            file_tag: "t",
            prompt: "hi",
            cwd,
            extra_dirs: Vec::new(),
            tools: ToolPolicy::Full,
            access: Access::ReadOnly,
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

    // ---- stream parsing --------------------------------------------------------------------

    #[test]
    fn a_plain_answer_is_text_then_a_final_with_the_thread_usage() {
        let events: Vec<Value> = OK.lines().map(norm).collect();
        assert_eq!(events[0]["events"][0]["type"], "other");
        assert_eq!(events[0]["events"][0]["kind"], "thread.started");
        assert_eq!(events[1]["events"][0]["kind"], "turn.started");
        assert_eq!(events[2]["events"][0], serde_json::json!({ "type": "text", "text": "ok" }));
        let fin = &events[3]["events"][0];
        assert_eq!((fin["type"].as_str(), fin["ok"].as_bool(), fin["text"].as_str()), (Some("final"), Some(true), Some("")));
        assert_eq!(fin["usage"]["input_tokens"], 20832);
        assert_eq!(fin["usage"]["output_tokens"], 5);
        assert_eq!(fin["usage"]["cost_usd"], 0.0);
    }

    #[test]
    fn a_command_is_a_tool_call_then_its_result() {
        let events: Vec<Value> = TOOL.lines().map(norm).collect();
        let call = &events[3]["events"][0];
        assert_eq!((call["type"].as_str(), call["name"].as_str(), call["id"].as_str()), (Some("tool_call"), Some("command"), Some("item_1")));
        assert_eq!(call["input"]["command"], "/bin/bash -lc 'cat note.txt'");
        let result = &events[4]["events"][0];
        assert_eq!((result["type"].as_str(), result["id"].as_str(), result["ok"].as_bool()), (Some("tool_result"), Some("item_1"), Some(true)));
        assert_eq!(result["text"], "hello\n");
        assert_eq!(events[5]["events"][0]["text"], "hello");
    }

    #[test]
    fn a_failed_command_is_a_failed_result() {
        let line = r#"{"type":"item.completed","item":{"id":"item_3","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"failed"}}"#;
        let ev = norm(line);
        assert_eq!((ev["events"][0]["type"].as_str(), ev["events"][0]["ok"].as_bool()), (Some("tool_result"), Some(false)));
    }

    #[test]
    fn a_file_change_is_a_call_and_a_result_listing_the_paths() {
        let events: Vec<Value> = FILE_CHANGE.lines().map(norm).collect();
        let call = &events[3]["events"][0];
        assert_eq!((call["type"].as_str(), call["name"].as_str()), (Some("tool_call"), Some("file_change")));
        assert_eq!(call["input"]["changes"][0]["kind"], "add");
        let result = &events[4]["events"][0];
        assert_eq!((result["ok"].as_bool(), result["text"].as_str()), (Some(true), Some("add /work/repo/hi.txt")));
    }

    #[test]
    fn a_failed_turn_is_a_failed_final_and_a_warning_item_stays_other() {
        let events: Vec<Value> = ERROR_MODEL.lines().map(norm).collect();
        // thread.started, error item (warning), turn.started, error, turn.failed
        assert_eq!(events[1]["events"][0]["type"], "other");
        assert_eq!(events[1]["events"][0]["kind"], "error");
        assert_eq!(events[3]["events"][0]["type"], "other");
        let fin = &events[4]["events"][0];
        assert_eq!((fin["type"].as_str(), fin["ok"].as_bool()), (Some("final"), Some(false)));
        assert!(fin["text"].as_str().unwrap().contains("not supported"), "{fin}");
    }

    #[test]
    fn unknown_and_unparseable_lines_degrade_to_other() {
        let ev = norm(r#"{"type":"turn.something_new","x":1}"#);
        assert_eq!((ev["events"][0]["type"].as_str(), ev["events"][0]["kind"].as_str()), (Some("other"), Some("turn.something_new")));
        let ev = norm(r#"{"type":"item.completed","item":{"id":"i","type":"reasoning","text":"hm"}}"#);
        assert_eq!(ev["events"][0]["kind"], "reasoning");
        let ev = norm("not json");
        assert_eq!((ev["events"][0]["type"].as_str(), ev["events"][0]["kind"].as_str()), (Some("other"), Some("raw")));
        assert_eq!(Codex.parse_line("{\"type\":\"turn.started\"}").0, "turn.started");
        assert_eq!(Codex.parse_line("nope").0, "raw");
    }

    #[test]
    fn the_session_id_comes_from_the_first_event() {
        assert_eq!(Codex.session_of_line(OK.lines().next().unwrap()).as_deref(), Some("01a1c0c8-7a1e-72a3-8ac2-10437c764cba"));
        assert_eq!(Codex.session_of_line(OK.lines().nth(1).unwrap()), None);
        assert_eq!(Codex.session_of_line("junk"), None);
    }

    #[test]
    fn cached_tokens_are_counted_apart_from_the_input() {
        let u = Codex.step_usage(RESUME.lines().last().unwrap()).unwrap();
        assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens, u.num_turns), (62752 - 20736, 20736, 87, 1));
        assert!(Codex.step_usage(OK.lines().next().unwrap()).is_none());
    }

    // ---- one-shot result -------------------------------------------------------------------

    #[test]
    fn interpret_returns_the_last_message_the_session_id_and_the_usage() {
        let (res, usage) = interpret(TOOL, "", 0, &RunUsage::default());
        let turn = res.unwrap();
        assert_eq!(turn.text, "hello");
        assert_eq!(turn.session_id.as_deref(), Some("01a1c0c8-adaa-7f93-b946-36ec1d2c8260"));
        assert_eq!((turn.input_tokens, turn.output_tokens, turn.cost_usd), (41790, 82, 0.0));
        assert_eq!((usage.input_tokens, usage.output_tokens, usage.num_turns), (41790, 82, 1));
        assert_eq!(usage.session_id.as_deref(), Some("01a1c0c8-adaa-7f93-b946-36ec1d2c8260"));
    }

    #[test]
    fn a_resumed_turn_is_charged_only_what_it_added() {
        let before = last_total_usage(&TOKENS.lines().take(2).collect::<Vec<_>>().join("\n")).unwrap();
        assert_eq!(before.input_tokens + before.cache_read_tokens, 41790);
        let (res, usage) = interpret(RESUME, "", 0, &before);
        assert!(res.is_ok());
        assert_eq!((usage.input_tokens, usage.cache_read_tokens, usage.output_tokens), (20962 - 20736, 20736, 5));
    }

    #[test]
    fn the_last_token_count_of_a_log_is_the_thread_total() {
        let u = last_total_usage(TOKENS).unwrap();
        assert_eq!(u.input_tokens + u.cache_read_tokens, 151200);
        assert!(last_total_usage("{}\n").is_none());
    }

    #[test]
    fn a_failed_turn_is_an_error_carrying_codexs_message() {
        let (res, _) = interpret(ERROR_MODEL, "", 1, &RunUsage::default());
        assert!(res.unwrap_err().to_string().contains("not supported"));
    }

    #[test]
    fn without_a_result_the_cleaned_stderr_is_the_error() {
        let (res, _) = interpret("", NOISE, 1, &RunUsage::default());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("no rollout found") && !msg.contains("rmcp") && !msg.contains("Reading additional"), "{msg}");
        let (res, _) = interpret("", "", 3, &RunUsage::default());
        assert!(res.unwrap_err().to_string().contains("exited 3"));
    }

    #[test]
    fn the_run_engine_fold_gets_text_usage_and_session_from_the_stream() {
        let h = Codex;
        let mut acc = RunAcc::default();
        let mut fin = None;
        for line in TOOL.lines() {
            let events = h.normalize(line);
            acc.push(&h, line, &events);
            if let Some(f) = events.iter().find_map(Event::as_final) {
                fin = Some(acc.complete(f));
            }
        }
        let fin = fin.unwrap();
        assert_eq!((fin.is_error, fin.text.as_deref()), (false, Some("hello")));
        assert_eq!(fin.usage.session_id.as_deref(), Some("01a1c0c8-adaa-7f93-b946-36ec1d2c8260"));
        assert_eq!(fin.usage.input_tokens, 41790);
    }

    // ---- command line ----------------------------------------------------------------------

    #[test]
    fn a_read_only_ephemeral_run() {
        let cwd = Path::new("/w");
        let cmd = command(&request(cwd)).unwrap();
        assert_eq!(cmd.get_current_dir(), Some(cwd));
        assert_eq!(
            argv(&cmd),
            strs(&["exec", "--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\"", "--ephemeral", "--sandbox", "read-only", "-"])
        );
    }

    #[test]
    fn the_access_level_picks_the_sandbox() {
        let cwd = Path::new("/w");
        let sandbox = |access: Access, extra: &[String]| {
            let mut req = request(cwd);
            req.access = access;
            req.extra_args = extra.to_vec();
            command(&req).map(|c| {
                let a = argv(&c);
                a[a.iter().position(|x| x == "--sandbox").unwrap() + 1].clone()
            })
        };
        let (full, sb) = (|| vec![mark_full()], |m: &str| vec![mark_sandbox(m)]);
        assert_eq!(sandbox(Access::Default, &[]).unwrap(), "read-only");
        assert_eq!(sandbox(Access::ReadOnly, &[]).unwrap(), "read-only");
        assert_eq!(sandbox(Access::Edit, &[]).unwrap(), "workspace-write");
        // the profile's sandbox can lower the access-derived mode ...
        assert_eq!(sandbox(Access::Edit, &sb("read-only")).unwrap(), "read-only");
        assert_eq!(sandbox(Access::Full, &[full(), sb("workspace-write")].concat()).unwrap(), "workspace-write");
        // ... and never raise it
        assert_eq!(sandbox(Access::ReadOnly, &sb("workspace-write")).unwrap(), "read-only");
        assert_eq!(sandbox(Access::Edit, &[full(), sb("danger-full-access")].concat()).unwrap(), "workspace-write");
        // full access means no sandbox: only when the profile says so
        let err = sandbox(Access::Full, &[]).unwrap_err().to_string();
        assert!(err.contains("allow_full"), "{err}");
        assert_eq!(sandbox(Access::Full, &full()).unwrap(), "danger-full-access");
    }

    #[test]
    fn add_dir_is_validated_on_save_and_at_run_time() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().canonicalize().unwrap();
        let repo = t.join("repo");
        let wt = repo.join(".git/worktrees/x");
        std::fs::create_dir_all(&wt).unwrap();
        let plain = t.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let cwd = repo.join("sub");
        std::fs::create_dir_all(&cwd).unwrap();
        let home = dirs::home_dir().expect("home");
        let home_parent = home.parent().unwrap_or(Path::new("/")).to_path_buf();
        let link = t.join("homelink");
        symlink(&home, &link).unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();

        let ok = [s(&wt), s(&repo.join(".git")), s(&plain), format!("{}/./plain/../plain", s(&t))];
        for v in &ok {
            for args in [vec!["--add-dir".to_string(), v.clone()], vec![format!("--add-dir={v}")]] {
                assert!(check_extra(&args, ExtraCheck::Save).is_ok(), "save refused {args:?}");
                assert!(check_extra(&args, ExtraCheck::Run(&cwd)).is_ok(), "run refused {args:?}");
                for session in [SessionMode::Ephemeral, SessionMode::New("s"), SessionMode::Resume("01a1c0c8-adaa-7f93-b946-36ec1d2c8260")] {
                    let mut req = request(&cwd);
                    req.session = session;
                    req.extra_args = args.clone();
                    assert!(command(&req).is_ok(), "command refused {args:?}");
                }
            }
        }
        // A path that does not exist yet passes on save only.
        let missing = vec!["--add-dir".to_string(), s(&t.join("later"))];
        assert!(check_extra(&missing, ExtraCheck::Save).is_ok());
        assert!(check_extra(&missing, ExtraCheck::Run(&cwd)).is_err());

        let bad = [
            "/".to_string(),
            s(&home),
            s(&home_parent),
            "/etc".to_string(),
            "/etc/ssh".to_string(),
            s(&home.join(".ssh")),
            s(&home.join(".ssh/keys")),
            "relative/dir".to_string(),
            "~/x".to_string(),
            "~".to_string(),
            "/tmp/../../..".to_string(),
            String::new(),
            "/tmp/a\nb".to_string(),
            s(&link),
            s(&link.join(".ssh")),
        ];
        for v in &bad {
            for args in [vec!["--add-dir".to_string(), v.clone()], vec![format!("--add-dir={v}")]] {
                assert!(check_extra(&args, ExtraCheck::Save).is_err(), "save accepted {args:?}");
                for session in [SessionMode::Ephemeral, SessionMode::Resume("01a1c0c8-adaa-7f93-b946-36ec1d2c8260")] {
                    let mut req = request(&cwd);
                    req.session = session;
                    req.extra_args = args.clone();
                    assert!(command(&req).is_err(), "command accepted {args:?}");
                }
            }
        }
        // The working directory and its ancestors are refused at run time only.
        for v in [&cwd, &repo, &t] {
            let args = vec!["--add-dir".to_string(), s(v)];
            assert!(check_extra(&args, ExtraCheck::Save).is_ok(), "save refused {args:?}");
            assert!(check_extra(&args, ExtraCheck::Run(&cwd)).is_err(), "run accepted {args:?}");
        }
        // A missing value is an error.
        assert!(check_extra(&strs(&["--add-dir"]), ExtraCheck::Save).is_err());
        // The profile-save path.
        let good = serde_json::json!({"extra_args": ["--add-dir", s(&wt)]});
        let evil = serde_json::json!({"extra_args": ["--add-dir=/"]});
        assert!(Codex.validate_settings(&good).is_ok());
        assert!(Codex.validate_settings(&evil).is_err());
    }

    #[test]
    fn extra_args_cannot_change_the_sandbox_or_approvals() {
        let cwd = Path::new("/w");
        let bad: &[&[&str]] = &[
            &["--dangerously-bypass-approvals-and-sandbox"],
            &["--yolo"],
            &["--full-auto"],
            &["-s", "danger-full-access"],
            &["-sdanger-full-access"],
            &["--sandbox", "danger-full-access"],
            &["--sandbox=danger-full-access"],
            &["-a", "never"],
            &["--ask-for-approval=never"],
            &["-p", "yolo"],
            &["--profile", "yolo"],
            &["--profile=yolo"],
            &["-c", "sandbox_mode=\"danger-full-access\""],
            &["--config", "sandbox_mode=\"danger-full-access\""],
            &["--config=approval_policy=\"never\""],
            &["-c", "approval_policy=\"on-request\""],
            &["-c", "sandbox_workspace_write.network_access=true"],
            &["-csandbox_mode=\"danger-full-access\""],
            &["-c=sandbox_mode=\"danger-full-access\""],
            &["-c", "\"sandbox_mode\"=\"danger-full-access\""],
            &["-c", "'sandbox_mode'='danger-full-access'"],
            &["-c", " sandbox_mode = \"danger-full-access\""],
            &["-c", "\"approval_policy\"=\"never\""],
            &["--config=\"approval_policy\"=\"never\""],
            &["-c", "\"sandbox_workspace_write\".network_access=true"],
            &["-c", "sandbox_workspace_write . 'network_access'=true"],
            &["-c", "profiles.x.sandbox_mode=\"danger-full-access\""],
            &["-c", "profiles.\"x\".\"sandbox_mode\"=\"danger-full-access\""],
            &["-c", "profile=\"x\""],
            &["-c", "default_permissions=\":workspace\""],
            &["-c", "permissions.x.filesystem.\":root\"=\"write\""],
            &["-c", "features.guardian_approval=false"],
            &["-c", "shell_environment_policy.inherit=all"],
            &["-c", "model.sandbox_mode=\"danger-full-access\""],
            &["-c", "tools.web_search=true"],
            &["-c", "x={sandbox_mode=\"danger-full-access\"}"],
            &["-c", "\"unterminated=1"],
            &["-c", "no_equals"],
            &["--enable", "guardian_approval"],
            &["--disable=hooks"],
            &["-C", "/"],
            &["--cd=/"],
            &["--approve-for-me"],
            &["--approve-for-me=true"],
            &["--dangerously-bypass-hook-trust"],
            &["--worktree"],
            &["--worktree", "x"],
            &["--worktree=x"],
            &["-o", "/tmp/x"],
            &["-o/tmp/x"],
            &["--output-last-message", "/tmp/x"],
            &["--output-last-message=/tmp/x"],
            &["--ignore-rules"],
            &["--ignore-user-config"],
            &["--cr-allow-full"],
            &["--cr-allow-full=guess"],
            &["--cr-sandbox=danger-full-access"],
        ];
        for session in [SessionMode::Ephemeral, SessionMode::Resume("01a1c0c8-adaa-7f93-b946-36ec1d2c8260")] {
            for args in bad {
                let mut req = request(cwd);
                req.session = session;
                req.extra_args = strs(args);
                let err = command(&req).err().unwrap_or_else(|| panic!("{args:?} was accepted")).to_string();
                assert!(err.contains("extra_args may not contain"), "{args:?}: {err}");
            }
        }
        // harmless flags and other config keys still pass
        for ok in [
            &["--color", "never", "-c", "model_verbosity=\"low\""][..],
            &["-c", "\"model\"=\"o3\"", "--config", " model_reasoning_summary = \"auto\"", "-cmodel_reasoning_effort=\"low\""],
            &["--config=personality=\"friendly\""],
        ] {
            for session in [SessionMode::Ephemeral, SessionMode::New("s"), SessionMode::Resume("01a1c0c8-adaa-7f93-b946-36ec1d2c8260")] {
                let mut req = request(cwd);
                req.session = session;
                req.extra_args = strs(ok);
                assert!(command(&req).is_ok(), "{ok:?} was refused");
            }
        }
    }

    #[test]
    fn the_config_key_parser_strips_quotes_and_whitespace_per_segment() {
        let k = |s: &str| config_key(s);
        let v = |a: &[&str]| Some(a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(k("model=\"o3\""), v(&["model"]));
        assert_eq!(k(" \"sandbox_mode\" = 1"), v(&["sandbox_mode"]));
        assert_eq!(k("'sandbox_mode'=1"), v(&["sandbox_mode"]));
        assert_eq!(k("a . \"b.c\" . 'd' = 1"), v(&["a", "b.c", "d"]));
        assert_eq!(k("\"a\\\"b\"=1"), v(&["a\"b"]));
        // only the first `=` ends the key; inline tables and arrays in the value are not read
        assert_eq!(k("x={sandbox_mode=\"y\"}"), v(&["x"]));
        assert_eq!(k("x=[\"a=b\"]"), v(&["x"]));
        assert_eq!(k("no_equals"), None);
        assert_eq!(k("\"open=1"), None);
        assert!(!forbidden_config("model = \"o3\""));
        assert!(forbidden_config("model.x = 1"));
        assert!(forbidden_config("\"sandbox_mode\"=1"));
    }

    #[test]
    fn allow_full_comes_only_from_the_settings_key() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.access = Access::Full;
        // typed by hand in extra_args: refused, not honoured
        req.extra_args = Codex.settings_args(&serde_json::json!({ "extra_args": ["--cr-allow-full"] }));
        assert!(command(&req).is_err());
        req.extra_args = Codex.settings_args(&serde_json::json!({ "allow_full": true }));
        assert!(command(&req).is_ok());
    }

    #[test]
    fn a_new_edit_session_with_a_model_effort_and_extra_dirs() {
        let cwd = Path::new("/w");
        let (a, b) = (Path::new("/skills"), Path::new("/scratch dir"));
        let mut req = request(cwd);
        req.access = Access::Edit;
        req.model = "gpt-5.6-sol";
        req.effort = Some("low");
        req.extra_dirs = vec![a, b];
        // Codex picks the id itself: the one named is not passed.
        req.session = SessionMode::New("app-uuid");
        req.extra_args = vec![mark_full(), "--color".into(), "never".into()];
        let args = argv(&command(&req).unwrap());
        assert_eq!(
            args,
            strs(&[
                "exec", "--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\"", "--model", "gpt-5.6-sol", "-c",
                "model_reasoning_effort=\"low\"", "--sandbox", "workspace-write", "--add-dir", "/skills", "--add-dir",
                "/scratch dir", "--color", "never", "-"
            ])
        );
        assert!(!args.contains(&"app-uuid".to_string()));
    }

    #[test]
    fn extra_dirs_are_not_granted_in_a_read_only_sandbox() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.extra_dirs = vec![Path::new("/skills")];
        assert!(!argv(&command(&req).unwrap()).iter().any(|a| a.contains("/skills")));
    }

    #[test]
    fn resume_uses_config_overrides_because_exec_resume_has_no_sandbox_flags() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.access = Access::Edit;
        req.session = SessionMode::Resume("01a1c0c8-adaa-7f93-b946-36ec1d2c8260");
        req.extra_dirs = vec![Path::new("/skills")];
        let args = argv(&command(&req).unwrap());
        assert_eq!(
            args,
            strs(&[
                "exec", "resume", "--json", "--skip-git-repo-check", "-c", "approval_policy=\"never\"", "-c",
                "sandbox_mode=\"workspace-write\"", "-c", "sandbox_workspace_write.writable_roots=[\"/skills\"]",
                "01a1c0c8-adaa-7f93-b946-36ec1d2c8260", "-"
            ])
        );
        assert!(!args.iter().any(|a| a == "--sandbox" || a == "--ephemeral" || a == "--add-dir"));
    }

    #[test]
    fn forking_is_refused() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.session = SessionMode::Fork("x");
        assert!(command(&req).is_err());
    }

    #[test]
    fn the_system_prompt_goes_in_front_of_the_prompt_on_stdin() {
        let cwd = Path::new("/w");
        let mut req = request(cwd);
        req.system_prompt = Some(crate::ai::SystemPrompt::Text("be brief"));
        assert_eq!(message(&req).unwrap(), "be brief\n\n---\n\nhi");
    }

    // ---- caps, settings, interactive -------------------------------------------------------

    #[test]
    fn caps_say_what_codex_cannot_do() {
        let caps = Codex.caps();
        assert!(caps.sessions && caps.resume && caps.streaming && caps.interactive_tui);
        assert!(!caps.fork && !caps.tool_allowlist && !caps.reports_cost && !caps.pins_session_id);
        // the run engine therefore puts the system prompt in front of a streamed run's prompt
        assert!(!caps.replaces_system_prompt && !crate::ai::opencode::OpenCode.caps().replaces_system_prompt);
        let needs = |access, tools, tool_allowlist| Needs { access, tools, tool_allowlist };
        check_caps(&Codex, "r", &needs(Access::ReadOnly, true, false)).unwrap();
        check_caps(&Codex, "r", &needs(Access::Full, true, false)).unwrap();
        let err = check_caps(&Codex, "“Babysit”", &needs(Access::Edit, true, true)).unwrap_err().to_string();
        assert!(err.contains("a tool allowlist") && err.contains("Codex"), "{err}");
    }

    #[test]
    fn settings_become_markers_and_extra_args() {
        let h = Codex;
        assert!(h.settings_args(&serde_json::json!({})).is_empty());
        assert_eq!(
            h.settings_args(&serde_json::json!({ "sandbox": " workspace-write ", "allow_full": true, "extra_args": ["--color", "never", 3] })),
            vec![mark_sandbox("workspace-write"), mark_full(), "--color".into(), "never".into()]
        );
        // unknown sandboxes and a false allow_full are ignored
        assert!(h.settings_args(&serde_json::json!({ "sandbox": "yolo", "allow_full": false })).is_empty());
    }

    #[test]
    fn models_are_the_listed_ones_best_first() {
        assert_eq!(
            parse_models(MODELS),
            ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-6.1-terra", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"]
        );
        assert!(parse_models("garbage").is_empty());
    }

    #[test]
    fn interactive_argv_resumes_by_id_and_cannot_pin_or_fork() {
        let opts = |session| Interactive { session, system_prompt_file: None, system_prompt_mode: SystemPromptMode::Append };
        assert_eq!(Codex.interactive_argv(&opts(SessionMode::Ephemeral)), Some(strs(&["codex"])));
        assert_eq!(Codex.interactive_argv(&opts(SessionMode::New("app-id"))), Some(strs(&["codex"])));
        assert_eq!(Codex.interactive_argv(&opts(SessionMode::Resume("abc"))), Some(strs(&["codex", "resume", "abc"])));
        assert_eq!(Codex.interactive_argv(&opts(SessionMode::Fork("abc"))), None);
    }

    #[test]
    fn the_context_file_is_the_opening_prompt_of_a_new_session_only() {
        let dir = std::env::temp_dir().join(format!("cr-codex-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ctx.md");
        std::fs::write(&file, "- issue 12\nfix it").unwrap();
        let opts = |session| Interactive { session, system_prompt_file: Some(&file), system_prompt_mode: SystemPromptMode::Append };
        let new = opts(SessionMode::New("x"));
        assert_eq!(Codex.interactive_argv(&new), Some(strs(&["codex", "--", "- issue 12\nfix it"])));
        assert!(Codex.context_file_note(&new).unwrap().contains("opening prompt"));
        let resume = opts(SessionMode::Resume("abc"));
        assert_eq!(Codex.interactive_argv(&resume), Some(strs(&["codex", "resume", "abc"])));
        assert!(Codex.context_file_note(&resume).unwrap().contains("skipped"));
        assert_eq!(Codex.context_file_note(&Interactive { session: SessionMode::New("x"), system_prompt_file: None, system_prompt_mode: SystemPromptMode::Append }), None);
        std::fs::remove_dir_all(dir).ok();
    }

    // ---- session logs and discovery --------------------------------------------------------

    #[test]
    fn a_log_header_gives_id_cwd_and_who_wrote_it() {
        let tui = parse_rollout_meta(META_TUI).unwrap();
        assert_eq!((tui.id.as_str(), tui.cwd.as_str(), tui.originator.as_str()), ("01a1c0cb-03cb-76a1-8d70-dfea848b0904", "/work/repo", "codex-tui"));
        assert!(tui.interactive());
        let exec = parse_rollout_meta(META_EXEC).unwrap();
        assert_eq!(exec.originator, "codex_exec");
        assert!(!exec.interactive());
        assert!(parse_rollout_meta("{\"type\":\"event_msg\"}").is_none() && parse_rollout_meta("junk").is_none());
    }

    #[test]
    fn titles_come_from_the_session_index_last_entry_wins() {
        let t = parse_titles("{\"id\":\"a\",\"thread_name\":\"First\"}\nnot json\n{\"id\":\"b\",\"thread_name\":\"  \"}\n{\"id\":\"a\",\"thread_name\":\"Renamed\"}\n");
        assert_eq!(t.get("a").map(String::as_str), Some("Renamed"));
        assert!(!t.contains_key("b"));
    }

    fn a(s: &[&str]) -> Vec<String> {
        strs(s)
    }

    #[test]
    fn only_interactive_codex_commands_are_tuis() {
        let some = |s: &[&str]| parse_tui_args(&a(s));
        assert_eq!(some(&["codex"]), Some((None, None)));
        assert_eq!(some(&["/home/u/.codex/bin/codex", "-m", "gpt-5.6-sol", "-C", "/x"]), Some((None, Some("/x".into()))));
        assert_eq!(some(&["codex", "--cd=/y"]), Some((None, Some("/y".into()))));
        // an opening prompt, even one that looks like a subcommand, is not read as one
        assert_eq!(some(&["codex", "fix the exec bug"]), Some((None, None)));
        assert_eq!(some(&["codex", "-c", "exec=1", "login now"]), Some((None, None)));
        let id = "01a1c0cb-03cb-76a1-8d70-dfea848b0904";
        assert_eq!(some(&["codex", "resume", id]), Some((Some(id.into()), None)));
        assert_eq!(some(&["codex", "-s", "read-only", "resume", id, "go on"]), Some((Some(id.into()), None)));
        assert_eq!(some(&["codex", "resume", "--last"]), Some((None, None)));
        assert_eq!(some(&["codex", "resume", "my-thread-name"]), Some((None, None)));
        assert_eq!(some(&["codex", "fork", id]), Some((None, None)));
        for sub in ["exec", "app-server", "mcp-server", "login", "review", "sandbox", "e"] {
            assert_eq!(some(&["codex", sub]), None, "{sub}");
        }
        assert_eq!(some(&["codex", "-m", "exec", "exec"]), None);
        assert_eq!(some(&["node", "codex"]), None);
        assert_eq!(some(&["codex-code-mode-host"]), None);
    }

    #[test]
    fn images_and_a_trailing_cd_do_not_hide_the_session_or_directory() {
        let some = |s: &[&str]| parse_tui_args(&a(s));
        let id = "01a1c0cb-03cb-76a1-8d70-dfea848b0904";
        assert_eq!(some(&["codex", "resume", "-i", "a.png", "b.png", id]), Some((Some(id.into()), None)));
        assert_eq!(some(&["codex", "resume", "--image", "a.png", id, "-C", "/x"]), Some((Some(id.into()), Some("/x".into()))));
        assert_eq!(some(&["codex", "-i", "a.png", "-C", "/x"]), Some((None, Some("/x".into()))));
        assert_eq!(some(&["codex", "do it", "-C", "/x"]), Some((None, Some("/x".into()))));
        assert_eq!(some(&["codex", "resume", id, "go on", "--cd=/y"]), Some((Some(id.into()), Some("/y".into()))));
        assert_eq!(some(&["codex", "do it", "exec"]), Some((None, None)));
    }

    #[test]
    fn the_log_tail_is_bounded_and_tolerates_bad_utf8() {
        let path = std::env::temp_dir().join(format!("cr-codex-tail-{}.jsonl", std::process::id()));
        let mut data = vec![b'a'; (TAIL_BYTES as usize) * 2];
        data.extend_from_slice(b"\n\xff\xfe{\"last\":1}\n");
        std::fs::write(&path, &data).unwrap();
        let t = read_tail(&path).unwrap();
        assert!(t.len() as u64 <= TAIL_BYTES * 3 && t.ends_with("{\"last\":1}\n"));
        assert!(read_tail(&path.with_extension("missing")).is_err());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn ps_lines_become_tuis() {
        let ps = "\
  101 pts/3     05:00 codex
  102 ?         01:00 /usr/lib/chatgpt/resources/codex app-server --analytics-default-enabled
  103 pts/4     00:10 /home/u/.local/bin/codex resume 01a1c0cb-03cb-76a1-8d70-dfea848b0904
  104 pts/5     00:10 codex exec --json -
  105 pts/6     00:10 vim codex
  106 ?         00:10 codex -C rel
";
        let cwd = |pid| match pid {
            101 => Some("/work/a".to_string()),
            103 => Some("/work/b".to_string()),
            106 => Some("/work/c".to_string()),
            _ => None,
        };
        let tuis = parse_tuis(ps, |_| None, cwd, |d| d == "/work/c/rel", 1_000_000);
        assert_eq!(tuis.iter().map(|t| t.pid).collect::<Vec<_>>(), [101, 103, 106]);
        assert_eq!((tuis[0].cwd.as_str(), tuis[0].tty.as_deref(), tuis[0].started_ms, tuis[0].session.clone()), ("/work/a", Some("pts/3"), 1_000_000 - 300_000, None));
        assert_eq!(tuis[1].session.as_deref(), Some("01a1c0cb-03cb-76a1-8d70-dfea848b0904"));
        assert_eq!((tuis[2].cwd.as_str(), tuis[2].tty.as_deref()), ("/work/c/rel", None));
    }

    fn tui(pid: i64, cwd: &str, session: Option<&str>, started_ms: i64) -> Tui {
        Tui { pid, tty: Some("pts/1".into()), cwd: cwd.into(), session: session.map(String::from), started_ms }
    }

    fn rollout(id: &str, cwd: &str, originator: &str, updated: i64) -> Rollout {
        Rollout { id: id.into(), cwd: cwd.into(), originator: originator.into(), source: "vscode".into(), updated }
    }

    #[test]
    fn a_resumed_tui_is_reported_on_its_id_even_without_a_log() {
        let titles = parse_titles("{\"id\":\"s1\",\"thread_name\":\"Fix the bug\"}");
        let found = rollout("s1", "/work/a", "codex-tui", 5_000);
        let out = match_tuis(&[tui(1, "/work/a", Some("s1"), 1_000), tui(2, "/work/z", Some("s9"), 2_000)], |_| Vec::new(), |id| (id == "s1").then(|| found.clone()), &titles);
        assert_eq!(out.len(), 2);
        let s1 = out.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!((s1.harness.as_str(), s1.name.as_deref(), s1.updated_at, s1.pid, s1.live), ("codex", Some("Fix the bug"), 5_000, Some(1), true));
        let s9 = out.iter().find(|s| s.session_id == "s9").unwrap();
        assert_eq!((s9.cwd.as_str(), s9.name.clone(), s9.updated_at), ("/work/z", None, 2_000));
    }

    #[test]
    fn an_unpinned_tui_takes_the_newest_interactive_log_written_since_it_started() {
        let logs = vec![
            rollout("old", "/work/a", "codex-tui", 500),
            rollout("exec", "/work/a", "codex_exec", 9_000),
            rollout("desktop", "/work/a", "Codex Desktop", 9_500),
            rollout("elsewhere", "/work/b", "codex-tui", 9_000),
            rollout("mine1", "/work/a", "codex-tui", 3_000),
            rollout("mine2", "/work/a", "codex-tui", 4_000),
        ];
        let out = match_tuis(&[tui(1, "/work/a", None, 2_000)], |_| logs.clone(), |_| None, &HashMap::new());
        assert_eq!(out.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(), ["mine2"]);
        assert_eq!(out[0].updated_at, 4_000);
    }

    #[test]
    fn an_unpinned_tui_without_a_log_yet_or_sharing_its_directory_is_left_out() {
        let logs = vec![rollout("x", "/work/a", "codex-tui", 3_000)];
        // started after the only log was written: no session yet
        assert!(match_tuis(&[tui(1, "/work/a", None, 10_000)], |_| logs.clone(), |_| None, &HashMap::new()).is_empty());
        // two unpinned TUIs in one directory: can't tell which is whose
        assert!(match_tuis(&[tui(1, "/work/a", None, 1_000), tui(2, "/work/a", None, 1_500)], |_| logs.clone(), |_| None, &HashMap::new()).is_empty());
    }

    #[test]
    fn a_log_taken_by_a_pinned_tui_is_not_given_to_an_unpinned_one() {
        let logs = vec![rollout("pinned", "/work/a", "codex-tui", 4_000), rollout("free", "/work/a", "codex-tui", 3_000)];
        let by_id = |id: &str| logs.iter().find(|r| r.id == id).cloned();
        let out = match_tuis(&[tui(1, "/work/a", Some("pinned"), 1_000), tui(2, "/work/a", None, 2_000)], |_| logs.clone(), by_id, &HashMap::new());
        let mut ids: Vec<_> = out.iter().map(|s| (s.session_id.as_str(), s.pid)).collect();
        ids.sort();
        assert_eq!(ids, [("free", Some(2)), ("pinned", Some(1))]);
    }

    #[test]
    fn logs_are_found_by_id_and_by_recency_on_disk() {
        let root = std::env::temp_dir().join(format!("cr-codex-logs-{}", std::process::id()));
        let day = root.join("2026/11/09");
        std::fs::create_dir_all(&day).unwrap();
        let id = "01a1c0cb-03cb-76a1-8d70-dfea848b0904";
        std::fs::write(day.join(format!("rollout-2026-11-09T15-51-32-{id}.jsonl")), format!("{META_TUI}{{\"type\":\"event_msg\"}}\n")).unwrap();
        std::fs::write(day.join("rollout-2026-11-09T15-50-14-xx.jsonl"), "garbage\n").unwrap();
        std::fs::write(day.join("notes.txt"), "x").unwrap();
        let path = find_rollout_path(&root, id).unwrap();
        let r = read_rollout(&path).unwrap();
        assert_eq!((r.id.as_str(), r.cwd.as_str()), (id, "/work/repo"));
        assert!(r.updated > 0);
        assert!(find_rollout_path(&root, "nope").is_none());
        // the cache remembers where a session's log is
        assert_eq!(cached_rollout_path(&root, id), Some(path.clone()));
        assert_eq!(rollout_cache().lock().unwrap().get(id), Some(&path));
        let recent = recent_rollouts(&root, 0);
        assert_eq!(recent.len(), 1);
        assert!(recent_rollouts(&root, i64::MAX).is_empty());
        std::fs::remove_dir_all(root).ok();
    }
}
