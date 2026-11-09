//! Routines: named `claude -p` runs with their own prompt, scoped to a context (a PR, a review, an
//! issue or the board). `manager` owns the processes (streaming, queue, dedup, kill on exit),
//! `context` turns a launch reference into prompt variables and a working directory.

pub mod context;
pub mod handlers;
pub mod manager;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{Access, Needs, SystemPromptMode};

use crate::error::{AppError, AppResult};

pub const BABYSIT_KEY: &str = "babysit";

/// `plain` routines are editable and may run anywhere; `system` ones have a Rust handler, run
/// locally and only expose their profile, model, timeout and enabled flag.
pub const KIND_PLAIN: &str = "plain";
pub const KIND_SYSTEM: &str = "system";

/// The neutral access levels of a routine, least to most.
pub const ACCESS_LEVELS: &[&str] = &["read_only", "edit", "full"];

/// A routine definition, as stored and as edited in the Config tab.
#[derive(Debug, Clone, Serialize)]
pub struct Routine {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub description: String,
    pub context_type: String,
    pub prompt_template: String,
    pub system_prompt: String,
    pub kind: String,
    pub needs_worktree: bool,
    /// `read_only`, `edit` or `full`; each harness translates it.
    pub access: String,
    /// Per-harness details keyed by harness id (`{"claude": {permission_mode, allowed_tools,
    /// extra_args, system_prompt_mode}}`).
    pub harness_overrides: Value,
    pub profile_id: Option<i64>,
    pub model: String,
    pub timeout_min: i64,
    pub builtin: bool,
    pub handler: Option<String>,
    pub enabled: bool,
}

/// The Claude Code part of a routine's `harness_overrides`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeOptions {
    /// A native `--permission-mode`; `None` derives it from the routine's access.
    pub permission_mode: Option<String>,
    pub allowed_tools: Vec<String>,
    pub extra_args: Vec<String>,
    pub system_prompt_mode: SystemPromptMode,
}

impl ClaudeOptions {
    fn from_overrides(overrides: &Value) -> Self {
        let claude = overrides.get("claude").unwrap_or(&Value::Null);
        let list = |key: &str| -> Vec<String> {
            claude
                .get(key)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        Self {
            permission_mode: claude
                .get("permission_mode")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map(str::to_string),
            allowed_tools: list("allowed_tools"),
            extra_args: list("extra_args"),
            system_prompt_mode: match claude.get("system_prompt_mode").and_then(Value::as_str) {
                Some("replace") => SystemPromptMode::Replace,
                _ => SystemPromptMode::Append,
            },
        }
    }
}

/// What a harness gets from a routine's definition.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    pub system_prompt_mode: SystemPromptMode,
    pub allowed_tools: Vec<String>,
    pub extra_args: Vec<String>,
    pub access: Access,
}

impl Routine {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let overrides: String = row.get("harness_overrides")?;
        Ok(Self {
            id: row.get("id")?,
            key: row.get("key")?,
            name: row.get("name")?,
            description: row.get("description")?,
            context_type: row.get("context_type")?,
            prompt_template: row.get("prompt_template")?,
            system_prompt: row.get("system_prompt")?,
            kind: row.get("kind")?,
            needs_worktree: row.get::<_, i64>("needs_worktree")? != 0,
            access: row.get("access")?,
            harness_overrides: serde_json::from_str(&overrides).unwrap_or_else(|_| json!({})),
            profile_id: row.get("profile_id")?,
            model: row.get("model")?,
            timeout_min: row.get("timeout_min")?,
            builtin: row.get::<_, i64>("builtin")? != 0,
            handler: row.get("handler")?,
            enabled: row.get::<_, i64>("enabled")? != 0,
        })
    }

    pub fn is_system(&self) -> bool {
        self.kind == KIND_SYSTEM
    }

    pub fn claude(&self) -> ClaudeOptions {
        ClaudeOptions::from_overrides(&self.harness_overrides)
    }

    /// The neutral access level of the routine.
    pub fn neutral_access(&self) -> Access {
        neutral_access(&self.access)
    }

    /// The options a run of `harness_id` gets: Claude reads its own overrides (a native permission
    /// mode wins over the neutral access), other harnesses use the neutral access only.
    pub fn run_options(&self, harness_id: &str) -> RunOptions {
        if harness_id == "claude" {
            let c = self.claude();
            RunOptions {
                system_prompt_mode: c.system_prompt_mode,
                allowed_tools: c.allowed_tools,
                extra_args: c.extra_args,
                access: c.permission_mode.map_or_else(|| self.neutral_access(), Access::Native),
            }
        } else {
            RunOptions {
                system_prompt_mode: SystemPromptMode::Append,
                allowed_tools: Vec::new(),
                extra_args: Vec::new(),
                access: self.neutral_access(),
            }
        }
    }
}

/// What a run of `routine` needs from `harness_id`, for the capability check at launch.
pub fn launch_needs(routine: &Routine, harness_id: &str) -> Needs {
    let opts = routine.run_options(harness_id);
    let allowlist = !opts.allowed_tools.is_empty();
    Needs {
        access: routine.neutral_access(),
        tools: routine.needs_worktree || allowlist || routine.access != "read_only",
        tool_allowlist: allowlist,
    }
}

fn neutral_access(access: &str) -> Access {
    match access {
        "full" => Access::Full,
        "edit" => Access::Edit,
        _ => Access::ReadOnly,
    }
}

