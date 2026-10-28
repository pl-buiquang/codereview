//! Search-based "go to definition" for the diff view.
//!
//! No language server: like GitHub's code navigation we `git grep` the pinned commit for a
//! declaration of the clicked identifier, using a few per-language regexes. Cheap, offline, and
//! good enough to answer "where is this defined?" while reviewing.

use std::path::Path;

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::git;

/// Hits returned per lookup; common names (`new`, `get`) would otherwise flood the popover.
const MAX_HITS: usize = 10;
/// Lines of source returned with each hit, starting at the declaration.
const SNIPPET_LINES: usize = 20;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionHit {
    pub path: String,
    /// 1-based line of the declaration.
    pub line: u32,
    /// `SNIPPET_LINES` lines of the file starting at `line`.
    pub snippet: String,
}

/// True for identifiers we are willing to interpolate into a regex (`foo`, `$el`, `empty?`).
pub fn is_valid_symbol(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if symbol.len() > 100 || !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    let body = match rest.last() {
        Some('?') | Some('!') => &rest[..rest.len() - 1],
        _ => &rest[..],
    };
    body.iter().all(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
}

/// Pathspec globs and definition regexes (ERE) for the language family of `file_path`.
/// `sym` is already validated by [`is_valid_symbol`].
fn patterns_for(file_path: &str, sym: &str) -> Option<(&'static [&'static str], Vec<String>)> {
    let ext = file_path.rsplit('.').next()?.to_ascii_lowercase();
    let s = sym.replace('$', "\\$").replace('?', "\\?");
    let end = "([^A-Za-z0-9_$]|$)";
    let sp = "[[:space:]]+";
    let generic = format!(
        "(class|interface|enum|struct|record|trait|object|fun|func|def|function|type){sp}{s}{end}"
    );
    Some(match ext.as_str() {
        "rs" => (
            &["*.rs"][..],
            vec![format!("(fn|struct|enum|trait|type|const|static|mod|union|macro_rules!){sp}{s}{end}")],
        ),
        "py" | "pyi" => (&["*.py", "*.pyi"][..], vec![format!("(def|class){sp}{s}{end}")]),
        "rb" | "rake" | "erb" => (
            &["*.rb", "*.rake"][..],
            vec![format!("(def|class|module){sp}(self\\.)?{s}([^A-Za-z0-9_?!]|$)")],
        ),
        "go" => (
            &["*.go"][..],
            vec![format!("func{sp}(\\([^)]*\\)[[:space:]]*)?{s}[[:space:]]*[(\\[]"), format!("type{sp}{s}{sp}")],
        ),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "vue" | "svelte" => (
            &["*.ts", "*.tsx", "*.js", "*.jsx", "*.mjs", "*.cjs", "*.vue", "*.svelte"][..],
            vec![
                format!("(function\\*?|class|interface|type|enum|const|let|var){sp}{s}{end}"),
                // method shorthand: `  async name(args): T {`
                format!("^[[:space:]]+((async|static|get|set)[[:space:]]+)*{s}[[:space:]]*(<[^>]*>)?\\(.*\\)[[:space:]]*(:[^=]+)?\\{{[[:space:]]*$"),
            ],
        ),
        "java" | "kt" | "kts" | "cs" | "scala" | "swift" | "php" | "dart" => (
            &["*.java", "*.kt", "*.kts", "*.cs", "*.scala", "*.swift", "*.php", "*.dart"][..],
            vec![
                generic,
                format!("^[[:space:]]*[A-Za-z_<>,.?[:space:]]+{sp}{s}[[:space:]]*\\([^;]*$"),
            ],
        ),
        "c" | "h" | "cpp" | "cc" | "hpp" | "cxx" => (
            &["*.c", "*.h", "*.cpp", "*.cc", "*.hpp", "*.cxx"][..],
            vec![
                format!("(class|struct|enum|union|typedef|namespace){sp}{s}{end}"),
                format!("^[A-Za-z_][A-Za-z0-9_:<>*&[:space:]]*[[:space:]*&]{s}[[:space:]]*\\([^;]*$"),
            ],
        ),
        _ => return None,
    })
}

/// Declarations of `symbol` at `sha` in `repo`, best first (same file as the click, then by path).
pub fn find(repo: &Path, sha: &str, symbol: &str, from_file: &str) -> AppResult<Vec<DefinitionHit>> {
    if !is_valid_symbol(symbol) {
        return Ok(vec![]);
    }
    let Some((globs, regexes)) = patterns_for(from_file, symbol) else {
        return Ok(vec![]);
    };
    let mut hits: Vec<(String, u32)> = Vec::new();
    for re in &regexes {
        for (path, line, _) in git::grep_at(repo, sha, re, globs)? {
            if !hits.iter().any(|(p, l)| *p == path && *l == line) {
                hits.push((path, line));
            }
        }
    }
    hits.sort_by(|a, b| {
        ((a.0 != from_file) as u8, &a.0, a.1).cmp(&((b.0 != from_file) as u8, &b.0, b.1))
    });
    hits.truncate(MAX_HITS);
    hits.into_iter()
        .map(|(path, line)| {
            let source = git::show_file(repo, sha, &path)
                .map_err(|e| AppError::Other(format!("reading {path}: {e}")))?;
            let snippet = source
                .lines()
                .skip(line.saturating_sub(1) as usize)
                .take(SNIPPET_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            Ok(DefinitionHit { path, line, snippet })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn symbol_validation() {
        for ok in ["foo", "_x", "$el", "Foo9", "empty?", "save!"] {
            assert!(is_valid_symbol(ok), "{ok}");
        }
        for bad in ["", "9a", "a-b", "a b", "a.*", "(x)", "?", "a?b", "a\\"] {
            assert!(!is_valid_symbol(bad), "{bad}");
        }
    }

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

    #[test]
    fn finds_rust_and_ts_declarations_and_ranks_same_file_first() {
        let repo = repo_with(&[
            ("a/lib.rs", "// x\npub fn compute(a: i32) -> i32 {\n    a\n}\n"),
            ("b/other.rs", "fn compute_more() {}\nfn compute() {}\n"),
            ("web/x.ts", "export function compute() {}\nclass Foo {\n  async load(id: string): Promise<void> {\n  }\n}\n"),
        ]);
        let hits = find(repo.path(), "main", "compute", "b/other.rs").unwrap();
        let got: Vec<_> = hits.iter().map(|h| (h.path.as_str(), h.line)).collect();
        assert_eq!(got, vec![("b/other.rs", 2), ("a/lib.rs", 2)]); // `compute_more` and .ts excluded
        assert!(hits[1].snippet.starts_with("pub fn compute"));

        let ts = find(repo.path(), "main", "load", "web/x.ts").unwrap();
        assert_eq!(ts.len(), 1);
        assert_eq!((ts[0].path.as_str(), ts[0].line), ("web/x.ts", 3));
    }

    #[test]
    fn no_match_or_unsupported_language_is_empty() {
        let repo = repo_with(&[("a.py", "def foo():\n    pass\n")]);
        assert!(find(repo.path(), "main", "nope", "a.py").unwrap().is_empty());
        assert!(find(repo.path(), "main", "foo", "notes.txt").unwrap().is_empty());
        assert_eq!(find(repo.path(), "main", "foo", "a.py").unwrap().len(), 1);
    }
}
