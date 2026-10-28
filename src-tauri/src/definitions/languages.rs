//! Language registry for go-to-definition: the one place to edit to support a new language.
//! See "Adding a language" in `docs/features/review.md`.
//!
//! A language is looked up by the extension of the file the user clicked in. It has up to two
//! ways to find declarations in the pinned commit:
//!
//! - **Parsed** (`parsers` + `shortlist`): a loose `git grep -l` picks candidate files, then
//!   tree-sitter confirms which of them really declare the symbol (no hits in comments/strings).
//! - **Grep only** (`grep_globs` + `exact`): one stricter `git grep -n`, trusted as is.

use tree_sitter::Language as TsLanguage;

/// Extra pattern a declaration may take, shared by the regex builders below.
const END: &str = "([^A-Za-z0-9_$]|$)";
const SP: &str = "[[:space:]]+";

pub struct Parser {
    /// Cache key for the compiled query; unique per grammar.
    pub name: &'static str,
    /// Extensions (no dot) of the files this grammar parses.
    pub extensions: &'static [&'static str],
    pub grammar: fn() -> TsLanguage,
    /// Definition query: `@name` on the declared identifier, `@definition.<kind>` on the node.
    pub query: &'static str,
}

pub struct Language {
    pub name: &'static str,
    /// Extensions (no dot) of files this language's lookups can start from.
    pub extensions: &'static [&'static str],
    pub parsers: &'static [Parser],
    /// Pathspecs of the files `parsers` can read.
    pub parsed_globs: &'static [&'static str],
    /// Loose ERE patterns that every declaration of `sym` matches (false positives are fine).
    pub shortlist: fn(sym: &str) -> Vec<String>,
    /// Pathspecs searched with `exact` only, for files without a parser.
    pub grep_globs: &'static [&'static str],
    /// Strict ERE patterns for a declaration of `sym` on one line.
    pub exact: fn(sym: &str) -> Vec<String>,
}

impl Language {
    pub fn for_path(path: &str) -> Option<&'static Language> {
        let ext = extension(path)?;
        LANGUAGES.iter().find(|l| l.extensions.contains(&ext.as_str()))
    }
}

impl Parser {
    /// The parser for `path`, if any grammar of `lang` reads that extension.
    pub fn for_path(lang: &'static Language, path: &str) -> Option<&'static Parser> {
        let ext = extension(path)?;
        lang.parsers.iter().find(|p| p.extensions.contains(&ext.as_str()))
    }
}

fn extension(path: &str) -> Option<String> {
    let (_, ext) = path.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// `sym` as it must appear inside an ERE (`$` and `?` are legal in identifiers).
fn esc(sym: &str) -> String {
    sym.replace('$', "\\$").replace('?', "\\?")
}

fn none(_: &str) -> Vec<String> {
    vec![]
}

// ---- shortlist patterns (parsed languages) ---------------------------------------------------

fn ruby_shortlist(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        // def foo / def self.foo / class A::Foo / module Foo
        format!("(def|class|module){SP}([A-Za-z0-9_:]+(::|\\.))?{s}([^A-Za-z0-9_?!]|$)"),
        // FOO = ...
        format!("^[[:space:]]*{s}[[:space:]]*=[^=~]"),
        // attr_accessor :foo, scope :foo, has_many :foo, ...
        format!(
            "(attr_[a-z_]+|scope|define_method|has_many|has_one|belongs_to|has_and_belongs_to_many|delegate)[^#]*:{s}([^A-Za-z0-9_]|$)"
        ),
    ]
}

fn python_shortlist(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(def|class){SP}{s}([^A-Za-z0-9_]|$)"),
        format!("^{s}[[:space:]]*(:[^=]*)?=[^=]"),
    ]
}

fn rust_shortlist(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![format!("(fn|struct|enum|trait|type|const|static|mod|union|macro_rules!){SP}{s}{END}")]
}