/// The neutral access a Claude `--permission-mode` stands for (same mapping as migration 0043).
pub fn access_for_permission_mode(mode: &str) -> &'static str {
    match mode {
        "bypassPermissions" => "full",
        "acceptEdits" => "edit",
        _ => "read_only",
    }
}

fn claude_overrides(permission_mode: &str, tools: &[&str], args: &[&str], system_prompt_mode: &str) -> Value {
    json!({ "claude": {
        "permission_mode": permission_mode,
        "allowed_tools": tools,
        "extra_args": args,
        "system_prompt_mode": system_prompt_mode,
    }})
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Routine> {
    conn.query_row("SELECT * FROM routine WHERE id = ?1", params![id], Routine::from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::Other(format!("routine {id} not found")),
            other => other.into(),
        })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Routine>> {
    let mut stmt = conn.prepare("SELECT * FROM routine ORDER BY builtin DESC, name COLLATE NOCASE")?;
    let rows = stmt.query_map([], Routine::from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// The fields a built-in is reset to.
pub struct Defaults {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub context_type: &'static str,
    pub prompt_template: &'static str,
    pub system_prompt: &'static str,
    pub system_prompt_mode: &'static str,
    pub needs_worktree: bool,
    pub permission_mode: &'static str,
    pub allowed_tools: &'static [&'static str],
    pub extra_args: &'static [&'static str],
    pub timeout_min: i64,
    /// Rust post-processing of the run (`handlers`); `None` for plain routines.
    pub handler: Option<&'static str>,
}

const BABYSIT_PROMPT: &str = "/goal PR {{repo}}#{{number}} has every required check passing on its latest head, its branch is not behind {{base_ref}} and it has no merge conflicts, or you have stopped and reported NEEDS_ATTENTION";

const BABYSIT_SYSTEM: &str = "\
You are babysitting pull request {{url}} ({{title}}). Your working directory is its checked-out worktree \
({{worktree}}), on branch `{{head_ref}}` (base `{{base_ref}}`). Keep going until the goal holds.

Loop:
1. Read the PR state: `gh pr view {{number}} --json mergeStateStatus,mergeable,statusCheckRollup,headRefOid` and \
`gh pr checks {{number}}`. Only the latest head commit matters.
2. If checks are still running, wait for them without burning turns: run `gh pr checks {{number}} --watch` in the \
foreground, or start a background `gh run watch` and wait for it.
3. If the branch is BEHIND `{{base_ref}}`, update it: `gh pr update-branch {{number}}`. If the PR is part of a stack \
(use the gh-stack skill), rebase it on its parent instead; `git push --force-with-lease` is allowed only after such a rebase.
3b. If the PR has merge conflicts (`mergeable` is CONFLICTING or `mergeStateStatus` is DIRTY), merge or rebase \
`origin/{{base_ref}}` into the worktree branch (rebase, then `--force-with-lease`, only if the branch is already rebased-style \
or part of a stack; otherwise a merge commit and a plain `git push`) and resolve the conflicts only if they are simple: \
adjacent or whitespace edits, import or list ordering, lockfile or generated-file regeneration, or one side clearly \
superseding the other with no intent to reconcile. Run the relevant tests afterwards. If any conflict needs a judgement \
about behaviour or intent, or touches logic both sides changed, run `git merge --abort` (or `git rebase --abort`) and stop \
with NEEDS_ATTENTION.
4. If a check failed, read its log (`gh run view <id> --log-failed`). A flaky or infrastructure failure (network, runner, \
timeout, unrelated test): rerun just the failed jobs (`gh run rerun <id> --failed`), at most 3 times per run. A real failure \
caused by the branch and simple to fix (a typo, lint or format error, an obvious test or snapshot update, a missing import): \
fix it in the worktree, run the relevant tests locally, commit with a one-line Conventional Commit \
message (no body), and `git push` (never a plain `--force`). Then go back to step 1.
5. Never merge the PR, never post comments or reviews on GitHub, never touch other branches.

Every fix and every conflict resolution must be simple. If it is not obvious and low-risk, do not attempt it.

Stop when the goal holds, or when you cannot make progress without a human (a failure or conflict that is not simple, a decision only the \
author can take, repeated reruns of the same failure). In that last case finish with `NEEDS_ATTENTION: <one-sentence diagnosis>`.";

const ISSUE_PLAN_SYSTEM: &str = concat!(include_str!("builtin/preamble.md"), "\n", include_str!("builtin/issue_plan.md"));

const ISSUE_PLAN_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "Agent",
    "Skill",
    "Bash(gh issue view:*)",
    "Bash(gh pr list:*)",
    "Bash(gh search:*)",
    "Bash(gh api:*)",
    "Bash(gh repo view:*)",
    "Bash(git log:*)",
    "Bash(git show:*)",
    "Bash(git grep:*)",
    "Bash(git fetch:*)",
    "Bash(cr:*)",
    "Bash(jq:*)",
    "Bash(ls:*)",
    "Bash(mkdir:*)",
    "Bash(notify:*)",
];

const ISSUE_IMPLEMENT_SYSTEM: &str = concat!(
    include_str!("builtin/preamble.md"),
    "\n",
    include_str!("builtin/issue_implement.md"),
    "\n",
    include_str!("builtin/worktree.md"),
    "\n",
    include_str!("builtin/pr_tail.md"),
);

const SMALL_ISSUE_KEY: &str = "address-small-issue";

const SMALL_ISSUE_SYSTEM: &str = concat!(
    include_str!("builtin/preamble.md"),
    "\n",
    include_str!("builtin/small_issue.md"),
    "\n",
    include_str!("builtin/worktree.md"),
    "\n",
    include_str!("builtin/pr_tail.md"),
);

