# Spec 19 — PENDING (draft) GitHub reviews, narrow v1

Implements ROADMAP §3 "PENDING (draft) GitHub reviews — support GitHub's draft-review flow (add
comments to a pending review, then submit) in addition to one-shot publish" (`ROADMAP.md:43-44`).
**Narrow v1**: push the whole local draft to GitHub as a PENDING review, then Submit or Discard it.
No incremental comment-by-comment sync with the pending review (out of scope below).

## Problem

Publishing is all-or-nothing today: `publish_review` (`src-tauri/src/commands/review.rs:835-885`)
POSTs the payload with an `event` always set (`build_publish_payload`, `review.rs:709-714` maps the
verdict to `APPROVE`/`REQUEST_CHANGES`/`COMMENT`), so GitHub immediately *submits* the review.
There is no way to stage the review on GitHub as PENDING (visible only to you), eyeball it in the
GitHub UI, and then submit or throw it away:

- The REST API creates a PENDING review when `event` is **omitted** from
  `POST repos/{o}/{n}/pulls/{number}/reviews` — we never exercise that path
  (`gh::post_review`, `src-tauri/src/gh.rs:125-139`).
- `review.status` is a two-state CHECK — `'draft' | 'published'`
  (`src-tauri/src/db/migrations/0001_init.sql:34`) — with no value for "on GitHub but not
  submitted". The frontend mirrors it (`src/lib/types.ts:38`).
- There is no `gh.rs` wrapper for the submit endpoint
  (`POST .../reviews/{review_id}/events`) or the delete-pending endpoint
  (`DELETE .../reviews/{review_id}`).

## Decisions (locked)

- **Pending publish = same payload, `event` omitted.** Reuse `build_publish_payload` and strip the
  `"event"` key (`build_pending_payload` below) — GitHub's REST API makes a review PENDING exactly
  when `event` is absent. Body, inline comments, lost-comment folding and `commit_id` behave
  identically to normal publish. Do **not** change `build_publish_payload`'s signature (it has ~15
  test call sites).
- **New status value `'published_pending'`**, between `'draft'` and `'published'`. SQLite cannot
  ALTER a CHECK constraint, so migration **0009** rebuilds the `review` table (full recipe below).
- **Migration number 0009 is reserved for this spec.** 0007 = spec 12 (`resolved_at`), 0008 =
  spec 16 (`anchored_base_sha`). `db::migrate` is **positional** (`src-tauri/src/db/mod.rs:33-42`:
  the array index *is* the schema version) — so this spec's migration must be the **9th entry** in
  `MIGRATIONS`. Hard sequencing prerequisite: implement only after migrations 0007 and 0008 exist
  in the array; if they haven't landed yet, land them (or coordinate renumbering) first. Never
  skip-number the array.
- **While `published_pending` the review is locked locally, exactly like `published`.**
  `ensure_draft` rejects edits — the content already lives on GitHub; editing locally would
  silently desync. Allowed while pending: **Submit**, **Discard**, Export, `set_file_viewed`
  (already allowed on published, `review.rs:644`).
- **Only Submit and Discard leave the pending state.** Submit (`POST .../reviews/{id}/events` with
  the event derived from the *stored* verdict) → `status='published'`. Discard
  (`DELETE .../reviews/{id}`) → `status='draft'`, `github_review_id` cleared — the local draft is
  fully editable and re-publishable (either flavour) again. This is the **one sanctioned
  exception** to "a review can never be re-published".
- **`delete_review` is blocked while pending** (it currently has no guard, `review.rs:887-892`).
  Deleting the local row would orphan a PENDING review on GitHub, which then blocks all future
  pending publishes (GitHub allows max **one** pending review per user per PR). Discard first.
- **`published_at` stays NULL while pending; set on Submit.** It means "submitted to GitHub", not
  "uploaded". Pending state is recognizable from `status` alone.
