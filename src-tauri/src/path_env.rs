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
//!
//! On Linux the login shell is deliberately NOT sourced (the macOS heuristic
//! below would always see `/usr/local/bin` and skip it anyway). Instead we
//! prepend well-known per-user bin dirs (`~/.local/bin`, …) when they exist,
//! which is where `claude` is typically installed.

pub fn ensure_login_path() {
    // Kept for compatibility; the real work is in ensure_login_env.
    ensure_login_env();
}

/// Prepend each of `dirs` that is not already a PATH entry of `current`,
/// preserving the order of `dirs`. Pure so it can be unit-tested.
#[cfg(all(unix, not(target_os = "macos")))]
fn prepend_missing(current: &str, dirs: &[std::path::PathBuf]) -> String {
    let present: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        let s = d.to_string_lossy().into_owned();
        if !present.contains(&s.as_str()) && !out.contains(&s) {
            out.push(s);
        }
    }
    if !current.is_empty() {
        out.push(current.to_string());
    }
    out.join(":")
}

/// Linux: add existing per-user bin dirs to PATH (no shell rc files are run).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn ensure_login_env() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let dirs: Vec<std::path::PathBuf> = [".local/bin", "bin", ".cargo/bin", ".claude/local"]
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.is_dir())
        .collect();
    let current = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", prepend_missing(&current, &dirs));
}

/// Source the login shell's full environment into the current process. Existing
/// vars are NOT overwritten — we only fill in what's missing.
#[cfg(target_os = "macos")]
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

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn prepends_missing_dir() {
        let r = prepend_missing("/usr/bin:/bin", &[PathBuf::from("/h/.local/bin")]);
        assert_eq!(r, "/h/.local/bin:/usr/bin:/bin");
    }

    #[test]
    fn skips_dir_already_present() {
        let r = prepend_missing("/usr/bin:/h/.local/bin", &[PathBuf::from("/h/.local/bin")]);
        assert_eq!(r, "/usr/bin:/h/.local/bin");
    }

    #[test]
    fn empty_current_path() {
        let r = prepend_missing("", &[PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(r, "/a:/b");
    }
}
