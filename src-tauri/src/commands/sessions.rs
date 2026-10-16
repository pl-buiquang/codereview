use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
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

/// Open an ended session in a new tmux window running `claude --resume`.
#[tauri::command]
pub async fn resume_claude_session(
    session_id: String,
    cwd: String,
    repo: Option<String>,
    host: terminal::TerminalHost,
    tmux_target: terminal::TmuxTarget,
    tmux_session: String,
) -> AppResult<()> {
    if claude_sessions::live_sessions().iter().any(|s| s.session_id == session_id) {
        return Err(AppError::Other("this session is already running — focus it instead".into()));
    }
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(AppError::Other(format!("{cwd} no longer exists")));
    }
    let repo = repo.unwrap_or_else(|| cwd.trim_end_matches('/').rsplit('/').next().unwrap_or(&cwd).to_string());
    let window_name: String = session_id.chars().take(8).collect();
    let argv = ["claude".to_string(), "--resume".to_string(), session_id];
    terminal::open_session(host, tmux_target, &tmux_session, &repo, &window_name, &cwd, &argv)
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

fn is_focusable(s: &ClaudeSession) -> bool {
    s.focus == claude_sessions::FocusState::Ready
        || s.agent_view.as_ref().is_some_and(|v| v.focus == claude_sessions::FocusState::Ready)
}

fn in_dir(cwd: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    let cwd = cwd.trim_end_matches('/');
    cwd == dir || cwd.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// Focusable live sessions for a review: those running inside its chat worktree, or belonging
/// to the review's PR board card.
fn match_review_sessions(
    worktree: Option<&str>,
    card: Option<&PrCard>,
    meta: &SessionIdsByCard,
    sessions: Vec<ClaudeSession>,
) -> Vec<ClaudeSession> {
    let by_card: HashSet<String> = card
        .map(|c| match_card_sessions(std::slice::from_ref(c), meta, &sessions))
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.session_id)
        .collect();
    sessions
        .into_iter()
        .filter(is_focusable)
        .filter(|s| by_card.contains(&s.session_id) || worktree.is_some_and(|w| in_dir(&s.cwd, w)))
        .collect()
}

struct ReviewSessionCtx {
    detail: crate::db::models::ReviewDetail,
    worktree: Option<String>,
    card: Option<PrCard>,
}

fn review_session_ctx(conn: &Connection, review_id: i64) -> AppResult<ReviewSessionCtx> {
    let detail = crate::commands::review::load_detail(conn, review_id)?;
    let worktree: Option<String> = conn
        .query_row("SELECT worktree_path FROM chat WHERE review_id = ?1", params![review_id], |r| r.get(0))
        .optional()?
        .flatten();
    let card = match (&detail.remote_owner, &detail.remote_name, detail.target.github_pr_number) {
        (Some(owner), Some(name), Some(number)) if detail.target.kind == "github_pr" => conn
            .query_row(
                "SELECT id, repo, number, head_ref FROM board_card WHERE repo = ?1 AND number = ?2",
                params![format!("{owner}/{name}"), number],
                |r| Ok(PrCard { id: r.get(0)?, repo: r.get(1)?, number: r.get(2)?, head_ref: r.get(3)? }),
            )
            .optional()?,
        _ => None,
    };
    Ok(ReviewSessionCtx { detail, worktree, card })
}

#[tauri::command]
pub async fn review_agent_sessions(review_id: i64, db: State<'_, Db>) -> AppResult<Vec<ClaudeSession>> {
    let (ctx, meta) = {
        let conn = db.0.lock().unwrap();
        let ctx = review_session_ctx(&conn, review_id)?;
        let (_, meta) = load_pr_cards(&conn)?;
        (ctx, meta)
    };
    let sessions = claude_sessions::live_sessions();
    Ok(match_review_sessions(ctx.worktree.as_deref(), ctx.card.as_ref(), &meta, sessions))
}

/// Ensure the review's chat worktree exists, then open a tmux window there running Claude —
/// resuming the PR card's latest recorded session when it isn't already live.
#[tauri::command]
pub async fn review_open_agent(
    review_id: i64,
    host: terminal::TerminalHost,
    tmux_target: terminal::TmuxTarget,
    tmux_session: String,
    db: State<'_, Db>,
) -> AppResult<()> {
    let worktree_path = crate::commands::chat::ensure_chat_worktree(review_id, db.clone()).await?;
    let live: HashSet<String> = claude_sessions::live_sessions().into_iter().map(|s| s.session_id).collect();
    let (ctx, resume) = {
        let conn = db.0.lock().unwrap();
        let ctx = review_session_ctx(&conn, review_id)?;
        let resume = match &ctx.card {
            Some(card) => resumable_card_session(&conn, &card.id, &live)?,
            None => None,
        };
        (ctx, resume)
    };
    let target = &ctx.detail.target;
    let repo = match (&ctx.detail.remote_owner, &ctx.detail.remote_name) {
        (Some(o), Some(n)) => format!("{o}/{n}"),
        _ => ctx.detail.local_path.clone().unwrap_or_default(),
    };
    let window_name = match target.github_pr_number {
        Some(n) => format!("#{n}"),
        None => target.head_ref.clone(),
    };
    let mut argv = vec!["claude".to_string()];
    if let Some(id) = resume {
        argv.extend(["--resume".to_string(), id]);
    }
    terminal::open_session(host, tmux_target, &tmux_session, &repo, &window_name, &worktree_path, &argv)
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
    #[test]
    fn review_sessions_match_worktree_or_card() {
        let mut in_wt = session("s1", None, None);
        in_wt.cwd = "/wt/pr-1/sub".into();
        let mut prefix_only = session("s2", None, None);
        prefix_only.cwd = "/wt/pr-10".into();
        let by_branch = session("s3", Some("org/repo"), Some("feat/x"));
        let mut detached = session("s4", None, None);
        detached.cwd = "/wt/pr-1".into();
        detached.focus = FocusState::Detached;
        let c = card("a", "org/repo", Some("feat/x"));
        let got: Vec<_> = match_review_sessions(
            Some("/wt/pr-1/"),
            Some(&c),
            &HashMap::new(),
            vec![in_wt, prefix_only, by_branch, detached],
        )
        .into_iter()
        .map(|s| s.session_id)
        .collect();
        assert_eq!(got, vec!["s1", "s3"]);
    }

    #[test]
    fn review_sessions_without_worktree_or_card_is_empty() {
        assert!(match_review_sessions(None, None, &HashMap::new(), vec![session("s1", None, None)]).is_empty());
    }
}