const AI_REVIEW_SYSTEM: &str = "\
Headless app mode. Review id = {{review_id}} already exists: do not create or open reviews. \
The `cr` CLI on PATH already points at the app's database. Nobody can answer questions, so \
never use AskUserQuestion; pick sensible defaults. This session ends when you stop, so it \
cannot wait for background notifications: run the Codex companion in the foreground, never as \
a background Bash call. Finish with a summary of at most 5 lines.";

const IMAGE_DIFF_REPORT_SYSTEM: &str = include_str!("builtin/image_diff_report.md");

const IMAGE_DIFF_REPORT_TOOLS: &[&str] = &["Read", "Glob", "Bash(cr:*)", "Bash(ls:*)"];

pub const IMAGE_DIFF_REPORT_KEY: &str = "image_diff_report";

const fn ai_review(
    key: &'static str,
    name: &'static str,
    description: &'static str,
    prompt_template: &'static str,
) -> Defaults {
    Defaults {
        key,
        name,
        description,
        context_type: "review",
        prompt_template,
        system_prompt: AI_REVIEW_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: false,
        // Explicit so the user's own settings (e.g. `defaultMode: plan`) never leak in.
        permission_mode: "acceptEdits",
        allowed_tools: crate::commands::ai_review::TOOLS,
        extra_args: &[],
        timeout_min: 90,
        handler: Some("ai_review"),
    }
}

pub const DEFAULTS: &[Defaults] = &[
    Defaults {
        key: BABYSIT_KEY,
        name: "Babysit",
        description: "Keep a PR's CI green: wait for checks, rerun flaky jobs, update the branch, resolve simple merge conflicts, fix simple real failures and push; otherwise flag it as needing attention.",
        context_type: "pr",
        prompt_template: BABYSIT_PROMPT,
        system_prompt: BABYSIT_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: true,
        permission_mode: "bypassPermissions",
        allowed_tools: &[],
        extra_args: &[],
        timeout_min: 240,
        handler: None,
    },
    Defaults {
        key: "board_overview",
        name: "Board overview",
        description: "AI insights shown above the board digest. The board snapshot is the prompt.",
        context_type: "board",
        prompt_template: "{{snapshot}}",
        system_prompt: crate::prompts::BOARD_OVERVIEW_DEFAULT,
        system_prompt_mode: "replace",
        needs_worktree: false,
        permission_mode: "default",
        allowed_tools: &[],
        extra_args: &["--tools", "", "--effort", "low"],
        timeout_min: 10,
        handler: Some("overview"),
    },
    Defaults {
        key: "focus",
        name: "Focus",
        description: "Triage a review's hunks into focus tiers. The app-mode suffix with the review id and the `cr focus set` instructions is always appended to the system prompt.",
        context_type: "review",
        prompt_template: "Generate the focus for review {{review_id}}.",
        system_prompt: crate::prompts::FOCUS_DEFAULT,
        system_prompt_mode: "append",
        needs_worktree: false,
        permission_mode: "default",
        allowed_tools: crate::commands::focus::FOCUS_TOOLS,
        extra_args: &[],
        timeout_min: 30,
        handler: Some("focus"),
    },
    Defaults {
        key: "issue_plan",
        name: "Plan issue",
        description: "Analyse an issue and write a plan file (tracker ticket format), linked to the card with the `plan` tag. Refine it in an agent session, then run Implement plan.",
        context_type: "issue",
        prompt_template: "Write the implementation plan for issue {{repo}}#{{number}} ({{url}}), following the routine instructions.",
        system_prompt: ISSUE_PLAN_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: true,
        // Explicit so the user's own settings never leak in; writes are granted by the handler.
        permission_mode: "default",
        allowed_tools: ISSUE_PLAN_TOOLS,
        extra_args: &[],
        timeout_min: 30,
        handler: Some("issue_plan"),
    },
    Defaults {
        key: "issue_implement",
        name: "Implement plan",
        description: "Implement the issue's linked plan (status ready-for-agent) in a worktree: tdd, code review, draft PR linked to the issue, local review.",
        context_type: "issue",
        prompt_template: "Implement the approved plan for issue {{repo}}#{{number}} ({{url}}) and open a draft PR, following the routine instructions.",
        system_prompt: ISSUE_IMPLEMENT_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: true,
        permission_mode: "bypassPermissions",
        allowed_tools: &[],
        extra_args: &[],
        timeout_min: 240,
        handler: None,
    },
    Defaults {
        key: SMALL_ISSUE_KEY,
        name: "Draft PR (small issue)",
        description: "Fix a small issue in a worktree, open a draft PR linked to it, tag it automated, local-review it, notify.",
        context_type: "issue",
        prompt_template: "Fix issue {{repo}}#{{number}} ({{url}}) end to end and open a draft PR, following the routine instructions.",
        system_prompt: SMALL_ISSUE_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: true,
        permission_mode: "bypassPermissions",
        allowed_tools: &[],
        extra_args: &[],
        timeout_min: 60,
        handler: None,
    },
    Defaults {
        key: IMAGE_DIFF_REPORT_KEY,
        name: "Image diff report",
        description: "Look at a review's generated image diffs and describe what changed in each, with before / after / diff crops. Shown in the image diffs panel; the app-mode suffix with the paths is always appended to the system prompt.",
        context_type: "review",
        prompt_template: "Write the image diff report for review {{review_id}}.",
        system_prompt: IMAGE_DIFF_REPORT_SYSTEM,
        system_prompt_mode: "append",
        needs_worktree: false,
        // Explicit so the user's own settings never leak in; the report file is granted by the handler.
        permission_mode: "default",
        allowed_tools: IMAGE_DIFF_REPORT_TOOLS,
        extra_args: &[],
        timeout_min: 30,
        handler: Some("image_diff_report"),
    },
    ai_review("ai_review_explain", "AI review: explain", "Headless `/local-review`: one explanatory comment per change.", "/local-review {{review_id}}"),
    ai_review("ai_review_critique", "AI review: critique", "Headless `/local-review --critique` (Standards + Spec sub-agents).", "/local-review {{review_id}} --critique"),
    ai_review("ai_review_codex", "AI review: codex", "Headless `/local-review --codex` (Codex adversarial review).", "/local-review {{review_id}} --codex"),
];

