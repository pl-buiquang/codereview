fn main() {
    tauri_build::build();
    println!("cargo:rustc-env=CODEREVIEW_VERSION={}", git_version());
    // Re-run when the commit or tags change.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/packed-refs");
}

/// Compute a human-readable version string from git.
///
/// Format:
///   - exact tag:              `0.1.0+abcdef1`
///   - N commits after tag:    `0.1.0-3+abcdef1`
///   - no matching tag:        `0.0.0+abcdef1`
fn git_version() -> String {
    let sha = short_sha().unwrap_or_else(|| "unknown".into());

    // --long always emits "{tag}-{n}-g{sha}" even on the tagged commit (n=0).
    let describe = std::process::Command::new("git")
        .args([
            "describe",
            "--tags",
            "--match",
            "app-v*",
            "--long",
            "--abbrev=8",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    match describe {
        Some(desc) => {
            // desc = "app-v0.1.0-3-gabcdef12"
            let s = desc.trim_start_matches("app-v"); // "0.1.0-3-gabcdef12"
            if let Some(pos) = s.rfind("-g") {
                let base = &s[..pos]; // "0.1.0-3" or "0.1.0-0"
                let base = base.strip_suffix("-0").unwrap_or(base); // drop "-0" on exact tag
                format!("{base}+{sha}")
            } else {
                format!("{s}+{sha}")
            }
        }
        // No app-v* tag reachable — fall back to bare sha.
        None => format!("0.0.0+{sha}"),
    }
}

fn short_sha() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
    } else {
        None
    }
}
