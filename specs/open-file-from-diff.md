# Spec: open the file from the diff

## Summary / motivation

While reviewing, the user often wants the whole file, not just the diff hunks. This spec covers
two phases:

- **v1 (committed scope):** click an affordance on a file (or line) to open that file in the
  user's **default editor** on disk.
- **v2 (future, documented):** open a right-hand **slide-out pane** rendering the full file with
  syntax highlighting, where the existing inline-comment system works on the full file (not just
  the diff hunks).

## Current state

- **Opener plugin is already wired:**
  - `tauri-plugin-opener = "2"` in `src-tauri/Cargo.toml`; `@tauri-apps/plugin-opener` in
    `package.json`.
  - Registered in `src-tauri/src/lib.rs` (`.plugin(tauri_plugin_opener::init())`, ~line 22).
  - Capability `opener:default` present in `src-tauri/capabilities/default.json`. `opener:default`
    grants `open-url` / `reveal-item-in-dir` / default URL schemes but **not** `allow-open-path`.
- **Repo path is available frontend-side:** `ReviewDetail.repo_path` (`src/lib/types.ts`) is the
  absolute working-tree path; diff paths (`fileDisplayPath(file)`) are repo-relative, so the full
  path is `repo_path + "/" + path`.
- **No current usage** of the opener plugin anywhere in `src/` or `src-tauri/`.
- File header where an affordance fits: `.diff-file-header` (`ReviewView.tsx` line ~440), which
  already holds the path, stats, and the Viewed toggle.
- Command/permission patterns: commands live in `src-tauri/src/commands/*`, registered in
  `lib.rs` via `generate_handler!`, wrapped in `src/lib/api.ts`; errors are `AppResult<T>` /
  `AppError` (`error.rs`) and serialize to the frontend as a string.

## Goals & non-goals

**Goals (v1)**
- Open the diffed file in the OS default editor from a per-file affordance.
- Clear handling when the file isn't present on disk.

**Non-goals (v1)**
- Choosing a specific editor / configuring an editor command (out of scope; default app only).
- Jumping to the exact line in the editor (best-effort, see Open questions).
- The slide-out pane (that's v2).

## UX & behavior

**v1**
- An "open" icon/button in `.diff-file-header` (tooltip "Open in default editor"). Optionally a
  per-line action later.
- Click → resolve full path → `openPath`. On success the OS opens the file; on failure show a
  toast (reuse the existing `toast.error` used by the Viewed toggle).
- If the file does not exist on disk (e.g. deleted in the diff, or a PR head not checked out),
  the affordance is disabled with a tooltip explaining why, or the click surfaces a clear message.

## Technical design

### v1 — open in default editor

**Recommended: frontend-only (no new Rust command).**
- Import `openPath` from `@tauri-apps/plugin-opener` and call
  `openPath(`${detail.repo_path}/${path}`)` from the file-header handler.
- Add the permission `opener:allow-open-path` to `src-tauri/capabilities/default.json` (alongside
  `opener:default`). Without it the call is rejected by the ACL.

**Alternative: thin Rust command** (`commands/editor.rs` → `open_file(review_id, file_path)`),
mirroring the existing command pattern: load `repo_path` (reuse `review::load_detail` or a
narrower query), join the path, and invoke the opener from Rust. Wrap in `src/lib/api.ts`. Choose
this only if we want server-side existence checks / path validation centralized; otherwise the
frontend path is simpler and consistent with `repo_path` already being exposed.

**Existence check.** Whichever path: before enabling the action, the file should exist on disk.
Frontend can't `stat` directly; options: (a) a small `file_exists(reviewId, filePath)` command, or
(b) let `openPath` fail and toast the error. v1 may start with (b) and add (a) if the error UX is
poor.

**Critical caveat (must be documented in UI/help).** The on-disk file is the **working-tree**
version, which may differ from the diffed revision:
- For **local virtual-PR** targets, the diff is `base...head`; the working tree may be on a
  different branch or have uncommitted edits.
- For **GitHub PR** targets, the PR head commit may not be checked out (or even fetched) locally,
  so the working-tree file may be unrelated or absent.
- **Deleted** files won't exist on disk at all.
v1 behavior: open whatever is on disk and rely on the user understanding it's the working copy; OR
disable when we can detect a mismatch. Recommend: open the working-tree file, label the action
"Open working copy", disable for `type === "delete"`.

### v2 — full-file slide-out review pane

- **New backend command** to fetch full source at the diffed revision, e.g.
  `file_source(reviewId, filePath, side)` in `commands/review.rs`, shelling out via `git.rs`:
  `git show <sha>:<path>` using the target's resolved base/head SHA (LEFT→base, RIGHT→head).
  - **GitHub-PR caveat:** the head/base commit may not be local. Fallbacks: `gh api
    repos/{owner}/{repo}/contents/{path}?ref={sha}` (base64), or fetch the commit first. Document
    that this may require network and `gh` auth.
- **Frontend:** a right-hand pane (overlay/resizable) rendering the full file with `refractor`
  highlighting (reuse `languageForPath` / a generalized `tokenizeFile`).
- **Anchoring implications (important).** The comment contract is `(file_path, side, line)` (see
  CLAUDE.md). The diff anchoring in `src/lib/diff.ts::indexFile` walks **hunk** changes; full-file
  commenting must produce the same `(side, line)` triple for absolute file line numbers and keep
  it consistent with how `publish_review` maps to the GitHub API. This is the main risk and should
  be designed carefully (a comment placed in the pane must round-trip to the same DB row and
  GitHub position as one placed in the diff).

## Edge cases

- File deleted in diff → no working-tree file; disable or message.
- Renames → open at `newPath` (the on-disk name).
- Binary files → opening in the default app is fine (image/PDF viewer); pane (v2) shows a note.
- Repo path moved/deleted after the review was created → `openPath` fails; toast the error
  (ROADMAP §6 repo/filesystem cases).
- Path with spaces/unicode → pass as a single argument (no shell interpolation); the opener API
  handles this.

## Phasing

- **v1:** default-editor open from the file header (+ permission), working-copy caveat, disable on
  delete.
- **v1.1:** existence check command; optional per-line open; best-effort open-at-line for known
  editors (config-driven).
- **v2:** `file_source` command + slide-out full-file pane with highlighting and full-file
  commenting (depends on the anchoring rework above; coordinate with ROADMAP §2 re-anchoring).

## Open questions

- Open the *working copy* (simple) vs. the *exact diffed blob* via a temp file (accurate but
  heavier) in v1?
- Open-at-line: editor-specific (`code -g file:line`, `subl file:line`) requires knowing/Configuring
  the editor — defer to a settings feature?
- v2 pane: modal overlay vs. resizable split; does it reuse `ReviewView` comment components?

## Acceptance criteria & verification

- **v1:** a file affordance opens the working-tree file in the OS default app; deleted files are
  disabled; a missing file surfaces a clear toast rather than a silent failure.
- `opener:allow-open-path` present in `capabilities/default.json`; `pnpm exec tsc --noEmit` passes;
  `cargo build --manifest-path src-tauri/Cargo.toml` passes if a Rust command was added.
- Manual: in `pnpm tauri dev`, open a local-target review, click open on a modified file → it
  opens; confirm the working-copy caveat behavior on a PR whose head isn't checked out.
