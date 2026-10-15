# CONTEXT

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

## Board

- **Card session** — a live session that belongs to a PR card: either recorded on the card as a
  `claude_session` metadata item, or running in a checkout of the card's head branch.
- **Chat worktree** — the checkout of a PR's head branch the app uses for chat and editing
  (an existing worktree on that branch, or one it creates).
