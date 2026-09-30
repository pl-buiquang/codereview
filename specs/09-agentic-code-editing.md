# Spec 09 — Agentic code editing via chat

Extends the chat feature so Claude can **edit, commit, and push code** from within a review,
not just read it.

## Problem

`ensure_worktree` always creates a `--detach` worktree at the PR's head SHA.  Detached HEAD means:

- Claude can read and run code normally.
- Any commits Claude makes land on an anonymous commit with no branch ref.
- `git push` has no target — there is nothing to push to.
- The work is silently lost when `git worktree remove` runs on chat clear.

For Claude to produce usable output (a commit, a PR update, a fixup branch) the worktree needs a
**real branch checkout**.

## Decisions (to lock before implementation)

### Branch strategy

When the user switches the chat panel to **Edit mode**, a new branch is created from `head_sha`
and the worktree is checked out on that branch instead of detached:

```
branch name:  cr/<review_id>-<head_sha_short>
example:      cr/42-a1b2c3d4
```

Using `review_id` makes it easy to find and clean up; the SHA suffix prevents collisions if the
same review is re-opened after a head refresh.  The branch is created in the worktree with
`git checkout -b <branch> <head_sha>`, so the main repo's branch list is unaffected.

### Worktree path

Edit-mode worktree lives alongside the read-only one:

```
~/.codereview/worktrees/<repo_name>/edit-<review_id>-<head_sha_short>/
```

Distinct from the read-only path (`<sha_short>/`) so both can coexist.

### Lifecycle

| Event | Action |
|---|---|
| User enables Edit mode | Create branch + worktree if not already present; persist path in `chat.edit_worktree_path` (new nullable column) |
| User disables Edit mode / switches back to Read | Leave worktree intact (work in progress) |
| Chat cleared | `git worktree remove --force` the edit worktree; delete the branch only if it has no upstream and no commits beyond `head_sha` (i.e. nothing was done); otherwise leave the branch for the user |
| Review deleted | Same cleanup as chat clear |

### UI

The chat panel already has an `Auto / Plan` mode toggle.  Add a second toggle above or beside it:

```
Mode:  [ Read ]  [ Edit ]
```

- **Read** (default): current behaviour — detached worktree, Claude has read/run access only.
- **Edit**: branch worktree; Claude gets full write access including `git commit` and `git push`.

When Edit mode is active, show a small indicator with the branch name so the user knows where
any commits will land.

### Push / PR workflow

Claude is responsible for running `git push -u origin <branch>` from the worktree when asked.
The app does not automate this.  After a push the user can open a PR from the branch normally
(or Claude can run `gh pr create`).  No special app plumbing needed.

### Constraints

- Only one edit worktree per review at a time (enforce in `ensure_edit_worktree`; return existing
  path if `edit_worktree_path` is already set and the directory exists).
- The branch **must** be new — never reuse an existing branch name.  If the name already exists
  locally, append `-2`, `-3`, etc.
- `git worktree add` can create a new branch in one step:
  `git worktree add -b <branch> <path> <head_sha>`.
- The `code` CLI open button in the Worktrees tab already covers opening the edit worktree in
  VSCode; no extra UI needed there.

## Schema change

```sql
-- migration 0013_chat_edit_worktree.sql
ALTER TABLE chat ADD COLUMN edit_worktree_path TEXT;
```

## Files to touch

| File | Change |
|---|---|
| `src-tauri/src/worktree.rs` | Add `ensure_edit_worktree(repo, review_id, head_sha)` and `cleanup_edit_worktree` |
| `src-tauri/src/commands/chat.rs` | New command `chat_set_edit_mode(review_id, enabled)` that creates/tears down the edit worktree and persists the path |
| `src-tauri/src/db/migrations/0013_chat_edit_worktree.sql` | Schema change above |
| `src-tauri/src/db/models.rs` | Add `edit_worktree_path: Option<String>` to `Chat` |
| `src/components/ChatPanel.tsx` | Read/Edit toggle; show branch name when in Edit mode |
| `src/lib/api.ts` | Add `chatSetEditMode(reviewId, enabled)` |
