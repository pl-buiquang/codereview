use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::models::{Chat, ChatMessage, Target};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::worktree;

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Generate a UUID v4 without an external crate. Uses microsecond timestamp +
/// an atomic counter + process id for uniqueness.
fn new_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;

    let a = (ts >> 16) as u32;
    let b = (ts & 0xffff) as u16;
    let c = 0x4000u16 | ((counter >> 48) as u16 & 0x0fff); // version 4
    let d = 0x8000u16 | ((pid ^ counter) as u16 & 0x3fff); // variant 10
    let e = (counter ^ (pid << 16)) & 0x0000_ffff_ffff_ffff;
    format!("{a:08x}-{b:04x}-{c:04x}-{d:04x}-{e:012x}")
}

/// JSON envelope returned by `claude -p --output-format json`.
#[derive(Deserialize)]
struct ClaudeEnvelope {
    result: Option<serde_json::Value>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    total_cost_usd: f64,
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
}

pub(crate) struct TurnResult {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Response returned to the frontend after a chat turn.
#[derive(Serialize)]
pub struct ChatTurnResult {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Options for one non-interactive `claude -p` run.
pub(crate) struct ClaudeRun<'a> {
    pub model: &'a str,
    pub allowed_tools: &'a [&'a str],
    pub permission_mode: &'a str,
    pub system_prompt: Option<&'a str>,
    /// Name for the temp system-prompt file; must be unique per concurrent run.
    pub prompt_file_tag: &'a str,
    pub cwd: &'a Path,
    /// Directories beyond `cwd` the tools may access.
    pub extra_dirs: &'a [&'a Path],
    /// `(session_id, is_first_turn)`; None runs without session persistence.
    pub session: Option<(&'a str, bool)>,
    pub env: Vec<(String, std::ffi::OsString)>,
    pub extra_args: &'a [&'a str],
    /// Replace Claude Code's default system prompt instead of appending to it.
    pub replace_system_prompt: bool,
    pub prompt: &'a str,
}

/// Call `claude -p` for one conversation turn. On the first turn (no session)
/// establishes a new session with `--session-id`; on subsequent turns resumes
/// with `--resume`. The system prompt is only sent on the first turn.
fn call_claude_turn(
    session_id: &str,
    is_first_turn: bool,
    model: &str,
    system_prompt: Option<&str>,
    worktree_path: &Path,
    text: &str,
) -> AppResult<TurnResult> {
    run_claude(ClaudeRun {
        model,
        allowed_tools: &["Bash", "Read", "Glob", "Grep", "Edit", "Write"],
        // Explicit, so the user's own `~/.claude/settings.json` (e.g. a
        // `defaultMode: plan`) never leaks into this non-interactive session.
        permission_mode: "acceptEdits",
        system_prompt: if is_first_turn { system_prompt } else { None },
        prompt_file_tag: &format!("chat_{session_id}"),
        cwd: worktree_path,
        extra_dirs: &[],
        session: Some((session_id, is_first_turn)),
        env: Vec::new(),
        extra_args: &[],
        replace_system_prompt: false,
        prompt: text,
    })
}

/// Model resolution priority:
/// 1. Explicit setting from the app (passed by the frontend)
/// 2. CODEREVIEW_CHAT_MODEL env var (manual override)
/// 3. ANTHROPIC_DEFAULT_OPUS_MODEL env var (Bedrock ARN etc.)
/// 4. Empty → no --model flag; claude uses its own configured default
pub(crate) fn resolve_model(model: Option<String>) -> String {
    model
        .filter(|m| !m.is_empty())
        .or_else(|| std::env::var("CODEREVIEW_CHAT_MODEL").ok())
        .or_else(|| std::env::var("ANTHROPIC_DEFAULT_OPUS_MODEL").ok())
        .unwrap_or_default()
}

