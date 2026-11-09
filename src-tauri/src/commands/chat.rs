use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::ai::profiles::{self, Feature, Resolved};
use crate::ai::{Access, OneShot, SessionMode, SystemPrompt, SystemPromptMode, ToolPolicy, TurnResult};
use crate::ai_usage::RunTag;
use crate::db::models::{Chat, ChatMessage, Target};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::worktree;

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Generate a UUID v4 without an external crate. Uses microsecond timestamp +
/// an atomic counter + process id for uniqueness.
pub(crate) fn new_session_id() -> String {
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

/// Response returned to the frontend after a chat turn.
#[derive(Serialize)]
pub struct ChatTurnResult {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Run one conversation turn on the chat's harness. On the first turn (no session) establishes a
/// new session; on subsequent turns resumes it. The system prompt is only sent on the first turn.
#[allow(clippy::too_many_arguments)]
fn call_turn(
    resolved: &Resolved,
    review_id: i64,
    session_id: &str,
    is_first_turn: bool,
    model: &str,
    system_prompt: Option<&str>,
    worktree_path: &Path,
    text: &str,
) -> AppResult<TurnResult> {
    let file_tag = format!("chat_{session_id}");
    let mut req = turn_request(review_id, session_id, &file_tag, is_first_turn, model, system_prompt, worktree_path, text);
    req.usage_tag = req.usage_tag.harness(resolved.harness.id());
    req.extra_args.extend(resolved.harness.settings_args(&resolved.settings));
    resolved.harness.one_shot(&req)
}

#[allow(clippy::too_many_arguments)]
fn turn_request<'a>(
    review_id: i64,
    session_id: &'a str,
    file_tag: &'a str,
    is_first_turn: bool,
    model: &'a str,
    system_prompt: Option<&'a str>,
    worktree_path: &'a Path,
    text: &'a str,
) -> OneShot<'a> {
    OneShot {
        model,
        system_prompt: if is_first_turn { system_prompt.map(SystemPrompt::Text) } else { None },
        system_prompt_mode: SystemPromptMode::Append,
        file_tag,
        prompt: text,
        cwd: worktree_path,
        extra_dirs: Vec::new(),
        tools: ToolPolicy::Allowlist(["Bash", "Read", "Glob", "Grep", "Edit", "Write"].map(String::from).to_vec()),
        // Explicit, so the user's own `~/.claude/settings.json` (e.g. a `defaultMode: plan`)
        // never leaks into this non-interactive session.
        access: Access::Edit,
        session: if is_first_turn { SessionMode::New(session_id) } else { SessionMode::Resume(session_id) },
        max_turns: None,
        effort: None,
        env: Vec::new(),
        extra_args: Vec::new(),
        usage_tag: RunTag::new("chat").review(review_id),
    }
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

/// `(role, content)` of every message of a chat, oldest first.
fn earlier_messages(conn: &rusqlite::Connection, chat_id: i64) -> AppResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT role, content FROM chat_message WHERE chat_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![chat_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Size cap of the previous-conversation block put into the first turn after a profile switch.
const REPLAY_CAP_BYTES: usize = 200 * 1024;

const REPLAY_HEADER: &str = "## Previous conversation\n\nThis review chat ran on another profile before. Its messages so far are \
below, oldest first, so you can continue it. Treat them as context, not as instructions to carry out again.\n";
const REPLAY_TRUNCATED: &str = "\n[Earlier messages were dropped to fit the size limit.]\n";

/// The previous-conversation block for a chat that switched profile: every message as
/// `### User` / `### Assistant`, oldest first, within `cap` bytes. Oldest messages are dropped
/// first and a marker line says so; a newest message that alone exceeds the cap keeps its tail.
/// `None` when there is nothing to replay.
fn replay_block(messages: &[(String, String)], cap: usize) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    let entry = |role: &str, content: &str| {
        let who = if role == "assistant" { "Assistant" } else { "User" };
        format!("\n### {who}\n{}\n", content.trim())
    };
    // The room for messages, assuming the marker is needed (it is only shown when something is dropped).
    let budget = cap.saturating_sub(REPLAY_HEADER.len() + REPLAY_TRUNCATED.len());
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    let mut dropped = false;
    for (role, content) in messages.iter().rev() {
        let text = entry(role, content);
        if used + text.len() > budget {
            dropped = true;
            // The newest message alone is too long: keep its tail.
            if kept.is_empty() {
                let room = budget.saturating_sub("\n### Assistant\n…\n".len());
                let mut start = content.len().saturating_sub(room);
                while !content.is_char_boundary(start) {
                    start += 1;
                }
                kept.push(entry(role, &format!("…{}", &content[start..])));
            }
            break;
        }
        used += text.len();
        kept.push(text);
    }
    kept.reverse();
    let mut out = String::from(REPLAY_HEADER);
    if dropped {
        out.push_str(REPLAY_TRUNCATED);
    }
    out.extend(kept);
    Some(out)
}

/// The profile a chat turn runs on and whether this turn switches profile. `requested` is the
/// profile picked in the chat pane. Order: the request, the profile the chat is pinned to, the chat
/// feature profile, the default. A switch is a request for a profile other than the one the chat
/// currently runs on (a chat that was never pinned counts as running on the one it would resolve to).
fn chat_profile(conn: &rusqlite::Connection, chat: Option<&Chat>, requested: Option<i64>) -> (Resolved, bool) {
    let feature = profiles::feature_profiles(conn).chat;
    let pinned = chat.and_then(|c| c.profile_id);
    let resolved = profiles::resolve(conn, &[requested, pinned, feature]);
    let current = profiles::resolve(conn, &[pinned, feature]).profile_id;
    let has_session = chat.is_some_and(|c| c.session_id.is_some());
    let switched = has_session && requested.is_some() && resolved.profile_id != current;
    // A session only resumes in the harness that started it: a pinned profile that was deleted, or a
    // changed default, can leave the chat on another harness without anyone having switched.
    let other_harness = chat
        .filter(|c| c.session_id.is_some())
        .is_some_and(|c| c.session_harness.as_deref().unwrap_or("claude") != resolved.harness.id());
    (resolved, switched || other_harness)
}

/// The profile the chat pane shows: the one the review's chat runs on.
#[tauri::command]
pub fn chat_profile_id(review_id: i64, db: State<Db>) -> AppResult<Option<i64>> {
    let conn = db.0.lock().unwrap();
    let chat: Option<Chat> = conn
        .query_row("SELECT * FROM chat WHERE review_id = ?1", params![review_id], Chat::from_row)
        .optional()?;
    Ok(chat_profile(&conn, chat.as_ref(), None).0.profile_id)
}

/// Send a message in the review chat. Creates the chat row on first call.
/// Uses the split-lock pattern: gathers context under DB lock, drops it for
/// the slow claude CLI subprocess, then re-locks to write results.
#[tauri::command]
pub async fn chat_send(
    review_id: i64,
    text: String,
    profile_id: Option<i64>,
    db: State<'_, Db>,
) -> AppResult<ChatTurnResult> {
    crate::ai_gate::ensure_enabled()?;
    // --- Phase 1: gather context under DB lock ---
    let (detail, existing_chat, diff_source, chat_template, resolved, switched, replay) = {
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

        let (resolved, switched) = chat_profile(&conn, existing_chat.as_ref(), profile_id);
        // A switch starts a new session, so the conversation so far goes into its first prompt.
        let replay = match (&existing_chat, switched) {
            (Some(chat), true) => replay_block(&earlier_messages(&conn, chat.id)?, REPLAY_CAP_BYTES),
            _ => None,
        };

        (detail, existing_chat, diff_source, chat_template, resolved, switched, replay)
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
    let worktree_path = resolve_chat_worktree(repo_path, &detail.target, detail.review.head_sha.as_deref(), stored_path.as_deref())?;

    // `claude --resume` looks sessions up by cwd, so a chat that moved to a
    // different checkout starts a fresh session there.
    let moved = stored_path
        .as_deref()
        .is_some_and(|p| Path::new(p) != worktree_path);
    // A profile switch starts a fresh session on the new profile.
    let existing_session = existing_chat
        .as_ref()
        .and_then(|c| c.session_id.clone())
        .filter(|_| !moved && !switched);
    let is_first_turn = existing_session.is_none();
    let session_id = existing_session.unwrap_or_else(new_session_id);

    let model = resolved.model_for("");

    let system_prompt = if is_first_turn {
        let diff_summary = diff_file_summary(&diff);
        let comments_section = format_review_comments(&detail.comments, 50);
        let mut prompt = build_system_prompt(
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
        );
        if let Some(replay) = &replay {
            prompt.push_str("\n\n");
            prompt.push_str(replay);
        }
        Some(prompt)
    } else {
        None
    };

    // --- Phase 3: call claude CLI (no DB lock) ---
    let turn = call_turn(
        &resolved,
        review_id,
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
                      total_output_tokens, total_cost_usd, worktree_path, profile_id, session_harness, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?10, ?9, ?9)",
                    params![
                        review_id,
                        model,
                        session_id,
                        turn.input_tokens,
                        turn.output_tokens,
                        turn.cost_usd,
                        worktree_path.to_str(),
                        resolved.profile_id,
                        ts,
                        resolved.harness.id(),
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
                        profile_id = ?6,
                        model = ?7,
                        updated_at = ?8,
                        session_harness = ?10
                     WHERE id = ?9",
                    params![
                        session_id,
                        turn.input_tokens,
                        turn.output_tokens,
                        turn.cost_usd,
                        worktree_path.to_str(),
                        resolved.profile_id,
                        model,
                        ts,
                        existing.id,
                        resolved.harness.id(),
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
fn resolve_chat_worktree(
    repo: &Path,
    target: &Target,
    head_sha: Option<&str>,
    stored: Option<&str>,
) -> AppResult<PathBuf> {
    if let Some(p) = stored.map(Path::new).filter(|p| p.exists()) {
        if worktree::is_on_branch(p, &target.head_ref) {
            return Ok(p.to_path_buf());
        }
    }
    worktree::ensure_branch_worktree(repo, target, head_sha)
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
    let path = resolve_chat_worktree(
        Path::new(local_path_str),
        &detail.target,
        detail.review.head_sha.as_deref(),
        stored.as_deref(),
    )?;
    let path_str = path.to_string_lossy().into_owned();

    if stored.as_deref() != Some(path_str.as_str()) {
        let conn = db.0.lock().unwrap();
        let model = profiles::resolve_feature(&conn, Feature::Chat).model_for("");
        let ts = now();
        conn.execute(
            "INSERT INTO chat (review_id, model, worktree_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(review_id) DO UPDATE SET
               worktree_path = excluded.worktree_path,
               session_id = CASE WHEN chat.worktree_path IS NULL THEN chat.session_id END,
               updated_at = excluded.updated_at",
            params![review_id, model, path_str, ts],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::claude;
    use crate::test_support::{argv, envs, strs};

    const WORKTREE: &str = "/w/tree";

    #[test]
    fn first_turn_pins_the_session_and_appends_the_system_prompt() {
        let wt = Path::new(WORKTREE);
        let req = turn_request(7, "sid", "chat_sid", true, "opus", Some("sys"), wt, "hello");
        let prompt = Path::new("/tmp/codereview_chat_sid.txt");
        let cmd = claude::command(&req, false, Some(prompt));
        assert_eq!(
            argv(&cmd),
            strs(&[
                "-p", "--output-format", "json", "--strict-mcp-config", "--permission-mode", "acceptEdits",
                "--add-dir", WORKTREE, "--allowedTools", "Bash", "Read", "Glob", "Grep", "Edit", "Write",
                "--model", "opus", "--session-id", "sid", "--append-system-prompt-file",
                "/tmp/codereview_chat_sid.txt",
            ])
        );
        assert!(envs(&cmd).is_empty());
        assert_eq!(cmd.get_current_dir(), Some(wt));
    }

    #[test]
    fn next_turn_resumes_and_sends_no_system_prompt() {
        let wt = Path::new(WORKTREE);
        let req = turn_request(7, "sid", "chat_sid", false, "", Some("sys"), wt, "again");
        assert!(req.system_prompt.is_none());
        let cmd = claude::command(&req, false, None);
        assert_eq!(
            argv(&cmd),
            strs(&[
                "-p", "--output-format", "json", "--strict-mcp-config", "--permission-mode", "acceptEdits",
                "--add-dir", WORKTREE, "--allowedTools", "Bash", "Read", "Glob", "Grep", "Edit", "Write",
                "--resume", "sid",
            ])
        );
        assert!(envs(&cmd).is_empty());
    }

    #[test]
    fn extra_dirs_and_replace_mode_are_honoured() {
        let wt = Path::new(WORKTREE);
        let mut req = turn_request(1, "s", "t", true, "m", Some("x"), wt, "p");
        req.extra_dirs = vec![Path::new("/other")];
        req.system_prompt_mode = SystemPromptMode::Replace;
        let cmd = claude::command(&req, false, Some(Path::new("/p.txt")));
        let a = argv(&cmd);
        assert_eq!(&a[6..10], &strs(&["--add-dir", WORKTREE, "--add-dir", "/other"])[..]);
        assert_eq!(&a[a.len() - 2..], &strs(&["--system-prompt-file", "/p.txt"])[..]);
    }

    fn chat_row(conn: &rusqlite::Connection, session: Option<&str>, profile: Option<i64>) -> Chat {
        conn.execute("INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'n')", []).unwrap();
        conn.execute("INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at) VALUES (1, 'local', 't', 'a', 'b', 'n')", []).unwrap();
        conn.execute("INSERT INTO review (target_id, body, status, created_at, updated_at) VALUES (1, '', 'draft', 'n', 'n')", []).unwrap();
        conn.execute(
            "INSERT INTO chat (review_id, model, session_id, profile_id, created_at, updated_at) VALUES (1, '', ?1, ?2, 'n', 'n')",
            params![session, profile],
        )
        .unwrap();
        conn.query_row("SELECT * FROM chat", [], Chat::from_row).unwrap()
    }

    fn profile(conn: &mut rusqlite::Connection, name: &str) -> i64 {
        let input = profiles::AiProfileInput {
            id: None,
            name: name.into(),
            harness: "claude".into(),
            model: String::new(),
            settings: serde_json::json!({}),
            host_id: None,
            is_default: false,
        };
        profiles::save(conn, input).unwrap().id
    }

    #[test]
    fn a_new_chat_runs_on_the_chat_feature_profile_else_the_default() {
        let mut conn = crate::db::open_memory();
        let default = profiles::default_profile(&conn).unwrap().unwrap().id;
        assert_eq!(chat_profile(&conn, None, None).0.profile_id, Some(default));
        let mapped = profile(&mut conn, "Chat");
        profiles::set_feature_profiles(&mut conn, &profiles::FeatureProfiles { chat: Some(mapped), ..Default::default() }).unwrap();
        let (r, switched) = chat_profile(&conn, None, None);
        assert_eq!((r.profile_id, switched), (Some(mapped), false));
    }

    #[test]
    fn a_pinned_chat_keeps_its_profile_when_the_default_changes() {
        let mut conn = crate::db::open_memory();
        let pinned = profile(&mut conn, "Pinned");
        let chat = chat_row(&conn, Some("sid"), Some(pinned));
        let other = profile(&mut conn, "Other");
        profiles::save(
            &mut conn,
            profiles::AiProfileInput {
                id: Some(other),
                name: "Other".into(),
                harness: "claude".into(),
                model: String::new(),
                settings: serde_json::json!({}),
                host_id: None,
                is_default: true,
            },
        )
        .unwrap();
        let (r, switched) = chat_profile(&conn, Some(&chat), None);
        assert_eq!((r.profile_id, switched), (Some(pinned), false));
    }

    #[test]
    fn requesting_another_profile_switches_only_a_chat_that_has_a_session() {
        let mut conn = crate::db::open_memory();
        let a = profile(&mut conn, "A");
        let b = profile(&mut conn, "B");
        let chat = chat_row(&conn, Some("sid"), Some(a));
        let (r, switched) = chat_profile(&conn, Some(&chat), Some(b));
        assert_eq!((r.profile_id, switched), (Some(b), true));
        // The same profile again is not a switch.
        assert!(!chat_profile(&conn, Some(&chat), Some(a)).1);

        conn.execute("UPDATE chat SET session_id = NULL", []).unwrap();
        let fresh: Chat = conn.query_row("SELECT * FROM chat", [], Chat::from_row).unwrap();
        assert!(!chat_profile(&conn, Some(&fresh), Some(b)).1, "nothing to restart");
    }

    #[test]
    fn a_chat_whose_profile_now_resolves_to_another_harness_starts_a_new_session() {
        let mut conn = crate::db::open_memory();
        let oc = profiles::save(
            &mut conn,
            profiles::AiProfileInput {
                id: None,
                name: "OC".into(),
                harness: "opencode".into(),
                model: String::new(),
                settings: serde_json::json!({}),
                host_id: None,
                is_default: true,
            },
        )
        .unwrap();
        // started on Claude (no recorded harness), now resolving to opencode (e.g. the pinned profile was deleted)
        let chat = chat_row(&conn, Some("sid"), None);
        let (r, switched) = chat_profile(&conn, Some(&chat), None);
        assert_eq!((r.profile_id, switched), (Some(oc.id), true));
        // the same chat recorded as started on opencode resumes
        conn.execute("UPDATE chat SET session_harness = 'opencode'", []).unwrap();
        let chat: Chat = conn.query_row("SELECT * FROM chat", [], Chat::from_row).unwrap();
        assert!(!chat_profile(&conn, Some(&chat), None).1);
        // nothing to restart without a session
        conn.execute("UPDATE chat SET session_id = NULL, session_harness = NULL", []).unwrap();
        let chat: Chat = conn.query_row("SELECT * FROM chat", [], Chat::from_row).unwrap();
        assert!(!chat_profile(&conn, Some(&chat), None).1);
    }

    #[test]
    fn requesting_the_profile_an_unpinned_chat_already_runs_on_is_not_a_switch() {
        let mut conn = crate::db::open_memory();
        let default = profiles::default_profile(&conn).unwrap().unwrap().id;
        let chat = chat_row(&conn, Some("sid"), None);
        assert!(!chat_profile(&conn, Some(&chat), Some(default)).1);
        let _ = &mut conn;
    }

    fn msgs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(r, c)| (r.to_string(), c.to_string())).collect()
    }

    #[test]
    fn replay_is_empty_without_messages() {
        assert_eq!(replay_block(&[], REPLAY_CAP_BYTES), None);
    }

    #[test]
    fn replay_lists_messages_oldest_first_with_their_roles() {
        let block = replay_block(&msgs(&[("user", "first q"), ("assistant", "first a"), ("user", "second q")]), REPLAY_CAP_BYTES).unwrap();
        assert!(block.starts_with("## Previous conversation"));
        let q1 = block.find("### User\nfirst q").unwrap();
        let a1 = block.find("### Assistant\nfirst a").unwrap();
        let q2 = block.find("### User\nsecond q").unwrap();
        assert!(q1 < a1 && a1 < q2);
        assert!(!block.contains("dropped"), "nothing dropped, no marker");
    }

    #[test]
    fn replay_drops_the_oldest_messages_first_and_says_so() {
        let big = "x".repeat(400);
        let messages = msgs(&[("user", "OLDEST"), ("assistant", &big), ("user", &big), ("assistant", "NEWEST")]);
        let cap = REPLAY_HEADER.len() + REPLAY_TRUNCATED.len() + 2 * 420;
        let block = replay_block(&messages, cap).unwrap();
        assert!(block.len() <= cap, "{} > {cap}", block.len());
        assert!(block.contains("Earlier messages were dropped"));
        assert!(!block.contains("OLDEST"));
        assert!(block.contains("NEWEST"));
        // What is kept stays in order: the marker comes before every kept message.
        assert!(block.find("dropped").unwrap() < block.find("NEWEST").unwrap());
    }

    #[test]
    fn replay_never_exceeds_the_cap_even_for_one_huge_message() {
        let huge = format!("{}END", "é".repeat(300_000));
        let block = replay_block(&msgs(&[("user", "old"), ("assistant", &huge)]), REPLAY_CAP_BYTES).unwrap();
        assert!(block.len() <= REPLAY_CAP_BYTES, "{}", block.len());
        assert!(block.contains("Earlier messages were dropped"));
        assert!(block.contains("END"), "the tail of the newest message is kept");
        assert!(!block.contains("### User\nold"));
    }

    #[test]
    fn earlier_messages_come_back_in_conversation_order() {
        let conn = crate::db::open_memory();
        let c = chat_row(&conn, Some("sid"), None);
        for (role, content) in [("user", "a"), ("assistant", "b"), ("user", "c")] {
            conn.execute("INSERT INTO chat_message (chat_id, role, content, created_at) VALUES (?1, ?2, ?3, 'n')", params![c.id, role, content]).unwrap();
        }
        assert_eq!(earlier_messages(&conn, c.id).unwrap(), msgs(&[("user", "a"), ("assistant", "b"), ("user", "c")]));
    }
}
