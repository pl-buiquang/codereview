//! Repair the process `PATH` when the app is launched from a GUI (Finder / Dock /
//! `/Applications`), where it inherits launchd's minimal environment and can't find
//! tools installed under `/opt/homebrew/bin` or `/usr/local/bin` — notably `gh`.
//!
//! No-op on a terminal launch (PATH already good) and on non-unix targets.

#[cfg(unix)]
pub fn ensure_login_path() {
    use std::process::Command;

    let current = std::env::var("PATH").unwrap_or_default();
    // Terminal / `tauri dev` launches already inherit the user's PATH.
    if current.contains("/opt/homebrew/bin") || current.contains("/usr/local/bin") {
        return;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    const DELIM: &str = "__CR_PATH__";
    // `-i` sources interactive rc files (`.zshrc`/`.bashrc`) where PATH is often set;
    // the delimiters fence off any other stdout the rc files may emit.
    let script = format!("printf '{DELIM}%s{DELIM}' \"$PATH\"");

    let Ok(output) = Command::new(&shell).args(["-ilc", &script]).output() else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (Some(start), Some(end)) = (stdout.find(DELIM), stdout.rfind(DELIM)) else {
        return;
    };
    if start >= end {
        return;
    }
    let login_path = stdout[start + DELIM.len()..end].trim();
    if login_path.is_empty() {
        return;
    }

    let merged = if current.is_empty() {
        login_path.to_string()
    } else {
        format!("{login_path}:{current}")
    };
    std::env::set_var("PATH", merged);
}

#[cfg(not(unix))]
pub fn ensure_login_path() {}
