# Inbox and Board

Two views over the same GitHub data. The **Inbox** is a triage queue for new items; the **Board**
is where the items you care about live. Neither stores GitHub facts itself: both read the
disposable mirror ([ADR 0002](../adr/0002-disposable-github-mirror-soft-refs.md)) and keep only
their own state (triage state, tracking, metadata).

## Inbox

The Inbox is a triage queue, not a monitor ([ADR 0005](../adr/0005-inbox-is-a-triage-queue.md)).

- **New** lists what GitHub says needs you: review requested (directly or via a team), mentions,
  assignments and issues you opened. Your own PRs are never queued.
- Per item:
  - **Track** (or **Open as review**, which also tracks) moves it to the Board and out of the queue.
  - **Archive** dismisses it.
- Items that close on GitHub archive themselves. An archived item comes back only when it gains a
  new reason (e.g. you are re-requested as reviewer), never on plain activity.
- An item no search finds any more is marked **stale** and stays until triaged.
- Bot-authored items are tagged **bot** (GitHub's account type plus the list in Settings) and
  filterable.
- **Archive** tab: everything triaged (dismissed, closed, discarded from the Board, tracked), with
  search and no cap. Archived items have a **Restore** button that moves them back to New.
- Discovery runs as soon as a conditional `GET /notifications` poll (about every 60s) sees a change; the interval in Settings → Sync (5 minutes by default, at least 30 minutes while notifications work) is a fallback, since notifications record events, not current state.
- CLI: `cr inbox list|archive|restore|sync`.

## Board

### Lanes

Lanes are derived from GitHub state, never dragged by hand
(`lane_for` in `src-tauri/src/board.rs`):

| Lane | Contents |
|---|---|
| Backlog | Tracked open issues with no open linked PR |
| In progress | Your own draft PRs |
| Submitted | Your own non-draft open PRs |
| Reviews | Open PRs by other people |
| Done | Merged or closed PRs |

Closed issues, discarded and archived cards don't appear. Lanes can be collapsed to a slim column.

### Tracking

- **Track** button in the header (PR URL), the **+** in the Backlog and Reviews lane headers
  (issue or PR, URL or `owner/repo#n`), the Inbox's **Track** / **Open as review**, or
  `cr board track <url|owner/repo#n>`.
- Your own open PRs in registered repos are tracked automatically on sync.
- **Discard** removes a card from the Board into the Inbox's **Archive** tab, with its metadata
  kept; re-tracking restores the same card. Your own PRs can't be discarded.

### Issue and PR linking

A PR is linked to a backlog issue automatically when it closes the issue on GitHub ("Fixes #n") or
its branch name contains `issue-<n>` (e.g. `pbq/issue-123`, same repo only). You can also paste a PR
URL into the issue card's link field, and unlink from the card (`cr board link` does the same from
the terminal). While an issue has an open linked PR it is hidden and the PR sits in
In progress / Submitted / Reviews.

### Done purge

The archive button in the Done lane header permanently archives Done cards whose PR closed or
merged more than N days ago. N defaults to 7 and is set in Settings → Board (1 to 365). A setting
also controls whether Done cards count in the filter numbers.

### Card contents

