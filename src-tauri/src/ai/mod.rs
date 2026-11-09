//! The AI harness seam: one object-safe, synchronous trait under every AI call of the app (chat,
//! theme suggestions, session summaries, routine runs, interactive sessions).
//!
//! Same shape as `provider.rs`: a trait, a registry ([`harness_by_id`]) and profile resolvers ([`profiles`]), a compile-time
//! object-safety assertion and a recipe for adding an implementation (bottom of this file).
//! Requests are neutral (tool policy, session mode, access level, system prompt mode); each
//! harness translates them to its own CLI. Capabilities are data ([`Caps`]) so callers can refuse
//! an unsupported combination up front instead of failing mid-run.
//!
//! The app stores no token: every harness delegates auth to its own CLI.

use std::ffi::OsString;
use std::fs::File;
use std::path::Path;
use std::process::Child;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::run_usage::{RunTag, RunUsage};
use crate::error::{AppError, AppResult};

pub mod claude;
pub mod codex;
pub mod models;
pub mod opencode;
pub mod profiles;
pub mod session;

pub use claude::ClaudeCode;
pub use codex::Codex;
pub use opencode::OpenCode;
pub use session::AgentSession;

/// What a harness can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub sessions: bool,
    pub resume: bool,
    pub fork: bool,
    pub tools: bool,
    pub tool_allowlist: bool,
    pub streaming: bool,
    pub interactive_tui: bool,
    pub reports_cost: bool,
    pub replaces_system_prompt: bool,
    /// The access levels the harness understands (never contains [`Access::Native`]).
    pub access_levels: Vec<Access>,
    /// Whether a new session can be started under an id chosen by the caller.
    pub pins_session_id: bool,
}

/// Which tools a run may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicy {
    /// No tools at all.
    None,
    ReadOnly,
    Edit,
    Full,
    /// Exactly these tool rules, in the harness' own syntax. Empty means "the harness' default".
    Allowlist(Vec<String>),
}

