//! Terminal hosts: focusing the tab/pane of a live Claude session and opening new terminals.

mod linux;
mod macos;

use std::process::Command;

use serde::Deserialize;

use crate::claude_sessions::{self, ClaudeSession, FocusState};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalHost {
    Iterm2,
    Ghostty,
    #[serde(rename = "terminal")]
    TerminalApp,
    Terminator,
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn command_line(argv: &[String]) -> String {
    argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

fn user_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into())
}

/// Opens a new terminal window in `cwd` running `argv`, leaving an interactive shell behind.
pub fn open_window(host: TerminalHost, cwd: &str, argv: &[String]) -> AppResult<()> {
    let typed = format!("cd {} && {}", shell_quote(cwd), command_line(argv));
    let shell = user_shell();
    let wrapped = format!("{}; exec {} -l", command_line(argv), shell_quote(&shell));
    match host {
        TerminalHost::Iterm2 if cfg!(target_os = "macos") => macos::iterm2_open(&typed),
        TerminalHost::TerminalApp if cfg!(target_os = "macos") => macos::terminal_app_open(&typed),
        TerminalHost::Ghostty if cfg!(target_os = "macos") => macos::ghostty_open(cwd, &command_line(argv)),
        TerminalHost::Terminator if cfg!(target_os = "linux") => linux::terminator_open(cwd, &shell, &wrapped),
        other => Err(AppError::Other(format!("{other:?} is not supported on this platform"))),
    }
}

fn focus_tty(tty: &str, cwd_hint: Option<&str>) -> AppResult<()> {
    if cfg!(target_os = "macos") {
        macos::focus_tty(tty, cwd_hint)
    } else if cfg!(target_os = "linux") {
        linux::focus_tty(tty)
    } else {
        Err(AppError::Other("focusing terminals is not supported on this platform".into()))
    }
}

fn tmux(args: &[&str]) -> AppResult<()> {
    let out = Command::new(crate::tools::tmux_bin())
        .args(args)
        .output()
        .map_err(|e| AppError::Other(format!("failed to run tmux: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Other(format!("tmux {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(())
}

/// Focus the session's own terminal, or — for a background session nobody is attached to — the
/// agent view it can be opened from.
pub fn focus_session(session: &ClaudeSession) -> AppResult<()> {
    if session.focus != FocusState::Ready {
        if let Some(view) = session.agent_view.as_ref().filter(|v| v.focus == FocusState::Ready) {
            return focus_target(view.tmux.as_ref(), Some(&view.tty), view.cwd.as_deref());
        }
    }
    match session.focus {
        FocusState::Ready => {}
        FocusState::Detached => return Err(AppError::Other("no terminal is attached to this tmux session".into())),
        FocusState::Background => return Err(AppError::Other("background sessions have no terminal".into())),
        FocusState::NoTerminal => return Err(AppError::Other("this session has no terminal".into())),
        FocusState::NotLive => return Err(AppError::Other("this session is not running".into())),
    }
    focus_target(session.tmux.as_ref(), session.tty.as_deref(), Some(&session.cwd))
}

fn focus_target(tmux_loc: Option<&claude_sessions::TmuxLoc>, tty: Option<&str>, cwd: Option<&str>) -> AppResult<()> {
    match (tmux_loc, tty) {
        (Some(loc), _) => {
            let client = claude_sessions::attached_client_tty(loc)
                .ok_or_else(|| AppError::Other("no terminal is attached to this tmux session".into()))?;
            tmux(&["switch-client", "-c", &client, "-t", &loc.pane])?;
            tmux(&["select-window", "-t", &loc.pane])?;
            tmux(&["select-pane", "-t", &loc.pane])?;
            // After a programmatic switch the outer terminal can keep showing the previous frame.
            tmux(&["refresh-client", "-t", &client])?;
            focus_tty(&client, None)
        }
        (None, Some(tty)) => focus_tty(tty, cwd),
        (None, None) => Err(AppError::Other("this session has no terminal".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_args() {
        assert_eq!(shell_quote("claude"), "claude");
        assert_eq!(shell_quote("/a/b-c_d.e"), "/a/b-c_d.e");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("$(rm -rf x)"), "'$(rm -rf x)'");
    }

    #[test]
    fn deserializes_hosts() {
        let h: TerminalHost = serde_json::from_str("\"terminal\"").unwrap();
        assert_eq!(h, TerminalHost::TerminalApp);
        let h: TerminalHost = serde_json::from_str("\"iterm2\"").unwrap();
        assert_eq!(h, TerminalHost::Iterm2);
    }
}
