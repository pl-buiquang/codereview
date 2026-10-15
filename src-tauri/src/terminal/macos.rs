use std::collections::HashMap;
use std::process::Command;

use crate::error::{AppError, AppResult};

/// Runs an AppleScript with `argv` passed as script arguments (never interpolated into source).
fn osascript(script: &str, argv: &[&str]) -> AppResult<String> {
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .args(argv)
        .output()
        .map_err(|e| AppError::Other(format!("failed to run osascript: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Other(format!("osascript: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

const ITERM_FOCUS: &str = r#"
on run argv
  set target to item 1 of argv
  if application "iTerm2" is not running then return "not-running"
  tell application "iTerm2"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with s in sessions of t
          if tty of s is target then
            select w
            select t
            select s
            activate
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
  end tell
  return "missing"
end run
"#;

#[derive(Debug, Clone, Copy, PartialEq)]
enum HostApp {
    Iterm2,
    Ghostty,
}

fn host_app_from_comm(comm: &str) -> Option<HostApp> {
    if comm.contains("/iTerm.app/") || comm.contains("/iTerm2/iTermServer") {
        Some(HostApp::Iterm2)
    } else if comm.contains("/Ghostty.app/") {
        Some(HostApp::Ghostty)
    } else {
        None
    }
}

struct Proc {
    ppid: i64,
    tty: String,
    comm: String,
}

fn parse_ps_all(out: &str) -> HashMap<i64, Proc> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let tty = it.next()?.to_string();
            let comm = it.collect::<Vec<_>>().join(" ");
            Some((pid, Proc { ppid, tty, comm }))
        })
        .collect()
}

/// Which terminal app owns `tty`, found by walking up from the tty's processes.
fn host_app_in(procs: &HashMap<i64, Proc>, tty: &str) -> Option<HostApp> {
    let name = tty.trim_start_matches("/dev/");
    procs.iter().filter(|(_, p)| p.tty == name).find_map(|(&pid, _)| {
        let mut cur = pid;
        for _ in 0..32 {
            let p = procs.get(&cur)?;
            if let Some(app) = host_app_from_comm(&p.comm) {
                return Some(app);
            }
            if p.ppid <= 1 {
                return None;
            }
            cur = p.ppid;
        }
        None
    })
}

fn host_app(tty: &str) -> Option<HostApp> {
    let out = Command::new("ps").args(["-axo", "pid=,ppid=,tty=,comm="]).output().ok()?;
    host_app_in(&parse_ps_all(&String::from_utf8_lossy(&out.stdout)), tty)
}

/// Focus the terminal tab showing `tty`. Ghostty can't be queried by tty, so its terminal is
/// picked by `cwd_hint` (or taken as-is when it has a single terminal).
pub fn focus_tty(tty: &str, cwd_hint: Option<&str>) -> AppResult<()> {
    match host_app(tty) {
        Some(HostApp::Ghostty) => ghostty_focus(cwd_hint.unwrap_or("")),
        Some(HostApp::Iterm2) => iterm2_focus_tty(tty),
        None => Err(AppError::Other(format!("{tty} is not in iTerm2 or Ghostty"))),
    }
}

const GHOSTTY_FOCUS: &str = r#"
on run argv
  set wd to item 1 of argv
  if application "Ghostty" is not running then return "not-running"
  tell application "Ghostty"
    set ts to terminals
    if (count of ts) is 1 and wd is "" then
      focus (item 1 of ts)
      activate
      return "ok"
    end if
    repeat with t in ts
      if wd is not "" and (working directory of t) is wd then
        focus t
        activate
        return "ok"
      end if
    end repeat
  end tell
  return "missing"
end run
"#;

fn ghostty_focus(cwd: &str) -> AppResult<()> {
    match osascript(GHOSTTY_FOCUS, &[cwd])?.as_str() {
        "ok" => Ok(()),
        "not-running" => Err(AppError::Other("Ghostty is not running".into())),
        _ => Err(AppError::Other("could not tell which Ghostty terminal shows this session".into())),
    }
}

fn iterm2_focus_tty(tty: &str) -> AppResult<()> {
    match osascript(ITERM_FOCUS, &[tty])?.as_str() {
        "ok" => Ok(()),
        "not-running" => Err(AppError::Other("iTerm2 is not running".into())),
        _ => Err(AppError::Other(format!("no iTerm2 tab shows {tty}"))),
    }
}

const ITERM_OPEN: &str = r#"
on run argv
  tell application "iTerm2"
    set w to (create window with default profile)
    set zoomed of w to true
    tell current session of w to write text (item 1 of argv)
    activate
  end tell
end run
"#;

pub fn iterm2_open(command: &str) -> AppResult<()> {
    osascript(ITERM_OPEN, &[command]).map(|_| ())
}

const TERMINAL_OPEN: &str = r#"
on run argv
  tell application "Terminal"
    do script (item 1 of argv)
    set zoomed of front window to true
    activate
  end tell
end run
"#;

pub fn terminal_app_open(command: &str) -> AppResult<()> {
    osascript(TERMINAL_OPEN, &[command]).map(|_| ())
}

// Requires Ghostty >= 1.3 (AppleScript dictionary). The command is typed into the user's shell
// so it stays open after Claude exits. Maximizing is a toggle, so with `maximize = true` in the
// Ghostty config the window is already maximized and must not be toggled.
const GHOSTTY_OPEN: &str = r#"
on run argv
  tell application "Ghostty"
    set cfg to new surface configuration
    set initial working directory of cfg to (item 1 of argv)
    set initial input of cfg to (item 2 of argv) & linefeed
    set w to (new window with configuration cfg)
    if (item 3 of argv) is "toggle" then perform action "toggle_maximize" on (focused terminal of selected tab of w)
    activate
  end tell
end run
"#;

fn ghostty_maximizes_by_default() -> bool {
    Command::new("/Applications/Ghostty.app/Contents/MacOS/ghostty")
        .args(["+show-config"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.replace(' ', "") == "maximize=true"))
        .unwrap_or(false)
}

pub fn ghostty_open(cwd: &str, command: &str) -> AppResult<()> {
    let maximize = if ghostty_maximizes_by_default() { "keep" } else { "toggle" };
    osascript(GHOSTTY_OPEN, &[cwd, command, maximize]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_host_app_by_ancestry() {
        let ps = "\
  687     1 ??       /Applications/iTerm.app/Contents/MacOS/iTerm2
 1974   687 ??       /Users/me/Library/Application Support/iTerm2/iTermServer-3.7.0
 1990  1974 ttys000  /usr/bin/login
 2012  1990 ttys000  -zsh
 6096     1 ??       /Applications/Ghostty.app/Contents/MacOS/ghostty
27391  6096 ttys005  /usr/bin/login
27872 27391 ttys005  claude
  900     1 ttys009  /usr/bin/login
";
        let procs = parse_ps_all(ps);
        assert_eq!(procs[&1974].comm, "/Users/me/Library/Application Support/iTerm2/iTermServer-3.7.0");
        assert_eq!(host_app_in(&procs, "/dev/ttys000"), Some(HostApp::Iterm2));
        assert_eq!(host_app_in(&procs, "/dev/ttys005"), Some(HostApp::Ghostty));
        assert_eq!(host_app_in(&procs, "/dev/ttys009"), None);
        assert_eq!(host_app_in(&procs, "/dev/ttys042"), None);
    }
}
