//! The neutral session DTO every harness reports from [`super::Harness::live_sessions`]. Claude
//! fills all of it; other harnesses fill the basics (id, cwd, last activity, live, title) and
//! leave the Claude-only fields at their defaults.

use serde::Serialize;

use crate::claude_sessions::{AgentView, FocusState, TmuxLoc};
use crate::session_summaries::SessionSummary;

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    /// Id of the [`super::Harness`] the session belongs to (`"claude"`).
    pub harness: String,
    pub session_id: String,
    pub pid: Option<i64>,
    pub live: bool,
    pub status: Option<String>,
    pub kind: Option<String>,
    /// A background session currently viewed through a `claude attach` client.
    pub attached: bool,
    /// Background job id taken by `claude attach|stop|logs`.
    pub job_id: Option<String>,
    /// What the session is blocked on (e.g. "input needed", "dialog open").
    pub waiting_for: Option<String>,
    /// For background sessions nobody is attached to: the `claude agents` view best placed to
    /// open it.
    pub agent_view: Option<AgentView>,
    pub name: Option<String>,
    pub cwd: String,
    pub git_branch: Option<String>,
    /// `owner/name` of the cwd's `origin` remote (live sessions only).
    pub repo: Option<String>,
    pub tmux: Option<TmuxLoc>,
    pub tty: Option<String>,
    /// Last activity, epoch milliseconds.
    pub updated_at: i64,
    pub focus: FocusState,
    /// Hook-written summaries for this session, newest first (history sessions only).
    pub summaries: Vec<SessionSummary>,
    /// The transcript-derived name when a summary title replaced it.
    pub original_name: Option<String>,
}