- Title (opens GitHub; right-click copies the link), repo (opens the repository in the app, or on
  GitHub if it isn't added), status border colour (side configurable in Settings).
- PR badges: CI, mergeable, stack position, merge state (behind, conflicting), auto-merge method,
  pending review threads.
- Buttons: **Review** (create or resume), **VSCode**, **Discard**.
- **Stack badge**: shows the PR's place in a stack; clicking it filters the Board to that stack and
  sorts it by stack position.
- Time since last update.

### Metadata

Each card carries metadata: a **file** (plan or spec), a **link**, a **note**, or a **claude_session**
(with a one-click "copy resume command": `cd <cwd>; claude --resume <id>`). Items can have a free
tag. A PR card also shows its linked issue's metadata, marked "from issue". More than three items
collapse behind a chevron.

In the UI you can add notes, files and links and delete your own items. Claude sessions are added
from the CLI:

```bash
cr board show --here                                    # resolve the card for the current repo+branch
cr board meta add --here --kind file --value /path/to/plan.md --tag plan
```

See [`skills/board/SKILL.md`](../../skills/board/SKILL.md) for the full agent workflow, and
`cr board --help` for every subcommand (`list`, `show`, `track`, `sync`, `link`, `meta add|list|rm`).
To have Claude Code auto-link its session to the card it's working on, add this to
`~/.claude/CLAUDE.md`:

```markdown
## CodeReview board

At the start of a session in a repo, run `cr board show --here`. If the card is tracked,
run `cr board meta add --here --kind claude_session --value <this session's id>` so the
board can show a "resume" link back to this session.
```

A live Claude session is also tied to a PR card when it runs on the card's head branch in that
repo, even without recorded metadata. Those sessions show on the card and in
[Sessions](sessions-worktrees.md).

### PR actions

The card's **PR actions** menu (disabled reasons shown as tooltips):

- Squash merge now (confirmation, deletes the branch)
- Enable / disable auto-merge (squash)
- Convert to draft / Mark ready for review
- Update branch (merge or rebase). Not offered for stacked PRs, or when the branch isn't behind or
  you can't update it.
- Re-run failed jobs / Re-run all jobs (only when CI is failing)
- Open in terminal (a shell in the PR's worktree), Focus an existing agent session, or open a new
  Claude session there (tmux window when tmux is available)

**Batch** in the header runs Update branch, Enable auto-merge, Squash merge now, and Re-run
failed/all jobs on every eligible PR matching the current filters, after one confirmation listing
them.

### Filters

Left rail: Status, CI, Merge (auto-merge, merge conflict, needs update), Repositories (multi),
Tags (include) and Exclude tags, and Stacks. Each facet's counts respect the other active filters.

### Auto-merge monitor

Off by default; toggle it with the header button (interval in Settings → Board, default 5 min).
Each pass looks at open, non-draft, tracked PRs that are **approved and have auto-merge on**
(`src-tauri/src/board_monitor.rs`), and GitHub does the merge itself. For each:

- If the branch is BEHIND, you can update it, and it isn't part of a stack: update the branch.
- Else if CI is failing: rerun the failed jobs of each failing workflow run, up to 3 attempts per
  run, then log "Gave up".

The chevron next to the button opens a log of what it did (updated, reran, gave up, error), newest
500 entries kept; click a reference to jump to the card.

### AI overview

The **AI Overview** button opens a pane with two parts:

- A local **digest**: Do next, Blocked, Waiting on others, Stale, By project, Hygiene.
- **AI insights**: **Refresh insights** sends a compact text snapshot of the open cards (lanes, CI,
  review state, stacks, tags, notes; no diffs) to the `claude` CLI with no tools, using the model
  from Settings → AI. The result is stored, shows cost and card count, flags "board changed" when
  cards were added or removed since, and turns `repo#N` mentions into links to cards. Needs the
  `claude` CLI.

## Known limits

- Metadata editing in the UI is add and delete only; changing an item means deleting and re-adding
  it. The UI can't add `claude_session` items (CLI only).
- Archiving Done cards is permanent: there is no button to restore an archived card.
- The header **Track** dialog takes PRs only and refuses already-tracked ones.

## Where it lives

- `src/components/InboxView.tsx`, `InboxItemRow.tsx` — Inbox
- `src/components/BoardView.tsx`, `BoardCard.tsx`, `TrackPrDialog.tsx` — Board UI
- `src/components/BoardOverviewPane.tsx`, `src-tauri/src/commands/overview.rs` — AI overview
- `src/components/BoardMonitorPane.tsx`, `src-tauri/src/board_monitor.rs` — monitor
- `src-tauri/src/board.rs` (lanes, branch matching), `src-tauri/src/commands/board.rs` (commands,
  auto-link, PR actions), `src-tauri/src/inbox.rs`, `src-tauri/src/sync/` — logic and sync
- `src-tauri/src/bin/cr.rs` — `cr board` / `cr inbox`
- [`skills/board/SKILL.md`](../../skills/board/SKILL.md) — agent workflow
