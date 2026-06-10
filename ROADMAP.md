# Roadmap & future work

Ideas for evolving **codereview** beyond the current baseline. Grouped by theme; roughly
ordered by value-to-effort, both across sections and within each one. Nothing here is committed
scope — it's a backlog to pull from.

Current baseline (for reference): virtual-PR + GitHub-PR diff review, autosaved review model
with inline & multi-line comments, Markdown/JSON export, and GitHub publish via `gh`.

---

## 1. Review experience

- **Expand diff context to file top/bottom** — today only the gaps *between* hunks expand; add
  expanders for the leading block (above the first hunk) and trailing block (below the last).
  Frontend-only via `react-diff-view`'s `expandFromRawCode` over old-side line ranges
  (`getCollapsedLinesCountBetween(null, hunk)` gives the leading count; the trailing size needs
  the fetched base file's line count). Prototyped once, then reverted.
- **Threaded replies** — the `comment.parent_id` column already exists but is unused. Render
  replies under a root comment and let the user reply, like a GitHub thread.
- **Resolve / unresolve threads** — mark a comment thread resolved; collapse resolved threads.
- **Suggested changes** — GitHub-style ```suggestion blocks that publish as suggestions.
- **Keyboard navigation** — next/prev file, next/prev comment, `c` to comment on the focused
  line, `j`/`k` movement.
- **Word-level intra-line highlighting** — `markEdits` from `react-diff-view`.

## 2. Comment anchoring & staleness

- **Re-anchor LEFT/base-side comments** — RIGHT-side comments now re-anchor to a moved head via the
  intervening diff (`anchor.rs` + `reanchor_comments`); base-side comments are still left untouched
  (would need an `anchored_base_sha` to map across base/merge-base movement). Content/context-based
  anchoring remains an alternative to line-mapping.

## 3. GitHub integration depth

- **Reply to existing threads** and **resolve** them via the API.
- **PENDING (draft) GitHub reviews** — support GitHub's draft-review flow (add comments to a
  pending review, then submit) in addition to one-shot publish.
- **Auto-refresh & polling** — a manual "Refresh" re-resolves SHAs and re-fetches a review's
  diff/threads today; still want PR-list refresh and optional interval polling.
- **Provider abstraction** — factor `gh.rs` behind a trait so GitLab/Bitbucket/Gitea could be
  added later.

## 4. Export

- **Export templates** — let the user customize the Markdown format; add a "copy to clipboard"
  option and "export all reviews for this repo".
- **Round-trip** — define the Markdown/JSON format precisely and support re-importing an
  AI-edited review back into the model.
- **Filename handling** — when saving, don't double the extension if the user typed one (saw
  `name.md.md` during testing); normalize the suffix.

## 5. Performance & scale

- **Virtualize large diffs** — lazy-render hunks and virtualize the file list for big PRs;
  today every file/hunk renders eagerly.
- **Tokenize off the main thread** — `react-diff-view` ships `withTokenizeWorker`; move syntax
  highlighting to a web worker for large files.
- **Cache diffs** — memoize/parse diffs per (target, head_sha) so re-opening a review is instant.
- **DB concurrency** — the backend uses a single `Mutex<Connection>`; if commands ever run
  concurrently and contend, consider a small connection pool (WAL already enabled).

## 6. Edge cases to test & cover

Diff/render:
- Empty diff (no changes between refs) — verify the "No changes" path.
- Pure file **additions**, **deletions** (LEFT-side comments), **renames**, **copies**, mode
  changes, and **binary** files (binary is handled with a note; confirm).
- Very large files / very large PRs (perf + memory).
- Files with no trailing newline; CRLF vs LF; tabs; very long lines (wrapping).
- Unicode / emoji in file paths, code, and comment bodies (export + GitHub payload).
- Comments on **context (normal)** lines, first/last line of a hunk, and ranges that **span
  multiple hunks**.

Review model:
- Reopen after app restart reconstructs comments at the right lines (regression-guard the
  anchoring).
- Multiple reviews on one target stay independent; deleting one cascades only its comments.
- Editing/deleting a published review is blocked (lock enforced in `ensure_draft`).
- Concurrent edits / rapid autosave debounce correctness.

GitHub:
- `gh` not installed / not authenticated / wrong scopes → clear guidance.
- Repo with **no GitHub remote** opened on the "GitHub PRs" tab.
- Network failure / API rate limit / 422 from invalid comment positions.
- Publishing a review with **zero comments** (summary-only) and each verdict
  (comment/approve/request-changes); approving your own PR (GitHub restriction).
- Multi-line publish payload (`start_line`/`start_side`) lands on the correct lines — **the M5
  publish path has not been verified live yet.**

Repo/filesystem:
- Repo path deleted/moved after being added; non-git folder selected; bare repo; submodules;
  worktrees; detached HEAD; very large repos.

## 7. Testing infrastructure

- **Rust unit tests** — `export.rs` (model → Markdown/JSON snapshots), diff/SHA parsing, and the
  comment → GitHub-payload mapping (`side`/`line`/`start_line`).
- **Rust integration tests** — `git.rs` against a temp git repo fixture; `gh.rs` mocked or gated
  behind a live-network flag.
- **Frontend tests** — component tests for the diff/commenting flow; the `lib/diff.ts` anchoring
  helpers (`indexFile`, `languageForPath`) are pure and easy to unit-test.

## 8. Packaging, distribution & CI

- **Code signing & notarization** (macOS) and signing (Windows) for distributable builds.
- **Auto-update** via the Tauri updater plugin.
- **CI pipeline** — GitHub Actions matrix (macOS/Windows/Linux) to build and attach artifacts
  to releases; run `tsc`, `cargo test`, lints on PRs.
- **Reproducible builds** — `Cargo.lock` and `pnpm-lock.yaml` are committed; pin toolchains
  (`rust-toolchain.toml`, `.nvmrc`).

## 9. Nice-to-haves

- Search/filter the PR list (the Reviews list already supports faceted filtering + sorting).
- Per-comment severity/labels (nit, blocker, question) reflected in export.
- Image/asset diffs.
- More syntax grammars (e.g. Dockerfile isn't in refractor-common today, so `Dockerfile` shows
  unhighlighted — register extra languages).
