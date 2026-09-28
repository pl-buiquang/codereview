use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Result returned to callers so they can surface a one-time notification.
pub struct CrInstallResult {
    pub path: PathBuf,
    pub updated: bool, // true = replaced an older version, false = fresh install
}

/// Copy the bundled `cr` sidecar to the best available bin directory.
///
/// Priority:
///   1. Replace `cr` in-place wherever it already lives on PATH (keeps the
///      user's existing setup intact — e.g. ~/.cargo/bin/cr stays there).
///   2. ~/.cargo/bin  — if the directory exists (Rust devs always have this
///                      in PATH, so no shell-profile edits needed).
///   3. ~/.local/bin  — XDG standard fallback; may need PATH setup.
///   4. %LOCALAPPDATA%\CodeReview\bin  — Windows fallback.
///
/// Silent no-op when the sidecar is not bundled (dev mode) or when the
/// destination already matches the source (same size = same build).
/// Returns Some(result) when a copy was actually performed.
pub fn maybe_install() -> Option<CrInstallResult> {
    let exe = std::env::current_exe().ok()?;
    let bin_dir = exe.parent()?;

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let sidecar = bin_dir.join(format!("cr{ext}"));

    if !sidecar.exists() {
        return None; // dev mode — not bundled
    }

    let dest = pick_dest(ext)?;
    let updated = dest.exists();

    // Skip when sizes match (same build already installed).
    if updated {
        let src_len = std::fs::metadata(&sidecar).map(|m| m.len()).unwrap_or(0);
        let dst_len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(1);
        if src_len == dst_len {
            return None;
        }
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    std::fs::copy(&sidecar, &dest).ok()?;

    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    Some(CrInstallResult { path: dest, updated })
}

/// Pick the best destination for the `cr` binary.
fn pick_dest(ext: &str) -> Option<PathBuf> {
    // 1. Replace wherever cr already lives on PATH.
    if let Ok(existing) = which::which("cr") {
        return Some(existing);
    }

    // 2. ~/.cargo/bin if the directory exists (already on PATH for Rust devs).
    #[cfg(unix)]
    if let Some(cargo_bin) = cargo_bin_dir() {
        if cargo_bin.exists() {
            return Some(cargo_bin.join("cr"));
        }
    }

    // 3. ~/.local/bin (XDG) / Windows fallback.
    platform_fallback(ext)
}

#[cfg(unix)]
fn cargo_bin_dir() -> Option<PathBuf> {
    // Respect $CARGO_HOME if set, otherwise default to ~/.cargo/bin.
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        return Some(PathBuf::from(cargo_home).join("bin"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".cargo/bin"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn platform_fallback(_ext: &str) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local/bin/cr"))
}

#[cfg(target_os = "windows")]
fn platform_fallback(ext: &str) -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        PathBuf::from(local)
            .join("CodeReview")
            .join("bin")
            .join(format!("cr{ext}")),
    )
}