- **The GitHub one-pending-review-per-PR 422 gets a friendly error.** `gh` surfaces the API error
  text (`run_gh_stdin`, `gh.rs:81-84`); when the message contains `"pending review"`
  (case-insensitive), remap to a clear `AppError::Other` telling the user to submit/discard their
  existing pending review (possibly created on github.com). Match only that substring — other 422s
  (e.g. stale `commit_id`) must surface verbatim.
- **Submit sends `{"event": …}` only.** Body and comments were attached at pending-creation time;
  re-sending the body via the events endpoint would duplicate-or-override it.
- **Discard failures change no local state.** If GitHub errors (e.g. the pending review was
  submitted or deleted on github.com), surface the error verbatim; reconciling external state is
  out of scope.
- **Status labels via one helper.** `statusLabel()` in a new `src/lib/status.ts` so the three badge
  sites don't render the raw string `published_pending`.

## Design

### 1. Migration — `src-tauri/src/db/migrations/0009_review_status_pending.sql` (NEW)

SQLite cannot modify a CHECK constraint in place; rebuild the table. Two other tables hold
`REFERENCES review(id) ON DELETE CASCADE` FKs — `comment` (`0001_init.sql:45`) and
`file_view_state` (`0002_file_view_state.sql`) — and with `foreign_keys = ON` (set at open,
`db/mod.rs:28`) a `DROP TABLE review` would fire those cascades and **delete every comment**.
`PRAGMA foreign_keys` is a no-op inside a transaction, so toggle it *outside* the transaction.
`db::migrate` runs each script via `execute_batch` in autocommit (no implicit wrapping
transaction), so this works as one script:

```sql
-- Add the 'published_pending' review status (review staged on GitHub as a
-- PENDING review, not yet submitted). SQLite cannot ALTER a CHECK constraint,
-- so rebuild the table. foreign_keys must be OFF around the DROP: comment and
-- file_view_state reference review(id) ON DELETE CASCADE, and DROP TABLE with
-- FKs on performs an implicit DELETE FROM that would fire those cascades.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE review_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id        INTEGER NOT NULL REFERENCES target(id) ON DELETE CASCADE,
    body             TEXT NOT NULL DEFAULT '',
    event            TEXT CHECK (event IN ('comment', 'approve', 'request_changes')),
    status           TEXT NOT NULL DEFAULT 'draft'
                     CHECK (status IN ('draft', 'published_pending', 'published')),
    published_at     TEXT,
    github_review_id INTEGER,
    last_exported_at TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

INSERT INTO review_new (id, target_id, body, event, status, published_at,
                        github_review_id, last_exported_at, created_at, updated_at)
    SELECT id, target_id, body, event, status, published_at,
           github_review_id, last_exported_at, created_at, updated_at
    FROM review;

DROP TABLE review;
ALTER TABLE review_new RENAME TO review;

-- Recreate the review index from 0001 (dropped with the old table). There are
-- no triggers and no other review indexes in migrations 0001-0008.
CREATE INDEX idx_review_target ON review(target_id);

COMMIT;

PRAGMA foreign_keys = ON;
```

Notes for the implementer:

- The explicit column list on `INSERT … SELECT` keeps row `id`s; `comment.review_id` /
  `file_view_state.review_id` rows therefore stay valid. `ALTER TABLE … RENAME TO` rewrites
  references to `review_new` only — the FK clauses in `comment`/`file_view_state` name `review`
  and end up pointing at the rebuilt table.
- The rebuilt table keeps `AUTOINCREMENT`; the `sqlite_sequence` row for `review` is dropped with
  the old table, but AUTOINCREMENT re-seeds from `max(id)` of the copied rows, so ids never reuse.
- Spec 12's 0007 and spec 16's 0008 touch only `comment` — the column list above (exactly 0001's
  `review` columns) is unaffected by them. Re-check `git grep -n review src-tauri/src/db/migrations`
  before landing in case another spec added a review column in the meantime; if so, add it to both
  the `CREATE TABLE` and the column lists.

