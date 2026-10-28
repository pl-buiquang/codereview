//! Search-based "go to definition" for the diff view.
//!
//! No language server and no index: a lookup costs a few `git grep` runs plus a handful of
//! parses, and keeps nothing afterwards, so several big reviews can be open at once.
//!
//! 1. **Shortlist.** One loose `git grep -l` over the pinned commit lists the files that contain
//!    something shaped like a declaration of the symbol.
//! 2. **Confirm.** The first [`MAX_PARSED_FILES`] of them (clicked file, then files in the diff,
//!    then the rest) are read in one `git cat-file --batch` and parsed with tree-sitter, keeping
//!    only declarations whose name is the symbol. Hits in comments and strings fall away here.
//! 3. Languages without a grammar fall back to a strict one-line `git grep -n`.
//!
//! Adding a language: [`languages`].

mod grep;
pub mod languages;
mod tags;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::git;
use languages::{Language, Parser};

/// Hits returned per lookup; common names (`new`, `get`) would otherwise flood the popover.
const MAX_HITS: usize = 10;
/// Lines of source returned with each hit, starting at the declaration.
const SNIPPET_LINES: usize = 20;
/// Candidate files parsed per lookup.
const MAX_PARSED_FILES: usize = 30;
/// Candidate files bigger than this are skipped (generated or minified code).
const MAX_FILE_BYTES: usize = 512 * 1024;
/// Lookups running at once, across every open review.
const MAX_CONCURRENT: usize = 2;

pub const CANCELLED: &str = "cancelled";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionHit {
    pub path: String,
    /// 1-based line of the declaration.
    pub line: u32,
    /// `class`, `method`, ... (`declaration` when only a regex matched).
    pub kind: String,
    /// `SNIPPET_LINES` lines of the file starting at `line`.
    pub snippet: String,
}

// ---- scheduling ------------------------------------------------------------------------------

/// Counting semaphore over blocking threads.
struct Gate {
    free: Mutex<usize>,
    cv: Condvar,
}

struct Permit<'a>(&'a Gate);

impl Gate {
    const fn new(n: usize) -> Self {
        Gate { free: Mutex::new(n), cv: Condvar::new() }
    }

    fn acquire(&self) -> Permit<'_> {
        let mut free = self.free.lock().unwrap();
        while *free == 0 {
            free = self.cv.wait(free).unwrap();
        }
        *free -= 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap() += 1;
        self.0.cv.notify_one();
    }
}

static GATE: Gate = Gate::new(MAX_CONCURRENT);
/// Bumped by every lookup; a hover lookup that is no longer the latest is abandoned.
static LATEST: AtomicU64 = AtomicU64::new(0);

/// Throttled entry point for the command layer. `hover` lookups are speculative: they are
/// dropped (with [`CANCELLED`]) as soon as a newer lookup starts, before the next file is parsed.
pub fn lookup(
    repo: &Path,
    sha: &str,
    symbol: &str,
    from_file: &str,
    diff_files: &[String],
    hover: bool,
) -> AppResult<Vec<DefinitionHit>> {
    let ticket = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    let stale = || hover && LATEST.load(Ordering::SeqCst) != ticket;
    let _permit = GATE.acquire();
    find(repo, sha, symbol, from_file, diff_files, &stale)
}

// ---- lookup ----------------------------------------------------------------------------------

struct Candidate {
    path: String,
    line: u32,
    kind: String,
    /// File contents, when already read (parsed hits).
    source: Option<std::sync::Arc<str>>,
}