impl Defaults {
    fn kind(&self) -> &'static str {
        if self.handler.is_some() {
            KIND_SYSTEM
        } else {
            KIND_PLAIN
        }
    }

    fn overrides_json(&self) -> String {
        claude_overrides(self.permission_mode, self.allowed_tools, self.extra_args, self.system_prompt_mode).to_string()
    }
}

/// Insert the built-in routines that don't exist yet. Existing rows (possibly edited) are left alone.
pub fn seed(conn: &Connection) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for d in DEFAULTS {
        // The old Claude columns are still written (not read) so a rollback keeps working.
        conn.execute(
            "INSERT OR IGNORE INTO routine
               (key, name, description, context_type, prompt_template, system_prompt, system_prompt_mode,
                needs_worktree, permission_mode, allowed_tools, extra_args, timeout_min, handler, builtin,
                kind, access, harness_overrides, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?14, ?15, ?16, ?17, ?17)",
            params![
                d.key,
                d.name,
                d.description,
                d.context_type,
                d.prompt_template,
                d.system_prompt,
                d.system_prompt_mode,
                d.needs_worktree as i64,
                d.permission_mode,
                serde_json::to_string(d.allowed_tools).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(d.extra_args).unwrap_or_else(|_| "[]".into()),
                d.timeout_min,
                d.handler,
                d.kind(),
                access_for_permission_mode(d.permission_mode),
                d.overrides_json(),
                now
            ],
        )?;
    }
    Ok(())
}

/// Routines that began as user-made rows and now ship built in. A row with such a key and
/// `builtin = 0` is adopted: flagged built-in and set to the shipped definition (a Reset would
/// do the same). Its id, and so its run history, is kept. Idempotent.
const ADOPTED_KEYS: &[&str] = &[SMALL_ISSUE_KEY];

pub fn adopt_builtin_keys(conn: &Connection) -> AppResult<()> {
    for d in DEFAULTS.iter().filter(|d| ADOPTED_KEYS.contains(&d.key)) {
        let id: Option<i64> =
            conn.query_row("SELECT id FROM routine WHERE key = ?1 AND builtin = 0", params![d.key], |r| r.get(0)).optional()?;
        if let Some(id) = id {
            conn.execute("UPDATE routine SET builtin = 1, handler = ?2, kind = ?3 WHERE id = ?1", params![id, d.handler, d.kind()])?;
            reset(conn, id)?;
        }
    }
    Ok(())
}

/// One-time move of the former Settings → Prompts entries into the routines that now own them: a
/// built-in `board_overview` / `focus` with an empty system prompt (rows seeded before the move)
/// gets the user's override, else the shipped text. The old override rows are then dropped.
/// Idempotent: a routine that already has a system prompt is left alone.
pub fn adopt_prompt_overrides(conn: &Connection) -> AppResult<()> {
    use crate::prompts::{BOARD_OVERVIEW, BOARD_OVERVIEW_DEFAULT, FOCUS_DEFAULT, FOCUS_SYSTEM};
    for (routine_key, prompt_key, default) in
        [("board_overview", BOARD_OVERVIEW, BOARD_OVERVIEW_DEFAULT), ("focus", FOCUS_SYSTEM, FOCUS_DEFAULT)]
    {
        let custom: Option<String> = conn
            .query_row("SELECT body FROM prompt_override WHERE key = ?1", params![prompt_key], |r| r.get(0))
            .optional()?;
        conn.execute(
            "UPDATE routine SET system_prompt = ?2 WHERE key = ?1 AND builtin = 1 AND TRIM(system_prompt) = ''",
            params![routine_key, custom.as_deref().unwrap_or(default)],
        )?;
        conn.execute("DELETE FROM prompt_override WHERE key = ?1", params![prompt_key])?;
    }
    Ok(())
}

/// Put a built-in back to its shipped prompts, access, tools, arguments, profile and timeout.
pub fn reset(conn: &Connection, id: i64) -> AppResult<Routine> {
    let routine = get(conn, id)?;
    let d = DEFAULTS
        .iter()
        .find(|d| d.key == routine.key)
        .ok_or_else(|| AppError::Other("only built-in routines can be reset".into()))?;
    conn.execute(
        "UPDATE routine SET name = ?2, description = ?3, prompt_template = ?4, system_prompt = ?5,
                system_prompt_mode = ?6, needs_worktree = ?7, permission_mode = ?8, allowed_tools = ?9,
                extra_args = ?10, model = '', timeout_min = ?11, enabled = 1, updated_at = ?12,
                kind = ?13, access = ?14, harness_overrides = ?15, profile_id = NULL
         WHERE id = ?1",
        params![
            id,
            d.name,
            d.description,
            d.prompt_template,
            d.system_prompt,
            d.system_prompt_mode,
            d.needs_worktree as i64,
            d.permission_mode,
            serde_json::to_string(d.allowed_tools).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(d.extra_args).unwrap_or_else(|_| "[]".into()),
            d.timeout_min,
            chrono::Utc::now().to_rfc3339(),
            d.kind(),
            access_for_permission_mode(d.permission_mode),
            d.overrides_json(),
        ],
    )?;
    get(conn, id)
}