Append to `MIGRATIONS` in `src-tauri/src/db/mod.rs:16-23` (as the **9th** entry, after 0007/0008):

```rust
include_str!("migrations/0009_review_status_pending.sql"),
```

No model change: `Review.status` is a plain `String` (`src-tauri/src/db/models.rs:66`).

### 2. Backend — `src-tauri/src/commands/review.rs`

**`ensure_draft` (`review.rs:107-114`)** — reject anything that isn't `draft`, with a
status-specific message:

```rust
fn ensure_draft(conn: &Connection, review_id: i64) -> AppResult<()> {
    match review_status(conn, review_id)?.as_str() {
        "draft" => Ok(()),
        "published_pending" => Err(AppError::Other(
            "this review is pending on GitHub — submit or discard it before editing".into(),
        )),
        _ => Err(AppError::Other(
            "this review is published and can no longer be edited".into(),
        )),
    }
}
```

**Shared event mapping** — extract from `build_publish_payload` (`review.rs:710-714`) so Submit
reuses it:

```rust
/// Map the stored verdict to a GitHub review event. None/'comment' => COMMENT.
fn gh_event(event: Option<&str>) -> &'static str {
    match event {
        Some("approve") => "APPROVE",
        Some("request_changes") => "REQUEST_CHANGES",
        _ => "COMMENT",
    }
}
```

**Pending payload** — wrapper, `build_publish_payload` untouched:

```rust
/// Same payload as a normal publish but with `event` omitted: GitHub's REST API
/// creates a PENDING review exactly when `event` is absent.
fn build_pending_payload(detail: &ReviewDetail) -> serde_json::Value {
    let mut payload = build_publish_payload(detail);
    payload.as_object_mut().expect("payload is an object").remove("event");
    payload
}
```

**Shared publish prep** — `publish_review` (`review.rs:835-885`) and the new pending command run
the same guards / remote lookup / head-refresh / re-anchor sequence. Factor lines 836-872 into:

```rust
/// Guards + remote lookup + head refresh + re-anchor shared by both publish
/// flavours. Returns the post-re-anchor detail and (owner, name, pr_number).
fn prepare_publish(review_id: i64, db: &State<Db>) -> AppResult<(ReviewDetail, String, String, i64)>
```

with the existing guard at `review.rs:840` tightened from `== "published"` to `!= "draft"`
(message: pending → "already pending on GitHub — submit or discard it", published → existing
"this review is already published"). The owner/name lookup (`review.rs:852-863`) moves into a
small helper reused by Submit/Discard:

```rust
/// (remote_owner, remote_name) for a repo, or an error if it has no GitHub remote.
fn pr_remote(conn: &Connection, repo_id: i64) -> AppResult<(String, String)>
```

**New commands** (mirror `publish_review`'s lock pattern, which holds the mutex across the `gh`
call — `review.rs:869-883`):

