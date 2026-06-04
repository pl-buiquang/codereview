# Roadmap & future work

Ideas for evolving **codereview** beyond the current baseline. Grouped by theme; roughly
ordered by value-to-effort within each section. Nothing here is committed scope — it's a
backlog to pull from.

Current baseline (for reference): virtual-PR + GitHub-PR diff review, autosaved review model
with inline & multi-line comments, Markdown/JSON export, and GitHub publish via `gh`.

---

## Known bugs

- **"Viewed" state not saved** — the per-file viewed/collapsed state is ephemeral and is lost on
  reopen. Persist it per (review, file). See the "Persist 'viewed' state" item under §2.
- **Window size/position not preserved** — the app opens at the default geometry on every
  launch; persist and restore window size/position (e.g. the Tauri window-state plugin, or save
  to the config/SQLite).
- **Undefined comment behavior when a branch/PR updates** — when `target.head_sha` advances (new
  commits pushed, branch force-updated, PR rebased), it's unclear and untested what happens to
  existing comments: they may silently become "orphans" or land on the wrong lines. Decide the
  intended behavior (re-anchor vs. flag as stale) and implement it. See §4 (comment anchoring &
  staleness).

## 1. Finish the started polish — ✅ done

This section is complete (see git history):

- **Settings screen** — ✅ a persisted Settings screen (gear in the sidebar) for theme
  (dark/light/system), diff font size, and default split/unified + three-dot. Stored in a
  persisted Zustand store (`lib/settings.ts`); theme via `data-theme` on `<html>`.
- **GUI `PATH` resolution** — ✅ `tools.rs` resolves absolute `git`/`gh` paths after PATH repair
  and caches them; `run_git`/`run_gh` spawn the resolved path; a Settings **Environment** panel
  surfaces detected paths + `gh` auth with clear errors. _Deferred:_ a manual binary-path
  override (detect-only for now — re-open if a user needs it).
- **Replace `alert()` / `confirm()`** — ✅ in-app toast system + promise-based confirm dialog;
  destructive actions (remove repo, delete review, publish review) are gated by the dialog.
- **App icons & metadata** — ✅ custom icon set via `tauri icon`; real `Cargo.toml`
  description/authors and `tauri.conf.json` bundle metadata (category/copyright/descriptions/
  publisher).

## 2. Review experience

- **Threaded replies** — the `comment.parent_id` column already exists but is unused. Render
  replies under a root comment and let the user reply, like a GitHub thread.
- **Resolve / unresolve threads** — mark a comment thread resolved; collapse resolved threads.
- **Comment Markdown** — render comment bodies as Markdown (preview toggle) since they're
  destined for Markdown export / GitHub anyway.
