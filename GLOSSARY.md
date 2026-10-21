# Glossary

Glossary of domain terms used in this app.

## Claude sessions

- **Session** — one Claude Code conversation, identified by its `sessionId`. A session outlives
  the process that ran it and can be resumed later. The Sessions view shows one row per session.
- **Live session** — a session currently held by a running Claude Code process. Live sessions
  are either *interactive* (attached to a terminal) or *background* (no terminal; read-only here).
- **Session history** — sessions that are no longer live but were active in the last few days.
- **Pane** — the tmux pane hosting a live interactive session.
- **Terminal host** — the GUI terminal application that displays a pane or a shell
  (iTerm2, Ghostty, Terminal.app, Terminator).
- **Focus** — bring the terminal host forward on the exact tab and pane of a live session.
  Only possible when a terminal host is attached to the session's terminal.
- **Detached** — a live session whose tmux session has no attached terminal host; it can't be
  focused.
- **Tmux target** — the tmux session a new terminal is added to as a window: the currently
  attached one, a shared named session, or one session per repository. The terminal host only
  attaches tmux when nothing shows that session yet.

## Reviews

- **Target** — the thing being reviewed: a GitHub PR, or a pair of local branches (a virtual
  PR). Many reviews can share one target. A target names *what* is reviewed, never *which
  commits*.
- **Review** — the user's unit of work on a target: comments, body and verdict. A published
  review is locked.
- **Pin** — the base and head commits a review was written against. It belongs to the review,
  not the target, so opening the same PR again never moves an older review.
  _Avoid_: target SHA
- **Live head** — the current head of the target: the PR's head on GitHub, or the local branch
  tip. When it differs from a review's pin, the review shows "N new commits".
- **Update review** — move a draft review's pin to the live head and re-anchor its comments.
  Comments that can't follow are marked outdated. Published reviews never move.

## Sync

- **Mirror** — the app's local copy of GitHub facts (PRs, issues, users). It is disposable: it
  holds no user decisions and can be wiped and rebuilt from GitHub. Inbox, Board and Review
  read the mirror and keep only their own state.
- **Sync** — bring the mirror up to date. *Light* sync checks cheaply what changed and
  fetches only that; it runs automatically. A sync the user asks for is always fresh for
  everything on screen. *Full* sync ignores what it thinks is fresh and refetches everything.
  _Avoid_: refresh
- **Discovery** — finding PRs and issues the mirror doesn't know yet (review requests,
  mentions, team requests, newly authored PRs). Part of every sync the user asks for; otherwise
  it runs on its own slower schedule while the app is visible.
  _Avoid_: inbox refresh
- **Reset mirror** — wipe the mirror and let it rebuild. Never touches user state.
- **Fetch** — download a repository's remote branches (`git fetch`). Always explicit or
  opt-in, never a side effect of looking at a view.
  _Avoid_: refresh, sync
- **Reload** — re-read local git or disk state. No network.
  _Avoid_: refresh
- **Registered repository** — a repository the user added or linked. Only registered
  repositories appear in the Repositories view and on the Board.
- **Auto-added repository** — a repository the app knows about only because a mirrored PR or
  issue lives there (e.g. a review request from an unrelated repo). Hidden, and forgotten when
  nothing refers to it any more.
- **Watcher** — live tracking of a local clone's branches and HEAD, active only while a view
  that needs it is open. Unwatched repositories reload when a view opens or the window
  regains focus.

## Inbox

- **Inbox** — the triage queue for new GitHub items that need the user's attention (review
  requested, mentioned, assigned, a team's review requested, an issue they opened). It finds
  things; the Board is where tracked work is monitored.
- **Inbox entry** — the user's triage decision about one issue or PR. It is in exactly one state:
  **New** (untriaged), **Tracked** (put on the Board; out of the queue) or **Archived**.
  Not to be confused with an *activity event* (see below).
- **New** — the Inbox tab listing untriaged entries, most recently updated first. The user's own
  PRs never appear (they are tracked automatically); their own issues do.
- **Archive** — the Inbox tab listing everything triaged: entries archived by hand
  (*dismissed*), that closed or merged on GitHub (*closed*), or whose Board card was discarded
  (*discarded*), plus Tracked ones. Searchable, never capped. Distinct from the Board Done
  lane, which is derived from GitHub state.
  _Avoid_: untrack (it means archive; Track always means the Board)
- **Track** — put an item on the Board. Also what *Open as review* does. The entry leaves New.
- **Reason** — why GitHub surfaced an item: `assigned`, `direct_review`, `mention`, `team_review`
  or `author`. An archived entry returns to New only when it gains a reason it didn't have, or
  one that lapsed and came back (re-requested as reviewer). Activity on the item never brings it back.
- **Stale** — a New entry that no search of the last complete discovery still found (the review
  request was withdrawn, the user was unassigned). It stays in New, marked, until triaged.
- **Bot** — an item authored by a GitHub Bot account, or by a login in the Settings bot list.
  A filter on the Inbox, not a tab.
- **Activity event** _(future)_ — something that happened on a tracked item (CI changed, a
  comment thread opened or updated). Separate from an inbox entry's state, with its own
  seen/unseen flag, and shown in its own Inbox view. Not built yet.

## Board

- **Card session** — a live session that belongs to a PR card: either recorded on the card as a
  `claude_session` metadata item, or running in a checkout of the card's head branch.
- **Chat worktree** — the checkout of a PR's head branch the app uses for chat and editing
  (an existing worktree on that branch, or one it creates). Looked up again on every chat turn:
  whichever checkout holds the branch now wins, including the main checkout (the chat shows a
  banner then). Moving to a different checkout starts a fresh Claude session. Deleting a review
  or clearing a chat never removes its worktree, since it may be shared with a Claude session.

## Worktrees

- **Worktree janitor** — scans every linked worktree of every repo with a local clone, only
  on demand from the Worktrees view. Scan is a dry run (it only fetches); Clean up applies the
  actions. It removes a worktree only when it is provably safe:
  - it is not the main checkout, not locked, and not used by a live Claude session, a tmux pane,
    or a local-diff chat;
  - `git status` is clean;
  - its commits exist elsewhere: the PR is merged or closed and HEAD is in the PR's final head
    (same commit, an ancestor, or patch-equal), or HEAD is already in trunk, or a detached HEAD
    is on a remote branch.

  Removal never uses `--force`. The local branch is deleted only in the PR case. Clean up
  fast-forwards clean worktrees that are only behind origin. When origin was rewritten (for
  example GitHub rebased a stack) and the local tip had been pushed before (it appears in the
  remote-tracking reflog), Clean up runs `git reset --keep @{u}`. Everything else is flagged
  in the Worktrees view, where each row can also open a terminal (tmux window), VSCode, or be
  removed.
