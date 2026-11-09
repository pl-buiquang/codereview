-- Remote hosts (a machine running `cr serve`, reached over an SSH tunnel) and the columns that
-- let a routine run live on one. A run with a `host_id` is mirrored from the remote: the remote
-- store is authoritative while it runs, `last_remote_seq` is how far the local copy got.

CREATE TABLE host (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- `ssh` now; `direct` (url + token) is reserved and not implemented.
    type            TEXT NOT NULL DEFAULT 'ssh' CHECK (type IN ('ssh', 'direct')),
    ssh_target      TEXT NOT NULL,              -- alias from ~/.ssh/config (never a credential)
    remote_endpoint TEXT NOT NULL,              -- absolute socket path or loopback port on the remote
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_seen       TEXT,
    last_protocol   INTEGER,
    last_version    TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- No foreign keys: a removed host must not turn its old runs into local ones.
ALTER TABLE routine_run ADD COLUMN host_id INTEGER;
ALTER TABLE routine_run ADD COLUMN remote_run_id INTEGER;
ALTER TABLE routine_run ADD COLUMN last_remote_seq INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_routine_run_host ON routine_run(host_id) WHERE host_id IS NOT NULL;

ALTER TABLE ai_run ADD COLUMN host_id INTEGER;

-- Outbox ops already applied locally (op ids are `<remote run id>:<n>`): applying twice is a no-op.
CREATE TABLE remote_applied_op (
    run_id     INTEGER NOT NULL REFERENCES routine_run(id) ON DELETE CASCADE,
    op_id      TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    PRIMARY KEY (run_id, op_id)
);
