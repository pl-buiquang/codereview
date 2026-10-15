//! Opening Claude sessions as tmux windows and focusing tmux panes. The terminal host only
//! attaches tmux and is raised; tmux itself selects the window and pane.

use std::process::Command;

use serde::Deserialize;

use super::TerminalHost;
use crate::claude_sessions::{self, TmuxLoc};
use crate::error::{AppError, AppResult};

/// Which tmux session new windows are added to.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TmuxTarget {
    /// The session of the most recently active attached client, else the shared session.
    Current,
    Shared,
    PerRepo,
}

const DEFAULT_SESSION: &str = "codereview";

pub fn available() -> bool {
    crate::tools::tmux_path().is_some()
}

fn run(args: &[&str]) -> AppResult<String> {
    let out = Command::new(crate::tools::tmux_bin())
        .args(args)
        .output()
        .map_err(|e| AppError::Other(format!("failed to run tmux: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Other(format!("tmux {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    pub session: String,
    pub tty: String,
    pub pid: Option<i64>,
    activity: i64,
}

const CLIENT_FORMAT: &str = "#{client_activity}\t#{session_name}\t#{client_tty}\t#{client_pid}";

fn parse_clients(out: &str) -> Vec<Client> {
    out.lines()
        .filter_map(|l| {
            let [activity, session, tty, pid] = l.split('\t').collect::<Vec<_>>()[..] else { return None };
            Some(Client {
                session: session.into(),
                tty: tty.into(),
                pid: pid.parse().ok(),
                activity: activity.parse().unwrap_or(0),
            })
        })
        .collect()
}

/// Attached clients; empty when no tmux server is running.
fn clients() -> Vec<Client> {
    run(&["list-clients", "-F", CLIENT_FORMAT]).map(|o| parse_clients(&o)).unwrap_or_default()
}

fn most_recent<'a>(clients: impl Iterator<Item = &'a Client>) -> Option<&'a Client> {
    clients.max_by_key(|c| c.activity)
}

pub fn attached_client(session: &str) -> Option<Client> {
    let all = clients();
    most_recent(all.iter().filter(|c| c.session == session)).cloned()
}

/// tmux rejects `.` and `:` in session names.
fn sanitize_session_name(s: &str) -> String {
    s.trim().chars().map(|c| if c == '.' || c == ':' { '_' } else { c }).collect()
}

fn session_name(target: TmuxTarget, shared: &str, repo: &str, clients: &[Client]) -> String {
    let shared = Some(sanitize_session_name(shared)).filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_SESSION.into());
    match target {
        TmuxTarget::Current => most_recent(clients.iter()).map(|c| c.session.clone()).unwrap_or(shared),
        TmuxTarget::Shared => shared,
        TmuxTarget::PerRepo => {
            let name = sanitize_session_name(repo.rsplit('/').next().unwrap_or(repo));
            if name.is_empty() { shared } else { name }
        }
    }
}

/// Switch the session's attached client to `loc`'s pane and raise the terminal host showing it.
pub fn focus_pane(loc: &TmuxLoc) -> AppResult<()> {
    let client = attached_client(&loc.session)
        .ok_or_else(|| AppError::Other("no terminal is attached to this tmux session".into()))?;
    run(&["switch-client", "-c", &client.tty, "-t", &loc.pane])?;
    run(&["select-window", "-t", &loc.pane])?;
    run(&["select-pane", "-t", &loc.pane])?;
    // After a programmatic switch the outer terminal can keep showing the previous frame.
    run(&["refresh-client", "-t", &client.tty])?;
    // The client's cwd is where `tmux attach` ran, which is what Ghostty reports for its terminal.
    let cwd = client.pid.and_then(claude_sessions::process_cwd);
    super::focus_tty(&client.tty, cwd.as_deref())
}

pub struct NewWindow<'a> {
    pub target: TmuxTarget,
    pub shared_session: &'a str,
    pub repo: &'a str,
    pub window_name: &'a str,
    pub cwd: &'a str,
    pub command: &'a str,
}

/// Add a window running `command` to the target session, then focus it — attaching the session
/// in a new `host` window when no terminal shows it yet.
pub fn open(host: TerminalHost, w: &NewWindow) -> AppResult<()> {
    let session = session_name(w.target, w.shared_session, w.repo, &clients());
    let exact = format!("={session}");
    let fmt = "#{window_id}\t#{pane_id}";
    let created = if run(&["has-session", "-t", &exact]).is_ok() {
        run(&["new-window", "-t", &format!("{exact}:"), "-n", w.window_name, "-c", w.cwd, "-P", "-F", fmt, w.command])?
    } else {
        run(&["new-session", "-d", "-s", &session, "-n", w.window_name, "-c", w.cwd, "-P", "-F", fmt, w.command])?
    };
    let (window, pane) = created
        .split_once('\t')
        .ok_or_else(|| AppError::Other(format!("unexpected tmux output: {created}")))?;
    let loc = TmuxLoc { session: session.clone(), window: window.into(), pane: pane.into(), label: None };
    if attached_client(&session).is_some() {
        return focus_pane(&loc);
    }
    run(&["select-window", "-t", &loc.pane])?;
    super::open_window(host, w.cwd, &["tmux".into(), "attach".into(), "-t".into(), exact])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(session: &str, activity: i64) -> Client {
        Client { session: session.into(), tty: "/dev/ttys001".into(), pid: Some(1), activity }
    }

    #[test]
    fn parses_clients() {
        let out = "1700000002\twork\t/dev/ttys003\t4242\n1700000001\tmain\t/dev/ttys001\t\nbad line\n";
        let got = parse_clients(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Client { session: "work".into(), tty: "/dev/ttys003".into(), pid: Some(4242), activity: 1700000002 });
        assert_eq!(got[1].pid, None);
    }

    #[test]
    fn current_picks_most_recently_active_client() {
        let clients = [client("a", 10), client("b", 30), client("c", 20)];
        assert_eq!(session_name(TmuxTarget::Current, "shared", "o/r", &clients), "b");
    }

    #[test]
    fn current_falls_back_to_shared_without_clients() {
        assert_eq!(session_name(TmuxTarget::Current, "mine", "o/r", &[]), "mine");
        assert_eq!(session_name(TmuxTarget::Current, "  ", "o/r", &[]), "codereview");
    }

    #[test]
    fn shared_ignores_clients() {
        assert_eq!(session_name(TmuxTarget::Shared, "my.sess", "o/r", &[client("a", 1)]), "my_sess");
    }

    #[test]
    fn per_repo_uses_sanitized_repo_name() {
        assert_eq!(session_name(TmuxTarget::PerRepo, "s", "org/my.repo", &[client("a", 1)]), "my_repo");
        assert_eq!(session_name(TmuxTarget::PerRepo, "s", "org/", &[]), "s");
    }

    #[test]
    fn deserializes_targets() {
        let t: TmuxTarget = serde_json::from_str("\"per_repo\"").unwrap();
        assert_eq!(t, TmuxTarget::PerRepo);
    }
}
