# Spec 11 — Threaded replies

Implements ROADMAP §1 "Threaded replies — the `comment.parent_id` column already exists but is
unused. Render replies under a root comment and let the user reply, like a GitHub thread."

## Problem

The schema and types were built for threads, but nothing reads or writes `parent_id`:

- `comment.parent_id INTEGER REFERENCES comment(id) ON DELETE CASCADE` exists since
  `src-tauri/src/db/migrations/0001_init.sql:52`, and `PRAGMA foreign_keys = ON` is set in
  `db/mod.rs:28`, so cascade-delete already works — it's just never exercised.
- `Comment.parent_id` is carried through `db/models.rs:106` and `src/lib/types.ts:57` but every
  writer leaves it NULL: `add_comment` (`commands/review.rs:894-921`) doesn't accept it, and
  `api.addComment` (`src/lib/api.ts:88-97`) doesn't send it.
- The UI renders a flat list: `ReviewView.tsx:591-607` groups line comments by anchor key only,
  `LineWidget` (`ReviewView.tsx:976-1020`) maps them straight to `CommentItem`s, and there is no
  reply affordance anywhere.
- Publish (`build_publish_payload`, `review.rs:709-750`) and export (`export.rs:13-78`, `81-117`)
  would treat a reply row as an independent top-level comment.

## Decisions (locked)

- **One level of nesting, GitHub-style.** A reply's parent must itself be a root
  (`parent_id IS NULL`); replying to a reply is rejected server-side.
- **Replies inherit the root's anchor server-side.** `add_comment` with `parent_id` set copies
  `file_path`, `subject_type`, `origin`, `side`, `line`, `start_line`, `diff_hunk`, and
  `anchored_head_sha` from the parent row and ignores the caller-supplied anchor args. One source
  of truth; a thread can never straddle two anchors.
- **Re-anchoring moves whole threads.** `reanchor_review_comments` (`review.rs:292-403`) considers
  **roots only**; every successful root UPDATE is followed by one cascading UPDATE matching
  `parent_id = root.id`. `ReanchorResult` counts **roots only** (so the numbers shown in the UI
  match visible threads). A `Lost` root leaves its replies untouched too — they keep the same
  stale SHA and show the same "outdated" badge.
- **Publish folds replies into the root comment's body as blockquotes.** The GitHub bulk reviews
  API (`POST /repos/{o}/{n}/pulls/{n}/reviews`, `gh.rs::post_review`) has no `in_reply_to` on its
  `comments[]` items — threaded replies cannot be created at publish time. Locked format, one
  block per reply in `created_at` order:

  ```
  {root body}

  > **reply by me:**
  > {reply line 1}
  > {reply line 2}
  ```

  The same folding applies to roots that land in the review body (file-level, file-view, and
  lost comments in `body_with_file_comments`, `review.rs:756-821`).
- **Export reuses the exact same folding.** `render_markdown` nests replies under their root with
  the same `fold_replies` helper (single deterministic format); `render_json` nests them as a
  `replies` array of `{body, created_at}` objects. Replies never appear top-level.
- **No migration.** `parent_id` shipped in 0001. Migrations 0007 and 0008 are reserved for specs
  12 and 16 — do not touch `db/mod.rs::MIGRATIONS`.
- **Reply affordance only on anchored threads** (diff `LineWidget`s and the file-view pane, which
  share `LineWidget`). File-level comments and orphan blocks render existing replies nested but
  offer no Reply button in v1 — orphaned roots should be re-anchored or deleted, not grown.
- **Frontend sends placeholder anchors for replies.** `api.addReply` calls the same `add_comment`
  command with `filePath: ""`, `side: "RIGHT"`, `line: 0` — documented as ignored server-side.
  Keeps one backend command and avoids making `add_comment`'s required params `Option`s.

## Design

### Backend — `src-tauri/src/commands/review.rs`

Factor the command body into a plain, testable helper (tauri `State` can't be constructed in
tests; mirror how `reanchor_review_comments` is a plain fn):

```rust
/// Validate a reply target: the parent must exist, belong to `review_id`, and be a
/// root comment (one level of nesting, GitHub-style). Returns the parent row so the
/// caller inherits its anchor columns.
fn parent_for_reply(conn: &Connection, review_id: i64, parent_id: i64) -> AppResult<Comment>;

#[allow(clippy::too_many_arguments)]
fn add_comment_impl(
    conn: &Connection,
    review_id: i64,
    file_path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
    diff_hunk: Option<String>,
    body: String,
    anchored_head_sha: Option<String>,
    parent_id: Option<i64>,
) -> AppResult<Comment>;

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn add_comment(/* existing params */, parent_id: Option<i64>, db: State<Db>) -> AppResult<Comment>;
```