pub(crate) fn run_claude(run: ClaudeRun) -> AppResult<TurnResult> {
    let mut cmd = Command::new(crate::tools::claude_bin());
    // Run from the reviewed project so CLAUDE.md, git context, etc. come from
    // it, not the Tauri app's directory.
    cmd.current_dir(run.cwd);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("json")
        // --strict-mcp-config: don't load MCP servers (they add latency/noise).
        // Note: we deliberately omit --setting-sources "" so claude can read its
        // own auth and Bedrock configuration from ~/.claude/.
        .arg("--strict-mcp-config")
        .arg("--permission-mode")
        .arg(run.permission_mode)
        .arg("--add-dir")
        .arg(run.cwd)
        .args(run.extra_dirs.iter().flat_map(|d| [std::ffi::OsStr::new("--add-dir"), d.as_os_str()]));
    if !run.allowed_tools.is_empty() {
        cmd.arg("--allowedTools").args(run.allowed_tools);
    }
    for (k, v) in &run.env {
        cmd.env(k, v);
    }

    if !run.model.is_empty() {
        cmd.arg("--model").arg(run.model);
    }
    cmd.args(run.extra_args);

    match run.session {
        Some((id, true)) => {
            cmd.arg("--session-id").arg(id);
        }
        Some((id, false)) => {
            cmd.arg("--resume").arg(id);
        }
        None => {
            cmd.arg("--no-session-persistence");
        }
    }

    // The file name is deterministic per tag to avoid leaking temp files on crash.
    let system_prompt_path = match run.system_prompt {
        Some(sys) => {
            let path = std::env::temp_dir().join(format!("codereview_{}.txt", run.prompt_file_tag));
            std::fs::write(&path, sys)
                .map_err(|e| AppError::Chat(format!("failed to write system prompt: {e}")))?;
            let flag = if run.replace_system_prompt {
                "--system-prompt-file"
            } else {
                "--append-system-prompt-file"
            };
            cmd.arg(flag).arg(&path);
            Some(path)
        }
        None => None,
    };
    let text = run.prompt;

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Chat(format!("failed to spawn claude: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| AppError::Chat(format!("failed to write prompt: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| AppError::Chat(format!("claude process error: {e}")))?;

    // Clean up temp system prompt file.
    if let Some(p) = system_prompt_path {
        let _ = std::fs::remove_file(p);
    }

    // Even on non-zero exit, claude may have written a JSON envelope with a
    // human-readable `result` — parse it first for a clean error message.
    let raw = String::from_utf8_lossy(&output.stdout);
    if let Ok(envelope) = serde_json::from_str::<ClaudeEnvelope>(&raw) {
        if envelope.is_error {
            let detail = envelope
                .result
                .map(|v| match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "unknown error".to_string());
            return Err(AppError::Chat(detail));
        }
    }

    if !output.status.success() || output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Chat(if stderr.is_empty() {
            format!("claude exited {}", output.status.code().unwrap_or(-1))
        } else {
            stderr
        }));
    }

    let envelope: ClaudeEnvelope = serde_json::from_str(&raw)
        .map_err(|e| AppError::Chat(format!("failed to parse claude response: {e}")))?;

    let text_out = envelope
        .result
        .map(|v| match v {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default();

    let usage = envelope.usage.unwrap_or_default();
    Ok(TurnResult {
        text: text_out,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: envelope.total_cost_usd,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_system_prompt(
    template: &str,
    title: &str,
    owner: Option<&str>,
    repo_name: Option<&str>,
    pr_number: Option<i64>,
    base_ref: &str,
    head_ref: &str,
    worktree_path: &Path,
    diff_summary: &str,
    review_id: i64,
    comments_section: &str,
) -> String {
    let repo_label = match (owner, repo_name) {
        (Some(o), Some(n)) => format!("{o}/{n}"),
        _ => worktree_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_string()),
    };

    let pr_line = match pr_number {
        Some(n) => format!("- PR #{n}: {base_ref} → {head_ref}"),
        None => format!("- Local diff: {base_ref} → {head_ref}"),
    };

    let worktree = worktree_path.display().to_string();
    let review_id = review_id.to_string();
    crate::prompts::render(
        template,
        &[
            ("title", title),
            ("repo", &repo_label),
            ("pr_line", &pr_line),
            ("diff_summary", diff_summary),
            ("head_ref", head_ref),
            ("worktree_path", &worktree),
            ("comments", comments_section),
            ("review_id", &review_id),
        ],
    )
}

fn format_review_comments(comments: &[crate::db::models::Comment], max_comments: usize) -> String {
    let roots: Vec<&crate::db::models::Comment> = comments
        .iter()
        .filter(|c| c.parent_id.is_none())
        .take(max_comments)
        .collect();

    if roots.is_empty() {
        return String::new();
    }

    let total_root = comments.iter().filter(|c| c.parent_id.is_none()).count();
    let mut out = String::from("\n## Existing review comments\n");
    let mut current_file = "";

    for c in &roots {
        if c.file_path != current_file {
            current_file = &c.file_path;
            out.push_str(&format!("\n### {}\n", current_file));
        }

        let location = if c.subject_type == "file" {
            "(file-level)".to_string()
        } else if let Some(start) = c.start_line {
            format!("L{}-{} ({})", start, c.line, c.side)
        } else {
            format!("L{} ({})", c.line, c.side)
        };

        let status = if c.resolved_at.is_some() {
            " [RESOLVED]"
        } else {
            ""
        };

        let body = if c.body.len() > 500 {
            format!("{}...", &c.body[..500])
        } else {
            c.body.clone()
        };

        out.push_str(&format!(
            "- {} (id:{}){}:  {}\n",
            location, c.id, status, body
        ));
    }

    if total_root > max_comments {
        out.push_str(&format!(
            "\n(Showing {} of {} comments)\n",
            max_comments, total_root
        ));
    }

    out
}

fn diff_file_summary(diff: &str) -> String {
    let total = diff.matches("diff --git ").count();
    let mut lines: Vec<String> = Vec::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            let path = line
                .split_whitespace()
                .nth(3)
                .unwrap_or("")
                .trim_start_matches("b/");
            lines.push(format!("- {path}"));
            if lines.len() >= 40 {
                lines.push(format!("  ... and {} more", total.saturating_sub(40)));
                break;
            }
        }
    }
    if lines.is_empty() {
        "(no files)".to_string()
    } else {
        lines.join("\n")
    }
}

/// Send a message in the review chat. Creates the chat row on first call.
/// Uses the split-lock pattern: gathers context under DB lock, drops it for
/// the slow claude CLI subprocess, then re-locks to write results.
#[tauri::command]
pub async fn chat_send(
    review_id: i64,
    text: String,
    model: Option<String>,
    db: State<'_, Db>,
) -> AppResult<ChatTurnResult> {
    // --- Phase 1: gather context under DB lock ---
    let (detail, existing_chat, diff_source, chat_template) = {
        let conn = db.0.lock().unwrap();
        let chat_template = crate::prompts::resolve(&conn, crate::prompts::CHAT_SYSTEM)?;

        let detail = crate::commands::review::load_detail(&conn, review_id)?;

        let existing_chat: Option<Chat> = conn
            .query_row(
                "SELECT * FROM chat WHERE review_id = ?1",
                params![review_id],
                Chat::from_row,
            )
            .optional()?;

        let diff_source = crate::commands::review::diff_source(&conn, review_id).ok();

        (detail, existing_chat, diff_source, chat_template)
    };
    let diff = diff_source.and_then(|s| s.load().ok()).unwrap_or_default();

    // --- Phase 2: worktree + system prompt (no DB lock) ---
    let local_path_str = detail.local_path.as_deref().ok_or_else(|| {
        crate::error::AppError::Chat(
            "Chat requires a local clone. Link one in the Repositories section.".into(),
        )
    })?;
    let repo_path = Path::new(local_path_str);

    let stored_path = existing_chat.as_ref().and_then(|c| c.worktree_path.clone());
    let worktree_path = resolve_chat_worktree(repo_path, &detail.target, stored_path.as_deref())?;

    // `claude --resume` looks sessions up by cwd, so a chat that moved to a
    // different checkout starts a fresh session there.
    let moved = stored_path
        .as_deref()
        .is_some_and(|p| Path::new(p) != worktree_path);
    let existing_session = existing_chat
        .as_ref()
        .and_then(|c| c.session_id.clone())
        .filter(|_| !moved);
    let is_first_turn = existing_session.is_none();
    let session_id = existing_session.unwrap_or_else(new_session_id);

    let model = resolve_model(model);

    let system_prompt = if is_first_turn {
        let diff_summary = diff_file_summary(&diff);
        let comments_section = format_review_comments(&detail.comments, 50);
        Some(build_system_prompt(
            &chat_template,
            &detail.target.title,
            detail.remote_owner.as_deref(),
            detail.remote_name.as_deref(),
            detail.target.github_pr_number,
            &detail.target.base_ref,
            &detail.target.head_ref,
            &worktree_path,
            &diff_summary,
            review_id,
            &comments_section,
        ))
    } else {
        None
    };

    // --- Phase 3: call claude CLI (no DB lock) ---
    let turn = call_claude_turn(
        &session_id,
        is_first_turn,
        &model,
        system_prompt.as_deref(),
        &worktree_path,
        &text,
    )?;

    // --- Phase 4: persist results under DB lock ---
    {
        let conn = db.0.lock().unwrap();
        let ts = now();

        let chat_id = match existing_chat {
            None => {
                conn.execute(
                    "INSERT INTO chat (review_id, model, session_id, total_input_tokens,
                      total_output_tokens, total_cost_usd, worktree_path, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        review_id,
                        model,
                        session_id,
                        turn.input_tokens,
                        turn.output_tokens,
                        turn.cost_usd,
                        worktree_path.to_str(),
                        ts,
                    ],
                )?;
                conn.last_insert_rowid()
            }
            Some(existing) => {
                conn.execute(
                    "UPDATE chat SET
                        session_id = ?1,
                        total_input_tokens = total_input_tokens + ?2,
                        total_output_tokens = total_output_tokens + ?3,
                        total_cost_usd = total_cost_usd + ?4,
                        worktree_path = ?5,
                        updated_at = ?6
                     WHERE id = ?7",
                    params![
                        session_id,
                        turn.input_tokens,
                        turn.output_tokens,
                        turn.cost_usd,
                        worktree_path.to_str(),
                        ts,
                        existing.id,
                    ],
                )?;
                existing.id
            }
        };

        conn.execute(
            "INSERT INTO chat_message (chat_id, role, content, created_at) VALUES (?1, 'user', ?2, ?3)",
            params![chat_id, text, ts],
        )?;
        conn.execute(
            "INSERT INTO chat_message
               (chat_id, role, content, input_tokens, output_tokens, cost_usd, created_at)
             VALUES (?1, 'assistant', ?2, ?3, ?4, ?5, ?6)",
            params![
                chat_id,
                turn.text,
                turn.input_tokens,
                turn.output_tokens,
                turn.cost_usd,
                ts,
            ],
        )?;
    }

    Ok(ChatTurnResult {
        text: turn.text,
        input_tokens: turn.input_tokens,
        output_tokens: turn.output_tokens,
        cost_usd: turn.cost_usd,
    })
}

