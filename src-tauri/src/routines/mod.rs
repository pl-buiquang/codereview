//! Routines: named `claude -p` runs with their own prompt, scoped to a context (a PR, a review, an
//! issue or the board). `manager` owns the processes (streaming, queue, dedup, kill on exit),
//! `context` turns a launch reference into prompt variables and a working directory.

pub mod context;
pub mod handlers;
pub mod manager;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const BABYSIT_KEY: &str = "babysit";

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
    pub system_prompt_mode: String,
    pub needs_worktree: bool,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub extra_args: Vec<String>,
    pub model: String,
    pub timeout_min: i64,
    pub builtin: bool,
    pub handler: Option<String>,
    pub enabled: bool,
}

impl Routine {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let json_list = |col: &str| -> rusqlite::Result<Vec<String>> {
            let raw: String = row.get(col)?;
            Ok(serde_json::from_str(&raw).unwrap_or_default())
        };
        Ok(Self {
            id: row.get("id")?,
            key: row.get("key")?,
            name: row.get("name")?,
            description: row.get("description")?,
            context_type: row.get("context_type")?,
            prompt_template: row.get("prompt_template")?,
            system_prompt: row.get("system_prompt")?,
            system_prompt_mode: row.get("system_prompt_mode")?,
            needs_worktree: row.get::<_, i64>("needs_worktree")? != 0,
            permission_mode: row.get("permission_mode")?,
            allowed_tools: json_list("allowed_tools")?,
            extra_args: json_list("extra_args")?,
            model: row.get("model")?,
            timeout_min: row.get("timeout_min")?,
            builtin: row.get::<_, i64>("builtin")? != 0,
            handler: row.get("handler")?,
            enabled: row.get::<_, i64>("enabled")? != 0,
        })
    }
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

const BABYSIT_PROMPT: &str = "/goal PR {{repo}}#{{number}} has every required check passing on its latest head and its branch is not behind {{base_ref}}, or you have stopped and reported NEEDS_ATTENTION";

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
4. If a check failed, read its log (`gh run view <id> --log-failed`). A flaky or infrastructure failure (network, runner, \
timeout, unrelated test): rerun just the failed jobs (`gh run rerun <id> --failed`), at most 3 times per run. A real failure \
caused by the branch: fix it in the worktree, run the relevant tests locally, commit with a one-line Conventional Commit \
message (no body), and `git push` (never a plain `--force`). Then go back to step 1.
5. Never merge the PR, never post comments or reviews on GitHub, never touch other branches.

Stop when the goal holds, or when you cannot make progress without a human (a failure you cannot fix, a decision only the \
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
        description: "Keep a PR's CI green: wait for checks, rerun flaky jobs, update the branch, fix real failures and push.",
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
    ai_review("ai_review_explain", "AI review: explain", "Headless `/local-review`: one explanatory comment per change.", "/local-review {{review_id}}"),
    ai_review("ai_review_critique", "AI review: critique", "Headless `/local-review --critique` (Standards + Spec sub-agents).", "/local-review {{review_id}} --critique"),
    ai_review("ai_review_codex", "AI review: codex", "Headless `/local-review --codex` (Codex adversarial review).", "/local-review {{review_id}} --codex"),
];