- `add_comment_impl` starts with `ensure_draft(conn, review_id)?` (moved from the command, so
  replies on published reviews are rejected on the tested path), then:
  - `parent_id = None` → today's INSERT (`review.rs:910-915`) unchanged.
  - `parent_id = Some(pid)` → `let p = parent_for_reply(conn, review_id, pid)?;` then

    ```sql
    INSERT INTO comment
        (review_id, file_path, subject_type, origin, side, line, start_line,
         diff_hunk, body, parent_id, anchored_head_sha, created_at, updated_at)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
    ```

    with every anchor value taken from `p`, not the caller args. Note `subject_type`/`origin`
    are inherited too (a reply to a file comment is itself `subject_type='file'`; a reply to a
    file-view comment folds into the summary like its root).
- `parent_for_reply` errors (all `AppError::Other`): `"reply parent not found"`,
  `"reply parent belongs to a different review"`, `"replies can only target a top-level comment"`.
- `update_comment` / `delete_comment` (`review.rs:979-1006`) need **no change**: they look up
  `review_id` and call `ensure_draft`, which covers replies; deleting a root cascades replies via
  the existing FK.

`reanchor_review_comments` changes (two spots):

- Candidate filter (`review.rs:306`) gains `&& c.parent_id.is_none()`:

  ```rust
  .filter(|c| c.side == "RIGHT" && c.subject_type == "line" && c.origin != "file_view" && c.parent_id.is_none())
  ```

- After the root UPDATE (`review.rs:394-397`), cascade to the thread:

  ```rust
  conn.execute(
      "UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE parent_id = ?5",
      params![new_line, new_start, current, now(), c.id],
  )?;
  ```

  Replies inherit `start_line` from the root by construction, so overwriting it wholesale is
  correct. `result.reanchored` still increments once per root. Since `publish_review`
  (`review.rs:834-885`) already runs `reanchor_review_comments` before building the payload,
  threads arrive at publish fully moved.

`build_publish_payload` (`review.rs:709-750`):

- Inline-comments filter gains `&& c.parent_id.is_none()`.
- Each inline comment's `body` becomes
  `export::fold_replies(&c.body, replies.get(&c.id).map_or(&[][..], Vec::as_slice))` where
  `let replies = export::replies_by_root(&detail.comments);` is computed once at the top.

`body_with_file_comments` (`review.rs:756-821`): all three filters (`file_comments`,
`file_view_comments`, `lost_comments`) gain `&& c.parent_id.is_none()`, and each place that emits
`c.body.trim()` emits `fold_replies(...)` instead. (Multi-line bodies already break the `- L5: …`
list lines today; replies behave the same — accepted.)

### Backend — `src-tauri/src/export.rs`

```rust
use std::collections::HashMap;
use crate::db::models::Comment;

/// Map root comment id -> its replies, in created_at (then id) order.
pub fn replies_by_root(comments: &[Comment]) -> HashMap<i64, Vec<&Comment>>;

/// Append each reply to `body` as a "> **reply by me:**" blockquote block —
/// the single locked folding format shared by publish and Markdown export.
pub fn fold_replies(body: &str, replies: &[&Comment]) -> String;
```

- `fold_replies` with no replies returns `body.trim().to_string()`. Each reply appends
  `"\n\n> **reply by me:**"` then `"\n> {line}"` per line of `reply.body.trim()`.
- `render_markdown` (`export.rs:13-78`): compute `replies_by_root` once; `continue` on
  `c.parent_id.is_some()`; every `out.push_str(c.body.trim())` becomes the folded body (all three
  branches: whole-file, file-view, line).
- `render_json` (`export.rs:81-117`): skip replies in the top-level array; each root object gains
  `"replies": [{ "body": …, "created_at": … }, …]` (empty array when none).

### Frontend

NEW `src/lib/threads.ts` (pure, vitest-able):

```ts
import type { Comment } from "./types";

export interface CommentThread {
  root: Comment;
  replies: Comment[]; // created_at asc, id as tiebreak
}

/** Group flat comment rows into root threads. A reply whose parent is not in
 *  the input is promoted to a root (defensive — never drop data). Root order
 *  follows input order. */
export function groupThreads(comments: Comment[]): CommentThread[];
```

`src/lib/api.ts`:

```ts
addComment: (args: {
  reviewId: number; filePath: string; side: Side; line: number;
  startLine?: number | null; diffHunk?: string | null; body: string;
  anchoredHeadSha?: string | null; parentId?: number | null;
}) => invoke<Comment>("add_comment", args),

/** Reply to a root comment. Anchor args are placeholders — the backend copies
 *  the parent's anchor columns and ignores these. */
addReply: (args: { reviewId: number; parentId: number; body: string }) =>
  invoke<Comment>("add_comment", { ...args, filePath: "", side: "RIGHT" as Side, line: 0 }),
```