fn js_shortlist(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(function\\*?|class|interface|type|enum|const|let|var|namespace){SP}{s}{END}"),
        // members and object keys: `  private async name(`, `  name: T`, `  name = ...`
        format!(
            "^[[:space:]]*((public|private|protected|static|readonly|async|get|set|abstract|override|declare)[[:space:]]+)*{s}[?!]?[[:space:]]*[(:=<;]"
        ),
    ]
}

fn csharp_shortlist(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(class|interface|enum|struct|record|delegate){SP}{s}{END}"),
        // `Type Name(`, `Type Name {`, `Type Name =`, `Type Name;`
        format!("[]>A-Za-z0-9_?)][[:space:]]+{s}[[:space:]]*[({{=;]"),
    ]
}

// ---- exact patterns (grep-only languages) ----------------------------------------------------

fn js_exact(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(function\\*?|class|interface|type|enum|const|let|var){SP}{s}{END}"),
        // method shorthand: `  async name(args): T {`
        format!(
            "^[[:space:]]+((async|static|get|set)[[:space:]]+)*{s}[[:space:]]*(<[^>]*>)?\\(.*\\)[[:space:]]*(:[^=]+)?\\{{[[:space:]]*$"
        ),
    ]
}

fn go_exact(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("func{SP}(\\([^)]*\\)[[:space:]]*)?{s}[[:space:]]*[(\\[]"),
        format!("type{SP}{s}{SP}"),
    ]
}

fn jvm_exact(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(class|interface|enum|struct|record|trait|object|fun|func|def|function|type){SP}{s}{END}"),
        format!("^[[:space:]]*[A-Za-z_<>,.?[:space:]]+{SP}{s}[[:space:]]*\\([^;]*$"),
    ]
}

fn c_exact(sym: &str) -> Vec<String> {
    let s = esc(sym);
    vec![
        format!("(class|struct|enum|union|typedef|namespace){SP}{s}{END}"),
        format!("^[A-Za-z_][A-Za-z0-9_:<>*&[:space:]]*[[:space:]*&]{s}[[:space:]]*\\([^;]*$"),
    ]
}

const JS_FAMILY_GRAMMARS: &[Parser] = &[
    Parser {
        name: "typescript",
        extensions: &["ts", "mts", "cts"],
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        query: include_str!("queries/typescript.scm"),
    },
    Parser {
        name: "tsx",
        extensions: &["tsx"],
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        query: include_str!("queries/typescript.scm"),
    },
    Parser {
        name: "javascript",
        extensions: &["js", "jsx", "mjs", "cjs"],
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        query: include_str!("queries/javascript.scm"),
    },
];