/// The permission level of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// The harness' own default (explicit, so the user's global settings never leak in).
    Default,
    ReadOnly,
    Edit,
    Full,
    /// A harness-native value passed through verbatim (Claude's `--permission-mode`). Transitional:
    /// routines still store native values until profiles replace them.
    Native(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPromptMode {
    /// Added to the harness' own system prompt.
    Append,
    /// Replaces it.
    Replace,
}

/// Where the system prompt comes from.
#[derive(Debug, Clone, Copy)]
pub enum SystemPrompt<'a> {
    /// Written to a temp file for the duration of the run (one-shot only).
    Text(&'a str),
    /// A file the caller owns and removes.
    File(&'a Path),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode<'a> {
    /// Nothing is persisted.
    Ephemeral,
    /// Start a session under this id.
    New(&'a str),
    Resume(&'a str),
    /// Continue a copy of this session without touching the original.
    Fork(&'a str),
}

/// One blocking, non-interactive run.
pub struct OneShot<'a> {
    /// Empty: the harness picks its own default.
    pub model: &'a str,
    pub system_prompt: Option<SystemPrompt<'a>>,
    pub system_prompt_mode: SystemPromptMode,
    /// Names the temp system-prompt file; must be unique per concurrent run.
    pub file_tag: &'a str,
    pub prompt: &'a str,
    pub cwd: &'a Path,
    /// Directories beyond `cwd` the tools may access.
    pub extra_dirs: Vec<&'a Path>,
    pub tools: ToolPolicy,
    pub access: Access,
    pub session: SessionMode<'a>,
    pub max_turns: Option<u32>,
    pub effort: Option<&'a str>,
    pub env: Vec<(String, OsString)>,
    /// Harness-native flags, appended as they are.
    pub extra_args: Vec<String>,
    /// What this run is for; recorded in the usage log.
    pub usage_tag: RunTag,
}

/// A detached streaming run for the routine engine. `spawn` leaves the child's stdin piped (the
/// caller feeds `req.prompt` once it has recorded the pid) and starts it as a process group leader.
pub struct StreamRun<'a> {
    pub req: OneShot<'a>,
    pub stdout: File,
    pub stderr: File,
    /// Carried for the run engine; the caller enforces it.
    pub timeout: Duration,
}

/// A harness-neutral stream event. One raw line can yield several (an assistant message with text
/// and tool calls). Stored as the `norm` column next to the raw line, see [`Event::to_json`].
#[derive(Debug, Clone)]
pub enum Event {
    Text(String),
    /// `id` pairs the call with its [`Event::ToolResult`]; empty when the harness has none.
    ToolCall { id: String, name: String, input: Value },
    ToolResult { id: String, ok: bool, text: String },
    Final { ok: bool, text: String, usage: RunUsage },
    /// Anything else (system lines, unknown types, unparseable lines as kind `raw`).
    Other { kind: String, raw: Value },
}

/// Version of the normalized shape (`{"v":1,"events":[..]}`); bump on an incompatible change.
pub const NORM_VERSION: u32 = 1;

/// `s` cut to at most `max` bytes on a char boundary.
fn clip(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

impl Event {
    /// The stored form: `{"type": ..}` with every large payload cut to `max_bytes` (and then
    /// `"truncated": true`). The in-memory event is never truncated.
    pub fn to_json(&self, max_bytes: usize) -> Value {
        use serde_json::json;
        let text = |t: &str| {
            let (t, cut) = clip(t, max_bytes);
            (Value::String(t.to_string()), cut)
        };
        let mut v = match self {
            Event::Text(t) => {
                let (t, cut) = text(t);
                json!({ "type": "text", "text": t, "truncated": cut })
            }
            Event::ToolCall { id, name, input } => {
                let raw = input.to_string();
                let (input, cut) = if raw.len() > max_bytes { (text(&raw).0, true) } else { (input.clone(), false) };
                json!({ "type": "tool_call", "id": id, "name": name, "input": input, "truncated": cut })
            }
            Event::ToolResult { id, ok, text: t } => {
                let (t, cut) = text(t);
                json!({ "type": "tool_result", "id": id, "ok": ok, "text": t, "truncated": cut })
            }
            Event::Final { ok, text: t, usage } => {
                let (t, cut) = text(t);
                json!({
                    "type": "final", "ok": ok, "text": t, "truncated": cut,
                    "usage": {
                        "num_turns": usage.num_turns,
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens,
                        "cache_read_tokens": usage.cache_read_tokens,
                        "cache_creation_tokens": usage.cache_creation_tokens,
                        "cost_usd": usage.cost_usd,
                    }
                })
            }
            Event::Other { kind, raw } => {
                let s = raw.to_string();
                let (raw, cut) = if s.len() > max_bytes { (text(&s).0, true) } else { (raw.clone(), false) };
                json!({ "type": "other", "kind": kind, "raw": raw, "truncated": cut })
            }
        };
        // Keep the shape minimal: the flag only appears when something was cut.
        if v["truncated"] == Value::Bool(false) {
            v.as_object_mut().map(|o| o.remove("truncated"));
        }
        v
    }

    /// The final status, text and usage, when this is the final event.
    pub fn as_final(&self) -> Option<FinalResult> {
        match self {
            Event::Final { ok, text, usage } => {
                Some(FinalResult { is_error: !ok, text: Some(text.clone()), usage: usage.clone() })
            }
            _ => None,
        }
    }
}

/// The stored `norm` value of one raw line: `{"v":1,"events":[..]}`.
pub fn norm_json(events: &[Event], max_bytes: usize) -> String {
    serde_json::json!({ "v": NORM_VERSION, "events": events.iter().map(|e| e.to_json(max_bytes)).collect::<Vec<_>>() })
        .to_string()
}

/// The final status, text and usage a harness reports for a streamed run.
#[derive(Debug, Clone, Default)]
pub struct FinalResult {
    pub is_error: bool,
    pub text: Option<String>,
    pub usage: RunUsage,
}

/// The outcome of a one-shot run.
#[derive(Debug)]
pub struct TurnResult {
    /// The session id the harness assigned itself, when it can't be pinned (`None` for the id the
    /// request named); the caller stores it for the next turn.
    pub session_id: Option<String>,
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Options of an interactive (TUI) session.
pub struct Interactive<'a> {
    pub session: SessionMode<'a>,
    pub system_prompt_file: Option<&'a Path>,
    pub system_prompt_mode: SystemPromptMode,
}

/// A way of running an AI agent or model. Synchronous and object-safe; see [`harness_by_id`].
pub trait Harness: Send + Sync {
    /// Stable id (`"claude"`).
    fn id(&self) -> &'static str;
    /// Human-readable name.
    fn name(&self) -> &'static str;
    fn caps(&self) -> Caps;
    /// Whether the harness' binary was found.
    fn available(&self) -> bool;
    /// The installed version, when it can be told.
    fn version(&self) -> Option<String> {
        None
    }

    /// Run to completion, parse the result and record its usage (failed runs included).
    fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult>;
    /// Start a detached streaming run writing to the given files.
    ///
    /// Spec deviation: returns the [`Child`] rather than a bare pid, because the run engine still
    /// writes the prompt to its stdin, polls and reaps it.
    fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child>;
    /// `(kind, parsed line)` of one stream line (the `kind` column); unparseable lines are kept as kind `raw`.
    fn parse_line(&self, line: &str) -> (String, Option<Value>);
    /// The neutral events of one raw stream line (see [`Event`]); never fails: what is not
    /// understood becomes [`Event::Other`]. The engine takes a run's status, text and usage from
    /// the [`Event::Final`] among them.
    fn normalize(&self, line: &str) -> Vec<Event>;
    /// Whether `pid` is still this harness' streaming process (of that session, when known):
    /// guards re-adoption after a restart against pid reuse.
    fn is_own_process(&self, pid: i32, session_id: Option<&str>) -> bool;
    /// The argv that opens an interactive session, or `None` when the harness can't.
    fn interactive_argv(&self, opts: &Interactive<'_>) -> Option<Vec<String>>;
    /// What happens to `opts.system_prompt_file` when it can't go in as a system prompt: a note for
    /// the launch result (it was passed another way, or skipped). `None` for Claude, which appends it.
    fn context_file_note(&self, _opts: &Interactive<'_>) -> Option<String> {
        None
    }
    /// Live sessions of this harness, for the Sessions view. Empty when the harness has no
    /// discovery (or is not installed).
    fn live_sessions(&self) -> Vec<AgentSession> {
        Vec::new()
    }
    /// Model ids to offer in the profile editor: what the CLI lists, or a short static suggestion
    /// list. May run a subprocess; callers cache ([`models`]).
    fn models(&self) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }
    /// Refuses profile `settings` the harness would refuse at run time (called when a profile is saved).
    fn validate_settings(&self, _settings: &Value) -> AppResult<()> {
        Ok(())
    }
    /// The command-line flags a profile's `settings` stand for (an opencode agent, extra args),
    /// appended to a request's `extra_args`. Settings the harness doesn't know are ignored.
    fn settings_args(&self, _settings: &Value) -> Vec<String> {
        Vec::new()
    }
    /// Whether a run's final text and usage are the sum over its stream rather than one `Final`
    /// line (opencode reports per step): the run engine then folds them with [`RunAcc`].
    fn accumulates(&self) -> bool {
        false
    }
    /// The usage one stream line reports (one step), for harnesses that [accumulate](Self::accumulates).
    fn step_usage(&self, _line: &str) -> Option<RunUsage> {
        None
    }
    /// The session id one stream line announces, for a harness that assigns its own ids (Codex): the
    /// run engine folds it into the final usage's `session_id` ([`RunAcc`]).
    fn session_of_line(&self, _line: &str) -> Option<String> {
        None
    }
}

/// Folds the lines of a run into the final text and total usage of a harness that
/// [accumulates](Harness::accumulates): the last text any step produced, the usage of all steps.
#[derive(Debug, Default)]
pub struct RunAcc {
    text: String,
    usage: RunUsage,
    steps: i64,
    any: bool,
    session_id: Option<String>,
}

impl RunAcc {
    /// Take in one raw line and its normalized events.
    pub fn push(&mut self, harness: &dyn Harness, line: &str, events: &[Event]) {
        if !harness.accumulates() {
            return;
        }
        if self.session_id.is_none() {
            self.session_id = harness.session_of_line(line);
        }
        for e in events {
            if let Event::Text(t) = e {
                self.text = t.clone();
            }
        }
        if let Some(u) = harness.step_usage(line) {
            // Sums what each line reports. Codex's `turn.completed` carries the thread's cumulative
            // usage, so this would double count if a run ever printed more than one of them
            // (`codex exec` prints one; `before` is subtracted by the caller for resumed threads).
            self.any = true;
            self.steps += 1;
            self.usage.input_tokens += u.input_tokens;
            self.usage.output_tokens += u.output_tokens;
            self.usage.cache_read_tokens += u.cache_read_tokens;
            self.usage.cache_creation_tokens += u.cache_creation_tokens;
            self.usage.cost_usd += u.cost_usd;
        }
    }

    /// `fin` completed with what the run produced: a final without text gets the last text, and
    /// the usage is the total over all steps.
    pub fn complete(&self, mut fin: FinalResult) -> FinalResult {
        if !self.any && self.text.is_empty() && self.session_id.is_none() {
            return fin;
        }
        if fin.text.as_deref().is_none_or(str::is_empty) {
            fin.text = Some(self.text.clone());
        }
        let session_id = fin.usage.session_id.take().or_else(|| self.session_id.clone());
        fin.usage = RunUsage { num_turns: self.steps, session_id, ..self.usage.clone() };
        fin
    }
}

// Compile-time proof that `Harness` is object-safe: this fails to build if a method breaks it.
const _: Option<&dyn Harness> = None;

static CLAUDE: ClaudeCode = ClaudeCode;
static OPENCODE: OpenCode = OpenCode;
static CODEX: Codex = Codex;

/// Every harness the app knows, in display order.
pub fn harnesses() -> [&'static dyn Harness; 3] {
    [&CLAUDE, &OPENCODE, &CODEX]
}

/// Live sessions of every available harness except `skip` (Claude has a richer dedicated source),
/// in the order of [`harnesses`].
pub fn live_sessions_except(skip: &str) -> Vec<AgentSession> {
    live_sessions_of(&harnesses(), skip)
}

fn live_sessions_of(harnesses: &[&dyn Harness], skip: &str) -> Vec<AgentSession> {
    harnesses.iter().filter(|h| h.id() != skip && h.available()).flat_map(|h| h.live_sessions()).collect()
}

/// The harness with this id (`"claude"`).
pub fn harness_by_id(id: &str) -> Option<&'static dyn Harness> {
    harnesses().into_iter().find(|h| h.id() == id)
}

/// Claude Code, for what is not profile-driven: the run engine's process control and the
/// availability check. A feature that runs something resolves its harness from a profile
/// ([`profiles`]) instead.
pub fn harness_for() -> &'static dyn Harness {
    &CLAUDE
}

/// The harness an interactive session launches on: `requested` (the picker's choice) when given,
/// else the one the `sessions` feature profile selects.
pub fn sessions_harness(conn: &rusqlite::Connection, requested: Option<&str>) -> AppResult<&'static dyn Harness> {
    match requested.map(str::trim).filter(|h| !h.is_empty()) {
        Some(id) => harness_by_id(id).ok_or_else(|| AppError::Other(format!("unknown harness '{id}'"))),
        None => Ok(profiles::resolve_feature(conn, profiles::Feature::Sessions).harness),
    }
}

/// The argv that opens an interactive session on `harness`.
pub fn argv_of(harness: &dyn Harness, opts: &Interactive<'_>) -> AppResult<Vec<String>> {
    harness
        .interactive_argv(opts)
        .ok_or_else(|| AppError::Other(format!("{} cannot open an interactive session like this", harness.name())))
}

/// The argv of an interactive session of the harness the `sessions` feature profile selects.
pub fn interactive_argv(db: &crate::db::Db, opts: &Interactive<'_>) -> AppResult<Vec<String>> {
    interactive_argv_with(db, None, opts)
}

/// Like [`interactive_argv`], on the harness `requested` (a launch-time pick) when given.
pub fn interactive_argv_with(db: &crate::db::Db, requested: Option<&str>, opts: &Interactive<'_>) -> AppResult<Vec<String>> {
    let harness = sessions_harness(&db.0.lock().unwrap(), requested)?;
    argv_of(harness, opts)
}

/// What a run needs from its harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needs {
    pub access: Access,
    /// The run uses tools (edits a worktree, runs commands).
    pub tools: bool,
    /// The run restricts tools to an allowlist.
    pub tool_allowlist: bool,
}

/// Refuse a launch the harness cannot honour, naming the missing capability. `what` names the run
/// for the message.
pub fn check_caps(harness: &dyn Harness, what: &str, needs: &Needs) -> AppResult<()> {
    let caps = harness.caps();
    let missing = if needs.access != Access::Default
        && !matches!(needs.access, Access::Native(_))
        && !caps.access_levels.contains(&needs.access)
    {
        Some(match needs.access {
            Access::ReadOnly => "read-only access, which it cannot enforce (pick Edit or Full knowingly, or use another profile)".to_string(),
            _ => format!("{} access", access_label(&needs.access)),
        })
    } else if needs.tools && !caps.tools {
        Some("tools".to_string())
    } else if needs.tool_allowlist && !caps.tool_allowlist {
        Some("a tool allowlist".to_string())
    } else {
        None
    };
    match missing {
        Some(missing) => Err(AppError::Other(format!("{} can't run {what}: it does not support {missing}", harness.name()))),
        None => Ok(()),
    }
}

fn access_label(access: &Access) -> &'static str {
    match access {
        Access::Default => "default",
        Access::ReadOnly => "read-only",
        Access::Edit => "edit",
        Access::Full => "full",
        Access::Native(_) => "native",
    }
}

