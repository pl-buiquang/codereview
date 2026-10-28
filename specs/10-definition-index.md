# Spec 10 — definition lookup: tree-sitter confirmation, not an index

Go-to-definition in the diff view (`src-tauri/src/definitions/`). Status: **shipped** (v1). User
docs: `docs/features/review.md#go-to-definition`.

## What shipped

A lookup is stateless and per symbol:

1. a loose `git grep -l` over the pinned commit shortlists files that contain something shaped
   like a declaration of the symbol (per-language ERE, false positives allowed);
2. the first 30 of them (clicked file, files in the diff, then by path) are read through one
   `git cat-file --batch` and parsed with tree-sitter; only declarations *named* the symbol are
   kept, so look-alikes in comments and strings drop out;
3. languages without a grammar keep a strict one-line `git grep -n`.

Guards: at most 2 lookups at once across all open reviews, files over 512 KB skipped, hover
lookups abandoned as soon as a newer lookup starts. Measured on a 1,500-file Ruby repo
(`back`): 60–120 ms per lookup, nothing kept in memory afterwards.

## Why not the whole-commit index

The first draft of this spec proposed building a `HashMap<symbol, defs>` per `(repo, sha)` with
tree-sitter tags, lazily on the first lookup, and answering from memory. Rejected:

- reviews are often open several at a time and some are huge; an index per open review costs
  CPU on first use and memory for as long as it lives, and grows with the repo, not with what
  the user looks at;
- the shortlist makes a lookup cost scale with the *symbol* (a few files), which is already fast
  enough (and cached by the frontend for 5 minutes per symbol and pin).

## Deviations from the draft worth knowing

- Plain `tree-sitter` + our own small definition queries (`definitions/queries/*.scm`), not
  `tree-sitter-tags` with each grammar's stock `tags.scm`: the stock queries miss Ruby constants,
  `attr_*`/`scope`, and top-level JS/TS bindings, and the TS one only covers TS-specific nodes.
- Function-local bindings are not definitions (top-level and exported ones are).

## Open

- Incremental/persisted index: only worth it if the shortlist ever proves too slow on a monorepo.
- More languages: see "Adding a language" in the feature docs (Java/Kotlin, Go and C/C++ are
  grep-only today).
- Cross-file references (find usages, hover types) need a real LSP; out of scope.