```rust
/// Push the draft to GitHub as a PENDING review (event omitted), then lock it
/// locally as 'published_pending'. github_review_id stores the pending review.
#[tauri::command]
pub fn publish_review_pending(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let (detail, owner, name, number) = prepare_publish(review_id, &db)?;
    let conn = db.0.lock().unwrap();
    let payload = build_pending_payload(&detail);
    let gh_id = gh::post_review(&owner, &name, number, &payload.to_string())
        .map_err(map_pending_conflict)?;
    let ts = now();
    conn.execute(
        "UPDATE review SET status = 'published_pending', github_review_id = ?1, updated_at = ?2
         WHERE id = ?3",
        params![gh_id, ts, review_id],
    )?;
    get_review_row(&conn, review_id)
}

/// Remap GitHub's "max one pending review per user per PR" 422 to a clear message.
fn map_pending_conflict(e: AppError) -> AppError {
    match &e {
        AppError::Gh(msg) if msg.to_lowercase().contains("pending review") => AppError::Other(
            "GitHub allows one pending review per reviewer on a PR — submit or discard \
             your existing pending review (here or on github.com) and retry".into(),
        ),
        _ => e,
    }
}

/// Submit the PENDING review on GitHub with the stored verdict; lock as 'published'.
#[tauri::command]
pub fn submit_pending_review(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let gh_id = ensure_pending(&detail)?;                       // see below
    let number = detail.target.github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name) = pr_remote(&conn, detail.target.repo_id)?;
    gh::submit_pending_review(&owner, &name, number, gh_id,
        gh_event(detail.review.event.as_deref()))?;
    mark_submitted(&conn, review_id)?;
    get_review_row(&conn, review_id)
}

/// Delete the PENDING review on GitHub; unlock the local draft.
#[tauri::command]
pub fn discard_pending_review(review_id: i64, db: State<Db>) -> AppResult<Review> {
    let conn = db.0.lock().unwrap();
    let detail = load_detail(&conn, review_id)?;
    let gh_id = ensure_pending(&detail)?;
    let number = detail.target.github_pr_number
        .ok_or_else(|| AppError::Other("PR target missing number".into()))?;
    let (owner, name) = pr_remote(&conn, detail.target.repo_id)?;
    gh::delete_pending_review(&owner, &name, number, gh_id)?;
    mark_discarded(&conn, review_id)?;
    get_review_row(&conn, review_id)
}
```

Pure helpers (testable without `gh`, per the Spec 00 shared-primitive convention):

```rust
/// The review must be 'published_pending' with a stored GitHub review id.
fn ensure_pending(detail: &ReviewDetail) -> AppResult<i64> {
    if detail.review.status != "published_pending" {
        return Err(AppError::Other("this review has no pending GitHub review".into()));
    }
    detail.review.github_review_id
        .ok_or_else(|| AppError::Other("pending review is missing its GitHub review id".into()))
}

/// published_pending -> published (sets published_at = now).
fn mark_submitted(conn: &Connection, review_id: i64) -> AppResult<()>;
/// published_pending -> draft (clears github_review_id; published_at stays NULL).
fn mark_discarded(conn: &Connection, review_id: i64) -> AppResult<()>;
```

`mark_submitted`: `UPDATE review SET status = 'published', published_at = ?ts, updated_at = ?ts
WHERE id = ?`. `mark_discarded`: `UPDATE review SET status = 'draft', github_review_id = NULL,
updated_at = ?ts WHERE id = ?`. (`comment.github_comment_id` is never written —
`0001_init.sql:54`, CLAUDE.md — so there is nothing else to clear on discard.)

**`delete_review` guard** (`review.rs:887-892`) — before the DELETE:

```rust
if review_status(&conn, review_id)? == "published_pending" {
    return Err(AppError::Other(
        "this review is pending on GitHub — discard the pending review before deleting it".into(),
    ));
}
```

### 3. `gh.rs` — two thin wrappers (after `post_review`, `gh.rs:139`)

```rust
/// Submit a PENDING review with the given event (APPROVE | REQUEST_CHANGES | COMMENT).
/// Body/comments were attached at creation, so the payload carries only the event.
pub fn submit_pending_review(
    owner: &str, name: &str, number: i64, review_id: i64, event: &str,
) -> AppResult<()> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/reviews/{review_id}/events");
    let ctx = GhRepo::Remote { owner: owner.to_string(), name: name.to_string() };
    let payload = serde_json::json!({ "event": event }).to_string();
    run_gh_stdin(&ctx, &["api", &endpoint, "--method", "POST", "--input", "-"], &payload)?;
    Ok(())
}

/// Delete a PENDING (never-submitted) review. GitHub rejects this for submitted reviews.
pub fn delete_pending_review(owner: &str, name: &str, number: i64, review_id: i64) -> AppResult<()> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/reviews/{review_id}");
    let ctx = GhRepo::Remote { owner: owner.to_string(), name: name.to_string() };
    run_gh(&ctx, &["api", &endpoint, "--method", "DELETE"])?;
    Ok(())
}
```