- **Suggested changes** — GitHub-style ```suggestion blocks that publish as suggestions.
- **Keyboard navigation** — next/prev file, next/prev comment, `c` to comment on the focused
  line, `j`/`k` movement.
- **In-review file tree / jump list** — a sidebar of changed files with comment counts and
  "viewed" state to jump around large diffs.
- **Persist "viewed" state** — currently ephemeral per render; store per (review, file) so it
  survives reopening, and show "N of M files viewed". (See Known bugs.)
- **Open the file from the diff** — click a file (or line) to open it. _v1:_ open in the user's
  default editor (`tauri-plugin-opener`, already a dependency). _v2:_ open in a right-hand
  slide-out pane with full-file syntax highlighting and the inline comment system still working
  on the full file (not just the diff hunks).
- **Diff context expansion (extend code before/after)** — `react-diff-view` supports expanding
  collapsed/unchanged lines (`useSourceExpansion` / `expandFromRawCode`); wire it up so the user
  can extend a hunk with the lines before/after it and place comments on context the diff didn't
  include.
- **Syntax-highlight mode within the diff** — let the user pick/override the highlighting
  language for a diff (and toggle highlighting on/off), independent of the file extension — useful
  for extensionless files or embedded languages.
- **Word-level intra-line highlighting** — `markEdits` from `react-diff-view`.

## 3. GitHub integration depth

- **Show existing PR threads** — fetch existing review comments/threads
  (`gh api .../pulls/{n}/comments` + `/reviews`) and render them inline, distinct from local
  drafts. (Listed in the original M4 plan; not yet built.)
- **Reply to existing threads** and **resolve** them via the API.
- **PENDING (draft) GitHub reviews** — support GitHub's draft-review flow (add comments to a
  pending review, then submit) in addition to one-shot publish.
- **PR metadata** — show PR description/body, labels, check status, mergeability, and existing
  approval state in the review header.
- **Auto-refresh & polling** — refresh the PR list and re-fetch a PR's diff/threads on demand
  or on an interval.
- **`commit_id` freshness on publish** — publish currently posts against the stored
  `target.head_sha`; if the PR head advanced since the review was opened, re-fetch and warn (or
  re-anchor) before posting to avoid 422s.
- **Provider abstraction** — factor `gh.rs` behind a trait so GitLab/Bitbucket/Gitea could be
  added later.

## 4. Comment anchoring & staleness

- **Robust re-anchoring** — when `target.head_sha` changes, comments are currently shown as
  "orphans" if their `(side, line)` no longer matches the diff. Add real re-anchoring: map old
  line → new line via the intervening diff, or anchor to content/context rather than raw line
  numbers.
- **Surface staleness in the UI** — badge a review whose head moved, with a "refresh diff"
  action.

## 5. Export

- **Richer hunk context** — include a few surrounding lines (not just the commented line) in the
  exported diff block, so an AI sees more context.
- **Export templates** — let the user customize the Markdown format; add a "copy to clipboard"
  option and "export all reviews for this repo".
- **Round-trip** — define the Markdown/JSON format precisely and support re-importing an
  AI-edited review back into the model.
- **Filename handling** — when saving, don't double the extension if the user typed one (saw
  `name.md.md` during testing); normalize the suffix.

## 6. Performance & scale

- **Virtualize large diffs** — lazy-render hunks and virtualize the file list for big PRs;
  today every file/hunk renders eagerly.
- **Tokenize off the main thread** — `react-diff-view` ships `withTokenizeWorker`; move syntax
  highlighting to a web worker for large files.
- **Cache diffs** — memoize/parse diffs per (target, head_sha) so re-opening a review is instant.
- **DB concurrency** — the backend uses a single `Mutex<Connection>`; if commands ever run
  concurrently and contend, consider a small connection pool (WAL already enabled).

## 7. Edge cases to test & cover

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

## 8. Packaging, distribution & CI

- **Code signing & notarization** (macOS) and signing (Windows) for distributable builds.
- **Auto-update** via the Tauri updater plugin.
- **CI pipeline** — GitHub Actions matrix (macOS/Windows/Linux) to build and attach artifacts
  to releases; run `tsc`, `cargo test`, lints on PRs.
- **Reproducible builds** — `Cargo.lock` and `pnpm-lock.yaml` are committed; pin toolchains
  (`rust-toolchain.toml`, `.nvmrc`).

## 9. Testing infrastructure

- **Rust unit tests** — `export.rs` (model → Markdown/JSON snapshots), diff/SHA parsing, and the
  comment → GitHub-payload mapping (`side`/`line`/`start_line`).
- **Rust integration tests** — `git.rs` against a temp git repo fixture; `gh.rs` mocked or gated
  behind a live-network flag.
- **Frontend tests** — component tests for the diff/commenting flow; the `lib/diff.ts` anchoring
  helpers (`indexFile`, `languageForPath`) are pure and easy to unit-test.

## 10. Nice-to-haves

- Light theme; configurable diff color scheme.
- Search/filter across PRs and the Reviews list.
- Per-comment severity/labels (nit, blocker, question) reflected in export.
- Image/asset diffs.
- More syntax grammars (e.g. Dockerfile isn't in refractor-common today, so `Dockerfile` shows
  unhighlighted — register extra languages).
- Multi-window or tabbed reviews.
