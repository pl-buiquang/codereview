-- Last worktree scan report (JSON), so the Worktrees view survives restarts.
CREATE TABLE worktree_gc_report (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    report     TEXT NOT NULL,
    created_at TEXT NOT NULL
);