/// What the Config tab sends. `id: None` creates a user routine. On a system routine only
/// `profile_id`, `model`, `timeout_min` and `enabled` are applied.
#[derive(Debug, Deserialize)]
pub struct RoutineInput {
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub context_type: String,
    pub prompt_template: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub needs_worktree: bool,
    #[serde(default = "default_access")]
    pub access: String,
    #[serde(default = "empty_object")]
    pub harness_overrides: Value,
    #[serde(default)]
    pub profile_id: Option<i64>,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_timeout")]
    pub timeout_min: i64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_access() -> String {
    "full".into()
}
fn empty_object() -> Value {
    json!({})
}
fn default_timeout() -> i64 {
    240
}
fn default_true() -> bool {
    true
}

const PERMISSION_MODES: &[&str] = &["default", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"];

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() {
        "routine".into()
    } else {
        s
    }
}

/// Validate the overrides and rewrite the Claude entry in its canonical shape; entries of other
/// harnesses are kept as sent.
fn normalize_overrides(raw: &Value) -> AppResult<Value> {
    let Value::Object(map) = raw else {
        return Err(AppError::Other("harness overrides must be a JSON object".into()));
    };
    let mut out = map.clone();
    if let Some(claude) = map.get("claude") {
        let opts = ClaudeOptions::from_overrides(raw);
        if let Some(mode) = &opts.permission_mode {
            if !PERMISSION_MODES.contains(&mode.as_str()) {
                return Err(AppError::Other(format!("unknown permission mode `{mode}`")));
            }
        }
        if let Some(mode) = claude.get("system_prompt_mode").and_then(Value::as_str) {
            if !["append", "replace"].contains(&mode) {
                return Err(AppError::Other("system prompt mode must be append or replace".into()));
            }
        }
        out.insert(
            "claude".into(),
            json!({
                "permission_mode": opts.permission_mode.unwrap_or_default(),
                "allowed_tools": opts.allowed_tools,
                "extra_args": opts.extra_args,
                "system_prompt_mode": if opts.system_prompt_mode == SystemPromptMode::Replace { "replace" } else { "append" },
            }),
        );
    }
    Ok(Value::Object(out))
}

/// The old Claude columns, kept in step with the overrides for one release (rollback safety).
fn legacy_columns(access: &str, overrides: &Value) -> (String, String, String, String) {
    let c = ClaudeOptions::from_overrides(overrides);
    let mode = c.permission_mode.unwrap_or_else(|| {
        match access {
            "full" => "bypassPermissions",
            "edit" => "acceptEdits",
            _ => "plan",
        }
        .to_string()
    });
    (
        mode,
        serde_json::to_string(&c.allowed_tools).unwrap_or_else(|_| "[]".into()),
        serde_json::to_string(&c.extra_args).unwrap_or_else(|_| "[]".into()),
        if c.system_prompt_mode == SystemPromptMode::Replace { "replace" } else { "append" }.to_string(),
    )
}

fn check_profile(conn: &Connection, profile_id: Option<i64>) -> AppResult<()> {
    if let Some(id) = profile_id {
        let exists: bool = conn.query_row("SELECT COUNT(*) FROM ai_profile WHERE id = ?1", params![id], |r| r.get::<_, i64>(0))? > 0;
        if !exists {
            return Err(AppError::Other(format!("profile {id} not found")));
        }
    }
    Ok(())
}

pub fn save(conn: &Connection, input: RoutineInput) -> AppResult<Routine> {
    check_profile(conn, input.profile_id)?;
    let timeout = input.timeout_min.clamp(1, 24 * 60);
    let now = chrono::Utc::now().to_rfc3339();

    // System routines: the handler owns everything but the profile, model, timeout and enabled flag.
    if let Some(id) = input.id {
        if get(conn, id)?.is_system() {
            conn.execute(
                "UPDATE routine SET profile_id = ?2, model = ?3, timeout_min = ?4, enabled = ?5, updated_at = ?6 WHERE id = ?1",
                params![id, input.profile_id, input.model.trim(), timeout, input.enabled as i64, now],
            )?;
            return get(conn, id);
        }
    }

    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("a routine needs a name".into()));
    }
    if input.prompt_template.trim().is_empty() {
        return Err(AppError::Other("a routine needs a prompt".into()));
    }
    if !["pr", "review", "issue", "board"].contains(&input.context_type.as_str()) {
        return Err(AppError::Other(format!("unknown context type `{}`", input.context_type)));
    }
    if !ACCESS_LEVELS.contains(&input.access.as_str()) {
        return Err(AppError::Other(format!("unknown access level `{}`", input.access)));
    }
    let mut overrides = normalize_overrides(&input.harness_overrides)?;

    match input.id {
        Some(id) => {
            // Built-ins keep their key and context type: the board, review and handlers rely on them.
            let existing = get(conn, id)?;
            // A native permission mode wins over the access, so changing the access alone would
            // look like it did nothing: drop the mode that still stands for the old access.
            if input.access != existing.access
                && ClaudeOptions::from_overrides(&overrides).permission_mode == existing.claude().permission_mode
            {
                if let Some(claude) = overrides.get_mut("claude").and_then(Value::as_object_mut) {
                    claude.insert("permission_mode".into(), json!(""));
                }
            }
            let context_type = if existing.builtin { existing.context_type.clone() } else { input.context_type };
            let (mode, tools, args, prompt_mode) = legacy_columns(&input.access, &overrides);
            conn.execute(
                "UPDATE routine SET name = ?2, description = ?3, context_type = ?4, prompt_template = ?5,
                        system_prompt = ?6, needs_worktree = ?7, access = ?8, harness_overrides = ?9,
                        profile_id = ?10, model = ?11, timeout_min = ?12, enabled = ?13, updated_at = ?14,
                        permission_mode = ?15, allowed_tools = ?16, extra_args = ?17, system_prompt_mode = ?18
                 WHERE id = ?1",
                params![
                    id,
                    name,
                    input.description,
                    context_type,
                    input.prompt_template,
                    input.system_prompt,
                    input.needs_worktree as i64,
                    input.access,
                    overrides.to_string(),
                    input.profile_id,
                    input.model.trim(),
                    timeout,
                    input.enabled as i64,
                    now,
                    mode,
                    tools,
                    args,
                    prompt_mode
                ],
            )?;
            get(conn, id)
        }
        None => {
            let base = slug(&name);
            let mut key = base.clone();
            let mut n = 2;
            while conn
                .query_row("SELECT 1 FROM routine WHERE key = ?1", params![key], |_| Ok(()))
                .is_ok()
            {
                key = format!("{base}-{n}");
                n += 1;
            }
            let (mode, tools, args, prompt_mode) = legacy_columns(&input.access, &overrides);
            conn.execute(
                "INSERT INTO routine
                   (key, name, description, context_type, prompt_template, system_prompt, needs_worktree, access,
                    harness_overrides, profile_id, model, timeout_min, enabled, builtin, kind, created_at, updated_at,
                    permission_mode, allowed_tools, extra_args, system_prompt_mode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 'plain', ?14, ?14, ?15, ?16, ?17, ?18)",
                params![
                    key,
                    name,
                    input.description,
                    input.context_type,
                    input.prompt_template,
                    input.system_prompt,
                    input.needs_worktree as i64,
                    input.access,
                    overrides.to_string(),
                    input.profile_id,
                    input.model.trim(),
                    timeout,
                    input.enabled as i64,
                    now,
                    mode,
                    tools,
                    args,
                    prompt_mode
                ],
            )?;
            get(conn, conn.last_insert_rowid())
        }
    }
}