`src/components/ReviewView.tsx`:

- Grouping memo (`:591-607`): run `groupThreads` over the file's line comments first, then key
  each **thread** by the root's anchor (`keyByAnchor.get(`${root.side}:${root.line}`)`).
  `commentsByKey` becomes `Map<string, CommentThread[]>`, `orphans` becomes `CommentThread[]`.
- `fileComments` (`:584-588`): add `&& c.parent_id === null`? No — run `groupThreads` over the
  file-subject comments and render `ThreadItem`s (no reply affordance), so pre-existing replies
  to file comments still nest.
- NEW `ThreadItem` (exported next to `LineWidget`/`CommentItem`; `FileViewPane` imports from here,
  see `FileViewPane.tsx:19`):

  ```tsx
  export function ThreadItem({ thread, headSha, readOnly, showOrigin, canReply,
    onSaving, onSaved, onCommentsChanged }: {
    thread: CommentThread; headSha: string | null; readOnly: boolean;
    showOrigin?: boolean; canReply?: boolean;
    onSaving: () => void; onSaved: () => void; onCommentsChanged: () => void;
  })
  ```

  Holds `replyOpen` state. Renders the root `CommentItem` (passing `onReply` when
  `canReply && !readOnly`, plus `replyCount={thread.replies.length}`), then replies in an indented
  `.comment-replies` div (each a plain `CommentItem` — editable/deletable, no `onReply`), then a
  `Composer` when `replyOpen`. Submit:

  ```ts
  await api.addReply({ reviewId: thread.root.review_id, parentId: thread.root.id, body: text });
  ```

  then close, `onSaved()`, `onCommentsChanged()` (mirrors `submitSelectionComment`,
  `ReviewView.tsx:656-686`).
- `CommentItem` (`:1022-1132`): two optional props — `onReply?: () => void` renders a "Reply"
  button beside the delete button; `replyCount?: number` changes the delete confirm message
  (`:1113-1121`) to "Delete this comment and its N replies?" when > 0.
- `LineWidget` (`:976-1020`): `comments: Comment[]` → `threads: CommentThread[]`; maps to
  `ThreadItem` with `canReply={!readOnly}`. Widget loop (`:707-720`) passes
  `threads={commentsByKey.get(key) ?? []}`.
- `FileBody` (`:830-942`): `orphans: Comment[]` → `orphans: CommentThread[]`; orphan block
  (`:876-893`) renders `ThreadItem` with `canReply={false}`.
- `Composer` (`:1134-1173`): optional `submitLabel?: string` prop (default "Add comment");
  `ThreadItem` passes "Reply".

`src/components/FileViewPane.tsx`: grouping memo (`:96-110`) → `groupThreads` + thread keying,
same as above; `LineWidget` call (`:169-181`) passes `threads`; orphan list (`:226-237`) renders
`ThreadItem` with `canReply={false}`.

`src/styles.css` (near `.comment-item` at `:1032`): add `.comment-replies` (left margin + left
border to read as nesting) and a `.comment-reply-btn` if needed.

Widget sketch:

```
┌ line-widget ───────────────────────────────────┐
│ ┌ comment-item (root) ─────────────┐           │
│ │ [outdated badge?] body  [Reply] [🗑] │       │
│ └──────────────────────────────────┘           │
│   ┃ ┌ comment-item (reply) ────────┐  ← .comment-replies (indent + left border)
│   ┃ │ body                    [🗑] │           │
│   ┃ └──────────────────────────────┘           │
│ ┌ composer (when Reply clicked) ───┐           │
│ │ textarea       [Cancel] [Reply]  │           │
│ └──────────────────────────────────┘           │
└────────────────────────────────────────────────┘
```

Data flow: Reply click → `api.addReply` → `add_comment(parent_id)` → backend copies parent's
anchor columns → `onCommentsChanged()` invalidates `["review", id]` → `groupThreads` re-nests on
the next render. Re-anchor/refresh/publish all operate on roots; replies ride along via the
cascading UPDATE.

## Tasks

1. **review.rs:** add `parent_for_reply` + `add_comment_impl` (with `ensure_draft` inside), make
   the `add_comment` command a thin delegate gaining `parent_id: Option<i64>`. No `lib.rs`
   change (no new command). Rust tests for validation + inheritance.
2. **review.rs:** roots-only filter + cascading UPDATE in `reanchor_review_comments`; tests that
   threads move together and counts stay root-based.
3. **export.rs:** `replies_by_root` + `fold_replies`; nest replies in `render_markdown` /
   `render_json`; tests.
4. **review.rs:** fold replies in `build_publish_payload` + `body_with_file_comments` and exclude
   them from the inline array; payload tests.
5. **Frontend lib:** `src/lib/threads.ts` + `threads.test.ts`; `api.ts` `parentId`/`addReply` +
   `api.test.ts` cases.
