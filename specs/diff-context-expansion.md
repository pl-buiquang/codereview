# Spec: diff context expansion (extend code before/after)

## Summary / motivation

A unified diff only includes a few context lines around each change. Reviewers frequently need
the surrounding code to judge a change, and sometimes want to comment on a line the diff didn't
include. `react-diff-view` supports expanding collapsed/unchanged lines (the gap between hunks,
and the head/tail of the file). This spec wires that up so a reviewer can extend a hunk with the
lines before/after it and place comments on that newly revealed context.

## Current state

- **No expansion APIs are used yet.** `useSourceExpansion`, `expandFromRawCode`,
  `getCollapsedLinesCountBetween`, and `<Decoration>` are not imported anywhere
  (`src/components/ReviewView.tsx`, `src/lib/diff.ts`).
- **Diff comes as text only.** `review_diff` (`src/lib/api.ts:39` → `commands/review.rs`) returns
  the unified diff **string**; the frontend `parseDiff`-es it (`ReviewView.tsx:285`). The backend
  does **not** expose full file source — this is the key gap, because expansion needs the raw
  file to fill in collapsed lines.
- `FileBody` (`ReviewView.tsx` line ~481) renders:
  ```tsx
  <Diff viewType diffType hunks tokens widgets selectedChanges gutterEvents codeEvents>
    {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
  </Diff>
  ```
- **Anchoring** (`src/lib/diff.ts::indexFile`): `normal` (context) changes are anchored on **both**
  sides (`keyByAnchor` sets both `RIGHT:newLineNumber` and `LEFT:oldLineNumber`). Expanded lines
  are `normal` changes, so the existing anchoring model already covers them — provided the
  expanded hunks are fed through `indexFile` after expansion.
- The target stores resolved base/head SHAs; local diffs default to three-dot (`base...head`).
  `git.rs::diff` shells out to `git`/`gh`.

## Goals & non-goals

**Goals**
- Show how many lines are collapsed between hunks (and above the first / below the last hunk).
- Let the user expand those lines (by a fixed N, and/or fully) and have them render as context.
- Allow comments on expanded context lines, anchored consistently with existing comments.

**Non-goals**
- Re-anchoring stale comments (ROADMAP §2 — separate).
- Loading every file's full source eagerly (must be lazy/on-demand).
- Word-level intra-line highlighting (separate ROADMAP §1 item).

## UX & behavior

- Between two hunks, render a clickable `<Decoration>` row, e.g. "⋯ Expand 24 lines" with
  sub-actions: expand a chunk (e.g. 20 lines toward the click) or expand all.
- Above the first hunk / below the last: "Expand up" / "Expand down" controls.
- Newly revealed lines look like normal context lines and are clickable to start a comment, same
  as today (`onLineClick`).
- Expansion state is per-file and ephemeral (resets on reload); does not need persistence in v1.

## Technical design

**Backend (new)**
- Add a command to fetch full source at a revision, e.g.
  `file_source(reviewId: number, filePath: string, side: "LEFT" | "RIGHT") -> string` in
  `commands/review.rs`:
  - Resolve the target's base/head SHA (LEFT→base, RIGHT→head); for normal context the RIGHT/new
    file is what react-diff-view expands against, so head source is the primary need.
  - Shell out via `git.rs`: `git show <sha>:<path>` in the repo dir.
  - **GitHub-PR caveat:** the PR head commit may not be present locally. Fallback options:
    `gh api repos/{owner}/{repo}/contents/{path}?ref={sha}` (base64-decode), or fetch the commit.
    Document the network/`gh`-auth dependency and that local targets are the simple case.
- Wrap in `src/lib/api.ts` (e.g. `fileSource(reviewId, filePath, side)`), returning the raw text.
- **Lazy fetch:** only call this when the user clicks expand on a given file — never preload all
  files (perf, ROADMAP §5).

**Frontend**
- In `FileBody` (or `FileReview`), adopt `react-diff-view`'s expansion model:
  - `useSourceExpansion(initialHunks, rawCode)` (or manage hunks in state and call
    `expandFromRawCode(hunks, rawCode, start, end)`).
  - Use `getCollapsedLinesCountBetween(prevHunk, nextHunk)` to render the count in the
    `<Decoration>` row.
  - Render `<Decoration>` rows between/around hunks with expand controls.
- Fetch `rawCode` lazily on first expand for that file (store in component state); show a small
  loading state while fetching.
- **Anchoring after expansion:** re-run `indexFile(file)` on the **expanded** hunks so `metaByKey`
  / `keyByAnchor` include the new context lines. Today `indexFile` is memoized on `file`
  (`ReviewView.tsx:330`); change the memo dependency to the current (possibly expanded) hunks so
  comments can anchor to revealed lines. Confirm `tokenizeFile` re-tokenizes expanded hunks too
  (it's memoized on `file` at line 331 — switch to hunks).
- Comments on expanded context use the existing `submitSelectionComment` path; `side`/`line` come
  from `metaByKey` exactly as for in-diff context lines, so the DB row + publish payload are
  unchanged.

**Data**
- No schema change. Comments on context lines already store `(file_path, side, line, diff_hunk)`
  like any other.

**CSS**
- Style the `<Decoration>` expand rows (`src/styles.css`) using existing variables; keep them
  visually distinct from `.diff-hunk-header`.

## Edge cases

- **No newline at EOF / CRLF:** `git show` output must be split consistently with how
  `parseDiff` counted lines; off-by-one in `rawCode` line indexing is the main risk — test
  expansion lands on the correct lines.
- **Binary files:** no expansion (already shown as a note).
- **File added (no LEFT) / deleted (no RIGHT):** only the existing side has source; expansion on
  the missing side is a no-op/disabled.
- **Renames:** fetch source by the side-appropriate path (`oldPath` for LEFT, `newPath` for RIGHT).
- **Head moved since review opened:** source fetched at the stored SHA may differ from a freshly
  re-fetched diff; keep using the target's resolved SHA so expansion matches the rendered diff.
- **Large files:** fetching full source for a huge file is heavy — acceptable since it's
  on-demand; consider a size guard / chunked expansion in v2.
- A comment placed on an expanded line that later collapses (re-render) becomes an orphan via the
  existing orphan path — acceptable, but note it.

## Phasing

- **v1:** expand between hunks (by N and expand-all), lazy source fetch for **local** targets,
  comments on expanded context.
- **v1.1:** expand to top/bottom of file; GitHub-PR source via `gh api` fallback.
- **v2:** size guards / chunked expansion for very large files; coordinate with virtualization
  (ROADMAP §5).

## Open questions

- Source side: do we ever need LEFT (base) source for expansion, or is RIGHT/new sufficient for
  the common case? (react-diff-view expands the new file by default.)
- For GitHub PRs, prefer `gh api contents` (no fetch, needs auth/network) vs. `git fetch` then
  `git show`?
- Expansion chunk size (10/20 lines) — fixed or configurable?

## Acceptance criteria & verification

- Between hunks, an expand control shows the correct collapsed-line count and reveals the right
  lines on click; expand-all works.
- A comment placed on an expanded context line saves with the correct `(side, line)` and reopens
  on the same line after reload (regression-guard the anchoring).
- Local-target expansion works offline; GitHub-PR expansion documents/handles the
  not-fetched-locally case.
- `cargo build`/`cargo clippy` (new command) and `pnpm exec tsc --noEmit` pass; verified live in
  `pnpm tauri dev` on a virtual-PR review.
