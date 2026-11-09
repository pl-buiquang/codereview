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

use crate::ai_usage::{RunTag, RunUsage};
use crate::claude_sessions::ClaudeSession;
use crate::error::{AppError, AppResult};

pub mod claude;
pub mod profiles;

pub use claude::ClaudeCode;

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

/// A normalized stream event. Defined here, produced from S04; until then `parse_line` hands back
/// the raw kind and JSON.
#[derive(Debug, Clone)]
pub enum Event {
    Text(String),
    ToolCall { name: String, input: Value },
    ToolResult { ok: bool, text: String },
    Final { ok: bool, text: String, usage: RunUsage },
    Other(Value),
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
    /// `(kind, parsed line)` of one stream line; unparseable lines are kept as kind `raw`.
    fn parse_line(&self, line: &str) -> (String, Option<Value>);
    /// Status, text and usage out of the last (`result`) event of a stream.
    fn final_result(&self, event: &Value) -> FinalResult;
    /// Whether `pid` is still this harness' streaming process (of that session, when known):
    /// guards re-adoption after a restart against pid reuse.
    fn is_own_process(&self, pid: i32, session_id: Option<&str>) -> bool;
    /// The argv that opens an interactive session, or `None` when the harness can't.
    fn interactive_argv(&self, opts: &Interactive<'_>) -> Option<Vec<String>>;
    /// Live sessions of this harness. Filled in by the sessions work; empty until then.
    fn live_sessions(&self) -> Vec<ClaudeSession> {
        Vec::new()
    }
}

// Compile-time proof that `Harness` is object-safe: this fails to build if a method breaks it.
const _: Option<&dyn Harness> = None;

static CLAUDE: ClaudeCode = ClaudeCode;

/// Every harness the app knows, in display order.
pub fn harnesses() -> [&'static dyn Harness; 1] {
    [&CLAUDE]
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

/// The argv of an interactive session of the harness the `sessions` feature profile selects.
pub fn interactive_argv(db: &crate::db::Db, opts: &Interactive<'_>) -> AppResult<Vec<String>> {
    let resolved = profiles::resolve_feature(&db.0.lock().unwrap(), profiles::Feature::Sessions);
    let harness = resolved.harness;
    harness
        .interactive_argv(opts)
        .ok_or_else(|| AppError::Other(format!("{} cannot open an interactive session like this", harness.name())))
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
        Some(format!("{} access", access_label(&needs.access)))
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
        assert!(h.live_sessions().is_empty());
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
        fn final_result(&self, _: &Value) -> FinalResult {
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
