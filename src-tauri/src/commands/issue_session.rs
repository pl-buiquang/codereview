//! Agent session opened from an issue card: Claude in the repository's main clone, with the card's
//! metadata (notes, files, links, the linked plan) appended to its system prompt. This is the
//! refine step between the Plan and Implement routines.

use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::Connection;
use tauri::State;

use crate::commands::board::{board_meta_add_impl, board_meta_list_impl, card_row};
use crate::commands::chat::new_session_id;
use crate::commands::repo::expand_home;
use crate::commands::sessions::resumable_card_session;
use crate::db::models::BoardMetaRow;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::routines::{context::repo_local_path, manager::routine_settings};
use crate::{claude_sessions, prompts, terminal};

/// Newest `file` meta tagged `plan`: the plan the Plan routine (or the user) linked.
fn linked_plan(meta: &[BoardMetaRow]) -> Option<&BoardMetaRow> {
    meta.iter().rev().find(|m| m.kind == "file" && m.tag.as_deref() == Some("plan"))
}

/// The frontmatter `status:` of a ticket file, if it has one.
fn plan_status(text: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    let front = body.split("\n---").next()?;
    front.lines().find_map(|l| l.strip_prefix("status:")).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// One line per note / file / link; recorded Claude sessions are bookkeeping, not context.
fn render_meta(meta: &[BoardMetaRow]) -> String {
    let lines: Vec<String> = meta
        .iter()
        .filter(|m| m.kind != "claude_session")
        .map(|m| {
            let tag = m.tag.as_deref().filter(|t| !t.is_empty()).map(|t| format!(" [{t}]")).unwrap_or_default();
            let label = m.label.as_deref().filter(|l| !l.is_empty()).map(|l| format!(" {l}:")).unwrap_or_default();
            let value = if m.value.is_empty() { "(tag only)" } else { m.value.as_str() };
            format!("- {}{tag}{label} {value}", m.kind)
        })
        .collect();
    if lines.is_empty() {
        "(no notes, files or links yet)".to_string()
    } else {
        lines.join("\n")
    }
}

/// The system prompt appended to the session, from the (overridable) `issue.session` template.
pub(crate) fn render_context(conn: &Connection, card_id: &str) -> AppResult<String> {
    let card = card_row(conn, card_id)?;
    if card.typ != "issue" {
        return Err(AppError::Other("only issue cards have an issue session".into()));
    }
    let meta = board_meta_list_impl(conn, card_id)?;
    let plan = linked_plan(&meta);
    let status = plan
        .and_then(|p| std::fs::read_to_string(expand_home(&p.value)).ok())
        .and_then(|t| plan_status(&t))
        .unwrap_or_else(|| "unknown".into());
    let state = if card.closed_at.is_some() { "closed" } else { "open" };
    let number = card.number.to_string();
    let plans_dir = expand_home(&routine_settings(conn).plans_dir);
    let plan_path = plan.map(|p| p.value.clone()).unwrap_or_else(|| "none linked yet".into());
    let meta_text = render_meta(&meta);
    let template = prompts::resolve(conn, prompts::ISSUE_SESSION)?;
    Ok(prompts::render(
        &template,
        &[
            ("repo", &card.repo),
            ("number", &number),
            ("url", &card.url),
            ("title", &card.title),
            ("state", state),
            ("plans_dir", &plans_dir),
            ("plan_path", &plan_path),
            ("plan_status", &status),
            ("meta", &meta_text),
        ],
    ))
}

/// Whether Claude still has a transcript for this session id (an id recorded at launch has none
/// until the first message, and `--resume` fails on it).
fn transcript_exists(session_id: &str) -> bool {
    let Some(projects) = dirs::home_dir().map(|h| h.join(".claude/projects")) else { return false };
    std::fs::read_dir(projects)
        .map(|dirs| dirs.flatten().any(|d| d.path().join(format!("{session_id}.jsonl")).is_file()))
        .unwrap_or(false)
}

/// Open a tmux window in the issue's main clone running Claude with the card's context, resuming
/// the card's latest recorded session when it has a transcript and isn't already live.
#[tauri::command]
pub async fn board_open_issue_agent(
    card_id: String,
    host: terminal::TerminalHost,
    tmux_target: terminal::TmuxTarget,
    tmux_session: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    let live: HashSet<String> = claude_sessions::live_sessions().into_iter().map(|s| s.session_id).collect();
    let (card, local_path, context, resume) = {
        let conn = db.0.lock().unwrap();
        let card = card_row(&conn, &card_id)?;
        let local_path = repo_local_path(&conn, &card.repo)
            .filter(|p| std::path::Path::new(&expand_home(p)).is_dir())
            .map(|p| expand_home(&p))
            .ok_or_else(|| AppError::Other(format!("{} has no local clone — link one in Repositories", card.repo)))?;
        let context = render_context(&conn, &card_id)?;
        let resume = resumable_card_session(&conn, &card_id, &live)?.filter(|id| transcript_exists(id));
        (card, local_path, context, resume)
    };

    let context_file: PathBuf = std::env::temp_dir().join(format!("codereview_issue_session_{}.md", card.number));
    std::fs::write(&context_file, context)?;

    let mut argv = vec!["claude".to_string()];
    match resume {
        Some(id) => argv.extend(["--resume".to_string(), id]),
        None => {
            // Pin the id so the session can be recorded on the card: the SessionStart hook resolves
            // cards from the checked-out branch, which the main clone never matches.
            let id = new_session_id();
            board_meta_add_impl(&db.0.lock().unwrap(), &card_id, "claude_session", &id, None, None, Some(&local_path))?;
            argv.extend(["--session-id".to_string(), id]);
        }
    }
    argv.extend(["--append-system-prompt-file".to_string(), context_file.to_string_lossy().into_owned()]);
    terminal::open_session(host, tmux_target, &tmux_session, &card.repo, &format!("#{}", card.number), &local_path, &argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use rusqlite::params;

    fn meta(kind: &str, value: &str, tag: Option<&str>) -> BoardMetaRow {
        BoardMetaRow {
            id: 0,
            card_id: "c".into(),
            kind: kind.into(),
            value: value.into(),
            tag: tag.map(Into::into),
            label: None,
            cwd: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn reads_the_status_from_frontmatter() {
        assert_eq!(plan_status("---\ntype: ticket\nstatus: ready-for-agent\n---\n# x\nstatus: no").as_deref(), Some("ready-for-agent"));
        assert_eq!(plan_status("# no frontmatter\nstatus: done"), None);
    }

    #[test]
    fn the_newest_plan_wins_and_sessions_stay_out_of_the_context() {
        let items = vec![
            meta("file", "/old.md", Some("plan")),
            meta("claude_session", "abc", None),
            meta("note", "careful with the migration", Some("risk")),
            meta("file", "/new.md", Some("plan")),
        ];
        assert_eq!(linked_plan(&items).unwrap().value, "/new.md");
        let text = render_meta(&items);
        assert!(text.contains("- note [risk] careful with the migration"));
        assert!(!text.contains("abc"));
        assert_eq!(render_meta(&[]), "(no notes, files or links yet)");
    }

    fn seed_issue(conn: &Connection) {
        let repo_id = crate::mirror::store::ensure_repo(conn, "o", "r", "now").unwrap();
        conn.execute(
            "INSERT INTO gh_issue (node_id, repo_id, number, title, url, gh_updated_at, fetched_at)
             VALUES ('I_1', ?1, 7, 'Fix the thing', 'https://github.com/o/r/issues/7', 'now', 'now')",
            params![repo_id],
        )
        .unwrap();
        conn.execute("INSERT INTO board_card (id, type, tracked_at) VALUES ('I_1', 'issue', 'now')", []).unwrap();
    }

    #[test]
    fn renders_the_card_and_its_plan_into_the_template() {
        let conn = open_memory();
        seed_issue(&conn);
        let dir = std::env::temp_dir().join(format!("codereview_issue_session_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plan = dir.join("plan.md");
        std::fs::write(&plan, "---\nstatus: needs-info\n---\n# Plan\n").unwrap();
        board_meta_add_impl(&conn, "I_1", "file", &plan.to_string_lossy(), Some("plan"), Some("Plan"), None).unwrap();
        board_meta_add_impl(&conn, "I_1", "note", "waiting on design", None, None, None).unwrap();

        let text = render_context(&conn, "I_1").unwrap();
        assert!(text.contains("https://github.com/o/r/issues/7 (Fix the thing), state open, repo o/r"));
        assert!(text.contains(&format!("Plan file: {} (status: needs-info)", plan.display())));
        assert!(text.contains("- note waiting on design"));
        assert!(!text.contains("{{"), "every placeholder is filled");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_pr_card_has_no_issue_session() {
        let conn = open_memory();
        let repo_id = crate::mirror::store::ensure_repo(&conn, "o", "r", "now").unwrap();
        conn.execute(
            "INSERT INTO gh_pr (node_id, repo_id, number, title, url, gh_updated_at, fetched_at)
             VALUES ('PR_1', ?1, 8, 't', 'https://example.com', 'now', 'now')",
            params![repo_id],
        )
        .unwrap();
        conn.execute("INSERT INTO board_card (id, type, tracked_at) VALUES ('PR_1', 'pr', 'now')", []).unwrap();
        assert!(render_context(&conn, "PR_1").is_err());
    }
}