pub static LANGUAGES: &[Language] = &[
    Language {
        name: "Ruby",
        extensions: &["rb", "rake", "erb"],
        parsers: &[Parser {
            name: "ruby",
            extensions: &["rb", "rake"],
            grammar: || tree_sitter_ruby::LANGUAGE.into(),
            query: include_str!("queries/ruby.scm"),
        }],
        parsed_globs: &["*.rb", "*.rake"],
        shortlist: ruby_shortlist,
        grep_globs: &[],
        exact: none,
    },
    Language {
        name: "Python",
        extensions: &["py", "pyi"],
        parsers: &[Parser {
            name: "python",
            extensions: &["py", "pyi"],
            grammar: || tree_sitter_python::LANGUAGE.into(),
            query: include_str!("queries/python.scm"),
        }],
        parsed_globs: &["*.py", "*.pyi"],
        shortlist: python_shortlist,
        grep_globs: &[],
        exact: none,
    },
    Language {
        name: "Rust",
        extensions: &["rs"],
        parsers: &[Parser {
            name: "rust",
            extensions: &["rs"],
            grammar: || tree_sitter_rust::LANGUAGE.into(),
            query: include_str!("queries/rust.scm"),
        }],
        parsed_globs: &["*.rs"],
        shortlist: rust_shortlist,
        grep_globs: &[],
        exact: none,
    },
    Language {
        name: "TypeScript / JavaScript",
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "vue", "svelte"],
        parsers: JS_FAMILY_GRAMMARS,
        parsed_globs: &["*.ts", "*.tsx", "*.mts", "*.cts", "*.js", "*.jsx", "*.mjs", "*.cjs"],
        shortlist: js_shortlist,
        // single-file components embed their script; no grammar for the wrapper
        grep_globs: &["*.vue", "*.svelte"],
        exact: js_exact,
    },
    Language {
        name: "C#",
        extensions: &["cs"],
        parsers: &[Parser {
            name: "csharp",
            extensions: &["cs"],
            grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
            query: include_str!("queries/csharp.scm"),
        }],
        parsed_globs: &["*.cs"],
        shortlist: csharp_shortlist,
        grep_globs: &[],
        exact: none,
    },
    Language {
        name: "Go",
        extensions: &["go"],
        parsers: &[],
        parsed_globs: &[],
        shortlist: none,
        grep_globs: &["*.go"],
        exact: go_exact,
    },
    Language {
        name: "Java / Kotlin / Scala / Swift / PHP / Dart",
        extensions: &["java", "kt", "kts", "scala", "swift", "php", "dart"],
        parsers: &[],
        parsed_globs: &[],
        shortlist: none,
        grep_globs: &["*.java", "*.kt", "*.kts", "*.scala", "*.swift", "*.php", "*.dart"],
        exact: jvm_exact,
    },
    Language {
        name: "C / C++",
        extensions: &["c", "h", "cpp", "cc", "hpp", "cxx"],
        parsers: &[],
        parsed_globs: &[],
        shortlist: none,
        grep_globs: &["*.c", "*.h", "*.cpp", "*.cc", "*.hpp", "*.cxx"],
        exact: c_exact,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The "Supported languages" table in the feature docs must list every registered language
    /// and extension (and vice versa), so adding one without documenting it fails here.
    #[test]
    fn docs_table_matches_registry() {
        let docs = include_str!("../../../docs/features/review.md");
        let table: Vec<&str> = docs
            .split("### Supported languages")
            .nth(1)
            .expect("Supported languages section")
            .lines()
            .skip_while(|l| !l.starts_with('|'))
            .take_while(|l| l.starts_with('|'))
            .skip(2) // header + separator
            .collect();
        assert!(!table.is_empty());

        let mut documented: Vec<String> = Vec::new();
        for row in &table {
            let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
            let exts = cells[1];
            documented.extend(exts.split('`').skip(1).step_by(2).map(str::to_string));
        }
        for lang in LANGUAGES {
            assert!(table.iter().any(|r| r.contains(lang.name)), "{} is not in the docs table", lang.name);
            for ext in lang.extensions {
                assert!(documented.iter().any(|d| d == ext), "extension `{ext}` of {} is not in the docs table", lang.name);
            }
        }
        let known: Vec<&str> = LANGUAGES.iter().flat_map(|l| l.extensions.iter().copied()).collect();
        for d in &documented {
            assert!(known.contains(&d.as_str()), "docs table lists `{d}` which no language handles");
        }
    }

    #[test]
    fn extensions_are_unique_and_lookup_by_path_works() {
        let mut seen = std::collections::HashSet::new();
        for lang in LANGUAGES {
            for ext in lang.extensions {
                assert!(seen.insert(*ext), "extension `{ext}` is claimed twice");
            }
        }
        assert_eq!(Language::for_path("a/b/Foo.RB").unwrap().name, "Ruby");
        assert_eq!(Language::for_path("x.tsx").unwrap().name, "TypeScript / JavaScript");
        assert!(Language::for_path("Makefile").is_none());
        assert!(Language::for_path("notes.txt").is_none());
    }
}
