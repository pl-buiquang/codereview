//! Confirms candidate files with tree-sitter: parses one file and returns the declarations whose
//! name is exactly the clicked symbol. Nothing is kept between calls except the compiled queries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

use super::languages::Parser;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    /// 1-based line of the declared name.
    pub line: u32,
    /// `function`, `class`, `method`, ... (the `@definition.<kind>` capture).
    pub kind: String,
}

/// Compiled queries by grammar name. Compiling is the expensive part, and a `Query` is immutable.
fn query_for(parser: &Parser) -> AppResult<Arc<Query>> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Arc<Query>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(q) = cache.get(parser.name) {
        return Ok(q.clone());
    }
    let query = Query::new(&(parser.grammar)(), parser.query)
        .map_err(|e| AppError::Other(format!("{} definition query: {e}", parser.name)))?;
    let query = Arc::new(query);
    cache.insert(parser.name, query.clone());
    Ok(query)
}

/// Declarations of `symbol` in `source`, in source order.
pub fn definitions_in(parser: &Parser, source: &str, symbol: &str) -> AppResult<Vec<Def>> {
    let query = query_for(parser)?;
    let mut ts = tree_sitter::Parser::new();
    ts.set_language(&(parser.grammar)())
        .map_err(|e| AppError::Other(format!("{} grammar: {e}", parser.name)))?;
    let Some(tree) = ts.parse(source, None) else {
        return Ok(vec![]);
    };

    let names = query.capture_names();
    let bytes = source.as_bytes();
    let mut defs: Vec<Def> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        let mut name = None;
        let mut kind = "definition";
        for c in m.captures() {
            match names[c.index as usize] {
                "name" => name = Some(c.node),
                n => {
                    if let Some(k) = n.strip_prefix("definition.") {
                        kind = k;
                    }
                }
            }
        }
        let Some(node) = name else { continue };
        // `attr_reader :name` captures the symbol `:name`
        let text = node.utf8_text(bytes).unwrap_or("");
        if text.strip_prefix(':').unwrap_or(text) != symbol {
            continue;
        }
        let line = node.start_position().row as u32 + 1;
        if !defs.iter().any(|d| d.line == line) {
            defs.push(Def { line, kind: kind.to_string() });
        }
    }
    defs.sort_by_key(|d| d.line);
    Ok(defs)
}

#[cfg(test)]
mod tests {
    use super::super::languages::{Language, Parser};
    use super::*;

    fn defs(path: &str, source: &str, symbol: &str) -> Vec<(u32, String)> {
        let lang = Language::for_path(path).unwrap();
        let parser = Parser::for_path(lang, path).unwrap();
        definitions_in(parser, source, symbol)
            .unwrap()
            .into_iter()
            .map(|d| (d.line, d.kind))
            .collect()
    }

    #[test]
    fn every_registered_query_compiles() {
        for lang in super::super::languages::LANGUAGES {
            for p in lang.parsers {
                query_for(p).unwrap_or_else(|e| panic!("{}: {e}", p.name));
            }
        }
    }

    #[test]
    fn ruby_declarations() {
        let src = "\
module Billing
  class Invoice::Line < Base
    MAX = 3
    attr_accessor :total, :tax
    scope :paid, -> { where(paid: true) }
    has_many :items

    def self.build(x)
    end

    def total_with_tax?
      # def fake_in_comment
      \"class FakeInString\"
    end
  end
end
";
        assert_eq!(defs("a.rb", src, "Billing"), vec![(1, "module".into())]);
        assert_eq!(defs("a.rb", src, "Line"), vec![(2, "class".into())]);
        assert_eq!(defs("a.rb", src, "MAX"), vec![(3, "constant".into())]);
        assert_eq!(defs("a.rb", src, "tax"), vec![(4, "macro".into())]);
        assert_eq!(defs("a.rb", src, "paid"), vec![(5, "macro".into())]);
        assert_eq!(defs("a.rb", src, "items"), vec![(6, "macro".into())]);
        assert_eq!(defs("a.rb", src, "build"), vec![(8, "method".into())]);
        assert_eq!(defs("a.rb", src, "total_with_tax?"), vec![(11, "method".into())]);
        assert!(defs("a.rb", src, "fake_in_comment").is_empty());
        assert!(defs("a.rb", src, "FakeInString").is_empty());
        assert!(defs("a.rb", src, "where").is_empty());
    }

    #[test]
    fn typescript_declarations() {
        let src = "\
export interface Props { onSave(id: string): void; label: string }
export const compute = (a: number) => a;
const LIMIT = 3;
class Store {
  private cache = new Map();
  async load(id: string): Promise<void> {
    const local = 1;
  }
}
function helper() {}
// function inComment() {}
";
        assert_eq!(defs("a.ts", src, "Props"), vec![(1, "interface".into())]);
        assert_eq!(defs("a.ts", src, "onSave"), vec![(1, "method".into())]);
        assert_eq!(defs("a.ts", src, "compute"), vec![(2, "variable".into())]);
        assert_eq!(defs("a.ts", src, "LIMIT"), vec![(3, "variable".into())]);
        assert_eq!(defs("a.ts", src, "cache"), vec![(5, "field".into())]);
        assert_eq!(defs("a.ts", src, "load"), vec![(6, "method".into())]);
        assert_eq!(defs("a.ts", src, "helper"), vec![(10, "function".into())]);
        assert!(defs("a.ts", src, "local").is_empty()); // function-local
        assert!(defs("a.ts", src, "inComment").is_empty());
        assert_eq!(defs("b.tsx", "export function App() { return <div/>; }", "App").len(), 1);
        assert_eq!(defs("c.jsx", "export class A { run() {} }", "run").len(), 1);
    }

    #[test]
    fn python_rust_csharp_declarations() {
        assert_eq!(defs("a.py", "X = 1\nclass A:\n    def run(self): pass\n", "run").len(), 1);
        assert_eq!(defs("a.py", "X = 1\nclass A:\n    pass\n", "X").len(), 1);
        let rs = "pub struct Foo;\nimpl Foo { pub fn go(&self) {} }\nconst N: u8 = 1;\n";
        assert_eq!(defs("a.rs", rs, "Foo"), vec![(1, "struct".into())]);
        assert_eq!(defs("a.rs", rs, "go"), vec![(2, "function".into())]);
        assert_eq!(defs("a.rs", rs, "N"), vec![(3, "constant".into())]);
        let cs = "namespace N { public class Svc { public int Count { get; } public void Run() {} private int _x; } }";
        assert_eq!(defs("a.cs", cs, "Svc").len(), 1);
        assert_eq!(defs("a.cs", cs, "Count").len(), 1);
        assert_eq!(defs("a.cs", cs, "Run").len(), 1);
        assert_eq!(defs("a.cs", cs, "_x").len(), 1);
    }
}
