# Spec 10 — tree-sitter definition index

Upgrade for the diff view's Cmd/Ctrl-click go-to-definition (`src-tauri/src/definitions.rs`).
Status: **not started**. The shipped version is regex + `git grep`.

## Problem

Today every lookup scans the whole tree at the pinned commit (`git grep`, one run per regex) and
then runs one `git show` per hit for the snippet. Two weaknesses:

- **Speed.** Cost grows with repo size and nothing is cached between lookups. Fine on small repos,
  probably slow on large ones (e.g. `back`). Not yet measured: measure on a big repo first.
- **Precision.** Regexes match look-alikes (a `fn name` inside a comment or string, a call that
  happens to look like a method declaration) and miss declaration styles we did not write a
  pattern for.

## Proposal

Index definitions once per commit with tree-sitter tags, the same approach as GitHub's code
navigation, and answer lookups from memory.

- **Parser.** `tree-sitter` + `tree-sitter-tags` crates, with each language's grammar crate and its
  `tags.scm` (`@definition.function`, `@definition.class`, `@definition.method`, ...).
- **Index.** `HashMap<symbol, Vec<(path, line)>>` per `(repo, sha)`. A lookup is a map read
  (microseconds); the snippet comes from the already-read file contents.
- **Lazy build.** Nothing is parsed until the first Cmd-click in a review. Build on the blocking
  pool (`spawn_blocking`), read blobs with one `git cat-file --batch` stream instead of a process
  per file, and skip unsupported extensions and files over a size cap (generated and minified code).
- **Incremental.** When the pin moves (`update_review_pin`) or a second review shares most of the
  tree, start from the parent commit's index and re-parse only files in `git diff --name-only`.
- **Bounded memory.** LRU of a few indexes (one per recent `(repo, sha)`); drop on app idle.
- **UI.** Unchanged: same `find_definition` command and `DefinitionHit` shape. While the first
  build runs the popover shows "indexing…"; if it fails or the language has no grammar, fall back to
  the current `git grep` path so behaviour never regresses.

## Open questions

- Which languages first? Start with the ones in this workspace: Ruby, Python, TypeScript/TSX,
  Rust, C#.
- Binary size and build time of the grammar crates (each adds a C parser).
- Is a per-commit index worth persisting to SQLite, or is rebuilding on demand fast enough?
- Cross-file references (find *usages*, hover types) need a real LSP; out of scope here.

## Alternatives considered

- **Real LSP** (rust-analyzer, tsserver, pyright, ruby-lsp in the PR worktree): exact and
  type-aware, but each server must be installed, indexing takes seconds to minutes, memory is in
  the GB range, and the base side of the diff needs its own checkout. Revisit only if tags-level
  precision proves insufficient.
- **ctags / ripgrep index:** cheaper than tree-sitter, but ctags is another binary to ship and has
  the same look-alike problems as the regexes.