6. **Frontend UI:** `ThreadItem`, `CommentItem` `onReply`/`replyCount`, `LineWidget` threads prop,
   `ReviewView` grouping/`FileBody`/file-comments wiring, `FileViewPane` wiring, `styles.css`.
7. **Docs:** drop the "Threaded replies" bullet from `ROADMAP.md` §1.

Steps 1–4 are independently buildable backend commits; 5 before 6.

## Test matrix

Rust — `commands/review.rs` `mod tests` (extend `seed_comment` family with a
`seed_reply(conn, review_id, parent_id, …)` or add a `parent_id` column to a new variant; reuse
`seed_comment_anchored` at `:1051` for re-anchor fixtures):

| Test | Asserts |
|---|---|
| `reply_inherits_root_anchor_columns` | `add_comment_impl` with `parent_id` + junk anchors (`""`, `"LEFT"`, `0`) stores the parent's `file_path/side/line/start_line/diff_hunk/subject_type/origin/anchored_head_sha` and `parent_id = root.id` |
| `reply_to_reply_is_rejected` | second-level reply → `Err` containing "top-level" |
| `reply_across_reviews_is_rejected` | parent from another review → `Err` |
| `reply_to_missing_parent_is_rejected` | bogus id → `Err` |
| `reply_on_published_review_is_rejected` | `ensure_draft` path fires for replies |
| `reply_to_file_comment_inherits_subject_type` | reply row has `subject_type='file'`, `origin` inherited |
| `deleting_root_cascades_replies` | `DELETE` root → reply row gone (FK cascade) |
| `reanchor_moves_replies_with_their_root` | temp-git H1→H2 fixture (mirror existing re-anchor tests): root+reply anchored to H1; after `reanchor_review_comments` both rows have the new `line` and `anchored_head_sha == H2`; `reanchored == 1` (root only) |
| `reanchor_lost_root_leaves_replies_untouched` | root on a replaced line: root and reply both keep old line + H1; `lost == 1` |
| `publish_payload_excludes_replies_and_folds_them` | `build_publish_payload`: `comments` array has 1 entry; its `body` contains `> **reply by me:**` and the quoted reply text |
| `publish_body_folds_replies_of_file_and_lost_comments` | `body_with_file_comments` output quotes replies; reply rows don't appear as separate entries |

Rust — `export.rs` `mod tests` (extend the existing `comment(…)` fixture with a `reply(parent_id,
body)` builder):

| Test | Asserts |
|---|---|
| `fold_replies_quotes_multiline_bodies` | each reply line prefixed `> `, blocks separated by blank lines, no-reply case returns trimmed body |
| `markdown_nests_replies_under_root` | reply text appears quoted under the root's `###` section and not as its own section |
| `json_groups_replies_under_root` | top-level `comments` excludes replies; `comments[0].replies[0].body` set; roots without replies get `[]` |

vitest:

| Test file | Cases |
|---|---|
| `src/lib/threads.test.ts` | replies sort by `created_at` then `id`; reply with missing parent promoted to root; root input order preserved; flat list (no replies) round-trips |
| `src/lib/api.test.ts` | `addComment` forwards `parentId`; `addReply` invokes `add_comment` with `{ reviewId, parentId, body, filePath: "", side: "RIGHT", line: 0 }` |

## Gates

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific: `git diff --stat src-tauri/src/db/` must show no migration changes.

## Manual verify

1. `pnpm tauri dev`; open a local virtual-PR review with at least one diff comment.
2. Click "Reply" on a comment, submit — reply renders indented under the root; reload the review
   (navigate away/back): nesting persists.
3. Edit a reply (autosave) and delete a reply — both work; delete the root → confirm dialog
   mentions the reply count; thread disappears entirely.
4. Open the file-view pane on a file with a file-view comment: Reply works there too.
5. Re-anchor check: commit a new line above a commented line on the head branch, hit Refresh →
   Re-anchor; root **and** reply move together (no "outdated" badge on either).
6. Export preview (Markdown): reply appears as a `> **reply by me:**` block under its root, not
   as a separate `###` section. JSON preview: `replies` array nested.
7. (Optional, needs a scratch PR) Publish a PR review with one root+reply: GitHub shows a single
   inline comment whose body ends with the quoted reply block.

## Out of scope

- Reply affordance on file-level comments, orphan blocks, and read-only GitHub `PrThread`s
  (`GithubThread` stays display-only).
- Posting true GitHub threaded replies (the per-comment `in_reply_to` REST endpoint after review
  creation) — publish folds into the root body instead, per locked decision.
- Resolve/unresolve + collapsing threads (Spec 12, reserved migration 0007).
- Multi-level nesting, reply authorship/avatars, reply-count badges in the file list.
- Any DB migration.
