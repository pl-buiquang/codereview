# Settings

Open Settings from the app's gear. It has four tabs: **General**, **AI**, **Usage**, **Logs**. Changes apply immediately; there is no save button.

- Preferences are persisted in the webview's `localStorage` (key `codereview-settings`) and mirrored to `settings.json` in the app-data directory, so a wiped webview store is recovered from the file.
- Unknown or mistyped values are dropped on load; new settings take their default.
- The AI and Usage tabs are documented in [focus-and-ai.md](focus-and-ai.md).

## General

Defaults are from [`src/lib/settings.ts`](../../src/lib/settings.ts).

### Appearance

| Setting | Default | What it does |
|---|---|---|
| Direction | Continuity (`a`) | Design direction: Continuity, Modern, Terminal or GitHub (see [Themes](#themes)) |
| Mode | System | Dark, Light, or System (follows the OS) |
| Diff font size | 12.5 px | Slider, 10 to 20 px in 0.5 steps |

### Review defaults

| Setting | Default | What it does |
|---|---|---|
| Default diff view | Split | Split or Unified when a review opens |
| Default to merge-base (three-dot) diff | on | Default for new local (virtual PR) reviews; off means plain two-dot |
| Highlight intra-line changes | off | Character-level highlighting inside changed lines |
| Max diff lines per file | 3000 | Files with more changed lines (additions + deletions) are not rendered; open them in VS Code instead. Minimum 100 |

### Board

| Setting | Default | What it does |
|---|---|---|
| PR status border side | Top | Which edge of a card shows the PR-status color |
| Archive done cards after (days) | 7 | The Done lane's archive button archives cards whose PR closed or merged longer ago than this (1 to 365) |
| Count Done cards in filter counts | on | Whether Done cards are included in the numbers beside the Board's filters |

### Sync

| Setting | Default | What it does |
|---|---|---|
| Auto Sync | Periodic | When Inbox, Board and Review sync from GitHub by themselves: **Off**; **On focus** (when a view opens or the window regains focus); **Periodic** (also on a timer while the app is visible, slower when nothing changes) |
| Look for new PRs/issues every | 5 min | Discovery interval: off, 5, 15, 30, 60 min. Needs Auto Sync on; a manual Sync always looks |
| Auto-merge monitor every | 5 min | How often the Board's auto-merge monitor runs (2, 5, 10, 15, 30 min). The monitor itself is switched on from the Board: it updates BEHIND branches and reruns failed CI (up to 3 attempts) on approved PRs with auto-merge on, while the app is open |
| GitHub mirror: Reset mirror... | | Deletes the local copy of GitHub PRs, issues and users and rebuilds it. Inbox triage, board tracking and reviews are kept (placeholders show until the rebuild lands) |

### Repositories

| Setting | Default | What it does |
|---|---|---|
| Background fetch | off | `git fetch` a repo when its Graph opens or the window regains focus and the last fetch is older than 5, 15 or 30 min |
| Base paths for auto-linking | `~/projects` | Comma-separated directories searched (2 levels deep) for a local clone when a GitHub PR arrives. **Scan now** runs the link pass immediately |

### Sessions

For the Sessions view and terminals opened from the app.

| Setting | Default | What it does |
|---|---|---|
| Default terminal | iTerm2 | iTerm2, Ghostty, Terminal.app or Terminator (Linux). It attaches tmux, or opens windows when tmux is missing |
| Open new terminals in | Current tmux session | New terminals open as a tmux window in the current session, a shared session, or one session per repository |
| Shared tmux session | `codereview` | Name used by "Shared session", and by "Current" when no tmux client is attached |
| History (days) | 3 | How many days of past Claude sessions the view lists (0 to 30) |
| History summary directory | empty (off) | Directory of hook-written markdown session summaries (frontmatter with `session_id`, optional title) shown on history cards |
| Summarize live sessions by forking | off | Summarize a session active in the last 5 minutes by forking it (reuses its prompt cache; a cache miss is billed at the session's own model price). Off means a cheap `haiku` run over a transcript extract |

### File types

Two lists of globs, one per line (`#` starts a comment), used to classify changed files. Patterns without `/` match the file name (`*_spec.rb`); others match the full path (`**/tests/**`). **Tests** are checked first, then **Config & docs**; anything else counts as code. **Reset to defaults** restores both lists (defaults live in [`fileCategory.ts`](../../src/lib/fileCategory.ts)).

### Inbox

| Setting | Default | What it does |
|---|---|---|
| Bot logins | empty | Comma-separated GitHub logins treated as bots (case-insensitive; GitHub's own bot account type is also used) |
| Strip repo prefixes | empty | Comma-separated prefixes removed from repo names in the inbox, e.g. `philips-internal/cardiologs-`. Hovering shows the full `owner/name` |

### Backup

**Export...** writes the persisted settings, plus any edited AI prompts, to a JSON file (`codereview-settings.json`). **Import...** applies such a file (or a bare settings object): recognized keys are applied, others skipped, and prompt overrides in the file replace the current ones (prompts absent from the file are reset). Files that predate prompt export leave prompts alone.

### Environment

Shows the paths found for `git` and `gh` and whether `gh` is authenticated (`gh auth login` if not). The app version is shown at the bottom.

## AI

Models (chat & focus, board overview) and editable prompts. See [focus-and-ai.md](focus-and-ai.md#prompts-and-models-settings--ai).

## Usage

Cost and token history of the app's `claude -p` runs. See [focus-and-ai.md](focus-and-ai.md#usage-tracking-settings--usage).

## Logs

- **Log level**: Error, Warning (default), Info, Debug (verbose: traces every backend call, sync, git/gh command and query refetch).
- Shows the log file path (**Copy path**, **Open**), a **Refresh** button, and the last 400 lines, auto-refreshed every 3 seconds. The file rotates to `.log.old` past 5 MB.

## Themes

Four design directions, each in a dark and a light mode, and a **System** mode that follows the OS: 4 x 2 themes in total.

| Direction | Look |
|---|---|
| Continuity (`a`, default) | Atkinson Hyperlegible, blue, slate-navy |
| Modern (`b`) | Inter, indigo, near-black |
| Terminal (`c`) | JetBrains Mono, green, sharp |
| GitHub (`d`) | System fonts, high-contrast diffs, crisp |

The token contract and how to add a direction are in [THEMING.md](../../THEMING.md). Themes are not user-editable.

## Updates

Release builds check GitHub Releases (`.../releases/latest/download/latest.json`) once at startup. When a newer version exists, a banner reads "Update available: vX.Y.Z" with **Install & relaunch** (downloads, verifies the signature, installs, restarts) and **Dismiss** (for this session). The check is skipped in dev builds and fails silently when offline. Update bundles are verified with the updater key; OS-level code signing of the builds is covered in [../signing.md](../signing.md).

## Where the data lives

One SQLite database in the OS app-data directory, shared by the app and the [`cr` CLI](cli.md):

| OS | Directory |
|---|---|
| macOS | `~/Library/Application Support/com.codereview.app/` |
| Linux | `$XDG_DATA_HOME/com.codereview.app/` (default `~/.local/share/com.codereview.app/`) |
| Windows | `%APPDATA%\com.codereview.app\` |

- Release builds use `codereview.db` (and `codereview.log`); debug builds, including `pnpm tauri dev`, use `codereview-dev.db` (and `codereview-dev.log`) so development never touches your real data. The dev app also has its own bundle identifier (`com.codereview.app.dev`), and the app resolves its data directory from that identifier, so the dev database sits in `com.codereview.app.dev/`, not beside the release one.
- `cr` always looks in `com.codereview.app/` (use `--db <path>` or `CODEREVIEW_DB` to point it at another file, e.g. the dev database).
- `settings.json` (the preferences mirror) lives in the same directory as the app's database.
- Schema: `repository → target (local | github_pr) → review → comment`; GitHub facts live in a separate mirror. Migrations are append-only.