Both use absolute endpoints, so they work clone-less, mirroring `post_review` (`gh.rs:125-139`).

### 4. Registration — `src-tauri/src/lib.rs`

Add `commands::review::publish_review_pending`, `commands::review::submit_pending_review`,
`commands::review::discard_pending_review` to `generate_handler!` after
`commands::review::publish_review` (`lib.rs:64`).

### 5. Frontend boundary — `src/lib/types.ts`, `src/lib/api.ts`, `src/lib/status.ts` (NEW)

`types.ts:38`:

```ts
status: "draft" | "published_pending" | "published";
```

`api.ts`, after `publishReview` (`api.ts:71`):

```ts
publishReviewPending: (reviewId: number) =>
  invoke<Review>("publish_review_pending", { reviewId }),
submitPendingReview: (reviewId: number) =>
  invoke<Review>("submit_pending_review", { reviewId }),
discardPendingReview: (reviewId: number) =>
  invoke<Review>("discard_pending_review", { reviewId }),
```

`src/lib/status.ts` (NEW):

```ts
import type { Review } from "./types";

/** Human label for a review status badge. */
export function statusLabel(status: Review["status"]): string {
  return status === "published_pending" ? "pending on GitHub" : status;
}
```

### 6. UI — `src/components/ReviewView.tsx` (+ badge sites, CSS)

```
draft (GitHub PR):
┌ review-header-top ──────────────────────────────────────────────────────────┐
│ ← Back  Title  [draft]  Saved · Split|Unified · [Refresh] [Export]          │
│         [Publish]  [Publish as draft to GitHub]  [Delete]                   │
└──────────────────────────────────────────────────────────────────────────────┘
published_pending:
┌ review-header-top ──────────────────────────────────────────────────────────┐
│ ← Back  Title  [pending on GitHub]  Saved · Split|Unified · [Export]        │
│         [Submit review]  [Discard pending review]  [Delete (disabled)]      │
└──────────────────────────────────────────────────────────────────────────────┘
```

1. **`readOnly`** (`ReviewView.tsx:96`): `detail.review.status !== "draft"`. This alone locks the
   body/verdict editor, comment composers, Refresh and Re-anchor while pending (the backend
   `ensure_draft` change is the real guard; `reanchor_comments` already calls it).
2. **`ReviewHeader`** (`ReviewView.tsx:166`): alongside `published` (`:243`) add
   `const pending = review.status === "published_pending";`. New mutations next to
   `publishReview` (`:209-217`), each invalidating `["review", review.id]` + `["reviews"]`:
   - `publishPending` → `api.publishReviewPending(review.id)`; success toast
     "Draft review staged on GitHub as pending."
   - `submitPending` → `api.submitPendingReview(review.id)`; toast "Pending review submitted."
   - `discardPending` → `api.discardPendingReview(review.id)`; toast
     "Pending review discarded — draft unlocked."
