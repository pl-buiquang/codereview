//! Markdown session summaries written by a user `SessionEnd` hook: YAML-ish frontmatter
//! (`session_id`, `date`, optional `title`) followed by a markdown body.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSummary {
    pub date: Option<String>,
    pub title: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedSummary {
    session_id: String,
    summary: SessionSummary,
}

type Cache = HashMap<PathBuf, (SystemTime, Option<ParsedSummary>)>;

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Summaries under `dir` (recursive), grouped by session id, newest first.
pub fn load(dir: &Path) -> HashMap<String, Vec<SessionSummary>> {
    let mut files = Vec::new();
    collect_md(dir, &mut files);

    let mut guard = CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    let mut out: HashMap<String, Vec<SessionSummary>> = HashMap::new();
    for (path, mtime) in files {
        let parsed = match cache.get(&path) {
            Some((m, p)) if *m == mtime => p.clone(),
            _ => {
                let p = fs::read_to_string(&path).ok().and_then(|s| parse(&s));
                cache.insert(path, (mtime, p.clone()));
                p
            }
        };
        if let Some(p) = parsed {
            out.entry(p.session_id).or_default().push(p.summary);
        }
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| b.date.cmp(&a.date));
    }
    out
}

fn collect_md(dir: &Path, out: &mut Vec<(PathBuf, SystemTime)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            collect_md(&path, out);
        } else if path.extension().is_some_and(|x| x == "md") {
            if let Ok(mtime) = meta.modified() {
                out.push((path, mtime));
            }
        }
    }
}

fn parse(content: &str) -> Option<ParsedSummary> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let (front, after) = rest.split_at(end);
    let body = after["\n---".len()..].trim().to_string();

    let mut fields: HashMap<&str, String> = HashMap::new();
    for line in front.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        let v = if v.starts_with('"') {
            serde_json::from_str::<String>(v).unwrap_or_else(|_| v.trim_matches('"').to_string())
        } else {
            v.to_string()
        };
        fields.insert(k.trim(), v);
    }
    let session_id = fields.remove("session_id").filter(|s| !s.is_empty())?;
    Some(ParsedSummary {
        session_id,
        summary: SessionSummary {
            date: fields.remove("date").filter(|s| !s.is_empty()),
            title: fields.remove("title").filter(|s| !s.is_empty()),
            body,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let p = parse("---\ndate: 2026-10-16 10:25\nsession_id: abc\ncwd: /x\nreason: clear\n---\n\n## Goal\nDo it.\n")
            .unwrap();
        assert_eq!(p.session_id, "abc");
        assert_eq!(p.summary.date.as_deref(), Some("2026-10-16 10:25"));
        assert_eq!(p.summary.title, None);
        assert_eq!(p.summary.body, "## Goal\nDo it.");
    }

    #[test]
    fn decodes_quoted_title() {
        let p = parse("---\nsession_id: abc\ntitle: \"Fix \\\"x\\\": y\"\n---\nbody").unwrap();
        assert_eq!(p.summary.title.as_deref(), Some("Fix \"x\": y"));
    }

    #[test]
    fn plain_title() {
        let p = parse("---\nsession_id: abc\ntitle: Add menu\n---\nbody").unwrap();
        assert_eq!(p.summary.title.as_deref(), Some("Add menu"));
    }

    #[test]
    fn rejects_missing_session_or_frontmatter() {
        assert!(parse("---\ndate: x\n---\nbody").is_none());
        assert!(parse("# Just markdown").is_none());
    }

    #[test]
    fn loads_recursively_newest_first_skipping_dotfiles() {
        let dir = std::env::temp_dir().join(format!("cr-summaries-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.md"), "---\nsession_id: s1\ndate: 2026-01-01 10:00\n---\nold").unwrap();
        fs::write(dir.join("sub/b.md"), "---\nsession_id: s1\ndate: 2026-01-02 10:00\n---\nnew").unwrap();
        fs::write(dir.join(".hidden.md"), "---\nsession_id: s2\n---\nx").unwrap();
        fs::write(dir.join("c.log"), "---\nsession_id: s3\n---\nx").unwrap();
        let map = load(&dir);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(map.len(), 1);
        let bodies: Vec<_> = map["s1"].iter().map(|s| s.body.as_str()).collect();
        assert_eq!(bodies, ["new", "old"]);
    }
}
