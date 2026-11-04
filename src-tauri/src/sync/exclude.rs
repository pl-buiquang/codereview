//! Repositories the user excluded from sync (Settings → Inbox). One pattern per
//! line, matched case-insensitively against `owner/name`; `*` matches any run of
//! characters (so `acme/*` skips an org and `*/legacy-*` a naming pattern).
//! Blank lines and `#` comments are ignored. No regular expressions.

use rusqlite::Connection;

use crate::commands::inbox as inbox_cmd;
use crate::error::AppResult;

/// `meta` key holding the raw pattern text.
pub const META_KEY: &str = "excluded_repos";

#[derive(Debug, Clone, Default)]
pub struct ExcludedRepos {
    patterns: Vec<String>,
}

impl ExcludedRepos {
    pub fn parse(text: &str) -> Self {
        let patterns = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_lowercase)
            .collect();
        Self { patterns }
    }

    pub fn load(conn: &Connection) -> Self {
        Self::parse(&inbox_cmd::get_meta(conn, META_KEY).unwrap_or_default())
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether `owner/name` matches any pattern.
    pub fn matches(&self, repo: &str) -> bool {
        let repo = repo.to_lowercase();
        self.patterns.iter().any(|p| wildcard_match(p, &repo))
    }
}

pub fn save(conn: &Connection, text: &str) -> AppResult<()> {
    inbox_cmd::set_meta(conn, META_KEY, text)
}

/// Glob match where `*` is the only metacharacter.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_name_matches_case_insensitively() {
        let ex = ExcludedRepos::parse("Acme/Widgets");
        assert!(ex.matches("acme/widgets"));
        assert!(!ex.matches("acme/widgets2"));
        assert!(!ex.matches("other/widgets"));
    }

    #[test]
    fn star_covers_an_org_and_name_patterns() {
        let ex = ExcludedRepos::parse("acme/*\n*/legacy-*\nme/a*c");
        assert!(ex.matches("acme/anything"));
        assert!(ex.matches("x/legacy-api"));
        assert!(!ex.matches("x/new-legacy"));
        assert!(ex.matches("me/abc") && ex.matches("me/ac") && !ex.matches("me/ab"));
    }

    #[test]
    fn blank_lines_comments_and_regex_chars_are_literal() {
        let ex = ExcludedRepos::parse("\n  # skip\n  a.b/c+d  \n");
        assert!(ex.matches("a.b/c+d"));
        assert!(!ex.matches("axb/cccd"));
        assert!(ExcludedRepos::parse("# only a comment\n\n").is_empty());
        assert!(!ExcludedRepos::parse("").matches("a/b"));
    }
}
