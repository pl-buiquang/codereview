use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Copy the bundled `cr` sidecar to a user-writable bin directory so it is
/// available on the shell PATH without any manual install step.
///
/// - Silent no-op in dev mode (sidecar not present next to the executable).
/// - Overwrites the destination when the file size differs (i.e. the app was
///   updated), preserving the existing binary otherwise.
pub fn maybe_install() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(bin_dir) = exe.parent() else { return };

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let sidecar = bin_dir.join(format!("cr{ext}"));

    if !sidecar.exists() {
        // Dev mode — cr binary isn't bundled next to the exe.
        return;
    }

    let Some(dest) = install_dest() else { return };

    // Skip if destination already has the same size (same build).
    if dest.exists() {
        let src_len = std::fs::metadata(&sidecar).map(|m| m.len()).unwrap_or(0);
        let dst_len = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(1);
        if src_len == dst_len {
            return;
        }
    }

    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }

    if std::fs::copy(&sidecar, &dest).is_err() {
        return;
    }

    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn install_dest() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local/bin/cr"))
}

#[cfg(target_os = "windows")]
fn install_dest() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    Some(
        PathBuf::from(local)
            .join("CodeReview")
            .join("bin")
            .join("cr.exe"),
    )
}
