use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// What `maybe_install` did, so callers can surface a one-time notification.
pub enum CrInstallOutcome {
    /// No `cr` was installed before.
    Installed,
    /// An older or different `cr` was replaced.
    Updated { from: String },
    /// The installed `cr` is newer than the bundled one (an older app was launched): left as is.
    KeptNewer { installed: String },
    /// The bundled sidecar's version doesn't match this app's: nothing copied.
    SidecarMismatch { bundled: String },
}

pub struct CrInstallResult {
    pub path: PathBuf,
    pub bundled: String,
    pub outcome: CrInstallOutcome,
}

impl CrInstallResult {
    /// One-line message for the toast and the log.
    pub fn note(&self) -> String {
        let path = self.path.display();
        match &self.outcome {
            CrInstallOutcome::Installed => format!("cr CLI {} installed → {path}", self.bundled),
            CrInstallOutcome::Updated { from } => {
                format!("cr CLI updated {from} → {} ({path})", self.bundled)
            }
            CrInstallOutcome::KeptNewer { installed } => format!(
                "cr CLI at {path} is newer ({installed}) than this app's ({}): left unchanged",
                self.bundled
            ),
            CrInstallOutcome::SidecarMismatch { bundled } => format!(
                "bundled cr is {bundled} but the app is {}: reinstall the app (cr at {path} left unchanged)",
                env!("CODEREVIEW_VERSION")
            ),
        }
    }
}

/// Copy the bundled `cr` sidecar to the best available bin directory.
///
/// Priority:
///   1. Replace `cr` in-place wherever it already lives on PATH (keeps the
///      user's existing setup intact — e.g. ~/.cargo/bin/cr stays there).
///   2. ~/.cargo/bin — if the directory exists (Rust devs always have this
///      in PATH, so no shell-profile edits needed).
///   3. ~/.local/bin  — XDG standard fallback; may need PATH setup.
///   4. %LOCALAPPDATA%\CodeReview\bin  — Windows fallback.
///
/// Versions are compared through `cr --version`: the copy is skipped when the
/// installed `cr` reports the same version, or a newer one (so launching an old
/// app never downgrades it). A sidecar whose version differs from this app's is
/// never copied.
///
/// Silent no-op when the sidecar is not bundled (dev mode) or already installed.
/// Returns Some(result) when there is something to tell the user.
pub fn maybe_install() -> Option<CrInstallResult> {
    let exe = std::env::current_exe().ok()?;
    let bin_dir = exe.parent()?;

    let ext = if cfg!(windows) { ".exe" } else { "" };
    let sidecar = bin_dir.join(format!("cr{ext}"));

    if !sidecar.exists() {
        return None; // dev mode — not bundled
    }

    let dest = pick_dest(ext)?;
    let bundled = cr_version(&sidecar).unwrap_or_else(|| "unknown".into());
    let result = |outcome| {
        Some(CrInstallResult {
            path: dest.clone(),
            bundled: bundled.clone(),
            outcome,
        })
    };

    if bundled != env!("CODEREVIEW_VERSION") {
        return result(CrInstallOutcome::SidecarMismatch {
            bundled: bundled.clone(),
        });
    }

    let installed = if dest.exists() {
        cr_version(&dest)
    } else {
        None
    };
    if let Some(installed) = &installed {
        if *installed == bundled {
            return None;
        }
        if is_newer(installed, &bundled) {
            return result(CrInstallOutcome::KeptNewer {
                installed: installed.clone(),
            });
        }
    }
    let existed = dest.exists();

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }

    std::fs::copy(&sidecar, &dest).ok()?;

    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }

    let outcome = if existed {
        CrInstallOutcome::Updated {
            from: installed.unwrap_or_else(|| "unknown".into()),
        }
    } else {
        CrInstallOutcome::Installed
    };
    result(outcome)
}

/// Run `<path> --version` and return the version (`cr 0.7.3-27+29df25b6` → `0.7.3-27+29df25b6`).
/// Gives up after a few seconds so a hung binary can't block startup.
fn cr_version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_version_output(&out)
}

fn parse_version_output(out: &str) -> Option<String> {
    let line = out.lines().next()?.trim();
    let v = line.strip_prefix("cr ").unwrap_or(line).trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Parse the `build.rs` version format `X.Y.Z[-N]+sha` into `(X, Y, Z, N)`.
fn version_key(v: &str) -> Option<(u64, u64, u64, u64)> {
    let base = v.split('+').next()?;
    let (semver, n) = match base.split_once('-') {
        Some((s, n)) => (s, n.parse().ok()?),
        None => (base, 0),
    };
    let mut parts = semver.split('.').map(|p| p.parse::<u64>().ok());
    let key = (parts.next()??, parts.next()??, parts.next()??, n);
    parts.next().is_none().then_some(key)
}

/// True when `a` is strictly newer than `b`. Unparsable versions are never "newer".
fn is_newer(a: &str, b: &str) -> bool {
    match (version_key(a), version_key(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_output() {
        assert_eq!(
            parse_version_output("cr 0.7.3-27+29df25b6\n").as_deref(),
            Some("0.7.3-27+29df25b6")
        );
        assert_eq!(parse_version_output(""), None);
    }

    #[test]
    fn orders_versions() {
        assert!(is_newer("0.7.3-1+abc", "0.7.3+def"));
        assert!(is_newer("0.7.4+abc", "0.7.3-27+def"));
        assert!(is_newer("0.10.0+abc", "0.9.9-5+def"));
        assert!(!is_newer("0.7.3-10+abc", "0.7.3-27+def"));
        assert!(!is_newer("0.7.3+abc", "0.7.3+def"));
    }

    #[test]
    fn unparsable_is_never_newer() {
        assert!(!is_newer("garbage", "0.7.3+abc"));
        assert!(!is_newer("0.7.3+abc", "garbage"));
        assert!(!is_newer("1.2+abc", "0.7.3+abc"));
    }
}
