# Review screen

The review screen shows the diff of a GitHub PR or a local "virtual PR" (two branches), lets you comment on it, and tracks a verdict. Everything autosaves.

## Diff viewer

- **Split / unified.** Toggle in the review header. The initial mode comes from Settings (`defaultViewType`, default split).
- **Word-level highlighting.** Settings > "Highlight intra-line changes" (`markEdits`). Off by default. Even when on, it is skipped for files with more than 2000 changed lines.
- **Expand context.** Collapsed gaps between hunks, above the first hunk and below the last hunk have expanders: reveal 20 lines at a time, or the whole gap. The surrounding source is fetched on first use. Expansion past the file top or bottom is clamped (`leadingExpandRange` / `trailingExpandRange` in `src/lib/diff.ts`).
- **Large diffs.** A file with more changed lines (added + deleted) than Settings > "Max diff lines per file" (`largeDiffMaxLines`, default 3000, minimum 100) is not rendered. It shows a notice with an "Open in VSCode" button; "View file" is disabled and find-in-diff skips it.
- **File tree.** The left pane has `Files`, `Comments` and (PR reviews only) `Context` tabs. Files are grouped into a directory tree in diff order; single-child directory chains collapse into one node (`src/components`). The Comments tab lists threads with open/resolved filters. The Context tab shows the linked board card's metadata (plan files, links, notes, Claude sessions). Hide or show the pane with `b`.
- **Viewed.** Each file header has a checkbox that collapses the file. It is stored per review and still works on published reviews.
- **Go to definition.** `⌘/Ctrl + click` an identifier in the diff to see where it is declared. The backend runs `git grep` on the pinned commit (LEFT = base, RIGHT = head) with per-language declaration patterns (`src-tauri/src/definitions.rs`), and the popover shows up to 10 matches with a snippet, "Go to in diff" (when the file is in the diff) and "Open file". It needs a local clone with the commit; PRs without one find nothing. A faster, more precise tree-sitter index is planned: [spec 10](../../specs/10-definition-index.md).
- **Find.** `⌘/Ctrl + F` opens "Find in diff". `Enter` / `Shift+Enter` go to the next / previous match, `Esc` closes.

## Comments

Click a diff line to comment. Shift-click a second line on the same side to comment on a range. A comment is anchored by `(file, side, line)`, with `side` being `LEFT` (old) or `RIGHT` (new), plus `start_line` for ranges.

| Kind | How | Published as |
|---|---|---|
| Inline | Click a line (or `c` on the line cursor) | Inline GitHub review comment |
| Multi-line | Shift-click a second line | Inline comment with a start line |
| Whole-file | "Comment on file" in the file header | Folded into the review body under "File-level comments" |
| File view | "View file" opens the full head-side file in a slide-out pane; comment on any line, including lines outside the diff | Folded into the review body (`origin = file_view`) |

"View file" is disabled for deleted, binary and too-large files. Exports also keep whole-file and file-view comments, labelled "(whole file)" and "(file view)".

### Suggested changes

Write a fenced block with the `suggestion` info string; GitHub renders it as a suggested change.

````
```suggestion
replacement line(s)
```
````

For a RIGHT-side selection or comment, the composer shows "± Insert suggestion", which prefills the block with the current line(s). It is hidden for LEFT-side comments, whole-file and file-view comments, outdated comments, and lines that are not in the loaded diff text.

### Threads

- Reply to any root comment (one level of replies). Replies fold under their root on export and publish.
- "Resolve" / "Unresolve" on a comment thread. Resolved threads collapse to a one-line "Resolved" bar; click it to expand, "Hide resolved thread" to collapse again. Resolving collapses and unresolving expands.
- The file header shows "N comments, M resolved".
- Threads fetched from GitHub render separately and can be resolved from here; replies to them are drafts you publish with "Publish N draft replies".

## Staleness and "Update review"

Each review pins its own `base_sha` / `head_sha` ([ADR 0001](../adr/0001-pin-lives-on-review.md)). Reopening or refreshing the PR never moves an existing review.

- When the branch moves past the pin, the header shows a "new commits" badge. On a draft, **Update review** moves the pin to the latest commits and re-anchors the comments. A toast reports how many moved and how many could not follow (marked outdated). Published reviews never move.
- A comment anchored to a SHA its side has since moved past gets an `outdated · <sha>` badge, and the header shows "outdated comments".
- Review actions menu > **Re-anchor comments** moves comments onto the review's commits and clears the badges (drafts only).

## Other

- **Track on board.** For a PR that isn't on the Board (never tracked, or discarded), the review actions menu offers **Track on board**.
- **One open review per target.** Opening a PR or virtual PR from anywhere (Board, Inbox, Conversations, command palette, repository view, deep links, `cr create`) resumes its unpublished review (draft or pending on GitHub). A new review is created only when the target has none, or all of them are published.
- **Review switcher.** A target can have several reviews (for example a published round and a new draft). The header switcher appears only when there is more than one, and lets you start a new review on the same target once every review on it is published.
- **Autosave.** There is no save button. Comment text saves 400 ms after you stop typing; other edits hit the database as you make them.
- **Published reviews are locked.** Once published, edits and deletes are rejected, and a review cannot be published again. A review that is pending on GitHub must be submitted or discarded before editing. Viewed-file state, panel widths and sidebar state still change. Start a new review from the switcher for another round.
- Export, import and clipboard: see [Import and export](import-export.md).

## Keyboard shortcuts

From `BINDINGS` in `src/lib/keyboard.ts` (the `?` overlay):

| Keys | Action |
|---|---|
| `⌘/Ctrl + P` | Quick access — search reviews |
| `⌘/Ctrl + W` | Close current tab |
| `⌘/Alt + ←` / `⌘/Alt + →` | Navigate back / forward |
| `]` / `[` | Next / previous file |
| `n` / `p` | Next / previous comment thread |
| `j` / `k` | Move line cursor down / up (active file) |
| `c` | Comment on the focused line |
| `b` | Toggle file list sidebar |
| `⌘/Ctrl + F` | Find in the diff |
| `?` | Toggle this help |
| `Esc` | Close find / help / composer / line cursor |

Notes:

- All of these are wired. `⌘/Ctrl + P` (command palette), `⌘/Ctrl + W` and back/forward are handled in `src/App.tsx`; the back/forward modifier is `⌘` on macOS and `Alt` elsewhere. Mouse side buttons 3 / 4 also go back / forward.
- Single-key shortcuts do not fire while typing in an input, textarea or editable field.
- `'` toggles the chat panel but is not listed in the help overlay.

## Where it lives

- `src/components/ReviewView.tsx`: header, diff rendering, expanders, composer, threads, export modal.
- `src/components/FileJumpList.tsx`: left pane (file tree, comment list, Context tab).
- `src/components/FileViewPane.tsx`: full-file slide-out pane.
- `src/components/ReviewSwitcher.tsx`: switching between reviews of one target.
- `src/components/ContextPane.tsx`: board-card context tab.
- `src/lib/diff.ts`: parsing, tokenizing, expand ranges, suggestion fences, large-diff check.
- `src/lib/staleness.ts`: outdated-comment check.
- `src/lib/keyboard.ts`: shortcut table and key helpers.
- `src-tauri/src/commands/review.rs`: review/comment commands, `ensure_draft` locking, publish payload.
