use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::models::{Chat, ChatMessage};
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

struct TurnResult {
    text: String,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
}

/// Response returned to the frontend after a chat turn.
#[derive(Serialize)]
pub struct ChatTurnResult {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
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
    let mut cmd = Command::new(crate::tools::claude_bin());
    cmd.arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--setting-sources")
        .arg("")
        .arg("--strict-mcp-config")
        .arg("--allowedTools")
        .arg("Bash Read Glob Grep")
        .arg("--add-dir")
        .arg(worktree_path)
        .arg("--model")
        .arg(model);

    if is_first_turn {
        cmd.arg("--session-id").arg(session_id);
    } else {
        cmd.arg("--resume").arg(session_id);
    }

    // Write system prompt to a temp file (only on first turn). The file name
    // is deterministic per session to avoid leaking temp files on crash.
    let system_prompt_path = if is_first_turn {
        if let Some(sys) = system_prompt {
            let path = std::env::temp_dir().join(format!("codereview_chat_{session_id}.txt"));
            std::fs::write(&path, sys)
                .map_err(|e| AppError::Chat(format!("failed to write system prompt: {e}")))?;
            cmd.arg("--append-system-prompt-file").arg(&path);
            Some(path)
        } else {
            None
        }
    } else {
        None
    };

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

fn build_system_prompt(
    title: &str,
    owner: Option<&str>,
    repo_name: Option<&str>,
    pr_number: Option<i64>,
    base_ref: &str,
    head_ref: &str,
    worktree_path: &Path,
    diff_summary: &str,
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

    format!(
        "You are a code review assistant for: {title}

## Context
- Repository: {repo_label}
{pr_line}

## Changed files
{diff_summary}

## Capabilities
You have full access to the repository at the reviewed revision.
Code is checked out at: {worktree_path}
- Use Bash to run git commands (git log, git show, git blame, git diff, etc.)
- Use Read to view specific files
- Use Grep to search across the codebase
- Use Glob to discover file structure

## Guidelines
- Focus on understanding the code changes and their implications
- When asked about specific lines, read the actual file for full context
- Use git blame/log to understand the history of changed code when relevant
- Reference files by their repo-relative path
- Be concise but thorough",
        worktree_path = worktree_path.display(),
    )
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
    db: State<'_, Db>,
) -> AppResult<ChatTurnResult> {
    // --- Phase 1: gather context under DB lock ---
    let (detail, existing_chat, diff) = {
        let conn = db.0.lock().unwrap();

        let detail = crate::commands::review::load_detail(&conn, review_id)?;

        let existing_chat: Option<Chat> = conn
            .query_row(
                "SELECT * FROM chat WHERE review_id = ?1",
                params![review_id],
                Chat::from_row,
            )
            .optional()?;

        let diff = crate::commands::review::review_diff_impl(&conn, review_id)
            .unwrap_or_default();

        (detail, existing_chat, diff)
    };

    // --- Phase 2: worktree + system prompt (no DB lock) ---
    let repo_path = Path::new(&detail.repo_path);
    let head_sha = detail
        .target
        .head_sha
        .as_deref()
        .unwrap_or(&detail.target.head_ref);

    let worktree_path = worktree::ensure_worktree(repo_path, head_sha)?;

    let is_first_turn = existing_chat
        .as_ref()
        .and_then(|c| c.session_id.as_deref())
        .is_none();

    let session_id = existing_chat
        .as_ref()
        .and_then(|c| c.session_id.clone())
        .unwrap_or_else(new_session_id);

    let model = "claude-sonnet-4-5".to_string();

    let system_prompt = if is_first_turn {
        let diff_summary = diff_file_summary(&diff);
        Some(build_system_prompt(
            &detail.target.title,
            detail.remote_owner.as_deref(),
            detail.remote_name.as_deref(),
            detail.target.github_pr_number,
            &detail.target.base_ref,
            &detail.target.head_ref,
            &worktree_path,
            &diff_summary,
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
                        session_id = COALESCE(session_id, ?1),
                        total_input_tokens = total_input_tokens + ?2,
                        total_output_tokens = total_output_tokens + ?3,
                        total_cost_usd = total_cost_usd + ?4,
                        worktree_path = COALESCE(worktree_path, ?5),
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

/// Delete the chat (and all its messages) for a review.
#[tauri::command]
pub fn chat_clear(review_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM chat WHERE review_id = ?1", params![review_id])?;
    Ok(())
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
