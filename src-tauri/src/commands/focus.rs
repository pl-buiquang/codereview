use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::review::{add_comment_impl, add_file_comment_impl, review_diff_impl};
use crate::db::models::Comment;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::git;

pub const TIERS: [&str; 4] = ["trivial", "skim", "review", "critical"];
pub const LEVELS: [&str; 4] = ["off", "thorough", "focus", "critical"];

const PREVIEW_LINES: usize = 6;
const PREVIEW_WIDTH: usize = 160;
const SNIPPET_CONTEXT: usize = 3;

const LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "bun.lockb",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "go.sum",
    "composer.lock",
    "flake.lock",
    "mix.lock",
    "pubspec.lock",
    "Podfile.lock",
    "packages.lock.json",
];

/// One hunk of the review's current diff. Files without hunks (rename-only,
/// mode-only, binary, empty) are represented by a single pseudo-hunk with
/// `old_start = new_start = 0` and an empty header.
#[derive(Debug, Clone, Serialize)]
pub struct DiffHunk {
    pub hunk_id: String,
    pub file_path: String,
    pub old_start: i64,
    pub new_start: i64,
    pub header: String,
    #[serde(skip)]
    pub body: Vec<String>,
    pub additions: i64,
    pub deletions: i64,
    pub rule_tier: Option<String>,
    pub rule_reason: Option<String>,
    pub preview: Vec<String>,
}

impl DiffHunk {
    pub fn is_pseudo(&self) -> bool {
        self.header.is_empty()
    }

    pub fn text(&self) -> String {
        let mut out = self.header.clone();
        for l in &self.body {
            out.push('\n');
            out.push_str(l);
        }
        out
    }
}

#[derive(Default)]
struct FileAcc {
    old_path: Option<String>,
    new_path: Option<String>,
    deleted: bool,
    binary: bool,
    renamed: bool,
    mode_changed: bool,
    hunks: Vec<RawHunk>,
}

struct RawHunk {
    old_start: i64,
    new_start: i64,
    header: String,
    body: Vec<String>,
}

impl FileAcc {
    fn display_path(&self) -> String {
        let path = if self.deleted {
            self.old_path.as_ref().or(self.new_path.as_ref())
        } else {
            self.new_path.as_ref().or(self.old_path.as_ref())
        };
        path.cloned().unwrap_or_default()
    }
}

fn unquote(p: &str) -> &str {
    let p = p.trim_end_matches('\t');
    p.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(p)
}

fn strip_side_prefix(p: &str) -> Option<String> {
    let p = unquote(p);
    if p == "/dev/null" {
        return None;
    }
    let p = p
        .strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p);
    Some(p.to_string())
}

/// Split the `a/X b/Y` tail of a `diff --git` line. Unrenamed files have X == Y,
/// which disambiguates paths containing " b/".
fn split_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    if rest.len() >= 5 && (rest.len() - 5).is_multiple_of(2) {
        let n = (rest.len() - 5) / 2;
        if rest.is_char_boundary(2 + n) && rest.is_char_boundary(3 + n) {
            let (a, b) = (&rest[..2 + n], &rest[3 + n..]);
            if a.starts_with("a/") && b.starts_with("b/") && a[2..] == b[2..] {
                return (Some(a[2..].to_string()), Some(b[2..].to_string()));
            }
        }
    }
    match rest.rfind(" b/") {
        Some(i) => (strip_side_prefix(&rest[..i]), strip_side_prefix(&rest[i + 1..])),
        None => (None, None),
    }
}

/// `@@ -a,b +c,d @@` → (old_start, old_len, new_start, new_len).
fn parse_hunk_header(line: &str) -> Option<(i64, i64, i64, i64)> {
    let inner = line.strip_prefix("@@ ")?;
    let end = inner.find(" @@")?;
    let mut parts = inner[..end].split(' ');
    let range = |s: &str, sign: char| -> Option<(i64, i64)> {
        let s = s.strip_prefix(sign)?;
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (os, ol) = range(parts.next()?, '-')?;
    let (ns, nl) = range(parts.next()?, '+')?;
    Some((os, ol, ns, nl))
}

fn split_files(diff: &str) -> Vec<FileAcc> {
    let mut files: Vec<FileAcc> = Vec::new();
    let mut remaining: Option<(i64, i64)> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (a, b) = split_git_paths(rest);
            files.push(FileAcc {
                old_path: a,
                new_path: b,
                ..Default::default()
            });
            remaining = None;
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if let Some((old_rem, new_rem)) = remaining.as_mut() {
            let hunk = f.hunks.last_mut().expect("remaining implies a hunk");
            match line.as_bytes().first() {
                Some(b'+') => *new_rem -= 1,
                Some(b'-') => *old_rem -= 1,
                Some(b'\\') => {}
                _ => {
                    *old_rem -= 1;
                    *new_rem -= 1;
                }
            }
            hunk.body.push(if line.is_empty() { " ".into() } else { line.to_string() });
            if *old_rem <= 0 && *new_rem <= 0 {
                remaining = None;
            }
            continue;
        }
        if line.starts_with('\\') {
            if let Some(h) = f.hunks.last_mut() {
                h.body.push(line.to_string());
            }
        } else if line.starts_with("@@") {
            if let Some((os, ol, ns, nl)) = parse_hunk_header(line) {
                f.hunks.push(RawHunk {
                    old_start: os,
                    new_start: ns,
                    header: line.to_string(),
                    body: Vec::new(),
                });
                if ol > 0 || nl > 0 {
                    remaining = Some((ol, nl));
                }
            }
        } else if let Some(p) = line.strip_prefix("--- ") {
            if let Some(p) = strip_side_prefix(p) {
                f.old_path = Some(p);
            }
        } else if let Some(p) = line.strip_prefix("+++ ") {
            if let Some(p) = strip_side_prefix(p) {
                f.new_path = Some(p);
            }
        } else if let Some(p) = line.strip_prefix("rename from ") {
            f.renamed = true;
            f.old_path = Some(unquote(p).to_string());
        } else if let Some(p) = line.strip_prefix("rename to ") {
            f.renamed = true;
            f.new_path = Some(unquote(p).to_string());
        } else if line.starts_with("deleted file mode") {
            f.deleted = true;
        } else if line.starts_with("old mode") || line.starts_with("new mode") {
            f.mode_changed = true;
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            f.binary = true;
        }
    }
    files
}

