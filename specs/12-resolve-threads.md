# Spec 12 — Resolve / unresolve local comment threads

Implements ROADMAP §1 "Resolve / unresolve threads — mark a comment thread resolved; collapse
resolved threads" (`ROADMAP.md:27`). Builds directly on Spec 11 (threaded replies) — same UI
components, same wave; **implement strictly after Spec 11 lands** in the same worktree.

## Problem

There is no notion of a resolved thread for the app's *own* comments:

- The `comment` table (`src-tauri/src/db/migrations/0001_init.sql` + 0003/0004) has no
  resolved column; the `Comment` model (`src-tauri/src/db/models.rs:92-111`) carries
  `parent_id` (threads, Spec 11) but nothing to mark a thread settled.
- GitHub PR threads already display `isResolved` (`src/lib/types.ts:155`,
  `src/components/GithubThread.tsx`) — read-only, ephemeral. Local comments have no equivalent,
  so a long review accumulates handled comments with no way to tick them off.
- The file header (`src/components/ReviewView.tsx:732-781`) shows only `+add/−del` stats — no
  comment/resolution progress.

## Decisions (locked)

- **Migration number 0007 is reserved for this spec**: `0007_comment_resolved.sql`. (0008 is
  reserved for spec 16 — do not take it.)
- **Storage = `resolved_at TEXT` on `comment`**, NULL = unresolved. Only meaningful on **root**
  comments (`parent_id IS NULL`); replies inherit the root's state. Enforced in the command, not
  by a CHECK (SQLite CHECKs can't reference another row anyway).
- **`set_comment_resolved` is `ensure_draft`-guarded** and **rejects non-root comments** with an
  `AppError::Other`. Resolution is part of the draft and locks with it on publish.
- **Resolving is idempotent**: resolving an already-resolved root simply refreshes `resolved_at`;
  unresolving an unresolved root is a no-op UPDATE. No error either way — keeps the UI handler
  trivial.
- **Publish is unchanged**: resolved threads still publish inline (GitHub has no
  "resolved-at-creation" — a review comment can only be resolved after it exists on GitHub).
  `build_publish_payload` (`src-tauri/src/commands/review.rs:709`) is **not touched**.
- **Export marks resolution**: Markdown appends ` (resolved)` to the comment heading; JSON gains a
  `resolved_at` field. Resolved comments are never dropped from exports.
- **Collapse is per-thread UI state** (local `useState`, default collapsed when resolved), not
  persisted — same philosophy as diff-gap expansion ("ephemeral by design",
  `ReviewView.tsx:506-508`).
- **Any root comment is resolvable** — diff-line threads, whole-file comments, and file-view
  comments alike (the Resolve button lives in `CommentItem`, which all three render through).
  Collapsed-bar rendering ships for the diff threads and the file-comments block; the
  FileViewPane keeps full rendering (out of scope below).

## Design

### 1. Migration — `src-tauri/src/db/migrations/0007_comment_resolved.sql` (NEW)

```sql
-- When a comment thread was resolved (ISO-8601 UTC). NULL = unresolved.
-- Only meaningful on root comments (parent_id IS NULL); replies inherit the
-- root's state. Root-only is enforced by set_comment_resolved, not a CHECK.
ALTER TABLE comment ADD COLUMN resolved_at TEXT;
```

Append to the `MIGRATIONS` array in `src-tauri/src/db/mod.rs:16-22`:

```rust
include_str!("migrations/0007_comment_resolved.sql"),
```

(The existing `migrations_apply` test asserting `user_version == MIGRATIONS.len()` covers the
bump automatically.)

### 2. Model — `src-tauri/src/db/models.rs`

Add to `Comment` (after `github_comment_id`, `models.rs:108`) and to `Comment::from_row`:

```rust
pub resolved_at: Option<String>,
// from_row:
resolved_at: row.get("resolved_at")?,
```

Every `Comment { .. }` struct literal must gain `resolved_at: None`: the test fixtures in
`src-tauri/src/export.rs` (`comment()` at ~155) and `src-tauri/src/commands/review.rs` tests
(literal at ~1290-1310), plus any fixture Spec 11 added.

### 3. Command — `src-tauri/src/commands/review.rs`

Mirror the `update_comment` shape (`review.rs:980-993`: look up `review_id`, `ensure_draft`,
UPDATE, touch `review.updated_at`). Helper + thin command per the Spec 00 shared-primitive
convention so tests hit the helper with a bare `Connection`:

```rust
/// Mark a root comment's thread resolved/unresolved. Pure helper; caller holds the lock.
fn set_resolved(conn: &Connection, comment_id: i64, resolved: bool) -> AppResult<()> {
    let (review_id, parent_id): (i64, Option<i64>) = conn.query_row(
        "SELECT review_id, parent_id FROM comment WHERE id = ?1",
        params![comment_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure_draft(conn, review_id)?;
    if parent_id.is_some() {
        return Err(AppError::Other(
            "only the root comment of a thread can be resolved".into(),
        ));
    }
    let ts = now();
    let resolved_at: Option<&str> = resolved.then_some(ts.as_str());
    conn.execute(
        "UPDATE comment SET resolved_at = ?1, updated_at = ?2 WHERE id = ?3",
        params![resolved_at, ts, comment_id],
    )?;
    conn.execute(
        "UPDATE review SET updated_at = ?1 WHERE id = ?2",
        params![ts, review_id],
    )?;
    Ok(())
}

#[tauri::command]
pub fn set_comment_resolved(comment_id: i64, resolved: bool, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    set_resolved(&conn, comment_id, resolved)
}
```

Register `commands::review::set_comment_resolved` in the `generate_handler!` block in
`src-tauri/src/lib.rs` (after `commands::review::delete_comment`, line 60).

### 4. Export — `src-tauri/src/export.rs`

- `render_markdown` (`export.rs:13`): for **each** of the three comment shapes (whole-file at
  :41, file-view at :53, diff-line at :64), append ` (resolved)` to the `###` heading when
  `c.resolved_at.is_some()`. E.g. the diff-line heading becomes:

  ```rust
  let resolved = if c.resolved_at.is_some() { " (resolved)" } else { "" };
  out.push_str(&format!("### {} ({}){resolved}\n\n", loc, c.side));
  ```

  Replies (Spec 11) render under their root; only the root carries the marker (the command
  guarantees `resolved_at` is NULL on replies).
- `render_json` (`export.rs:81`): add `"resolved_at": c.resolved_at` to the per-comment `json!`
  object (`export.rs:88-97`).

### 5. Frontend boundary — `src/lib/api.ts`, `src/lib/types.ts`

`types.ts` `Comment` (after `github_comment_id`, :59):

```ts
resolved_at: string | null;
```

`api.ts`, comments section (after `deleteComment`, :110):

```ts
setCommentResolved: (commentId: number, resolved: boolean) =>
  invoke<void>("set_comment_resolved", { commentId, resolved }),
```

### 6. Helper — `src/lib/text.ts` (NEW)

One pure, vitest-able function for the collapsed bar's label:

```ts
/** First non-empty line of a comment body, trimmed and hard-capped for one-line display. */
export function summaryLine(body: string, max = 80): string {
  const line = body.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
  return line.length > max ? line.slice(0, max - 1) + "…" : line;
}
```

### 7. UI — `src/components/ReviewView.tsx` (Spec 11's thread components)

Spec 11 turns `LineWidget` (`ReviewView.tsx:976-1020`) groups into threads (root `CommentItem` +
replies, keyed off `parent_id`). Spec 12 layers on top of whatever container Spec 11 names —
referred to here as **the thread container** (root + its replies + reply composer):

```
┌ diff-file-header ───────────────────────────────────────────────────────┐
│ src/foo.ts  +12 −3 · 3 comments, 1 resolved  [💬 Comment on file] [View…]│
└──────────────────────────────────────────────────────────────────────────┘
   │ line widget — UNRESOLVED thread (Spec 11 layout + one new button)
   │ ┌────────────────────────────────────────────────────────┐
   │ │ root comment editor                       [Resolve] [🗑]│
   │ │   └ replies… / [Reply] composer (Spec 11)              │
   │ └────────────────────────────────────────────────────────┘
   │ line widget — RESOLVED thread, collapsed (default)
   │ ┌────────────────────────────────────────────────────────┐
   │ │ ✓ Resolved — Consider renaming this…                  ▸│  ← whole bar clickable
   │ └────────────────────────────────────────────────────────┘
   │     …expanded: full thread, button reads [Unresolve]
```

Changes:

1. **`CommentItem`** (`ReviewView.tsx:1022-1132`): next to the existing delete button
   (`:1109-1129`), when `!readOnly && comment.parent_id == null`, render a Resolve/Unresolve
   button:

   ```tsx
   <button
     className="btn-resolve"
     onClick={async () => {
       try {
         await api.setCommentResolved(comment.id, !comment.resolved_at);
         onCommentsChanged();
       } catch (e) { toast.error(String(e)); }
     }}
   >
     {comment.resolved_at ? "Unresolve" : "Resolve"}
   </button>
   ```

   No optimistic update — `onCommentsChanged()` already invalidates `["review", reviewId]` and
   the round-trip is local SQLite.

2. **Thread container collapse**: in the thread container component, when
   `root.resolved_at != null`, default to a collapsed one-line bar
   (`useState(() => root.resolved_at != null)` keyed per thread; re-derive when
   `root.resolved_at` flips so resolving collapses and unresolving expands):

   ```tsx
   <button className="resolved-bar" onClick={() => setExpanded(true)}>
     ✓ Resolved — {summaryLine(root.body)}
   </button>
   ```

   Clicking expands to the full thread (root editor, replies, Unresolve button); a small
   collapse affordance (re-click header / "▾") returns to the bar. In `readOnly` mode the bar
   still expands/collapses; only the buttons are hidden.

3. **File-comments block** (`ReviewView.tsx:782-803`): wrap each whole-file root in the same
   collapse behavior (these have no replies, so the container is trivial).

4. **File header counts** (`ReviewView.tsx:732-781`, inside the `diff-stats` span after the
   `−del` count): compute from `detail.comments` for this `path`:

   ```ts
   const roots = detail.comments.filter((c) => c.file_path === path && c.parent_id == null);
   const resolvedCount = roots.filter((c) => c.resolved_at != null).length;
   ```

   Render `· {roots.length} comments, {resolvedCount} resolved` only when `roots.length > 0`;
   omit `, M resolved` when `resolvedCount === 0`. (Counts include file-view-origin roots — they
   belong to the file even though they render in the pane.)

5. **CSS** (`src/App.css` or wherever `comment-item`/`diff-stats` live): `.resolved-bar`
   (full-width, muted, single line, ellipsis overflow) and `.btn-resolve`.

### Data flow

Click Resolve → `invoke("set_comment_resolved", { commentId, resolved })` → `set_resolved`
(ensure_draft + root check + UPDATE) → frontend invalidates `["review", reviewId]` →
`get_review` returns comments with `resolved_at` set → thread re-renders collapsed; header
counts update. Publish path reads the same rows but ignores `resolved_at` entirely.

### Files touched

- NEW `src-tauri/src/db/migrations/0007_comment_resolved.sql`
- `src-tauri/src/db/mod.rs` (MIGRATIONS append)
- `src-tauri/src/db/models.rs` (Comment field + from_row)
- `src-tauri/src/commands/review.rs` (`set_resolved` helper + `set_comment_resolved` command + tests)
- `src-tauri/src/export.rs` (markdown `(resolved)` marker, json `resolved_at`, fixture field)
- `src-tauri/src/lib.rs` (register command)
- `src/lib/types.ts`, `src/lib/api.ts`, NEW `src/lib/text.ts` (+ test)
- `src/components/ReviewView.tsx` (+ Spec 11's thread component if split out), CSS file

## Tasks

1. Migration 0007 + MIGRATIONS append + `Comment.resolved_at` in models.rs + fix all struct
   literals (export.rs/review.rs fixtures). Builds green on its own.
2. `set_resolved` helper + `set_comment_resolved` command + lib.rs registration + Rust tests
   (resolve/unresolve roundtrip, non-root rejection, published rejection).
3. Export changes in export.rs + tests (markdown marker, json field, publish-payload
   still-included test in review.rs).
4. `api.ts` wrapper + `types.ts` field + `src/lib/text.ts` + vitest tests (api wrapper,
   summaryLine).
5. UI: CommentItem button, thread-container collapse, file-comments collapse, header counts,
   CSS.

## Test matrix

Rust — `src-tauri/src/commands/review.rs` tests (use `open_memory()`, existing `seed_repo`/
`seed_comment` helpers; seed a reply with a direct INSERT setting `parent_id`, or reuse Spec 11's
reply seeder if it added one):

| Test | Asserts |
|---|---|
| `set_resolved_roundtrip` | resolve sets non-NULL `resolved_at` and bumps `updated_at`; unresolve clears it back to NULL |
| `set_resolved_rejects_reply` | comment with `parent_id = Some(root)` → `Err`, message mentions root; row untouched |
| `set_resolved_blocked_when_published` | mark review `status='published'`, `set_resolved` → `Err` (ensure_draft) |
| `set_resolved_idempotent` | resolving twice succeeds; second call refreshes `resolved_at` |
| `publish_payload_includes_resolved_comments` | a RIGHT diff comment with `resolved_at = Some(..)` still appears in `build_publish_payload`'s `comments` array |
| `new_comment_defaults_unresolved` | row inserted via `add_comment` path has `resolved_at` NULL |

Rust — `src-tauri/src/export.rs` tests (extend existing fixture, give `comment()` callers a
resolved variant):

| Test | Asserts |
|---|---|
| `markdown_marks_resolved_comment` | heading reads `### src/main.rs:3 (RIGHT) (resolved)`; body still present |
| `markdown_marks_resolved_file_comment` | `### src/main.rs (whole file) (resolved)` |
| `markdown_unresolved_has_no_marker` | `(resolved)` absent when `resolved_at` is None |
| `json_carries_resolved_at` | `comments[0]["resolved_at"]` equals the timestamp; null when unresolved |

Vitest:

| Test | Asserts |
|---|---|
| `api.test.ts`: setCommentResolved wrapper | `invoke` called with `("set_comment_resolved", { commentId: 3, resolved: true })` (mirror the updateComment case at `api.test.ts:101-102`) |
| `src/lib/text.test.ts`: summaryLine | first non-empty line picked, trimmed; >80 chars truncated with `…`; empty body → `""` |

## Gates

Standard suite (all must pass):

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test                                                   # vitest run
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
```

Spec-specific: launch once against an existing DB to confirm migration 0007 applies cleanly to a
database created at 0006 (no error on startup, old comments load with `resolved_at = null`).

## Manual verify (`pnpm tauri dev`)

1. Open an existing local review with comments (or create one between two branches and add a
   line comment + a reply via Spec 11).
2. Click **Resolve** on the root — the thread collapses to `✓ Resolved — <first line>`; the file
   header shows e.g. `1 comments, 1 resolved`.
3. Click the bar — thread expands, replies visible, button reads **Unresolve**; click it — bar
   gone, header drops the resolved count.
4. Resolve again, close the review, reopen — still collapsed (state persisted in SQLite).
5. Try Resolve on a reply via devtools/SQL or confirm no button renders on replies.
6. Export → Markdown preview: the resolved comment's heading carries `(resolved)`; JSON shows
   `"resolved_at"`.
7. On a GitHub-PR review: publish a draft containing a resolved thread — the comment posts
   inline normally (gh web UI shows it unresolved, as expected). Re-open the published review:
   Resolve buttons are gone (read-only) and clicking is impossible; verify via SQL that
   `set_comment_resolved` on it errors if invoked.

## Out of scope

- Resolving **GitHub** PR threads via the API (`resolveReviewThread` GraphQL) — ROADMAP §3,
  separate spec; `PrThread.isResolved` stays read-only display.
- Pushing local resolution state to GitHub on publish (no such API at review creation).
- Collapsed-bar rendering inside `FileViewPane.tsx` (the Resolve button appears there via the
  shared `CommentItem`; full rendering is kept).
- Persisting per-thread expand/collapse UI state.
- A SQL CHECK or trigger enforcing root-only `resolved_at`; cascading `resolved_at` cleanup when
  a resolved root is deleted (FK already cascades the rows themselves).
- Filtering/sorting threads by resolution, keyboard shortcuts, bulk resolve.
