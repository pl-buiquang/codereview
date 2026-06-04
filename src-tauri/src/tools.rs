//! Absolute-path resolution for the external `git` and `gh` binaries.
//!
//! GUI launches start with a minimal PATH (see [`crate::path_env`]); once that's
//! repaired we resolve each tool to an absolute path exactly once and cache it,
//! so later environment changes can't break command spawning. If resolution
//! fails we fall back to the bare command name and let the spawn error surface.

use std::sync::OnceLock;

static GIT: OnceLock<Option<String>> = OnceLock::new();
static GH: OnceLock<Option<String>> = OnceLock::new();

fn resolve(name: &str) -> Option<String> {
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Resolve and cache absolute paths for `git` and `gh`. Call once at startup,
/// after PATH has been repaired. Subsequent calls are no-ops.
pub fn init() {
    let _ = GIT.set(resolve("git"));
    let _ = GH.set(resolve("gh"));
}

/// The resolved absolute path to `git`, or `None` if it wasn't found / `init`
/// hasn't run. Intended for diagnostics.
pub fn git_path() -> Option<String> {
    GIT.get().cloned().flatten()
}

/// The resolved absolute path to `gh`, or `None`. Intended for diagnostics.
pub fn gh_path() -> Option<String> {
    GH.get().cloned().flatten()
}

/// The binary to spawn for `git`: the resolved absolute path, or `"git"`.
pub fn git_bin() -> String {
    git_path().unwrap_or_else(|| "git".to_string())
}

/// The binary to spawn for `gh`: the resolved absolute path, or `"gh"`.
pub fn gh_bin() -> String {
    gh_path().unwrap_or_else(|| "gh".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_known_binary() {
        // `git` is required to run this repo's test suite, so it must resolve.
        let p = resolve("git");
        assert!(p.is_some(), "expected to resolve git on PATH");
        assert!(p.unwrap().contains("git"));
    }

    #[test]
    fn bogus_binary_does_not_resolve() {
        assert!(resolve("definitely-not-a-real-binary-xyzzy").is_none());
    }

    #[test]
    fn bin_helpers_fall_back_to_bare_name() {
        // In unit tests `init()` may not have run; helpers must still return a
        // spawnable name rather than panicking.
        assert!(!git_bin().is_empty());
        assert!(!gh_bin().is_empty());
    }
}