fn hunk_id(path: &str, changed: &str, occurrence: usize) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(path.as_bytes());
    h.update(b"\0");
    h.update(changed.as_bytes());
    h.update(b"\0");
    h.update(occurrence.to_string().as_bytes());
    h.digest().to_string()[..12].to_string()
}

fn is_lockfile(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    LOCKFILES.contains(&base)
}

fn is_snapshot(path: &str) -> bool {
    path.contains("__snapshots__/") || path.ends_with(".snap")
}

/// A generated-code marker sitting in a comment, so code that merely mentions
/// the marker string isn't flagged.
fn has_generated_marker(body: &[String]) -> bool {
    body.iter().any(|l| {
        if l.starts_with('-') {
            return false;
        }
        let t = l.get(1..).unwrap_or("").trim_start();
        let commented = ["//", "#", "/*", "*", "<!--", "--", ";"]
            .iter()
            .any(|c| t.starts_with(c));
        commented
            && (t.contains("@generated")
                || (t.contains("Code generated") && t.contains("DO NOT EDIT")))
    })
}

fn preview(body: &[String]) -> Vec<String> {
    body.iter()
        .filter(|l| l.starts_with('+') || l.starts_with('-'))
        .take(PREVIEW_LINES)
        .map(|l| l.chars().take(PREVIEW_WIDTH).collect())
        .collect()
}

/// Split a multi-file unified diff into hunks with stable ids and rule tiers.
pub fn parse_diff_hunks(diff: &str) -> Vec<DiffHunk> {
    let mut out = Vec::new();
    for f in split_files(diff) {
        let path = f.display_path();
        let file_rule = if f.binary {
            Some("binary")
        } else if is_lockfile(&path) {
            Some("lockfile")
        } else if is_snapshot(&path) {
            Some("snapshot")
        } else if f.hunks.iter().any(|h| has_generated_marker(&h.body)) {
            Some("generated")
        } else {
            None
        };
        if f.hunks.is_empty() {
            let reason = file_rule.unwrap_or(if f.renamed {
                "renamed"
            } else if f.mode_changed {
                "mode change"
            } else {
                "empty file"
            });
            out.push(DiffHunk {
                hunk_id: hunk_id(&path, "", 0),
                file_path: path,
                old_start: 0,
                new_start: 0,
                header: String::new(),
                body: Vec::new(),
                additions: 0,
                deletions: 0,
                rule_tier: Some("trivial".into()),
                rule_reason: Some(reason.into()),
                preview: Vec::new(),
            });
            continue;
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for h in f.hunks {
            let changed: Vec<&str> = h
                .body
                .iter()
                .filter(|l| l.starts_with('+') || l.starts_with('-'))
                .map(String::as_str)
                .collect();
            let changed = changed.join("\n");
            let occ = seen.entry(changed.clone()).or_insert(0);
            let id = hunk_id(&path, &changed, *occ);
            *occ += 1;
            out.push(DiffHunk {
                hunk_id: id,
                file_path: path.clone(),
                old_start: h.old_start,
                new_start: h.new_start,
                additions: h.body.iter().filter(|l| l.starts_with('+')).count() as i64,
                deletions: h.body.iter().filter(|l| l.starts_with('-')).count() as i64,
                rule_tier: file_rule.map(|_| "trivial".into()),
                rule_reason: file_rule.map(Into::into),
                preview: preview(&h.body),
                header: h.header,
                body: h.body,
            });
        }
    }
    out
}

fn apply_generated(hunks: &mut [DiffHunk], generated: &HashSet<String>) {
    for h in hunks.iter_mut() {
        if h.rule_tier.is_none() && generated.contains(&h.file_path) {
            h.rule_tier = Some("trivial".into());
            h.rule_reason = Some("generated".into());
        }
    }
}

fn review_local_path(conn: &Connection, review_id: i64) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT repo.local_path FROM review r
         JOIN target t ON t.id = r.target_id
         JOIN repository repo ON repo.id = t.repo_id
         WHERE r.id = ?1",
        params![review_id],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// The review's current hunks, with `.gitattributes` `linguist-generated`
/// applied when a local clone is linked.
fn current_hunks(conn: &Connection, review_id: i64) -> AppResult<Vec<DiffHunk>> {
    let diff = review_diff_impl(conn, review_id)?;
    let mut hunks = parse_diff_hunks(&diff);
    if let Some(local) = review_local_path(conn, review_id)? {
        let mut paths: Vec<&str> = hunks
            .iter()
            .filter(|h| h.rule_tier.is_none())
            .map(|h| h.file_path.as_str())
            .collect();
        paths.dedup();
        if let Ok(generated) = git::linguist_generated(Path::new(&local), &paths) {
            let generated: HashSet<String> = generated.into_iter().collect();
            apply_generated(&mut hunks, &generated);
        }
    }
    Ok(hunks)
}

// ---------------------------------------------------------------------------
// Persistence

#[derive(Debug, Serialize)]
pub struct Focus {
    pub id: i64,
    pub review_id: i64,
    pub head_sha: Option<String>,
    pub base_sha: Option<String>,
    pub level: String,
    pub summary: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub planned_at: Option<String>,
    pub created_at: String,
}

impl Focus {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            review_id: row.get("review_id")?,
            head_sha: row.get("head_sha")?,
            base_sha: row.get("base_sha")?,
            level: row.get("level")?,
            summary: row.get("summary")?,
            model: row.get("model")?,
            status: row.get("status")?,
            error: row.get("error")?,
            planned_at: row.get("planned_at")?,
            created_at: row.get("created_at")?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct FocusEntry {
    pub hunk_id: String,
    pub file_path: String,
    pub old_start: i64,
    pub new_start: i64,
    pub header: String,
    pub additions: i64,
    pub deletions: i64,
    pub tier: String,
    pub source: Option<String>,
    pub reason: Option<String>,
    #[serde(rename = "override")]
    pub override_: Option<String>,
    pub is_new: bool,
}

#[derive(Debug, Serialize)]
pub struct FileOverride {
    pub file_path: String,
    #[serde(rename = "override")]
    pub override_: String,
}

#[derive(Debug, Serialize)]
pub struct FocusView {
    pub focus: Focus,
    pub stale: bool,
    pub hunks: Vec<FocusEntry>,
    pub file_overrides: Vec<FileOverride>,
}

#[derive(Debug, Deserialize)]
pub struct Plan {
    pub summary: Option<String>,
    #[serde(default)]
    pub hunks: Vec<PlanHunk>,
}

#[derive(Debug, Deserialize)]
pub struct PlanHunk {
    pub id: String,
    pub tier: String,
    pub reason: Option<String>,
    pub note: Option<String>,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn validate_level(level: &str) -> AppResult<()> {
    if LEVELS.contains(&level) {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "invalid level '{level}' (expected one of {})",
            LEVELS.join(", ")
        )))
    }
}