3. **Buttons** (the `isPr && …` block, `:319-342`):
   - Keep the primary **Publish** button; show it (and the new secondary) only when
     `review.status === "draft"`.
   - Secondary **"Publish as draft to GitHub"** beside it (plain `<button>`, not `btn-primary`),
     `confirmDialog` message: "Stage this review on GitHub as a pending (draft) review? It will be
     visible only to you until submitted." (not `danger` — it is reversible).
   - When `pending`: replace both with **"Submit review"** (`btn-primary`, confirm: "Submit the
     pending review to the PR with verdict <verdict>? This cannot be undone.", `danger: true`) and
     **"Discard pending review"** (confirm: "Delete the pending review from GitHub? Your local
     draft is kept and unlocked.", `danger: true`).
   - **Delete** button (`:343-358`): `disabled={pending}` with a title explaining discard-first.
4. **Status badges**: render `statusLabel(review.status)` instead of the raw status at
   `ReviewView.tsx:267`, `RepoView.tsx:305`, `ReviewsView.tsx:209` (the `className` keeps the raw
   value: `status-badge published_pending`). `ReviewsView.tsx:33`'s `statusOf` group key needs no
   change (pending reviews form their own group); pass the group header through `statusLabel` if
   it renders the raw key.
5. **CSS** (`src/styles.css:718-731`): add `.status-badge.published_pending` (amber/“in flight”
   tint, same shape as `.draft`/`.published`).

### Data flow

Publish-as-draft → `prepare_publish` (guards, head refresh, re-anchor) → `build_pending_payload`
(no `event`) → `gh::post_review` → review row `status='published_pending'`,
`github_review_id=<id>` → UI shows pending chip, content read-only. Submit →
`gh::submit_pending_review(…, gh_event(verdict))` → `status='published'`, `published_at=now`.
Discard → `gh::delete_pending_review` → `status='draft'`, `github_review_id=NULL` → fully
editable again.

### Files touched

- NEW `src-tauri/src/db/migrations/0009_review_status_pending.sql`
- `src-tauri/src/db/mod.rs` (MIGRATIONS append + rebuild test)
- `src-tauri/src/commands/review.rs` (ensure_draft, gh_event, build_pending_payload,
  prepare_publish/pr_remote refactor, 3 new commands + pure helpers, delete_review guard, tests)
- `src-tauri/src/gh.rs` (`submit_pending_review`, `delete_pending_review`)
- `src-tauri/src/lib.rs` (register 3 commands)
- `src/lib/types.ts`, `src/lib/api.ts`, NEW `src/lib/status.ts` (+ tests)
- `src/components/ReviewView.tsx`, `src/components/RepoView.tsx`,
  `src/components/ReviewsView.tsx`, `src/styles.css`

## Tasks

1. Confirm migrations 0007/0008 are in `MIGRATIONS` (sequencing prerequisite). Migration 0009 +
   append + db rebuild test. Builds green on its own.
2. `ensure_draft` three-way + `gh_event` extraction + `build_pending_payload` + Rust tests.
3. `prepare_publish`/`pr_remote` refactor of `publish_review` (behaviour unchanged; guard now
   `!= "draft"`) + tests still green.
4. `gh.rs` wrappers + `publish_review_pending` (with `map_pending_conflict`) +
   `submit_pending_review`/`discard_pending_review` + `mark_submitted`/`mark_discarded`/
   `ensure_pending` helpers + `delete_review` guard + `lib.rs` registration + tests.
5. `types.ts`/`api.ts`/`status.ts` + vitest tests.
6. UI: header buttons, pending chip via `statusLabel`, badge sites, CSS.

## Test matrix

Rust — `src-tauri/src/db/mod.rs`:

| Test | Asserts |
|---|---|
| `migration_0009_rebuilds_review_preserving_rows` | apply `MIGRATIONS[..8]` only; seed repository/target/review (`status='published'`, fixed id)/comment/file_view_state; apply the 0009 script; review row intact with same id+status, comment & file_view_state rows still present, `PRAGMA foreign_key_check` empty |
| `migration_0009_check_accepts_pending_rejects_bogus` | on `open_memory()`: INSERT review with `status='published_pending'` succeeds; `status='bogus'` fails the CHECK |
| `migration_0009_cascade_still_works` | after full migration, DELETE a review → its comment and file_view_state rows cascade away |

Rust — `src-tauri/src/commands/review.rs` (use `open_memory()` + existing seed helpers; all new
commands guard *before* any `gh` call, so guard tests need no network):

| Test | Asserts |
|---|---|
| `ensure_draft_blocks_pending_reviews` | `status='published_pending'` → `Err`, message mentions "pending"; existing `ensure_draft_blocks_published_reviews` (`review.rs:1134`) still green |
| `gh_event_mapping` | `approve`→`APPROVE`, `request_changes`→`REQUEST_CHANGES`, `comment`/`None`→`COMMENT` |
| `pending_payload_omits_event` | `build_pending_payload` has no `"event"` key; `comments`, `commit_id` and `body` identical to `build_publish_payload` output |
| `publish_rejects_pending_status` | review at `published_pending` → `publish_review` (and `publish_review_pending`) error before any network |
| `submit_discard_reject_non_pending` | `ensure_pending` on a `draft` and a `published` detail → `Err` |
| `ensure_pending_requires_github_id` | `published_pending` with `github_review_id = None` → `Err` |
| `mark_submitted_transitions` | `published_pending` → `published`, `published_at` set, `github_review_id` kept |
| `mark_discarded_transitions` | `published_pending` → `draft`, `github_review_id` NULL, `published_at` still NULL |
| `map_pending_conflict_remaps_only_pending_422` | `AppError::Gh("…one pending review per pull request…")` → `AppError::Other` friendly text; `AppError::Gh("HTTP 422: other")` passes through unchanged |
| `delete_review_blocked_while_pending` | review at `published_pending` → `delete_review` errs and the row survives |

Vitest:

| Test | Asserts |
|---|---|
| `api.test.ts`: three wrappers | `invoke` called with `("publish_review_pending", { reviewId })` etc. (mirror the `publishReview` case at `api.test.ts:88`) |
| `src/lib/status.test.ts`: `statusLabel` | `draft`→`"draft"`, `published`→`"published"`, `published_pending`→`"pending on GitHub"` |

## Gates

Standard suite (all must pass):

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
```

Spec-specific: launch once against a DB created at version 0008 and confirm migration 0009 applies
cleanly (app starts, existing reviews open, comments intact); run
`sqlite3 <db> "PRAGMA foreign_key_check;"` → empty.

## Manual verify (`pnpm tauri dev`, `gh` authenticated, a test PR you can review)

1. Open a GitHub PR review, add a line comment + a verdict (e.g. Request changes) + a body.
2. Click **Publish as draft to GitHub** → confirm. Chip flips to "pending on GitHub"; editor,
   composers, Refresh, Re-anchor and Delete are disabled. On github.com the PR shows your pending
   review (visible only to you) with the inline comment.
3. Try to add a comment via the diff — blocked (read-only); confirm the backend guard with a
   direct edit attempt if paranoid.
4. Click **Publish as draft** again from a *second* local review on the same PR → friendly
   one-pending-review error (not a raw 422 dump).
5. Click **Discard pending review** → pending review disappears on github.com; local review is a
   `draft` again, fully editable; `github_review_id` is NULL (check via sqlite3 if desired).
6. Publish as draft again, then **Submit review** → on github.com the review is submitted with the
   stored verdict (CHANGES_REQUESTED); locally the status is `published`, `published_at` set, and
   the review is locked exactly like a one-shot publish.
7. Regression: one-shot **Publish** on a fresh draft still works unchanged.

## Out of scope

- **Incremental pending-review sync** — adding/editing individual comments on an existing GitHub
  pending review (`POST .../pulls/{n}/comments` with `pull_request_review_id`); v1 freezes the
  local draft while pending. Likewise no `comment.github_comment_id` writes.
- Importing/adopting a pending review created on github.com (the 422 message points the user at
  it; we never list or take over foreign pending reviews).
- Editing the verdict between pending-publish and Submit (the stored verdict at pending time is
  what Submit sends; discard + re-publish to change it).
- Reconciling external state changes (pending review submitted/deleted on github.com while we
  hold `published_pending`) — Submit/Discard surface GitHub's error verbatim.
- Any change to export, re-anchoring, or the threads display; local virtual-PR targets (pending
  publish keeps `publish_review`'s GitHub-PR-only guard).
