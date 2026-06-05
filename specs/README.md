# Specs

Designed-but-unbuilt work for **codereview**. Where `ROADMAP.md` holds one-line ideas, this
directory holds the ones that have been worked up into implementation-ready specs.

These four were promoted out of ROADMAP §1 ("Review experience"):

- [File tree / jump list](file-tree-jump-list.md) — a per-review sidebar of changed files with
  +/− stats, comment counts, and viewed state; click to jump.
- [Open the file from the diff](open-file-from-diff.md) — open the diffed file in the user's
  default editor (v1); a full-file slide-out review pane (v2).
- [Diff context expansion](diff-context-expansion.md) — expand collapsed/unchanged lines around a
  hunk and comment on context the diff didn't include.
- [Syntax-highlight mode](syntax-highlight-mode.md) — override the highlighting language per file
  and toggle highlighting on/off, independent of the file extension.

Each spec is grounded in the current code (`file:line` references were accurate at authoring time —
re-verify before implementing). Specs are not committed scope; they're a ready-to-build design.
