use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::State;

use crate::claude_sessions::{self, ClaudeSession};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::terminal;

#[tauri::command]
pub async fn list_claude_sessions(history_days: u32) -> AppResult<Vec<ClaudeSession>> {
    let agents = claude_sessions::agents();
    let (mut sessions, mut exclude) = claude_sessions::live_sessions_and_hidden(&agents);
    exclude.extend(sessions.iter().map(|s| s.session_id.clone()));
    sessions.extend(claude_sessions::history_sessions(history_days, &exclude));
    Ok(sessions)
}

#[tauri::command]
pub async fn focus_claude_session(session_id: String) -> AppResult<()> {
    let session = claude_sessions::live_sessions()
        .into_iter()
        .find(|s| s.session_id == session_id)
        .ok_or_else(|| AppError::Other("this session is not running".into()))?;
    terminal::focus_session(&session)
}

/// Stop a background session via `claude stop <job id>`; its conversation is kept.
#[tauri::command]
pub async fn stop_claude_session(session_id: String) -> AppResult<()> {
    let job_id = claude_sessions::live_sessions()
        .into_iter()
        .find(|s| s.session_id == session_id)
        .and_then(|s| s.job_id)
        .ok_or_else(|| AppError::Other("not a running background session".into()))?;
    let out = std::process::Command::new(crate::tools::claude_bin())
        .args(["stop", &job_id])
        .output()
        .map_err(|e| AppError::Other(format!("failed to run claude stop: {e}")))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        let msg = if msg.trim().is_empty() { String::from_utf8_lossy(&out.stdout) } else { msg };
        return Err(AppError::Other(format!("claude stop: {}", msg.trim())));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct CardSession {
    pub card_id: String,
    pub repo: String,
    pub number: i64,
    pub session_id: String,
}

type SessionIdsByCard = HashMap<String, HashSet<String>>;

struct PrCard {
    id: String,
    repo: String,
    number: i64,
    head_ref: Option<String>,
}

fn load_pr_cards(conn: &Connection) -> AppResult<(Vec<PrCard>, SessionIdsByCard)> {
    let mut stmt = conn.prepare(
        "SELECT id, repo, number, head_ref FROM board_card
         WHERE type = 'pr' AND archived_at IS NULL AND discarded_at IS NULL",
    )?;
    let cards = stmt
        .query_map([], |r| Ok(PrCard { id: r.get(0)?, repo: r.get(1)?, number: r.get(2)?, head_ref: r.get(3)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut meta = SessionIdsByCard::new();
    let mut stmt = conn.prepare("SELECT card_id, value FROM board_meta WHERE kind = 'claude_session'")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (card, value) = row?;
        meta.entry(card).or_default().insert(value);
    }
    Ok((cards, meta))
}

/// A live session belongs to a PR card when the card records it as `claude_session` metadata,
/// or when it runs in a checkout of the card's head branch. Sessions arrive most-recent first,
/// and that order is preserved per card.
fn match_card_sessions(
    cards: &[PrCard],
    meta: &SessionIdsByCard,
    sessions: &[ClaudeSession],
) -> Vec<CardSession> {
    let mut out = Vec::new();
    for card in cards {
        let recorded = meta.get(&card.id);
        for s in sessions {
            let by_meta = recorded.is_some_and(|ids| ids.contains(&s.session_id));
            let by_branch = card.head_ref.is_some()
                && s.git_branch == card.head_ref
                && s.repo.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(&card.repo));
            if by_meta || by_branch {
                out.push(CardSession {
                    card_id: card.id.clone(),
                    repo: card.repo.clone(),
                    number: card.number,
                    session_id: s.session_id.clone(),
                });
            }
        }
    }
    out
}

#[tauri::command]
pub async fn board_card_sessions(db: State<'_, Db>) -> AppResult<Vec<CardSession>> {
    let (cards, meta) = {
        let conn = db.0.lock().unwrap();
        load_pr_cards(&conn)?
    };
    let sessions = claude_sessions::live_sessions();
    Ok(match_card_sessions(&cards, &meta, &sessions))
}

/// Most recent `claude_session` recorded on the card that isn't currently live (resuming a live
/// session would put two writers on one transcript).
pub(crate) fn resumable_card_session(conn: &Connection, card_id: &str, live: &HashSet<String>) -> AppResult<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT value FROM board_meta WHERE card_id = ?1 AND kind = 'claude_session'
         ORDER BY created_at DESC, id DESC",
    )?;
    let ids = stmt
        .query_map(params![card_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids.into_iter().find(|id| !live.contains(id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_sessions::FocusState;

    fn session(id: &str, repo: Option<&str>, branch: Option<&str>) -> ClaudeSession {
        ClaudeSession {
            session_id: id.into(),
            pid: Some(1),
            live: true,
            status: None,
            kind: Some("interactive".into()),
            attached: false,
            job_id: None,
            waiting_for: None,
            agent_view: None,
            name: None,
            cwd: "/x".into(),
            git_branch: branch.map(Into::into),
            repo: repo.map(Into::into),
            tmux: None,
            tty: None,
            updated_at: 0,
            focus: FocusState::Ready,
        }
    }

    fn card(id: &str, repo: &str, head_ref: Option<&str>) -> PrCard {
        PrCard { id: id.into(), repo: repo.into(), number: 1, head_ref: head_ref.map(Into::into) }
    }

    #[test]
    fn matches_by_meta_or_branch() {
        let cards = vec![card("a", "Org/Repo", Some("feat/x")), card("b", "org/other", Some("feat/x"))];
        let meta = HashMap::from([("b".to_string(), HashSet::from(["s3".to_string()]))]);
        let sessions = vec![
            session("s1", Some("org/repo"), Some("feat/x")),
            session("s2", Some("org/repo"), Some("main")),
            session("s3", None, None),
            session("s4", Some("org/third"), Some("feat/x")),
        ];
        let got: Vec<_> = match_card_sessions(&cards, &meta, &sessions)
            .into_iter()
            .map(|c| (c.card_id, c.session_id))
            .collect();
        assert_eq!(got, vec![("a".into(), "s1".into()), ("b".into(), "s3".into())]);
    }

    #[test]
    fn card_without_head_ref_matches_meta_only() {
        let cards = vec![card("a", "org/repo", None)];
        let sessions = vec![session("s1", Some("org/repo"), None)];
        assert!(match_card_sessions(&cards, &HashMap::new(), &sessions).is_empty());
    }

    #[test]
    fn resumable_skips_live_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE board_meta (id INTEGER PRIMARY KEY, card_id TEXT, kind TEXT, value TEXT, created_at TEXT);
             INSERT INTO board_meta (card_id, kind, value, created_at) VALUES
               ('c', 'claude_session', 'old', '2026-01-01'),
               ('c', 'claude_session', 'new', '2026-01-02'),
               ('c', 'note', 'x', '2026-01-03');",
        )
        .unwrap();
        assert_eq!(resumable_card_session(&conn, "c", &HashSet::new()).unwrap().as_deref(), Some("new"));
        let live = HashSet::from(["new".to_string()]);
        assert_eq!(resumable_card_session(&conn, "c", &live).unwrap().as_deref(), Some("old"));
        assert_eq!(resumable_card_session(&conn, "other", &live).unwrap(), None);
    }
}
