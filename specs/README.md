# Specs

Designed-but-unbuilt work for **codereview**. Where `ROADMAP.md` holds one-line ideas, this
directory holds the ones that have been worked up into implementation-ready specs.

Promoted out of ROADMAP §1 ("Review experience"):

- [File tree / jump list](file-tree-jump-list.md) — a per-review **sidebar tab** of changed files
  with +/− stats, comment counts, and viewed state; click to jump. Depends on the sidebar tab system.
- [Open the file from the diff](open-file-from-diff.md) — open the diffed file in the user's
  default editor (v1); a full-file slide-out review pane (v2).
- [Diff context expansion](diff-context-expansion.md) — expand collapsed/unchanged lines around a
  hunk and comment on context the diff didn't include.

Supporting design:

- [Sidebar tab system](sidebar-tabs.md) — restructure the left sidebar into a sticky main bar +
  Repos/Files tabs; prerequisite for the file tree / jump list.

Each spec is grounded in the current code (`file:line` references were accurate at authoring time —
re-verify before implementing). Specs are not committed scope; they're a ready-to-build design.
