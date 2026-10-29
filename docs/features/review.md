# Review screen

The review screen shows the diff of a GitHub PR or a local "virtual PR" (two branches), lets you comment on it, and tracks a verdict. Everything autosaves.

## Diff viewer

- **Split / unified.** Toggle in the review header. The initial mode comes from Settings (`defaultViewType`, default split).
- **Word-level highlighting.** Settings > "Highlight intra-line changes" (`markEdits`). Off by default. Even when on, it is skipped for files with more than 2000 changed lines.
- **Expand context.** Collapsed gaps between hunks, above the first hunk and below the last hunk have expanders: reveal 20 lines at a time, or the whole gap. The surrounding source is fetched on first use. Expansion past the file top or bottom is clamped (`leadingExpandRange` / `trailingExpandRange` in `src/lib/diff.ts`).
- **Large diffs.** A file with more changed lines (added + deleted) than Settings > "Max diff lines per file" (`largeDiffMaxLines`, default 3000, minimum 100) is not rendered. It shows a notice with an "Open in VSCode" button; "View file" is disabled and find-in-diff skips it.
- **File tree.** The left pane has `Files`, `Comments` and (PR reviews only) `Context` tabs. Files are grouped into a directory tree in diff order; single-child directory chains collapse into one node (`src/components`). The Comments tab lists threads with open/resolved filters. The Context tab shows the linked board card's metadata (plan files, links, notes, Claude sessions). Hide or show the pane with `b`.
- **Viewed.** Each file header has a checkbox that collapses the file. It is stored per review and still works on published reviews.
- **Go to definition.** Hold `⌘/Ctrl` and hover an identifier: it is underlined when a declaration exists. Click it to jump (see [Go to definition](#go-to-definition)).
- **Find.** `⌘/Ctrl + F` opens "Find in diff", or "Find in file" while the file pane is open (it searches that file only). `Enter` / `Shift+Enter` go to the next / previous match, `Esc` closes.

## Go to definition

Holding `⌘/Ctrl` over a word in the diff underlines it **only if a declaration of it exists** in the pinned commit (LEFT = base, RIGHT = head); the first `⌘/Ctrl` press is enough to start looking. `⌘/Ctrl + click` opens a popover at the pointer right away (with a spinner while the lookup runs):

- one declaration, and its line is in the diff: the view scrolls to that line and flashes it, no popover;
- otherwise the popover lists up to 10 declarations with a snippet, "Go to line" (line is in the diff), "View file" (the file pane, scrolled to the declaration) and "Open" (VSCode, in the chat worktree, at that line). The clicked file comes first, then the other files in the diff.

The same works inside the file pane (head side): one declaration opens straight in the pane.

A jump is a step in the app's Back/Forward history (`⌘←` / `⌘→`, mouse side buttons): Back returns to the scroll position (and closed file pane) you jumped from, Forward to where the jump landed (`src/lib/navHistory.ts`, `recordJump`).

It needs a local clone that has the commit; PRs without one find nothing.

**How it works** (`src-tauri/src/definitions/`). No language server and no index, so cost does not grow with the repo or the number of open reviews, and nothing is kept between lookups:

1. one loose `git grep -l` lists the files containing something shaped like a declaration of the word;
2. the first 30 of them (clicked file, then files in the diff, then the rest) are read in one `git cat-file --batch` and parsed with tree-sitter, keeping only declarations *named* the word. Hits in comments and strings fall away;
3. languages without a grammar use a strict one-line `git grep -n` instead.

At most 2 lookups run at once; a hover lookup is dropped as soon as a newer one starts. Files over 512 KB are skipped. On a 1,500-file Ruby repo a lookup takes 60–120 ms.

### Supported languages

| Language | Extensions | Mode |
|---|---|---|
| Ruby | `rb`, `rake` (`erb` as a starting point) | tree-sitter |
| Python | `py`, `pyi` | tree-sitter |
| Rust | `rs` | tree-sitter |
| TypeScript / JavaScript | `ts`, `tsx`, `mts`, `cts`, `js`, `jsx`, `mjs`, `cjs` | tree-sitter |
| TypeScript / JavaScript (single-file components) | `vue`, `svelte` | grep only |
| C# | `cs` | tree-sitter |
| Go | `go` | grep only |
| Java / Kotlin / Scala / Swift / PHP / Dart | `java`, `kt`, `kts`, `scala`, `swift`, `php`, `dart` | grep only |
| C / C++ | `c`, `h`, `cpp`, `cc`, `hpp`, `cxx` | grep only |

Any other file type finds nothing. A unit test checks that this table lists every extension of the registry.

### Adding a language

Everything is declared in `src-tauri/src/definitions/languages.rs` (`LANGUAGES`).

- **Grep only (quick).** Add a `Language` entry: `extensions`, `grep_globs` (pathspecs to search) and an `exact` function returning ERE patterns that match a one-line declaration of `sym`. Add a case to the `grep_only_languages_and_unknown_files` test.
- **Tree-sitter (precise).**
  1. `cargo add tree-sitter-<lang>` in `src-tauri`.
  2. Write `src-tauri/src/definitions/queries/<lang>.scm`: a pattern per kind of declaration, capturing the declared identifier as `@name` and the whole declaration as `@definition.<kind>`. Look at the existing queries; `tree-sitter parse` or a unit test in `tags.rs` shows the node names.
  3. Add a `Parser` (`grammar`, `query`, `extensions`) to the language's `parsers`, plus `parsed_globs` and a `shortlist` function: loose ERE patterns that every declaration of `sym` matches (false positives are fine, misses are not).
  4. Add a case to `src-tauri/src/definitions/tags.rs` tests. `every_registered_query_compiles` catches queries that do not fit the grammar.
- Update the table above.

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

- When the branch moves past the pin, the header shows a "new commits" badge. On a draft, **Update review** moves the pin to the latest commits and re-anchors the comments. A comment on a line the diff rewrote or moved is looked up by its text in the new revision (trimmed lines, plus up to 3 lines of context; a unique best match only, never a lone `}` or blank line). A toast reports how many moved (and how many of those were found by content) and how many could not follow (marked outdated). Published reviews never move.
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
