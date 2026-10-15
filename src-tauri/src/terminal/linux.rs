//! Terminator support. Focus is best-effort and X11-only (`wmctrl`); untested on real hardware.

use std::collections::HashSet;
use std::process::Command;

use crate::error::{AppError, AppResult};

pub fn terminator_open(cwd: &str, shell: &str, command: &str) -> AppResult<()> {
    Command::new("terminator")
        .arg(format!("--working-directory={cwd}"))
        .args(["-x", shell, "-lc", command])
        .spawn()
        .map_err(|e| AppError::Other(format!("failed to open Terminator: {e}")))?;
    Ok(())
}

fn stdout(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The process outside the tty that spawned it — for a terminal tab, the terminal emulator.
fn host_pid(tty: &str) -> Option<i64> {
    let name = tty.trim_start_matches("/dev/");
    let ps = stdout(Command::new("ps").args(["-o", "pid=,ppid=", "-t", name]))?;
    let rows: Vec<(i64, i64)> = ps
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    let on_tty: HashSet<i64> = rows.iter().map(|r| r.0).collect();
    rows.iter().map(|r| r.1).find(|ppid| !on_tty.contains(ppid))
}

pub fn focus_tty(tty: &str) -> AppResult<()> {
    let pid = host_pid(tty).ok_or_else(|| AppError::Other(format!("no terminal process found for {tty}")))?;
    let windows = stdout(Command::new("wmctrl").arg("-lp"))
        .ok_or_else(|| AppError::Other("wmctrl is required to focus terminals (X11 only)".into()))?;
    let window = windows
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            let id = it.next()?;
            let _desktop = it.next()?;
            (it.next()?.parse::<i64>().ok()? == pid).then(|| id.to_string())
        })
        .ok_or_else(|| AppError::Other("terminal window not found".into()))?;
    stdout(Command::new("wmctrl").args(["-ia", &window]))
        .map(|_| ())
        .ok_or_else(|| AppError::Other("wmctrl failed to activate the window".into()))
}