fn validate_override(value: Option<&str>) -> AppResult<()> {
    match value {
        None | Some("show") | Some("hide") => Ok(()),
        Some(v) => Err(AppError::Other(format!(
            "invalid override '{v}' (expected show or hide)"
        ))),
    }
}

fn load_focus(conn: &Connection, review_id: i64) -> AppResult<Option<Focus>> {
    conn.query_row(
        "SELECT * FROM focus WHERE review_id = ?1",
        params![review_id],
        Focus::from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn focus_id(conn: &Connection, review_id: i64) -> AppResult<i64> {
    conn.query_row(
        "SELECT id FROM focus WHERE review_id = ?1",
        params![review_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::Other(format!("review {review_id} has no focus")))
}

fn target_shas(conn: &Connection, review_id: i64) -> AppResult<(Option<String>, Option<String>)> {
    conn.query_row(
        "SELECT t.head_sha, t.base_sha FROM target t JOIN review r ON r.target_id = t.id
         WHERE r.id = ?1",
        params![review_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(Into::into)
}

struct StoredHunk {
    tier: String,
    source: String,
    reason: Option<String>,
}

pub fn get_focus_from(
    conn: &Connection,
    review_id: i64,
    hunks: &[DiffHunk],
) -> AppResult<Option<FocusView>> {
    let Some(focus) = load_focus(conn, review_id)? else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT hunk_id, tier, source, reason FROM focus_hunk
         WHERE focus_id = ?1",
    )?;
    let stored: HashMap<String, StoredHunk> = stmt
        .query_map(params![focus.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                StoredHunk {
                    tier: r.get(1)?,
                    source: r.get(2)?,
                    reason: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt =
        conn.prepare("SELECT hunk_id, override FROM focus_hunk_override WHERE focus_id = ?1")?;
    let overrides: HashMap<String, String> = stmt
        .query_map(params![focus.id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(
        "SELECT file_path, override FROM focus_file_override WHERE focus_id = ?1 ORDER BY file_path",
    )?;
    let file_overrides = stmt
        .query_map(params![focus.id], |r| {
            Ok(FileOverride {
                file_path: r.get(0)?,
                override_: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let planned = focus.planned_at.is_some();
    let entries = hunks
        .iter()
        .map(|h| {
            let base = FocusEntry {
                hunk_id: h.hunk_id.clone(),
                file_path: h.file_path.clone(),
                old_start: h.old_start,
                new_start: h.new_start,
                header: h.header.clone(),
                additions: h.additions,
                deletions: h.deletions,
                tier: "review".into(),
                source: None,
                reason: None,
                override_: overrides.get(&h.hunk_id).cloned(),
                is_new: false,
            };
            match stored.get(&h.hunk_id) {
                Some(s) => FocusEntry {
                    tier: s.tier.clone(),
                    source: Some(s.source.clone()),
                    reason: s.reason.clone(),
                    ..base
                },
                None if planned => FocusEntry { is_new: true, ..base },
                None => base,
            }
        })
        .collect();

    let (head, base) = target_shas(conn, review_id)?;
    let stale = planned && (focus.head_sha != head || focus.base_sha != base);
    Ok(Some(FocusView {
        focus,
        stale,
        hunks: entries,
        file_overrides,
    }))
}

pub fn set_focus_plan_from(
    conn: &Connection,
    review_id: i64,
    hunks: &[DiffHunk],
    plan: Plan,
    level: Option<&str>,
) -> AppResult<FocusView> {
    if let Some(l) = level {
        validate_level(l)?;
    }
    let known: HashSet<&str> = hunks.iter().map(|h| h.hunk_id.as_str()).collect();
    let mut planned: HashMap<&str, &PlanHunk> = HashMap::new();
    let mut unknown = Vec::new();
    for p in &plan.hunks {
        if !TIERS.contains(&p.tier.as_str()) {
            return Err(AppError::Other(format!(
                "hunk {}: invalid tier '{}' (expected one of {})",
                p.id,
                p.tier,
                TIERS.join(", ")
            )));
        }
        if !known.contains(p.id.as_str()) {
            unknown.push(p.id.clone());
        } else if planned.insert(p.id.as_str(), p).is_some() {
            return Err(AppError::Other(format!("hunk {} listed twice", p.id)));
        }
    }
    if !unknown.is_empty() {
        return Err(AppError::Other(format!(
            "unknown hunk ids (nothing written): {}",
            unknown.join(", ")
        )));
    }

    let (head, base) = target_shas(conn, review_id)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO focus (review_id, head_sha, base_sha, level, summary, status, planned_at)
         VALUES (?1, ?2, ?3, COALESCE(?4, 'focus'), ?5, 'ready', ?6)
         ON CONFLICT(review_id) DO UPDATE SET
           head_sha = excluded.head_sha,
           base_sha = excluded.base_sha,
           level = COALESCE(?4, focus.level),
           summary = excluded.summary,
           status = 'ready',
           error = NULL,
           planned_at = excluded.planned_at",
        params![review_id, head, base, level, plan.summary, now()],
    )?;
    let fid = focus_id(&tx, review_id)?;

    // Drop this focus's AI comments from the previous run: delete the ones
    // still tagged `ai` (untouched since then — cascades its focus_comment
    // link), then drop the link for any that were edited/retagged instead,
    // which leaves the comment itself as the user's own.
    {
        let mut stmt = tx.prepare(
            "SELECT fc.comment_id FROM focus_comment fc
             JOIN comment c ON c.id = fc.comment_id
             WHERE fc.focus_id = ?1 AND c.tag = 'ai'",
        )?;
        let ai_ids: Vec<i64> = stmt
            .query_map(params![fid], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in ai_ids {
            tx.execute("DELETE FROM comment WHERE id = ?1", params![id])?;
        }
        tx.execute("DELETE FROM focus_comment WHERE focus_id = ?1", params![fid])?;
    }

    tx.execute("DELETE FROM focus_hunk WHERE focus_id = ?1", params![fid])?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO focus_hunk
               (focus_id, hunk_id, file_path, old_start, new_start, tier, source, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for h in hunks {
            let (tier, source, reason, note) = match planned.get(h.hunk_id.as_str()) {
                Some(p) => (
                    p.tier.clone(),
                    "ai",
                    p.reason.clone().or_else(|| h.rule_reason.clone()),
                    p.note.clone().filter(|n| !n.trim().is_empty()),
                ),
                None => match &h.rule_tier {
                    Some(t) => (t.clone(), "rule", h.rule_reason.clone(), None),
                    None => ("review".into(), "default", None, None),
                },
            };
            ins.execute(params![
                fid,
                h.hunk_id,
                h.file_path,
                h.old_start,
                h.new_start,
                tier,
                source,
                reason,
            ])?;
            if let Some(note) = note {
                let comment_id = note_comment(&tx, review_id, h, &note)?;
                tx.execute(
                    "INSERT INTO focus_comment (focus_id, comment_id) VALUES (?1, ?2)",
                    params![fid, comment_id],
                )?;
            }
        }
    }
    tx.commit()?;
    get_focus_from(conn, review_id, hunks)?
        .ok_or_else(|| AppError::Other("focus vanished after write".into()))
}

pub fn set_focus_level_impl(conn: &Connection, review_id: i64, level: &str) -> AppResult<()> {
    validate_level(level)?;
    let fid = focus_id(conn, review_id)?;
    conn.execute("UPDATE focus SET level = ?1 WHERE id = ?2", params![level, fid])?;
    Ok(())
}

pub fn set_hunk_override_impl(
    conn: &Connection,
    review_id: i64,
    hunk_id: &str,
    value: Option<&str>,
) -> AppResult<()> {
    validate_override(value)?;
    let fid = focus_id(conn, review_id)?;
    match value {
        Some(v) => conn.execute(
            "INSERT INTO focus_hunk_override (focus_id, hunk_id, override) VALUES (?1, ?2, ?3)
             ON CONFLICT(focus_id, hunk_id) DO UPDATE SET override = excluded.override",
            params![fid, hunk_id, v],
        )?,
        None => conn.execute(
            "DELETE FROM focus_hunk_override WHERE focus_id = ?1 AND hunk_id = ?2",
            params![fid, hunk_id],
        )?,
    };
    Ok(())
}

pub fn set_file_override_impl(
    conn: &Connection,
    review_id: i64,
    file_path: &str,
    value: Option<&str>,
) -> AppResult<()> {
    validate_override(value)?;
    let fid = focus_id(conn, review_id)?;
    match value {
        Some(v) => conn.execute(
            "INSERT INTO focus_file_override (focus_id, file_path, override) VALUES (?1, ?2, ?3)
             ON CONFLICT(focus_id, file_path) DO UPDATE SET override = excluded.override",
            params![fid, file_path, v],
        )?,
        None => conn.execute(
            "DELETE FROM focus_file_override WHERE focus_id = ?1 AND file_path = ?2",
            params![fid, file_path],
        )?,
    };
    Ok(())
}

/// Remove the focus and its still-`ai`-tagged comments (edited/retagged ones
/// are kept, same rule as regenerating). Other comments are untouched.
pub fn clear_focus_impl(conn: &Connection, review_id: i64) -> AppResult<()> {
    if let Ok(fid) = focus_id(conn, review_id) {
        let mut stmt = conn.prepare(
            "SELECT fc.comment_id FROM focus_comment fc
             JOIN comment c ON c.id = fc.comment_id
             WHERE fc.focus_id = ?1 AND c.tag = 'ai'",
        )?;
        let ai_ids: Vec<i64> = stmt
            .query_map(params![fid], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in ai_ids {
            conn.execute("DELETE FROM comment WHERE id = ?1", params![id])?;
        }
    }
    conn.execute("DELETE FROM focus WHERE review_id = ?1", params![review_id])?;
    Ok(())
}

/// Mark focus runs interrupted by an app exit as failed.
pub fn fail_interrupted(conn: &Connection) -> AppResult<()> {
    conn.execute(
        "UPDATE focus SET status = 'failed', error = 'generation was interrupted'
         WHERE status = 'generating'",
        [],
    )?;
    Ok(())
}

/// Where an adopted note lands: the first added line (RIGHT), or the first
/// deleted line (LEFT) for delete-only hunks. Returns (side, line, body index).
fn first_changed_line(h: &DiffHunk) -> Option<(&'static str, i64, usize)> {
    let (mut old, mut new) = (h.old_start, h.new_start);
    let mut first_del = None;
    for (i, l) in h.body.iter().enumerate() {
        match l.as_bytes().first() {
            Some(b'+') => return Some(("RIGHT", new, i)),
            Some(b'-') => {
                first_del.get_or_insert(("LEFT", old, i));
                old += 1;
            }
            Some(b'\\') => {}
            _ => {
                old += 1;
                new += 1;
            }
        }
    }
    first_del
}

/// Create a hunk's AI note as an `ai`-tagged draft comment, anchored the same
/// way `cr comment add`/`add-file` would. Returns the new comment's id so the
/// caller can link it in `focus_comment`.
fn note_comment(conn: &Connection, review_id: i64, h: &DiffHunk, note: &str) -> AppResult<i64> {
    let comment = match first_changed_line(h) {
        None => {
            add_file_comment_impl(conn, review_id, h.file_path.clone(), note.into(), Some("ai"))?
        }
        Some((side, line, idx)) => {
            let lo = idx.saturating_sub(SNIPPET_CONTEXT);
            let hi = (idx + SNIPPET_CONTEXT).min(h.body.len() - 1);
            let snippet = std::iter::once(h.header.as_str())
                .chain(h.body[lo..=hi].iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            let (head, _) = target_shas(conn, review_id)?;
            add_comment_impl(
                conn,
                review_id,
                h.file_path.clone(),
                side.to_string(),
                line,
                None,
                Some(snippet),
                note.into(),
                head,
                None,
                Some("ai"),
            )?
        }
    };
    Ok(comment.id)
}

// ---------------------------------------------------------------------------
// Diff-fetching entry points (shared by Tauri commands and `cr`)

pub fn focus_hunks_impl(conn: &Connection, review_id: i64) -> AppResult<Vec<DiffHunk>> {
    current_hunks(conn, review_id)
}

/// Comments the current focus run created (the `ai`-tagged notes), for `cr
/// focus show`. Empty once the review has no focus or none of its notes
/// survive untouched (regenerating/clearing drops the ones still tagged `ai`).
pub fn focus_comments_impl(conn: &Connection, review_id: i64) -> AppResult<Vec<Comment>> {
    let Ok(fid) = focus_id(conn, review_id) else {
        return Ok(vec![]);
    };
    let mut stmt = conn.prepare(
        "SELECT c.* FROM focus_comment fc JOIN comment c ON c.id = fc.comment_id
         WHERE fc.focus_id = ?1 ORDER BY c.file_path, c.line",
    )?;
    let rows = stmt
        .query_map(params![fid], Comment::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn focus_hunk_text_impl(conn: &Connection, review_id: i64, hunk_id: &str) -> AppResult<String> {
    let hunks = current_hunks(conn, review_id)?;
    let h = hunks
        .iter()
        .find(|h| h.hunk_id == hunk_id)
        .ok_or_else(|| AppError::Other(format!("unknown hunk id {hunk_id}")))?;
    if h.is_pseudo() {
        return Ok(format!(
            "{} — no textual hunks ({})",
            h.file_path,
            h.rule_reason.as_deref().unwrap_or("no content")
        ));
    }
    Ok(h.text())
}

pub fn get_focus_impl(conn: &Connection, review_id: i64) -> AppResult<Option<FocusView>> {
    if load_focus(conn, review_id)?.is_none() {
        return Ok(None);
    }
    let hunks = current_hunks(conn, review_id)?;
    get_focus_from(conn, review_id, &hunks)
}

pub fn set_focus_plan_impl(
    conn: &Connection,
    review_id: i64,
    plan: Plan,
    level: Option<&str>,
) -> AppResult<FocusView> {
    let hunks = current_hunks(conn, review_id)?;
    set_focus_plan_from(conn, review_id, &hunks, plan, level)
}

// ---------------------------------------------------------------------------
// In-app runner

const FOCUS_TOOLS: &[&str] = &[
    "Bash(cr focus:*)",
    "Bash(cr diff:*)",
    "Bash(cr show:*)",
    "Bash(cr --json focus:*)",
    "Bash(cr --json show:*)",
    "Read",
    "Glob",
    "Grep",
];
const NO_PLAN_ERROR: &str = "the model finished without submitting a plan (`cr focus set` was never called)";

/// Mark the review's focus as generating (creating the row if needed).
pub fn begin_generation(conn: &Connection, review_id: i64, model: &str) -> AppResult<()> {
    if load_focus(conn, review_id)?.is_some_and(|f| f.status == "generating") {
        return Err(AppError::Other("a focus is already being generated for this review".into()));
    }
    let model = (!model.is_empty()).then_some(model);
    conn.execute(
        "INSERT INTO focus (review_id, status, model) VALUES (?1, 'generating', ?2)
         ON CONFLICT(review_id) DO UPDATE SET status = 'generating', error = NULL, model = excluded.model",
        params![review_id, model],
    )?;
    Ok(())
}

/// Close a run. `cr focus set` already flips the status to ready on success, so
/// a row still `generating` means the run failed or never submitted a plan.
pub fn finish_generation(conn: &Connection, review_id: i64, error: Option<String>) -> AppResult<()> {
    conn.execute(
        "UPDATE focus SET status = 'failed', error = ?2 WHERE review_id = ?1 AND status = 'generating'",
        params![review_id, error.unwrap_or_else(|| NO_PLAN_ERROR.into())],
    )?;
    Ok(())
}

fn app_mode_prompt(skill: &str, review_id: i64, plan_path: &Path) -> String {
    let plan = plan_path.display();
    format!(
        "{skill}\n\n---\n\nApp mode. Review id = {review_id}. Do not create or open reviews. \
         The `cr` CLI on PATH already points at the app's database. \
         Write the plan with the Write tool to `{plan}` (the only writable path), then run \
         `cr focus set {review_id} --file {plan}`."
    )
}

/// PATH for the child with the app's bundled `cr` sidecar first, when present.
fn child_path() -> Option<std::ffi::OsString> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let cr = exe_dir.join(if cfg!(windows) { "cr.exe" } else { "cr" });
    if !cr.exists() {
        return None;
    }
    let mut dirs = vec![exe_dir];
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(dirs).ok()
}

fn run_generation(
    skill: &str,
    review_id: i64,
    model: &str,
    local_path: Option<String>,
    db_path: &Path,
) -> AppResult<()> {
    let temp;
    let cwd = match &local_path {
        Some(p) if Path::new(p).exists() => Path::new(p),
        _ => {
            temp = std::env::temp_dir().join(format!("codereview_focus_{review_id}"));
            std::fs::create_dir_all(&temp)?;
            temp.as_path()
        }
    };
    let mut env = vec![("CODEREVIEW_DB".to_string(), db_path.as_os_str().to_owned())];
    if let Some(path) = child_path() {
        env.push(("PATH".to_string(), path));
    }
    // Plans go through a file: Claude Code refuses Bash commands that carry
    // inline JSON. Canonical so the Edit rule matches (macOS /var → /private/var).
    let scratch = std::env::temp_dir()
        .canonicalize()?
        .join(format!("codereview-focus-{review_id}"));
    std::fs::create_dir_all(&scratch)?;
    let plan_path = scratch.join("plan.json");
    let write_rule = format!("Edit(/{}/**)", scratch.display());
    let mut tools: Vec<&str> = FOCUS_TOOLS.to_vec();
    tools.push(&write_rule);
    let system_prompt = app_mode_prompt(skill, review_id, &plan_path);
    let result = crate::commands::chat::run_claude(crate::commands::chat::ClaudeRun {
        model,
        allowed_tools: &tools,
        permission_mode: "default",
        system_prompt: Some(&system_prompt),
        prompt_file_tag: &format!("focus_{review_id}"),
        cwd,
        extra_dirs: &[scratch.as_path()],
        session: None,
        env,
        extra_args: &[],
        replace_system_prompt: false,
        prompt: &format!("Generate the focus for review {review_id}."),
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result.map(|_| ())
}

#[derive(Clone, Serialize)]
struct FocusUpdated {
    review_id: i64,
}

/// Start a background focus run; emits `focus-updated` when it ends.
#[tauri::command]
pub fn generate_focus(
    app: tauri::AppHandle,
    review_id: i64,
    model: Option<String>,
    db: State<Db>,
    db_path: State<crate::db::DbPath>,
) -> AppResult<()> {
    use tauri::{Emitter, Manager};

    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    let model = crate::commands::chat::resolve_model(model);
    let (local_path, skill) = {
        let conn = db.0.lock().unwrap();
        let skill = crate::prompts::resolve(&conn, crate::prompts::FOCUS_SYSTEM)?;
        begin_generation(&conn, review_id, &model)?;
        (review_local_path(&conn, review_id)?, skill)
    };
    let db_path = db_path.0.clone();
    let _ = app.emit("focus-updated", FocusUpdated { review_id });
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_generation(&skill, review_id, &model, local_path, &db_path);
        {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let _ = finish_generation(&conn, review_id, result.err().map(|e| e.to_string()));
        }
        let _ = app.emit("focus-updated", FocusUpdated { review_id });
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands

#[tauri::command]
pub fn focus_hunks(review_id: i64, db: State<Db>) -> AppResult<Vec<DiffHunk>> {
    let conn = db.0.lock().unwrap();
    focus_hunks_impl(&conn, review_id)
}

#[tauri::command]
pub fn get_focus(review_id: i64, db: State<Db>) -> AppResult<Option<FocusView>> {
    let conn = db.0.lock().unwrap();
    get_focus_impl(&conn, review_id)
}

#[tauri::command]
pub fn set_focus_plan(
    review_id: i64,
    plan: Plan,
    level: Option<String>,
    db: State<Db>,
) -> AppResult<FocusView> {
    let conn = db.0.lock().unwrap();
    set_focus_plan_impl(&conn, review_id, plan, level.as_deref())
}

#[tauri::command]
pub fn set_focus_level(review_id: i64, level: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_focus_level_impl(&conn, review_id, &level)
}

#[tauri::command]
pub fn set_hunk_override(
    review_id: i64,
    hunk_id: String,
    value: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_hunk_override_impl(&conn, review_id, &hunk_id, value.as_deref())
}

#[tauri::command]
pub fn set_file_override(
    review_id: i64,
    file_path: String,
    value: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_file_override_impl(&conn, review_id, &file_path, value.as_deref())
}

#[tauri::command]
pub fn clear_focus(review_id: i64, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    clear_focus_impl(&conn, review_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    /// Shared with `src/lib/focus.test.ts` so both diff parsers are checked on the same text.
    const DIFF: &str = include_str!("focus_fixture.diff");

    fn by_file<'a>(hunks: &'a [DiffHunk], path: &str) -> Vec<&'a DiffHunk> {
        hunks.iter().filter(|h| h.file_path == path).collect()
    }

    #[test]
    fn splits_files_and_hunks_with_display_paths() {
        let hunks = parse_diff_hunks(DIFF);
        let files: Vec<&str> = hunks.iter().map(|h| h.file_path.as_str()).collect();
        assert_eq!(
            files,
            [
                "src/app.rs",
                "src/app.rs",
                "gone.txt",
                "new name.rs",
                "logo.png",
                "Cargo.lock",
                "web/new.ts"
            ]
        );
        let starts: Vec<(i64, i64)> = hunks.iter().map(|h| (h.old_start, h.new_start)).collect();
        assert_eq!(starts, [(1, 1), (10, 11), (1, 0), (0, 0), (0, 0), (5, 5), (0, 1)]);
        let app = by_file(&hunks, "src/app.rs");
        assert_eq!((app[0].old_start, app[0].new_start), (1, 1));
        assert_eq!((app[1].old_start, app[1].new_start), (10, 11));
        assert_eq!(app[0].body.len(), 4, "empty context line kept");
        assert_eq!((app[1].additions, app[1].deletions), (1, 1));
        assert!(app[0].rule_tier.is_none());

        let gone = by_file(&hunks, "gone.txt");
        assert_eq!(gone[0].deletions, 2, "'--' content line is body, not a header");
        let web = by_file(&hunks, "web/new.ts");
        assert_eq!(web[0].body.last().unwrap(), "\\ No newline at end of file");
    }

    #[test]
    fn rules_tier_noise_trivial() {
        let hunks = parse_diff_hunks(DIFF);
        let reason = |p: &str| by_file(&hunks, p)[0].rule_reason.clone();
        assert_eq!(reason("new name.rs").as_deref(), Some("renamed"));
        assert_eq!(reason("logo.png").as_deref(), Some("binary"));
        assert_eq!(reason("Cargo.lock").as_deref(), Some("lockfile"));
        assert_eq!(reason("web/new.ts").as_deref(), Some("generated"));
        assert_eq!(reason("gone.txt"), None);
        let renamed = by_file(&hunks, "new name.rs")[0];
        assert!(renamed.is_pseudo());
        assert_eq!(renamed.rule_tier.as_deref(), Some("trivial"));
    }

    #[test]
    fn generated_marker_must_be_a_comment() {
        assert!(!has_generated_marker(&["+let m = \"@generated\";".into()]));
        assert!(has_generated_marker(&["+# Code generated by x. DO NOT EDIT.".into()]));
        assert!(is_snapshot("src/__snapshots__/a.test.ts.snap"));
    }

    #[test]
    fn hunk_id_survives_line_shifts_and_disambiguates_repeats() {
        let a = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n ctx\n-x\n+y\n";
        let b = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -40,2 +42,2 @@ other\n other ctx\n-x\n+y\n";
        assert_eq!(parse_diff_hunks(a)[0].hunk_id, parse_diff_hunks(b)[0].hunk_id);

        let twice = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,1 +1,1 @@\n-x\n+y\n@@ -9,1 +9,1 @@\n-x\n+y\n";
        let h = parse_diff_hunks(twice);
        assert_eq!(h.len(), 2);
        assert_ne!(h[0].hunk_id, h[1].hunk_id);
        assert_eq!(h[0].hunk_id.len(), 12);
    }

    #[test]
    fn split_git_paths_handles_spaces() {
        assert_eq!(
            split_git_paths("a/x b/y.rs b/x b/y.rs"),
            (Some("x b/y.rs".into()), Some("x b/y.rs".into()))
        );
        assert_eq!(split_git_paths("a/o.rs b/n.rs"), (Some("o.rs".into()), Some("n.rs".into())));
    }

    fn seed_review(conn: &Connection, head: &str) -> i64 {
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, base_sha, head_sha, created_at)
             VALUES (1, 'local', 't', 'main', 'feat', 'base0', ?1, 'now')",
            params![head],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn plan(json: &str) -> Plan {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn set_plan_validates_and_fills_defaults() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        let app = by_file(&hunks, "src/app.rs");

        let err = set_focus_plan_from(&conn, rid, &hunks, plan(r#"{"hunks":[{"id":"nope","tier":"review"}]}"#), None)
            .unwrap_err();
        assert!(err.to_string().contains("nope"));
        assert!(load_focus(&conn, rid).unwrap().is_none(), "nothing written");

        let p = format!(
            r#"{{"summary":"S","hunks":[{{"id":"{}","tier":"critical","note":"check"}}]}}"#,
            app[0].hunk_id
        );
        let view = set_focus_plan_from(&conn, rid, &hunks, plan(&p), Some("thorough")).unwrap();
        assert_eq!(view.focus.level, "thorough");
        assert_eq!(view.focus.status, "ready");
        assert_eq!(view.focus.head_sha.as_deref(), Some("head1"));
        assert!(!view.stale);
        let e = |id: &str| view.hunks.iter().find(|h| h.hunk_id == id).unwrap();
        assert_eq!(e(&app[0].hunk_id).tier, "critical");
        let notes = focus_comments_impl(&conn, rid).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "check");
        assert_eq!(notes[0].tag.as_deref(), Some("ai"));
        assert_eq!(e(&app[1].hunk_id).tier, "review");
        assert_eq!(e(&app[1].hunk_id).source.as_deref(), Some("default"));
        let lock = by_file(&hunks, "Cargo.lock")[0];
        assert_eq!(e(&lock.hunk_id).tier, "trivial");
        assert_eq!(e(&lock.hunk_id).source.as_deref(), Some("rule"));
    }

    #[test]
    fn regenerate_keeps_overrides_and_flags_new_and_stale() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        let first = hunks[0].hunk_id.clone();
        set_focus_plan_from(&conn, rid, &hunks, plan("{}"), None).unwrap();
        set_hunk_override_impl(&conn, rid, &first, Some("hide")).unwrap();
        set_file_override_impl(&conn, rid, "Cargo.lock", Some("show")).unwrap();
        set_focus_level_impl(&conn, rid, "critical").unwrap();

        set_focus_plan_from(&conn, rid, &hunks, plan(r#"{"summary":"again"}"#), None).unwrap();
        conn.execute("UPDATE target SET head_sha = 'head2'", []).unwrap();
        let mut moved = hunks.clone();
        moved.push(DiffHunk {
            hunk_id: "fresh0000000".into(),
            ..hunks[0].clone()
        });
        let view = get_focus_from(&conn, rid, &moved).unwrap().unwrap();
        assert_eq!(view.focus.level, "critical", "level kept when not passed");
        assert_eq!(view.focus.summary.as_deref(), Some("again"));
        assert!(view.stale);
        assert_eq!(view.hunks[0].override_.as_deref(), Some("hide"));
        assert_eq!(view.file_overrides[0].file_path, "Cargo.lock");
        let fresh = view.hunks.last().unwrap();
        assert!(fresh.is_new);
        assert_eq!(fresh.tier, "review");

        set_hunk_override_impl(&conn, rid, &first, None).unwrap();
        let view = get_focus_from(&conn, rid, &hunks).unwrap().unwrap();
        assert!(view.hunks[0].override_.is_none());
        assert!(set_focus_level_impl(&conn, rid, "loud").is_err());
        assert!(set_hunk_override_impl(&conn, rid, &first, Some("maybe")).is_err());
    }

    #[test]
    fn plan_notes_land_as_ai_tagged_comments_on_the_first_changed_line() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        let app = by_file(&hunks, "src/app.rs");
        let gone = by_file(&hunks, "gone.txt")[0];
        let p = format!(
            r#"{{"hunks":[{{"id":"{}","tier":"review","note":"add?"}},{{"id":"{}","tier":"review","note":"del?"}}]}}"#,
            app[0].hunk_id, gone.hunk_id
        );
        set_focus_plan_from(&conn, rid, &hunks, plan(&p), None).unwrap();

        let notes = focus_comments_impl(&conn, rid).unwrap();
        assert_eq!(notes.len(), 2);
        let add_note = notes.iter().find(|c| c.body == "add?").unwrap();
        assert_eq!((add_note.side.as_str(), add_note.line), ("RIGHT", 2));
        assert_eq!(add_note.anchored_head_sha.as_deref(), Some("head1"));
        assert_eq!(add_note.tag.as_deref(), Some("ai"));
        assert!(add_note.diff_hunk.as_deref().unwrap().starts_with("@@ -1,3 +1,4 @@"));
        let del_note = notes.iter().find(|c| c.body == "del?").unwrap();
        assert_eq!((del_note.side.as_str(), del_note.line), ("LEFT", 1));
    }

    #[test]
    fn regenerating_focus_replaces_untouched_ai_comments_but_keeps_edited_ones() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        let app = by_file(&hunks, "src/app.rs");
        let p = format!(
            r#"{{"hunks":[{{"id":"{}","tier":"review","note":"first"}}]}}"#,
            app[0].hunk_id
        );
        set_focus_plan_from(&conn, rid, &hunks, plan(&p), None).unwrap();
        let first = focus_comments_impl(&conn, rid).unwrap();
        assert_eq!(first.len(), 1);
        let kept_id = first[0].id;
        crate::commands::review::update_comment_impl(&conn, kept_id, "edited by me".into())
            .unwrap();

        // Regenerating drops the link for the edited (now untagged) comment and
        // writes a fresh `ai` comment for the still-planned note.
        set_focus_plan_from(&conn, rid, &hunks, plan(&p), None).unwrap();
        let after = focus_comments_impl(&conn, rid).unwrap();
        assert_eq!(after.len(), 1, "a fresh ai comment replaces the untouched one");
        assert_ne!(after[0].id, kept_id);

        let edited: String = conn
            .query_row("SELECT body FROM comment WHERE id = ?1", params![kept_id], |r| r.get(0))
            .unwrap();
        assert_eq!(edited, "edited by me", "the edited comment survives, just untracked");
    }

    #[test]
    fn clear_focus_removes_only_untouched_ai_comments() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        let app = by_file(&hunks, "src/app.rs");
        let p = format!(
            r#"{{"hunks":[{{"id":"{}","tier":"review","note":"n"}}]}}"#,
            app[0].hunk_id
        );
        set_focus_plan_from(&conn, rid, &hunks, plan(&p), None).unwrap();
        clear_focus_impl(&conn, rid).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0, "the untouched ai comment is removed with the focus");
        assert!(load_focus(&conn, rid).unwrap().is_none());
    }

    #[test]
    fn generation_lifecycle() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);

        begin_generation(&conn, rid, "opus").unwrap();
        assert!(begin_generation(&conn, rid, "opus").is_err(), "one run at a time");
        finish_generation(&conn, rid, None).unwrap();
        let f = load_focus(&conn, rid).unwrap().unwrap();
        assert_eq!((f.status.as_str(), f.model.as_deref()), ("failed", Some("opus")));
        assert_eq!(f.error.as_deref(), Some(NO_PLAN_ERROR));

        begin_generation(&conn, rid, "").unwrap();
        set_focus_plan_from(&conn, rid, &hunks, plan(r#"{"summary":"ok"}"#), Some("critical")).unwrap();
        finish_generation(&conn, rid, Some("claude exited 1".into())).unwrap();
        let f = load_focus(&conn, rid).unwrap().unwrap();
        assert_eq!(f.status, "ready", "a submitted plan wins over a late error");
        assert_eq!(f.level, "critical");
        assert!(f.error.is_none());
        let prompt = app_mode_prompt("skill", rid, Path::new("/tmp/p.json"));
        assert!(prompt.contains("App mode. Review id = 1."));
        assert!(prompt.contains("--file /tmp/p.json"));
    }

    /// Runs the real `claude` CLI. `cargo build --bin cr` first, then:
    /// `cargo test live_generation -- --ignored`.
    #[test]
    #[ignore]
    fn live_generation() {
        use std::process::Command;
        crate::tools::init();
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success())
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("auth.py"), "def check(u):\n    return u.admin\n").unwrap();
        std::fs::write(repo.join("Cargo.lock"), "v=1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        git(&["checkout", "-qb", "feat"]);
        std::fs::write(repo.join("auth.py"), "def check(u):\n    return True\n").unwrap();
        std::fs::write(repo.join("Cargo.lock"), "v=2\n").unwrap();
        git(&["commit", "-qam", "change"]);

        let db_path = dir.path().join("db.sqlite");
        let rid = {
            let conn = crate::db::open(&db_path).unwrap();
            let repo_s = repo.to_str().unwrap();
            conn.execute(
                "INSERT INTO repository (local_path, default_branch, added_at) VALUES (?1, 'main', 'now')",
                params![repo_s],
            )
            .unwrap();
            let r = crate::commands::review::create_review_impl(&conn, 1, repo_s, "main", "feat", true, false)
                .unwrap();
            begin_generation(&conn, r.id, "").unwrap();
            r.id
        };
        let bin = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        std::env::set_var("PATH", path);

        let skill = crate::prompts::resolve(&crate::db::open(&db_path).unwrap(), crate::prompts::FOCUS_SYSTEM).unwrap();
        let result = run_generation(&skill, rid, "sonnet", Some(repo.to_str().unwrap().into()), &db_path);
        let conn = crate::db::open(&db_path).unwrap();
        finish_generation(&conn, rid, result.err().map(|e| e.to_string())).unwrap();
        let f = load_focus(&conn, rid).unwrap().unwrap();
        assert_eq!(f.status, "ready", "error: {:?}", f.error);
        let tiers: Vec<(String, String)> = conn
            .prepare("SELECT file_path, tier FROM focus_hunk WHERE focus_id = ?1 ORDER BY file_path")
            .unwrap()
            .query_map(params![f.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        eprintln!("summary: {:?}\ntiers: {tiers:?}", f.summary);
        assert!(tiers.contains(&("Cargo.lock".into(), "trivial".into())));
    }

    #[test]
    fn clear_and_cascade_and_interrupted() {
        let conn = open_memory();
        let rid = seed_review(&conn, "head1");
        let hunks = parse_diff_hunks(DIFF);
        set_focus_plan_from(&conn, rid, &hunks, plan("{}"), None).unwrap();
        clear_focus_impl(&conn, rid).unwrap();
        assert!(get_focus_from(&conn, rid, &hunks).unwrap().is_none());

        conn.execute(
            "INSERT INTO focus (review_id, status) VALUES (?1, 'generating')",
            params![rid],
        )
        .unwrap();
        fail_interrupted(&conn).unwrap();
        assert_eq!(load_focus(&conn, rid).unwrap().unwrap().status, "failed");

        conn.execute("DELETE FROM review WHERE id = ?1", params![rid]).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM focus", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
