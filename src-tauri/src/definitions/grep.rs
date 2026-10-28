//! The `git grep` half of the lookup: symbol validation and the strict one-line search used for
//! files without a tree-sitter grammar.

use std::path::Path;

use super::languages::Language;
use crate::error::AppResult;
use crate::git;

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

/// `(path, line)` of one-line declarations of `symbol` in the grep-only files of `lang`.
pub fn exact_hits(repo: &Path, sha: &str, lang: &Language, symbol: &str) -> AppResult<Vec<(String, u32)>> {
    if lang.grep_globs.is_empty() {
        return Ok(vec![]);
    }
    let mut hits = Vec::new();
    for re in (lang.exact)(symbol) {
        for (path, line, _) in git::grep_at(repo, sha, &re, lang.grep_globs)? {
            hits.push((path, line));
        }
    }
    Ok(hits)
}

/// Files (within `lang.parsed_globs`) with a line that looks like a declaration of `symbol`.
/// `sha` must be a full commit id.
pub fn shortlist(repo: &Path, sha: &str, lang: &Language, symbol: &str) -> AppResult<Vec<String>> {
    if lang.parsers.is_empty() {
        return Ok(vec![]);
    }
    git::grep_files_at(repo, sha, &(lang.shortlist)(symbol), lang.parsed_globs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_validation() {
        for ok in ["foo", "_x", "$el", "Foo9", "empty?", "save!"] {
            assert!(is_valid_symbol(ok), "{ok}");
        }
        for bad in ["", "9a", "a-b", "a b", "a.*", "(x)", "?", "a?b", "a\\"] {
            assert!(!is_valid_symbol(bad), "{bad}");
        }
    }
}