/// Return all chat messages for a review, ordered oldest first.
#[tauri::command]
pub fn chat_messages(review_id: i64, db: State<Db>) -> AppResult<Vec<ChatMessage>> {
    let conn = db.0.lock().unwrap();
    let chat_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM chat WHERE review_id = ?1",
            params![review_id],
            |r| r.get(0),
        )
        .optional()?;

    let Some(chat_id) = chat_id else {
        return Ok(vec![]);
    };

    let mut stmt =
        conn.prepare("SELECT * FROM chat_message WHERE chat_id = ?1 ORDER BY id")?;
    let messages = stmt
        .query_map(params![chat_id], ChatMessage::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(messages)
}

/// The checkout a review's chat edits, re-checked on every call: the stored
/// path if it still exists and is on the review's branch, otherwise whatever
/// checkout holds the branch now (the main checkout included), creating a
/// worktree only when none does.
fn resolve_chat_worktree(repo: &Path, target: &Target, stored: Option<&str>) -> AppResult<PathBuf> {
    if let Some(p) = stored.map(Path::new).filter(|p| p.exists()) {
        if worktree::is_on_branch(p, &target.head_ref) {
            return Ok(p.to_path_buf());
        }
    }
    worktree::ensure_branch_worktree(repo, target)
}

/// Delete the chat (and all its messages) for a review. Its worktree is left
/// alone — it may be shared with a Claude session; the worktree janitor
/// decides when it can go.
#[tauri::command]
pub fn chat_clear(review_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM chat WHERE review_id = ?1", params![review_id])?;
    Ok(())
}