/// Declarations of `symbol` at `sha` in `repo`, best first: same file as the click, then files
/// in the diff, then by path. `stale` is polled between files and aborts with [`CANCELLED`].
pub fn find(
    repo: &Path,
    sha: &str,
    symbol: &str,
    from_file: &str,
    diff_files: &[String],
    stale: &dyn Fn() -> bool,
) -> AppResult<Vec<DefinitionHit>> {
    if !grep::is_valid_symbol(symbol) {
        return Ok(vec![]);
    }
    let Some(lang) = Language::for_path(from_file) else {
        return Ok(vec![]);
    };
    let started = std::time::Instant::now();
    let in_diff = |p: &str| p == from_file || diff_files.iter().any(|d| d == p);
    // Resolved once: the git helpers below take a full commit id and do not re-verify it.
    let sha = &git::rev_parse(repo, sha)?;

    let mut found: Vec<Candidate> = Vec::new();

    // 1+2: shortlist, then confirm with tree-sitter.
    let mut files = grep::shortlist(repo, sha, lang, symbol)?;
    files.sort_by(|a, b| {
        (a != from_file, !in_diff(a), a).cmp(&(b != from_file, !in_diff(b), b))
    });
    files.truncate(MAX_PARSED_FILES);
    if !files.is_empty() {
        let sources = git::cat_files(repo, sha, &files, MAX_FILE_BYTES)?;
        for (path, source) in files.into_iter().zip(sources) {
            if stale() {
                return Err(AppError::Other(CANCELLED.into()));
            }
            let (Some(source), Some(parser)) = (source, Parser::for_path(lang, &path)) else {
                continue;
            };
            let defs = tags::definitions_in(parser, &source, symbol)?;
            if defs.is_empty() {
                continue;
            }
            let source: std::sync::Arc<str> = source.into();
            for d in defs {
                found.push(Candidate {
                    path: path.clone(),
                    line: d.line,
                    kind: d.kind,
                    source: Some(source.clone()),
                });
            }
        }
    }

    // 3: files without a grammar.
    if stale() {
        return Err(AppError::Other(CANCELLED.into()));
    }
    for (path, line) in grep::exact_hits(repo, sha, lang, symbol)? {
        found.push(Candidate { path, line, kind: "declaration".into(), source: None });
    }

    found.sort_by(|a, b| {
        (a.path != from_file, !in_diff(&a.path), &a.path, a.line)
            .cmp(&(b.path != from_file, !in_diff(&b.path), &b.path, b.line))
    });
    found.dedup_by(|b, a| a.path == b.path && a.line == b.line);
    found.truncate(MAX_HITS);

    log::debug!(
        target: "definitions",
        "{} `{symbol}`: {} declaration(s) in {}ms",
        lang.name,
        found.len(),
        started.elapsed().as_millis()
    );

    let mut read: HashMap<String, String> = HashMap::new();
    found
        .into_iter()
        .map(|c| {
            let snippet_of = |source: &str| {
                source
                    .lines()
                    .skip(c.line.saturating_sub(1) as usize)
                    .take(SNIPPET_LINES)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let snippet = match &c.source {
                Some(s) => snippet_of(s),
                None => {
                    if !read.contains_key(&c.path) {
                        let text = git::show_file(repo, sha, &c.path)
                            .map_err(|e| AppError::Other(format!("reading {}: {e}", c.path)))?;
                        read.insert(c.path.clone(), text);
                    }
                    snippet_of(&read[&c.path])
                }
            };
            Ok(DefinitionHit { path: c.path, line: c.line, kind: c.kind, snippet })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let st = Command::new("git").arg("-C").arg(dir.path()).args(args).status().unwrap();
            assert!(st.success(), "{args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        for (p, c) in files {
            let full = dir.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    fn lookup_at(repo: &Path, symbol: &str, from: &str, diff: &[&str]) -> Vec<(String, u32)> {
        let diff: Vec<String> = diff.iter().map(|s| s.to_string()).collect();
        find(repo, "main", symbol, from, &diff, &|| false)
            .unwrap()
            .into_iter()
            .map(|h| (h.path, h.line))
            .collect()
    }

    #[test]
    fn rust_ts_ranking_and_lookalikes() {
        let repo = repo_with(&[
            ("a/lib.rs", "// x\npub fn compute(a: i32) -> i32 {\n    a\n}\n"),
            ("b/other.rs", "fn compute_more() {}\nfn compute() {}\n// fn compute() in a comment\n"),
            ("web/x.ts", "export function compute() {}\nclass Foo {\n  async load(id: string): Promise<void> {\n  }\n}\n"),
        ]);
        // same file first, then the rest; `compute_more`, the comment and the .ts file are out
        assert_eq!(
            lookup_at(repo.path(), "compute", "b/other.rs", &[]),
            vec![("b/other.rs".into(), 2), ("a/lib.rs".into(), 2)]
        );
        assert_eq!(lookup_at(repo.path(), "load", "web/x.ts", &[]), vec![("web/x.ts".into(), 3)]);
    }

    #[test]
    fn ruby_declarations_across_files_and_diff_files_rank_first() {
        let repo = repo_with(&[
            ("app/models/invoice.rb", "module Billing\n  class Invoice\n    attr_reader :total\n    def pay!\n    end\n  end\nend\n"),
            ("lib/zzz.rb", "class Billing::Invoice\n  def pay!\n    # def pay!\n  end\nend\n"),
            ("app/use.rb", "Billing::Invoice.new.pay!\n"),
        ]);
        let got = lookup_at(repo.path(), "pay!", "app/use.rb", &["lib/zzz.rb"]);
        assert_eq!(got, vec![("lib/zzz.rb".into(), 2), ("app/models/invoice.rb".into(), 4)]);
        assert_eq!(lookup_at(repo.path(), "total", "app/use.rb", &[]), vec![("app/models/invoice.rb".into(), 3)]);
        assert_eq!(lookup_at(repo.path(), "Invoice", "app/use.rb", &[]).len(), 2); // both `class`es
    }

    #[test]
    fn grep_only_languages_and_unknown_files() {
        let repo = repo_with(&[
            ("a.go", "package a\nfunc Run(x int) {}\n"),
            ("x.vue", "<script>\nexport function helper() {}\n</script>\n"),
            ("y.ts", "export const other = 1;\n"),
        ]);
        assert_eq!(lookup_at(repo.path(), "Run", "a.go", &[]), vec![("a.go".into(), 2)]);
        assert_eq!(lookup_at(repo.path(), "helper", "y.ts", &[]), vec![("x.vue".into(), 2)]);
        assert!(lookup_at(repo.path(), "Run", "notes.txt", &[]).is_empty());
        assert!(lookup_at(repo.path(), "nope", "a.go", &[]).is_empty());
    }

    #[test]
    fn parses_at_most_the_file_cap_and_stale_lookups_abort() {
        let mut files: Vec<(String, String)> = (0..40)
            .map(|i| (format!("f{i:02}.rb"), "def shared\nend\n".to_string()))
            .collect();
        files.push(("z_clicked.rb".into(), "shared\n".into()));
        let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let repo = repo_with(&refs);
        let hits = lookup_at(repo.path(), "shared", "z_clicked.rb", &[]);
        assert_eq!(hits.len(), MAX_HITS);

        let err = find(repo.path(), "main", "shared", "z_clicked.rb", &[], &|| true).unwrap_err();
        assert_eq!(err.to_string(), CANCELLED);
    }

    /// Timing probe on a real clone: `CR_BENCH_REPO=~/projects/back CR_BENCH_REV=<sha>
    /// CR_BENCH_FROM=app/models/x.rb cargo test bench_real_repo -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_real_repo() {
        let (Ok(repo), Ok(rev), Ok(from)) = (
            std::env::var("CR_BENCH_REPO"),
            std::env::var("CR_BENCH_REV"),
            std::env::var("CR_BENCH_FROM"),
        ) else {
            return;
        };
        for sym in ["user", "perform", "call", "initialize", "RuleProcessing", "nope_nope"] {
            let t = std::time::Instant::now();
            let hits = find(Path::new(&repo), &rev, sym, &from, &[], &|| false).unwrap();
            println!("{sym:>16}: {:>4} ms, {} hits", t.elapsed().as_millis(), hits.len());
        }
    }

    #[test]
    fn cat_files_returns_none_for_missing_and_oversized() {
        let repo = repo_with(&[("a.txt", "hello\n"), ("big.txt", &"x".repeat(100))]);
        let paths = ["a.txt".to_string(), "nope.txt".to_string(), "big.txt".to_string()];
        let got = git::cat_files(repo.path(), "main", &paths, 50).unwrap();
        assert_eq!(got, vec![Some("hello\n".to_string()), None, None]);
    }
}