// Adding a harness
//
// 1. Add `ai/<name>.rs` with a unit struct implementing `Harness`. Register its binary in
//    `tools.rs` (login-shell PATH recovery) and show it in `check_environment`.
// 2. Translate the neutral request types to the CLI: `ToolPolicy`/`Access` to its permission
//    flags, `SessionMode` to its session flags, `SystemPromptMode` to its prompt flags. Anything it
//    cannot express must be reflected in `caps()` rather than silently dropped.
// 3. Write argv/env snapshot tests and stream-line fixtures before wiring it (see `claude.rs`).
// 4. Add it to `harnesses()` so profiles can select it. No caller builds a command line itself.
// 5. Auth stays in the CLI; never store a token in the app.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_for_is_claude_code() {
        let h = harness_for();
        assert_eq!(h.id(), "claude");
        assert!(h.caps().pins_session_id);
    }

    fn fake_session(harness: &str, id: &str) -> AgentSession {
        AgentSession {
            harness: harness.into(),
            session_id: id.into(),
            pid: None,
            live: true,
            status: None,
            kind: None,
            attached: false,
            job_id: None,
            waiting_for: None,
            agent_view: None,
            name: None,
            cwd: "/x".into(),
            git_branch: None,
            repo: None,
            tmux: None,
            tty: None,
            updated_at: 0,
            focus: crate::claude_sessions::FocusState::NotLive,
            summaries: Vec::new(),
            original_name: None,
        }
    }

    struct Listing {
        id: &'static str,
        available: bool,
    }

    impl Harness for Listing {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn caps(&self) -> Caps {
            unimplemented!()
        }
        fn available(&self) -> bool {
            self.available
        }
        fn one_shot(&self, _: &OneShot<'_>) -> AppResult<TurnResult> {
            unimplemented!()
        }
        fn spawn(&self, _: StreamRun<'_>) -> AppResult<Child> {
            unimplemented!()
        }
        fn parse_line(&self, _: &str) -> (String, Option<Value>) {
            unimplemented!()
        }
        fn normalize(&self, _: &str) -> Vec<Event> {
            unimplemented!()
        }
        fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
            false
        }
        fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
            None
        }
        fn live_sessions(&self) -> Vec<AgentSession> {
            vec![fake_session(self.id, "s")]
        }
    }

    #[test]
    fn live_sessions_merge_available_harnesses_in_order_minus_the_skipped() {
        let (a, b, c) = (Listing { id: "a", available: true }, Listing { id: "b", available: false }, Listing { id: "c", available: true });
        let ids = |skip| live_sessions_of(&[&a, &b, &c], skip).iter().map(|s| s.harness.clone()).collect::<Vec<_>>();
        assert_eq!(ids("none"), ["a", "c"]);
        assert_eq!(ids("a"), ["c"]);
    }

    #[test]
    fn the_picker_overrides_the_sessions_profile_default() {
        let conn = crate::db::open_memory();
        assert_eq!(sessions_harness(&conn, None).unwrap().id(), "claude");
        assert_eq!(sessions_harness(&conn, Some("")).unwrap().id(), "claude");
        assert_eq!(sessions_harness(&conn, Some("claude")).unwrap().id(), "claude");
        assert!(sessions_harness(&conn, Some("nope")).is_err());
    }

    #[test]
    fn harness_is_object_safe() {
        fn name(h: &dyn Harness) -> &'static str {
            h.name()
        }
        assert_eq!(name(&ClaudeCode), "Claude Code");
    }

    /// A harness with chosen capabilities, for the launch checks.
    struct Limited(Caps);

    impl Harness for Limited {
        fn id(&self) -> &'static str {
            "limited"
        }
        fn name(&self) -> &'static str {
            "Limited"
        }
        fn caps(&self) -> Caps {
            self.0.clone()
        }
        fn available(&self) -> bool {
            true
        }
        fn one_shot(&self, _: &OneShot<'_>) -> AppResult<TurnResult> {
            unimplemented!()
        }
        fn spawn(&self, _: StreamRun<'_>) -> AppResult<Child> {
            unimplemented!()
        }
        fn parse_line(&self, _: &str) -> (String, Option<Value>) {
            unimplemented!()
        }
        fn normalize(&self, _: &str) -> Vec<Event> {
            unimplemented!()
        }
        fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
            false
        }
        fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
            None
        }
    }

    fn limited(access_levels: Vec<Access>, tools: bool, tool_allowlist: bool) -> Limited {
        Limited(Caps { tools, tool_allowlist, access_levels, ..ClaudeCode.caps() })
    }

    fn needs(access: Access, tools: bool, tool_allowlist: bool) -> Needs {
        Needs { access, tools, tool_allowlist }
    }

    #[test]
    fn claude_satisfies_every_neutral_need() {
        for access in [Access::ReadOnly, Access::Edit, Access::Full, Access::Native("plan".into())] {
            check_caps(&ClaudeCode, "r", &needs(access, true, true)).unwrap();
        }
    }

    #[test]
    fn insufficient_caps_are_refused_naming_the_capability() {
        let h = limited(vec![Access::ReadOnly], false, false);
        let err = |n: Needs| check_caps(&h, "“Babysit”", &n).unwrap_err().to_string();
        assert!(err(needs(Access::Full, false, false)).contains("full access"));
        assert!(err(needs(Access::ReadOnly, true, false)).contains("tools"));
        let e = err(needs(Access::ReadOnly, false, true));
        assert!(e.contains("a tool allowlist") && e.contains("Limited") && e.contains("“Babysit”"), "{e}");
        check_caps(&h, "r", &needs(Access::ReadOnly, false, false)).unwrap();
    }
}