/// Resolves (creating if needed) the worktree `chat_send` would use for this
/// review, without starting a chat turn, and records it on the chat so the
/// janitor knows it's in use. Lets other UI (the "open in VSCode" buttons,
/// board terminals) point at the same checkout Claude edits.
#[tauri::command]
pub async fn ensure_chat_worktree(review_id: i64, db: State<'_, Db>) -> AppResult<String> {
    let (detail, stored) = {
        let conn = db.0.lock().unwrap();
        let detail = crate::commands::review::load_detail(&conn, review_id)?;
        let stored: Option<String> = conn
            .query_row(
                "SELECT worktree_path FROM chat WHERE review_id = ?1",
                params![review_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        (detail, stored)
    };

    let local_path_str = detail.local_path.as_deref().ok_or_else(|| {
        AppError::Chat("Chat requires a local clone. Link one in the Repositories section.".into())
    })?;
    let path = resolve_chat_worktree(Path::new(local_path_str), &detail.target, stored.as_deref())?;
    let path_str = path.to_string_lossy().into_owned();

    if stored.as_deref() != Some(path_str.as_str()) {
        let conn = db.0.lock().unwrap();
        let ts = now();
        conn.execute(
            "INSERT INTO chat (review_id, model, worktree_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(review_id) DO UPDATE SET
               worktree_path = excluded.worktree_path,
               session_id = CASE WHEN chat.worktree_path IS NULL THEN chat.session_id END,
               updated_at = excluded.updated_at",
            params![review_id, resolve_model(None), path_str, ts],
        )?;
    }
    Ok(path_str)
}

#[derive(Debug, Serialize)]
pub struct ChatWorktree {
    pub path: String,
    pub branch: Option<String>,
    /// The chat edits the repo's main checkout, not a linked worktree.
    pub is_main: bool,
}

/// The checkout recorded for the review's chat, if it still exists.
#[tauri::command]
pub fn chat_worktree(review_id: i64, db: State<Db>) -> AppResult<Option<ChatWorktree>> {
    let conn = db.0.lock().unwrap();
    let worktree_path: Option<String> = conn
        .query_row(
            "SELECT worktree_path FROM chat WHERE review_id = ?1",
            params![review_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    drop(conn);

    let Some(worktree_path) = worktree_path else {
        return Ok(None);
    };
    let path = Path::new(&worktree_path);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(ChatWorktree {
        branch: worktree::current_branch(path),
        is_main: worktree::is_main_checkout(path),
        path: worktree_path,
    }))
}

/// Persist the chat panel collapsed/expanded state. UI state — allowed on all reviews.
#[tauri::command]
pub fn set_chat_collapsed(review_id: i64, collapsed: bool, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE review SET chat_collapsed = ?1 WHERE id = ?2",
        params![collapsed as i64, review_id],
    )?;
    Ok(())
}
