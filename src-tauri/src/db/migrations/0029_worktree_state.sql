-- Per-worktree cache of the worktree scan's expensive git facts, reused while
-- `cache_key` (head, upstream, trunk, PR facts, ...) is unchanged.
CREATE TABLE worktree_state (
    path        TEXT PRIMARY KEY,
    repo_id     INTEGER NOT NULL REFERENCES repository(id) ON DELETE CASCADE,
    cache_key   TEXT NOT NULL,
    facts_json  TEXT NOT NULL,
    computed_at TEXT NOT NULL
);
CREATE INDEX worktree_state_repo ON worktree_state(repo_id);
