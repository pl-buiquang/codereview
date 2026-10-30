//! Visual diffs of the PNG/JPEG/SVG files a review changes: generated in the background into an
//! on-disk cache (`store`), shown by the image diff panel and described by an AI report.
//!
//! `pixelmatch` compares two rasters, `regions` groups the changed pixels into crops worth a
//! close look, `job` runs a whole review's files and `blob` reads the image bytes.

pub mod blob;
pub mod job;
pub mod pixelmatch;
pub mod regions;
pub mod report;
pub mod runner;
pub mod store;

use std::io::Cursor;

use store::Change;

/// A review is gone: drop its generated image diffs.
pub fn forget_review(conn: &rusqlite::Connection, review_id: i64) {
    runner::cancel(review_id);
    if let Some(dir) = store::cache_dir_for_conn(conn, review_id) {
        store::remove_cache(&dir);
    }
}

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "svg"];

pub fn is_image_path(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

pub fn is_svg_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("svg"))
}

/// One image file a review changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFile {
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    pub change: Change,
}

impl ImageFile {
    /// The path of the left (before) side.
    pub fn left_path(&self) -> &str {
        self.old_path.as_deref().unwrap_or(&self.path)
    }
}

#[derive(Default)]
struct Header {
    old: Option<String>,
    new: Option<String>,
    added: bool,
    deleted: bool,
    renamed: bool,
    /// Bytes changed: a hunk or a "Binary files differ" line (a pure rename or mode change has none).
    content: bool,
}

/// The image files a unified diff changes (read from its `diff --git` headers only).
pub fn changed_image_files(diff: &str) -> Vec<ImageFile> {
    let mut headers: Vec<Header> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (old, new) = crate::commands::focus::split_git_paths(rest);
            headers.push(Header { old, new, ..Default::default() });
            continue;
        }
        let Some(h) = headers.last_mut() else { continue };
        if line.starts_with("new file mode") {
            h.added = true;
        } else if line.starts_with("deleted file mode") {
            h.deleted = true;
        } else if let Some(p) = line.strip_prefix("rename from ") {
            h.renamed = true;
            h.old = Some(p.to_string());
        } else if let Some(p) = line.strip_prefix("rename to ") {
            h.new = Some(p.to_string());
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" || line.starts_with("@@") {
            h.content = true;
        }
    }
    headers
        .into_iter()
        .filter_map(|h| {
            if !(h.added || h.deleted || h.content) {
                return None;
            }
            let path = if h.deleted { h.old.clone().or(h.new.clone())? } else { h.new.clone().or(h.old.clone())? };
            if !is_image_path(&path) {
                return None;
            }
            let (change, old_path) = if h.added {
                (Change::Added, None)
            } else if h.deleted {
                (Change::Deleted, None)
            } else if h.renamed && h.old != h.new {
                (Change::Renamed, h.old)
            } else {
                (Change::Modified, None)
            };
            Some(ImageFile { path, old_path, change })
        })
        .collect()
}

fn numeric_prefix(v: &str) -> Option<f64> {
    let v = v.trim();
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(v.len());
    // Percentages say nothing about an absolute size.
    if v[end..].trim_start().starts_with('%') {
        return None;
    }
    v[..end].parse().ok()
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        let before_ok = tag[..at].chars().last().is_none_or(char::is_whitespace);
        let rest = tag[at + name.len()..].trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                let inner = &rest[1..];
                return inner.split(quote).next();
            }
        }
        from = at + name.len();
    }
    None
}

/// Intrinsic size of an SVG from the root tag (`width`/`height`, else `viewBox`, else the browser
/// default of 300×150), without parsing the document.
pub fn svg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(16 * 1024)]);
    let start = head.find("<svg")?;
    let tag = &head[start..start + head[start..].find('>')?];
    let view_box = attr(tag, "viewBox").and_then(|v| {
        let n: Vec<f64> = v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        (n.len() == 4).then(|| (n[2], n[3]))
    });
    let width = attr(tag, "width").and_then(numeric_prefix).or(view_box.map(|v| v.0)).unwrap_or(300.0);
    let height = attr(tag, "height").and_then(numeric_prefix).or(view_box.map(|v| v.1)).unwrap_or(150.0);
    Some((width.ceil().max(1.0) as u32, height.ceil().max(1.0) as u32))
}

/// Pixel size of an image from its header, without decoding it.
pub fn probe_dimensions(bytes: &[u8], path: &str) -> Option<(u32, u32)> {
    if is_svg_path(path) {
        return svg_dimensions(bytes);
    }
    image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/app.ts b/src/app.ts
index 1..2 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1 +1 @@
-a
+b
diff --git a/e2e/new.png b/e2e/new.png
new file mode 100644
index 0000000..2
Binary files /dev/null and b/e2e/new.png differ
diff --git a/e2e/old.PNG b/e2e/old.PNG
deleted file mode 100644
index 1..0000000
Binary files a/e2e/old.PNG and /dev/null differ
diff --git a/e2e/mod.png b/e2e/mod.png
index 1..2 100644
Binary files a/e2e/mod.png and b/e2e/mod.png differ
diff --git a/img/a.jpg b/img/b.jpg
similarity index 100%
rename from img/a.jpg
rename to img/b.jpg
diff --git a/img/c.png b/img/d.png
similarity index 90%
rename from img/c.png
rename to img/d.png
index 1..2 100644
Binary files a/img/c.png and b/img/d.png differ
diff --git a/logo.svg b/logo.svg
index 1..2 100644
--- a/logo.svg
+++ b/logo.svg
@@ -1 +1 @@
-<svg/>
+<svg />
diff --git a/mode.png b/mode.png
old mode 100644
new mode 100755
";

    #[test]
    fn lists_changed_images_only() {
        let files = changed_image_files(DIFF);
        let got: Vec<(&str, Change, Option<&str>)> =
            files.iter().map(|f| (f.path.as_str(), f.change, f.old_path.as_deref())).collect();
        assert_eq!(
            got,
            vec![
                ("e2e/new.png", Change::Added, None),
                ("e2e/old.PNG", Change::Deleted, None),
                ("e2e/mod.png", Change::Modified, None),
                ("img/d.png", Change::Renamed, Some("img/c.png")),
                ("logo.svg", Change::Modified, None),
            ],
            "a pure rename and a mode change have no pixels to compare"
        );
    }

    #[test]
    fn svg_size_comes_from_width_height_then_viewbox_then_the_default() {
        assert_eq!(svg_dimensions(br#"<svg xmlns="x" width="20px" height='10'>"#), Some((20, 10)));
        assert_eq!(svg_dimensions(br#"<?xml?><svg viewBox="0 0 48 24" width="100%">"#), Some((48, 24)));
        assert_eq!(svg_dimensions(b"<svg>"), Some((300, 150)));
        assert_eq!(svg_dimensions(b"not svg"), None);
    }

    #[test]
    fn extension_checks_ignore_case() {
        assert!(is_image_path("a/B.JPEG") && is_image_path("x.svg") && !is_image_path("x.gif"));
        assert!(!is_image_path("png"));
    }
}
