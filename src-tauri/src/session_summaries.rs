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
    /// Written by the app's Generate button rather than the user's SessionEnd hook.
    pub generated: bool,
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
            generated: false,
        },
    })
}

/// Split the `Title: …` first line (as the SessionEnd hook does) from the markdown body.
pub fn split_title(output: &str) -> (Option<String>, String) {
    let output = output.trim_start();
    let (first, rest) = output.split_once('\n').unwrap_or((output, ""));
    match first.trim().strip_prefix("Title:").map(str::trim).filter(|t| !t.is_empty()) {
        Some(title) => (Some(title.to_string()), rest.trim().to_string()),
        None => (None, output.trim().to_string()),
    }
}

/// The conversation as `USER: …` / `ASSISTANT: …` text blocks (meta, sidechain and empty turns
/// dropped), cut to its last `max_bytes`.
pub fn transcript_extract(path: &Path, max_bytes: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut out = String::new();
    for line in content.lines().filter(|l| l.contains("\"message\"")) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let role = match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => "USER",
            Some("assistant") => "ASSISTANT",
            _ => continue,
        };
        let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        if flag("isMeta") || flag("isSidechain") {
            continue;
        }
        let text = match v.pointer("/message/content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(parts)) => parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        out.push_str(&format!("{role}: {text}\n\n"));
    }
    if out.is_empty() {
        return None;
    }
    if out.len() > max_bytes {
        let mut cut = out.len() - max_bytes;
        while !out.is_char_boundary(cut) {
            cut += 1;
        }
        out = out[cut..].to_string();
    }
    Some(out)
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

    #[test]
    fn splits_title_line() {
        assert_eq!(
            split_title("Title: Fix the thing\n\n## Goal\nDo it.\n"),
            (Some("Fix the thing".into()), "## Goal\nDo it.".into())
        );
        assert_eq!(split_title("## Goal\nDo it."), (None, "## Goal\nDo it.".into()));
        assert_eq!(split_title("Title:\nbody"), (None, "Title:\nbody".into()));
    }

    #[test]
    fn extracts_conversation_text_only() {
        let dir = std::env::temp_dir().join(format!("cr-extract-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let lines = [
            r#"{"type":"user","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"x"},{"type":"text","text":"there"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"side"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"x"}]}}"#,
            r#"{"type":"last-prompt","lastPrompt":"x"}"#,
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        let full = transcript_extract(&path, 1000).unwrap();
        let tail = transcript_extract(&path, 8).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(full, "USER: hello\n\nASSISTANT: hi\nthere\n\n");
        assert_eq!(tail, "\nthere\n\n");
    }
}
