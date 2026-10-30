//! The on-disk cache of a review's image diffs: `<db dir>/image_diffs/<db stem>/<review id>/`.
//! The db stem keeps the dev and release databases (which share a directory) apart.
//!
//! ```text
//! manifest.json            written last, atomically: its presence means "generated"
//! progress.json            while generating (and after a failure)
//! f<i>/before.png after.png diff.png            full size
//! f<i>/preview-{before,after,diff}.png          at most PREVIEW_WIDTH wide
//! f<i>/crop-<region>-{before,after,diff}.png    a region with some context
//! report.json report.final.json                 the AI report
//! ```

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::regions::Region;
use crate::error::AppResult;

pub const MANIFEST: &str = "manifest.json";
pub const PROGRESS: &str = "progress.json";
pub const REPORT_FINAL: &str = "report.final.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dim {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Subdirectory `f<index>`.
    pub index: usize,
    pub path: String,
    pub old_path: Option<String>,
    pub change: Change,
    pub before: Option<Dim>,
    pub after: Option<Dim>,
    /// Real differences (anti-aliasing excluded); 0 for added/deleted/skipped files.
    pub mismatch_px: u64,
    pub aa_px: u64,
    pub diff_pct: f64,
    pub regions: Vec<Region>,
    pub other_regions: u32,
    /// Why there is no diff for this file (too big, unreadable, undecodable).
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub review_id: i64,
    /// The review's pin when generated, to tell later that the cache is outdated.
    pub pin: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub generated_at: String,
    /// Images wider than this have `preview-*.png` next to the full-size ones.
    pub preview_width: u32,
    /// Started without a click.
    pub auto: bool,
    pub files: Vec<FileEntry>,
}

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    /// `running` or `failed`.
    pub state: String,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
}

/// What identifies the diff a review shows: its pinned shas and its "since" pin.
pub fn pin_key(base: Option<&str>, head: Option<&str>, since: Option<(&str, &str)>) -> String {
    format!(
        "{}:{}:{}",
        base.unwrap_or(""),
        head.unwrap_or(""),
        since.map(|(b, h)| format!("{b}..{h}")).unwrap_or_default()
    )
}

pub fn cache_root(db_path: &Path) -> PathBuf {
    let stem = db_path.file_stem().and_then(|s| s.to_str()).unwrap_or("codereview");
    db_path.parent().unwrap_or(Path::new(".")).join("image_diffs").join(stem)
}

pub fn cache_dir(db_path: &Path, review_id: i64) -> PathBuf {
    cache_root(db_path).join(review_id.to_string())
}

/// The cache directory of a review as seen from its connection; `None` for an in-memory database.
pub fn cache_dir_for_conn(conn: &rusqlite::Connection, review_id: i64) -> Option<PathBuf> {
    conn.path().filter(|p| !p.is_empty()).map(|p| cache_dir(Path::new(p), review_id))
}

/// Delete a review's cache. Failures are logged: they must not fail what triggered the cleanup.
pub fn remove_cache(dir: &Path) {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!(target: "image_diff", "removing {} failed: {e}", dir.display()),
    }
}

pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()
}

pub fn read_progress(dir: &Path) -> Option<Progress> {
    serde_json::from_slice(&std::fs::read(dir.join(PROGRESS)).ok()?).ok()
}

/// Write `value` as JSON to `dir/name` through a temp file, so a reader never sees half of it.
pub fn write_json(dir: &Path, name: &str, value: &impl Serialize) -> AppResult<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(value).map_err(|e| crate::error::AppError::Other(e.to_string()))?)?;
    std::fs::rename(&tmp, dir.join(name))?;
    Ok(())
}

pub fn write_progress(dir: &Path, progress: &Progress) {
    if let Err(e) = write_json(dir, PROGRESS, progress) {
        log::warn!(target: "image_diff", "writing progress failed: {e}");
    }
}

/// Resolve a `crimg://` request path (`<review id>/<file under the cache dir>`) to a PNG inside
/// the cache. Anything that could leave the directory, or isn't a PNG, is refused.
pub fn resolve_asset(db_path: &Path, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    let (id, rel) = trimmed.split_once('/')?;
    let review_id: i64 = id.parse().ok()?;
    if !rel.to_ascii_lowercase().ends_with(".png") || rel.contains('\\') {
        return None;
    }
    let rel = Path::new(rel);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let dir = cache_dir(db_path, review_id).canonicalize().ok()?;
    let file = dir.join(rel).canonicalize().ok()?;
    file.starts_with(&dir).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cr-imgdiff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn cache_dir_is_per_db_stem_and_review() {
        let db = Path::new("/data/com.codereview.app/codereview-dev.db");
        assert_eq!(
            cache_dir(db, 405),
            Path::new("/data/com.codereview.app/image_diffs/codereview-dev/405")
        );
    }

    #[test]
    fn pin_key_changes_with_the_pin() {
        let a = pin_key(Some("b"), Some("h"), None);
        assert_eq!(a, pin_key(Some("b"), Some("h"), None));
        assert_ne!(a, pin_key(Some("b"), Some("h2"), None));
        assert_ne!(a, pin_key(Some("b"), Some("h"), Some(("x", "y"))));
    }

    #[test]
    fn resolve_asset_serves_only_pngs_inside_the_cache() {
        let root = tmp("resolve");
        let db = root.join("codereview.db");
        let dir = cache_dir(&db, 7);
        std::fs::create_dir_all(dir.join("f0")).unwrap();
        std::fs::write(dir.join("f0/diff.png"), b"png").unwrap();
        std::fs::write(dir.join(MANIFEST), b"{}").unwrap();
        std::fs::write(root.join("secret.png"), b"x").unwrap();

        assert!(resolve_asset(&db, "/7/f0/diff.png").is_some());
        assert!(resolve_asset(&db, "7/f0/diff.png").is_some());
        assert!(resolve_asset(&db, "/7/manifest.json").is_none(), "not a png");
        assert!(resolve_asset(&db, "/7/../../secret.png").is_none(), "traversal");
        assert!(resolve_asset(&db, "/7//etc/passwd.png").is_none(), "absolute");
        assert!(resolve_asset(&db, "/7/f0\\..\\diff.png").is_none(), "backslash");
        assert!(resolve_asset(&db, "/8/f0/diff.png").is_none(), "other review");
        assert!(resolve_asset(&db, "/x/f0/diff.png").is_none(), "bad id");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remove_cache_is_quiet_when_missing() {
        remove_cache(Path::new("/nonexistent/image_diffs/1"));
    }

    #[test]
    fn json_roundtrip_is_atomic_and_readable() {
        let dir = tmp("json");
        write_progress(&dir, &Progress { state: "running".into(), done: 1, total: 3, error: None });
        let p = read_progress(&dir).unwrap();
        assert_eq!((p.state.as_str(), p.done, p.total), ("running", 1, 3));
        assert!(!dir.join(".progress.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