/// Insert the built-in routines that don't exist yet. Existing rows (possibly edited) are left alone.
pub fn seed(conn: &Connection) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for d in DEFAULTS {
        conn.execute(
            "INSERT OR IGNORE INTO routine
               (key, name, description, context_type, prompt_template, system_prompt, system_prompt_mode,
                needs_worktree, permission_mode, allowed_tools, extra_args, timeout_min, handler, builtin,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?14, ?14)",
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
            conn.execute("UPDATE routine SET builtin = 1, handler = ?2 WHERE id = ?1", params![id, d.handler])?;
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

/// Put a built-in back to its shipped prompts, permissions, tools, arguments and timeout.
pub fn reset(conn: &Connection, id: i64) -> AppResult<Routine> {
    let routine = get(conn, id)?;
    let d = DEFAULTS
        .iter()
        .find(|d| d.key == routine.key)
        .ok_or_else(|| AppError::Other("only built-in routines can be reset".into()))?;
    conn.execute(
        "UPDATE routine SET name = ?2, description = ?3, prompt_template = ?4, system_prompt = ?5,
                system_prompt_mode = ?6, needs_worktree = ?7, permission_mode = ?8, allowed_tools = ?9,
                extra_args = ?10, model = '', timeout_min = ?11, enabled = 1, updated_at = ?12
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
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    get(conn, id)
}

/// What the Config tab sends. `id: None` creates a user routine.
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
    #[serde(default = "default_mode")]
    pub system_prompt_mode: String,
    #[serde(default)]
    pub needs_worktree: bool,
    #[serde(default = "default_permission")]
    pub permission_mode: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_timeout")]
    pub timeout_min: i64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_mode() -> String {
    "append".into()
}
fn default_permission() -> String {
    "bypassPermissions".into()
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

pub fn save(conn: &Connection, input: RoutineInput) -> AppResult<Routine> {
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
    if !["append", "replace"].contains(&input.system_prompt_mode.as_str()) {
        return Err(AppError::Other("system prompt mode must be append or replace".into()));
    }
    if !PERMISSION_MODES.contains(&input.permission_mode.as_str()) {
        return Err(AppError::Other(format!("unknown permission mode `{}`", input.permission_mode)));
    }
    let timeout = input.timeout_min.clamp(1, 24 * 60);
    let tools = serde_json::to_string(&input.allowed_tools).unwrap_or_else(|_| "[]".into());
    let args = serde_json::to_string(&input.extra_args).unwrap_or_else(|_| "[]".into());
    let now = chrono::Utc::now().to_rfc3339();

    match input.id {
        Some(id) => {
            // Built-ins keep their key and context type: the board, review and handlers rely on them.
            let existing = get(conn, id)?;
            let context_type = if existing.builtin { existing.context_type.clone() } else { input.context_type };
            conn.execute(
                "UPDATE routine SET name = ?2, description = ?3, context_type = ?4, prompt_template = ?5,
                        system_prompt = ?6, system_prompt_mode = ?7, needs_worktree = ?8, permission_mode = ?9,
                        allowed_tools = ?10, extra_args = ?11, model = ?12, timeout_min = ?13, enabled = ?14,
                        updated_at = ?15
                 WHERE id = ?1",
                params![
                    id,
                    name,
                    input.description,
                    context_type,
                    input.prompt_template,
                    input.system_prompt,
                    input.system_prompt_mode,
                    input.needs_worktree as i64,
                    input.permission_mode,
                    tools,
                    args,
                    input.model,
                    timeout,
                    input.enabled as i64,
                    now
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
            conn.execute(
                "INSERT INTO routine
                   (key, name, description, context_type, prompt_template, system_prompt, system_prompt_mode,
                    needs_worktree, permission_mode, allowed_tools, extra_args, model, timeout_min, enabled,
                    builtin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, ?15, ?15)",
                params![
                    key,
                    name,
                    input.description,
                    input.context_type,
                    input.prompt_template,
                    input.system_prompt,
                    input.system_prompt_mode,
                    input.needs_worktree as i64,
                    input.permission_mode,
                    tools,
                    args,
                    input.model,
                    timeout,
                    input.enabled as i64,
                    now
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
            system_prompt_mode: "append".into(),
            needs_worktree: false,
            permission_mode: "bypassPermissions".into(),
            allowed_tools: vec![],
            extra_args: vec![],
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
        assert!(babysit.builtin && babysit.needs_worktree && babysit.permission_mode == "bypassPermissions");
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
        assert_eq!(overview.system_prompt_mode, "replace");
        assert_eq!(overview.extra_args, ["--tools", "", "--effort", "low"]);
        let focus = get_key("focus");
        assert_eq!((focus.permission_mode.as_str(), focus.allowed_tools.len()), ("default", 8));
        let codex = get_key("ai_review_codex");
        assert_eq!(codex.prompt_template, "/local-review {{review_id}} --codex");
        assert_eq!(codex.permission_mode, "acceptEdits");
        assert!(codex.allowed_tools.contains(&"Bash(cr:*)".to_string()));
        // Reset restores the shipped tools and arguments.
        conn.execute("UPDATE routine SET allowed_tools = '[]' WHERE id = ?1", params![codex.id]).unwrap();
        assert!(!reset(&conn, codex.id).unwrap().allowed_tools.is_empty());
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
        bad.permission_mode = "yolo".into();
        assert!(save(&conn, bad).is_err());
        let mut empty = input("  ");
        empty.name = "  ".into();
        assert!(save(&conn, empty).is_err());
    }
}
