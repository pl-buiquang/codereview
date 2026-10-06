//! Repair the process environment when the app is launched from a GUI (Finder /
//! Dock / `/Applications`), where it inherits launchd's minimal environment and
//! can't find tools installed under `/opt/homebrew/bin` or `/usr/local/bin`.
//!
//! `ensure_login_env` loads the full login-shell environment so that subprocesses
//! (notably `claude -p`) inherit the same env vars a terminal user would have —
//! proxy settings, ANTHROPIC_* keys, etc. — without which OAuth token refresh can
//! silently fail.
//!
//! No-op on a terminal launch (PATH already good) and on non-unix targets.

pub fn ensure_login_path() {
    // Kept for compatibility; the real work is in ensure_login_env.
    ensure_login_env();
}

/// Source the login shell's full environment into the current process. Existing
/// vars are NOT overwritten — we only fill in what's missing.
#[cfg(unix)]
pub fn ensure_login_env() {
    use std::process::Command;

    let current_path = std::env::var("PATH").unwrap_or_default();
    // Terminal / `tauri dev` launches already have a good environment.
    if current_path.contains("/opt/homebrew/bin") || current_path.contains("/usr/local/bin") {
        return;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    // Run an interactive login shell and dump its environment between sentinels.
    // `-i` sources .zshrc/.bashrc; `-l` sources .zprofile/.bash_profile.
    const START: &str = "__CR_ENV_START__";
    const END: &str = "__CR_ENV_END__";
    let script = format!("printf '{START}'; env; printf '{END}'");

    let Ok(output) = Command::new(&shell).args(["-ilc", &script]).output() else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (Some(s), Some(e)) = (stdout.find(START), stdout.rfind(END)) else {
        return;
    };
    if s >= e {
        return;
    }
    let env_block = &stdout[s + START.len()..e];

    for line in env_block.lines() {
        if let Some((key, val)) = line.split_once('=') {
            // Only set vars that are not already present in the process env.
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, val);
            }
        }
    }

    // PATH gets the login shell's value prepended even if already set, so
    // Homebrew tools are found after a Finder launch.
    if let Some(login_path_line) = env_block.lines().find(|l| l.starts_with("PATH=")) {
        let login_path = &login_path_line["PATH=".len()..];
        let merged = if current_path.is_empty() {
            login_path.to_string()
        } else {
            format!("{login_path}:{current_path}")
        };
        std::env::set_var("PATH", merged);
    }
}

#[cfg(not(unix))]
pub fn ensure_login_env() {}
