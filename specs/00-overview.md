# Spec 00 — Overview: head-SHA freshness (re-anchoring, refresh, publish `commit_id`)

## Problem

A PR's head SHA can advance *after* a review is opened. Today the app only flags this cosmetically
(per-comment "outdated" badge + "orphan" rendering) and never acts on it:

- Opening an existing review (`load_detail`, `review.rs:378`) never re-resolves the target's SHAs,
  so a stale diff is shown.
- Comments are never re-mapped to their new line when the head moves.
- `publish_review` posts inline comments against the **stored** `target.head_sha`
  (`build_publish_payload`, `review.rs:511`); if the head advanced, GitHub returns 422.

## Three features, shipped together

1. **Robust re-anchoring** (ROADMAP §2) — map a comment's `(side, line)` from its
   `anchored_head_sha` to the current head via the intervening diff.
2. **Manual refresh** (ROADMAP §3) — a "Refresh" action that re-resolves SHAs and re-fetches the
   diff/threads, plus a review-level "head moved" badge + "Re-anchor comments" action.
3. **`commit_id` freshness on publish** (ROADMAP §3) — publish silently re-fetches the head,
   re-anchors, folds un-mappable comments into the body, and posts against the fresh `commit_id`.

## Decisions (locked with the user)

- Auto-refresh = **manual button only**. No interval polling, no settings change.
- Publish = **auto re-anchor silently** (no dialog, no `force` param; signature unchanged).
- Re-anchoring = **RIGHT-side, context-precise**. LEFT/base side left untouched (base rarely moves
  when only the head advances) — no `anchored_base_sha` migration this round.
- Tests = **Rust only**. No JS test runner is introduced.

## Dependency graph

```
P1 anchor.rs (pure remap)   P2 two-SHA diff      P3 refresh target SHAs
        │      │                  │  │                    │  │
        └──────┴──── F1 reanchor_review_comments ──┐      │  └── refresh_review cmd (feature 2)
                     helper (P1 + P2)              │      │
                                                   └── F2 publish_review change (feature 3) ──┘
```

P1/P2/P3 are mutually independent. The re-anchor helper needs P1+P2. `publish_review` reuses the
refresh-head helper (P3) and the re-anchor helper (F1).

## Shared-primitive contract

Factor reusable logic as **plain functions** (not `#[tauri::command]`) so the command layer *and*
the publish path share them. All new backend code lives in `src-tauri/src`.

```rust
// anchor.rs (NEW, pure — Spec 01)
pub enum Remap { Shifted(i64), Lost }
pub struct FileHunks { /* parsed hunks incl. per-line kinds */ }
pub fn parse_file_patch(patch: &str) -> FileHunks;
pub fn remap_right_line(line: i64, hunks: &FileHunks) -> Remap;

// git.rs (Spec 01/02) — plain two-dot diff between two commits, NOT three-dot.
pub fn diff_shas(repo: &Path, old_sha: &str, new_sha: &str) -> AppResult<String>;

// gh.rs (Spec 01) — clone-less compare; per-file unified patches.
pub struct ComparedFile { pub filename: String, pub patch: Option<String>, pub status: String }
pub fn compare(owner: &str, name: &str, base: &str, head: &str) -> AppResult<Vec<ComparedFile>>;

// commands/review.rs (Spec 02/03) — shared helpers + commands.
pub struct FreshnessResult { pub head_moved: bool, pub previous_head_sha: Option<String>, pub current_head_sha: Option<String> }
pub struct ReanchorResult  { pub reanchored: usize, pub lost: usize, pub skipped_no_change: usize }

fn refresh_target_shas(conn, &Target, ctx/repo info) -> AppResult<FreshnessResult>;   // re-resolve + persist
fn reanchor_review_comments(conn, &ReviewDetail) -> AppResult<ReanchorResult>;          // remap RIGHT comments in place

#[tauri::command] pub fn refresh_review(review_id, db) -> AppResult<FreshnessResult>;
#[tauri::command] pub fn reanchor_comments(review_id, db) -> AppResult<ReanchorResult>;
// publish_review: signature UNCHANGED; internals updated (Spec 03).
```

Both `FreshnessResult` and `ReanchorResult` derive `Serialize` with `#[serde(rename_all = "camelCase")]`.

## Picking the diff source per target kind

Mirror `file_source` (`review.rs:273`): a clone-less PR repo has `repo_path.starts_with("github:")`.

- **Local target / PR with a local clone:** `git::diff_shas(repo_path, old, new)`.
- **Clone-less PR (`github:` path):** `gh::compare(owner, name, old, new)` and read each file's `patch`.

## Command registration

Add `mod anchor;` to `src-tauri/src/lib.rs` (alongside `mod gh; mod git;` at lines 5–6) and register
`commands::review::refresh_review` and `commands::review::reanchor_comments` in the
`generate_handler!` block (after line 63).

## Files touched

- NEW `src-tauri/src/anchor.rs`
- `src-tauri/src/git.rs`, `src-tauri/src/gh.rs`, `src-tauri/src/lib.rs`
- `src-tauri/src/commands/review.rs`
- `src/lib/api.ts`, `src/lib/types.ts`, `src/components/ReviewView.tsx`

## Gates

- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo clippy --manifest-path src-tauri/Cargo.toml`
- `pnpm exec tsc --noEmit` (frontend tasks)