/// Whether a babysit run is queued or running on the context.
pub fn babysit_active(conn: &Connection, context_key: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM routine_run r JOIN routine ro ON ro.id = r.routine_id
         WHERE ro.key = ?1 AND r.context_key = ?2 AND r.status IN ('queued', 'running')",
        params![BABYSIT_KEY, context_key],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    if get(conn, id)?.builtin {
        return Err(AppError::Other("built-in routines can't be deleted (use reset)".into()));
    }
    conn.execute("DELETE FROM routine WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn input(name: &str) -> RoutineInput {
        RoutineInput {
            id: None,
            name: name.into(),
            description: String::new(),
            context_type: "pr".into(),
            prompt_template: "hi {{number}}".into(),
            system_prompt: String::new(),
            needs_worktree: false,
            access: "full".into(),
            harness_overrides: json!({}),
            profile_id: None,
            model: String::new(),
            timeout_min: 30,
            enabled: true,
        }
    }

    fn system_of(conn: &Connection, key: &str) -> String {
        conn.query_row("SELECT system_prompt FROM routine WHERE key = ?1", params![key], |r| r.get(0)).unwrap()
    }

    #[test]
    fn adopts_a_user_made_row_with_a_shipped_key() {
        let conn = open_memory();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO routine (key, name, context_type, prompt_template, system_prompt, needs_worktree, builtin, created_at, updated_at)
             VALUES ('address-small-issue', 'Draft PR (small issue)', 'issue', 'old', 'old text', 0, 0, ?1, ?1)",
            params![now],
        )
        .unwrap();
        let id: i64 = conn.query_row("SELECT id FROM routine WHERE key = 'address-small-issue'", [], |r| r.get(0)).unwrap();
        seed(&conn).unwrap();
        for _ in 0..2 {
            adopt_builtin_keys(&conn).unwrap();
        }
        let r = get(&conn, id).unwrap();
        assert!(r.builtin);
        assert!(r.needs_worktree);
        assert!(r.system_prompt.contains("Local review"));
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM routine WHERE key = 'address-small-issue'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn shipped_issue_prompts_have_no_unknown_placeholders() {
        let known = ["repo", "number", "url", "title", "plans_dir", "head_ref", "base_ref", "worktree", "review_id"];
        for d in DEFAULTS.iter().filter(|d| d.context_type == "issue") {
            for text in [d.system_prompt, d.prompt_template] {
                for part in text.split("{{").skip(1) {
                    let name = part.split("}}").next().unwrap();
                    assert!(known.contains(&name), "{}: unknown placeholder {{{{{name}}}}}", d.key);
                }
            }
        }
    }

    fn override_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM prompt_override", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn adopt_moves_overrides_and_defaults_into_the_routines() {
        let conn = open_memory();
        seed(&conn).unwrap();
        // A row seeded before the move has an empty system prompt.
        conn.execute("UPDATE routine SET system_prompt = '' WHERE key IN ('focus', 'board_overview')", []).unwrap();
        conn.execute(
            "INSERT INTO prompt_override (key, body, updated_at) VALUES ('board.overview', 'mine', 'now'), ('chat.system', 'chat', 'now')",
            [],
        )
        .unwrap();

        adopt_prompt_overrides(&conn).unwrap();
        assert_eq!(system_of(&conn, "board_overview"), "mine");
        assert_eq!(system_of(&conn, "focus"), crate::prompts::FOCUS_DEFAULT);
        assert_eq!(override_count(&conn), 1, "only the chat override is left");

        // Idempotent, and a routine that now has a prompt is not touched again.
        conn.execute("UPDATE routine SET system_prompt = 'edited' WHERE key = 'focus'", []).unwrap();
        adopt_prompt_overrides(&conn).unwrap();
        assert_eq!(system_of(&conn, "focus"), "edited");
        assert_eq!(system_of(&conn, "board_overview"), "mine");
    }

    #[test]
    fn seeded_overview_and_focus_ship_their_system_prompt() {
        let conn = open_memory();
        seed(&conn).unwrap();
        assert_eq!(system_of(&conn, "board_overview"), crate::prompts::BOARD_OVERVIEW_DEFAULT);
        assert_eq!(system_of(&conn, "focus"), crate::prompts::FOCUS_DEFAULT);
    }

    #[test]
    fn seed_is_idempotent_and_keeps_edits() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let babysit = list(&conn).unwrap().into_iter().find(|r| r.key == BABYSIT_KEY).unwrap();
        assert!(babysit.builtin && babysit.needs_worktree && babysit.claude().permission_mode.as_deref() == Some("bypassPermissions"));
        assert_eq!((babysit.kind.as_str(), babysit.access.as_str()), ("plain", "full"));
        assert!(babysit.prompt_template.starts_with("/goal "));

        conn.execute("UPDATE routine SET timeout_min = 5 WHERE id = ?1", params![babysit.id]).unwrap();
        seed(&conn).unwrap();
        assert_eq!(get(&conn, babysit.id).unwrap().timeout_min, 5);
        assert_eq!(reset(&conn, babysit.id).unwrap().timeout_min, 240);
    }

    #[test]
    fn feature_builtins_are_seeded_with_their_handlers_and_tools() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let all = list(&conn).unwrap();
        let get_key = |k: &str| all.iter().find(|r| r.key == k).unwrap();
        let overview = get_key("board_overview");
        assert_eq!((overview.handler.as_deref(), overview.context_type.as_str()), (Some("overview"), "board"));
        assert_eq!(overview.claude().system_prompt_mode, SystemPromptMode::Replace);
        assert_eq!(overview.claude().extra_args, ["--tools", "", "--effort", "low"]);
        assert_eq!((overview.kind.as_str(), overview.access.as_str()), ("system", "read_only"));
        let focus = get_key("focus");
        assert_eq!((focus.claude().permission_mode.as_deref(), focus.claude().allowed_tools.len()), (Some("default"), 8));
        let codex = get_key("ai_review_codex");
        assert_eq!(codex.prompt_template, "/local-review {{review_id}} --codex");
        assert_eq!(codex.claude().permission_mode.as_deref(), Some("acceptEdits"));
        assert_eq!((codex.kind.as_str(), codex.access.as_str()), ("system", "edit"));
        assert!(codex.claude().allowed_tools.contains(&"Bash(cr:*)".to_string()));
        // Reset restores the shipped tools and arguments.
        conn.execute(
            "UPDATE routine SET harness_overrides = '{}', access = 'full', profile_id = NULL WHERE id = ?1",
            params![codex.id],
        )
        .unwrap();
        let reset = reset(&conn, codex.id).unwrap();
        assert!(!reset.claude().allowed_tools.is_empty());
        assert_eq!(reset.access, "edit");
    }

    #[test]
    fn user_routines_get_unique_keys_and_can_be_deleted_but_builtins_cannot() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let a = save(&conn, input("Fix lint")).unwrap();
        let b = save(&conn, input("Fix lint")).unwrap();
        assert_eq!((a.key.as_str(), b.key.as_str()), ("fix-lint", "fix-lint-2"));
        delete(&conn, a.id).unwrap();
        let babysit = list(&conn).unwrap().into_iter().find(|r| r.key == BABYSIT_KEY).unwrap();
        assert!(delete(&conn, babysit.id).is_err());
    }

    #[test]
    fn builtin_context_type_cannot_change() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let babysit = list(&conn).unwrap().into_iter().find(|r| r.key == BABYSIT_KEY).unwrap();
        let mut edit = input("Babysit");
        edit.id = Some(babysit.id);
        edit.context_type = "issue".into();
        assert_eq!(save(&conn, edit).unwrap().context_type, "pr");
    }

    #[test]
    fn rejects_bad_input() {
        let conn = open_memory();
        let mut bad = input("x");
        bad.harness_overrides = json!({ "claude": { "permission_mode": "yolo" } });
        assert!(save(&conn, bad).is_err());
        let mut bad = input("x");
        bad.access = "root".into();
        assert!(save(&conn, bad).is_err());
        let mut bad = input("x");
        bad.profile_id = Some(999);
        assert!(save(&conn, bad).is_err());
        let mut bad = input("x");
        bad.harness_overrides = json!({ "claude": { "system_prompt_mode": "weird" } });
        assert!(save(&conn, bad).is_err());
        let mut empty = input("  ");
        empty.name = "  ".into();
        assert!(save(&conn, empty).is_err());
    }

    fn plain_by_key(conn: &Connection, key: &str) -> Routine {
        list(conn).unwrap().into_iter().find(|r| r.key == key).unwrap()
    }

    #[test]
    fn every_seeded_handler_routine_is_system_and_the_rest_plain() {
        let conn = open_memory();
        seed(&conn).unwrap();
        for r in list(&conn).unwrap() {
            assert_eq!(r.is_system(), r.handler.is_some(), "{}", r.key);
        }
        for key in [BABYSIT_KEY, "issue_implement", SMALL_ISSUE_KEY] {
            assert_eq!(plain_by_key(&conn, key).kind, "plain", "{key}");
        }
    }

    #[test]
    fn seeded_access_matches_the_migration_mapping() {
        let conn = open_memory();
        seed(&conn).unwrap();
        for d in DEFAULTS {
            let r = plain_by_key(&conn, d.key);
            assert_eq!(r.access, access_for_permission_mode(d.permission_mode), "{}", d.key);
            assert_eq!(r.claude().permission_mode.as_deref(), Some(d.permission_mode), "{}", d.key);
        }
    }

    #[test]
    fn system_routines_only_take_profile_model_timeout_and_enabled() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let focus = plain_by_key(&conn, "focus");
        let profile: i64 = conn.query_row("SELECT id FROM ai_profile", [], |r| r.get(0)).unwrap();
        let mut edit = input("Hacked");
        edit.id = Some(focus.id);
        edit.prompt_template = "evil".into();
        edit.access = "full".into();
        edit.profile_id = Some(profile);
        edit.model = "haiku".into();
        edit.timeout_min = 7;
        edit.enabled = false;
        let saved = save(&conn, edit).unwrap();
        assert_eq!(
            (saved.profile_id, saved.model.as_str(), saved.timeout_min, saved.enabled),
            (Some(profile), "haiku", 7, false)
        );
        assert_eq!(saved.name, focus.name);
        assert_eq!(saved.prompt_template, focus.prompt_template);
        assert_eq!(saved.access, focus.access);
        assert_eq!(saved.harness_overrides, focus.harness_overrides);
    }

    #[test]
    fn saving_a_plain_routine_keeps_the_legacy_columns_in_step() {
        let conn = open_memory();
        let mut new = input("Mirror");
        new.harness_overrides = json!({ "claude": {
            "permission_mode": "acceptEdits", "allowed_tools": ["Read"], "extra_args": ["--effort", "low"], "system_prompt_mode": "replace"
        }});
        new.access = "edit".into();
        let r = save(&conn, new).unwrap();
        assert_eq!(r.kind, "plain");
        let (mode, tools, args, spm): (String, String, String, String) = conn
            .query_row("SELECT permission_mode, allowed_tools, extra_args, system_prompt_mode FROM routine WHERE id = ?1", [r.id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        assert_eq!((mode.as_str(), tools.as_str(), args.as_str(), spm.as_str()), ("acceptEdits", "[\"Read\"]", "[\"--effort\",\"low\"]", "replace"));
    }

    #[test]
    fn changing_only_the_access_drops_the_stale_native_permission_mode() {
        let conn = open_memory();
        let mut new = input("Access");
        new.access = "edit".into();
        new.harness_overrides = json!({ "claude": { "permission_mode": "acceptEdits" } });
        let r = save(&conn, new).unwrap();
        assert_eq!(r.run_options("claude").access, Access::Native("acceptEdits".into()));

        let mut edit = input("Access");
        edit.id = Some(r.id);
        edit.access = "full".into();
        edit.harness_overrides = r.harness_overrides.clone();
        let r = save(&conn, edit).unwrap();
        assert_eq!(r.claude().permission_mode, None);
        assert_eq!(r.run_options("claude").access, Access::Full);

        // An explicit new mode alongside the new access is kept.
        let mut both = input("Access");
        both.id = Some(r.id);
        both.access = "read_only".into();
        both.harness_overrides = json!({ "claude": { "permission_mode": "plan" } });
        assert_eq!(save(&conn, both).unwrap().claude().permission_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn run_options_are_native_for_claude_and_neutral_elsewhere() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let focus = plain_by_key(&conn, "focus");
        let claude = focus.run_options("claude");
        assert_eq!(claude.access, Access::Native("default".into()));
        assert_eq!(claude.allowed_tools.len(), 8);
        let other = focus.run_options("opencode");
        assert_eq!(other.access, Access::ReadOnly);
        assert!(other.allowed_tools.is_empty() && other.extra_args.is_empty());
        assert_eq!(plain_by_key(&conn, BABYSIT_KEY).run_options("other").access, Access::Full);
    }

    #[test]
    fn launch_needs_reflect_access_tools_and_the_worktree() {
        let conn = open_memory();
        seed(&conn).unwrap();
        let babysit = plain_by_key(&conn, BABYSIT_KEY);
        let n = launch_needs(&babysit, "claude");
        assert_eq!((n.access, n.tools, n.tool_allowlist), (Access::Full, true, false));
        // Claude's allowlist is Claude's detail: another harness gets none.
        let focus = plain_by_key(&conn, "focus");
        let claude = launch_needs(&focus, "claude");
        assert_eq!((claude.access.clone(), claude.tools, claude.tool_allowlist), (Access::ReadOnly, true, true));
        let other = launch_needs(&focus, "other");
        assert_eq!((other.tools, other.tool_allowlist), (false, false));
        // The caps check passes on Claude for all of them.
        for r in list(&conn).unwrap() {
            crate::ai::check_caps(&crate::ai::ClaudeCode, &r.name, &launch_needs(&r, "claude")).unwrap();
        }
    }
}
