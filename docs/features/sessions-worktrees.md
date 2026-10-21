# Home, Sessions and Worktrees

## Home sidebar

The Home tab has a sidebar (`src/components/HomeNav.tsx`) with, in order:

| Item | Shows |
|---|---|
| Sessions | Live and recent Claude Code sessions (below) |
| Board | Kanban of tracked PRs and issues ([Inbox and Board](board-inbox.md)) |
| Inbox | Triage queue for new items |
| Reviews | Your local and GitHub reviews |
| Worktrees | Worktree cleanup (below) |
| Repositories | Registered repositories |

Below the items:

- **Open repositories.** Opening a repository adds it to an **Open** list in the sidebar (with an
  identicon). Each stays mounted inside the Home tab, so its state survives switching. Click to
  show it, drag to reorder, close with the x or a middle click; hover shows the full `owner/name`.
  Closing the shown repository falls back to its left neighbour, or to the last Home section when
  none is left. A review opened from a repository returns to it when closed. Repositories removed
  from the app are dropped from the list.
- **Footer.** A GitHub sync indicator (click for last Inbox / Board sync times, **Sync now** and
  **Full Sync**), **Settings**, and the app version.
- **Fold to icons.** The toggle in the brand row collapses the sidebar to a 52px icon rail; labels
  become tooltips and the open-repo close buttons hide. The state is remembered across restarts.

## Sessions

Home → Sessions (`src/components/SessionsView.tsx`) lists Claude Code sessions on this machine and
refreshes every 3 seconds while Home is visible.

### Where sessions come from

- **Live**: `~/.claude/sessions/<pid>.json` (written by Claude Code, an undocumented format, parsed
  leniently; the only source of the tmux pane), confirmed against running processes and enriched
  with status, "waiting for" and job ids from `claude agents --json`
  (`src-tauri/src/claude_sessions.rs`).
- **History**: transcripts in `~/.claude/projects/<cwd>/<id>.jsonl` modified in the last N days
  (Settings → Sessions, default 3; 0 hides History). Sessions continued in another transcript
  are not listed twice.
- **Names**: summary title, else custom title, agent name, AI title, or last prompt.

### Rows

- Status dot (busy, idle, waiting, ended), badges (`background` / `attached` for background
  sessions, `detached`, what it is waiting for), working directory, branch, tmux window, age.
- Board badges (`repo#n`) for live sessions on a tracked PR card: recorded as `claude_session`
  metadata, or running in a checkout of the card's head branch. Click to jump to the card.

### Actions

- **Focus** (live): raises the session's terminal. With tmux it switches the attached client to the
  pane and raises the terminal app; without it, it focuses the terminal by tty. macOS supports
  iTerm2, Ghostty and Terminal.app; Linux supports Terminator (X11, best effort via `wmctrl` /
  `xdotool`). Disabled with a reason when there is no attached tmux client, no terminal, or the
  session is a background one.
- **Background sessions** (`claude --bg`): **Agent view** focuses the `claude agents` terminal they
  can be opened from, else **Copy attach** copies `claude attach <job>`. **Stop** runs
  `claude stop <job>` after confirmation; the conversation is kept.
- **Resume** (history): opens a new terminal in the session's directory running
  `claude --resume <id>`; the copy button copies `cd <cwd>; claude --resume <id>`. Refuses if the
  session is already running or the directory is gone.

### Terminals and tmux

Settings choose the terminal app (iTerm2, Ghostty, Terminal.app, Terminator) and, when `tmux` is
installed, where new windows go (`src-tauri/src/terminal/`): the **current** tmux session (the most
recently used attached client, falling back to the shared name), a **shared** named session
(default `codereview`), or one session **per repository**. The terminal app only attaches tmux;
tmux selects the window. Without tmux a plain terminal window opens. The command runs followed by
a login shell so the window stays open. The same path serves the Board's "Open in terminal" and
agent actions and the Worktrees view's terminal button.

### Summaries

Expanding a row (live rows always, history rows when a summary exists) shows summaries.

- **From a hook**: point Settings → Sessions → *History summary directory* at a folder of markdown
  files written by your own `SessionEnd` hook. Each has frontmatter (`session_id`, `date`,
  optional `title`) and a markdown body; the title also renames the session, and the original name
  stays in a tooltip.
- **Generate / Regenerate summary** (live only): stored in the app database apart from hook
  summaries and shown with an "App summary" date. By default it runs `haiku` over a text extract of
  the transcript (up to 400 KB, no tools). With *fork* enabled in Settings, a session active in the
  last 5 minutes is forked instead to reuse its prompt cache; it is billed at the session's model,
  so a cache miss costs more. The prompt is editable under the prompts setting. Needs the `claude`
  CLI; usage is counted in AI usage.

## Worktrees

Home → Worktrees (`src/components/WorktreesView.tsx`, `src-tauri/src/worktree_gc.rs`) classifies
every linked worktree of every repository with a local clone and cleans up only what is provably
safe. Nothing is forced; the main checkout is never listed.

### Scanning

- On open it reloads locally (no fetch): only repositories whose git watcher saw a change since the
  last scan are rescanned, with PR facts from the GitHub mirror. Cached git facts are reused until
  a worktree's refs move.
- **Scan** fetches and refreshes PR state for all repositories, applying nothing.
- **Clean up** (confirmation) applies the actionable verdicts: removes and syncs.
- The header shows totals, how many need a look, and when the last scan ran. **Show kept** toggles
  the "keep" rows. A "Last cleanup" log lists each action.

### Verdicts

Sorted by urgency:

| Verdict | When | Action |
|---|---|---|
| needs a look | Uncommitted or untracked changes; detached HEAD with commits on no remote; closed/merged PR but the branch has commits not in it; diverged from upstream with unpushed commits; unpushed commits or never pushed, with no PR | none |
| safe to remove | PR merged or closed and the tip is in the PR (branch deleted too when the tip equals the PR head), or nothing unpushed; no PR but already in trunk; detached and already on a remote or in trunk | `git worktree remove` (+ `branch -D`) |
| folder gone | The directory no longer exists | `git worktree prune` |
| behind | Clean, only behind its upstream | `git merge --ff-only @{u}` |
| rewritten upstream | Upstream rewritten (e.g. GitHub rebased a stack) and the local tip had been pushed before | `git reset --keep @{u}` |
| keep | Main checkout, locked, open PR, pushed with no PR yet, or **in use** | none |

A worktree is "in use" (kept) when a live Claude session's directory is inside it, a tmux pane is
open in it, or a review chat uses it. Rows show the branch, ahead/behind, source (`claude`, `codex`,
`codereview` or `manual`, from the path), last commit age and the PR badge.

### Per-row actions

**Sync** (behind / rewritten rows; fetches first, and refuses with uncommitted or unpushed work),
open a terminal, **open in VSCode**, and **remove** (confirmation; git itself refuses if there are
uncommitted changes, and the branch is kept).

## Where it lives

- `src/components/HomeNav.tsx`, `src/store.ts` (open repos, sidebar state), `SyncIndicator.tsx`
- `src/components/SessionsView.tsx`, `src-tauri/src/commands/sessions.rs`
- `src-tauri/src/claude_sessions.rs`, `session_summaries.rs`
- `src-tauri/src/terminal/` (`mod.rs`, `tmux.rs`, `macos.rs`, `linux.rs`)
- `src/components/WorktreesView.tsx`, `src-tauri/src/worktree_gc.rs`, `worktree.rs`,
  `src-tauri/src/commands/worktree.rs`
