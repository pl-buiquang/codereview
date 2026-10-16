-- Remote fetch and local-change bookkeeping per repository. `default_branch`
-- stays on `repository`; this only records when it was last re-checked.
CREATE TABLE repo_sync (
    repo_id                   INTEGER PRIMARY KEY REFERENCES repository(id) ON DELETE CASCADE,
    default_branch_checked_at TEXT,
    last_fetch_at             TEXT,
    last_fetch_error          TEXT,
    last_local_change_at      TEXT
);
