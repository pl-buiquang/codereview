-- Routines: named `claude -p` runs with their own prompt, scoped to a context (PR, review, issue
-- or the board). Built-in rows are seeded from Rust at startup (`routines::seed`); this script only
-- creates the tables.
CREATE TABLE routine (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    key                TEXT NOT NULL UNIQUE,
    name               TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    context_type       TEXT NOT NULL CHECK (context_type IN ('pr', 'review', 'issue', 'board')),
    prompt_template    TEXT NOT NULL,
    system_prompt      TEXT NOT NULL DEFAULT '',
    system_prompt_mode TEXT NOT NULL DEFAULT 'append' CHECK (system_prompt_mode IN ('append', 'replace')),
    needs_worktree     INTEGER NOT NULL DEFAULT 0,
    permission_mode    TEXT NOT NULL DEFAULT 'bypassPermissions',
    allowed_tools      TEXT NOT NULL DEFAULT '[]',   -- JSON array
    extra_args         TEXT NOT NULL DEFAULT '[]',   -- JSON array
    model              TEXT NOT NULL DEFAULT '',
    timeout_min        INTEGER NOT NULL DEFAULT 240,
    builtin            INTEGER NOT NULL DEFAULT 0,
    handler            TEXT,
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

-- One row per launch. `context_key` is canonical (`pr:owner/name#12`, `issue:…`, `review:7`,
-- `board`) so the same PR launched from a card or a review dedups.
CREATE TABLE routine_run (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    routine_id    INTEGER NOT NULL REFERENCES routine(id) ON DELETE CASCADE,
    context_type  TEXT NOT NULL,
    context_key   TEXT NOT NULL,
    context_label TEXT NOT NULL DEFAULT '',
    context_json  TEXT NOT NULL,                  -- the launch ref, resolved when the run starts
    status        TEXT NOT NULL CHECK (status IN
                    ('queued', 'running', 'ok', 'needs_attention', 'error', 'killed', 'timeout', 'interrupted')),
    note          TEXT NOT NULL DEFAULT '',
    model         TEXT NOT NULL DEFAULT '',
    session_id    TEXT,
    pid           INTEGER,
    cwd           TEXT,
    result_text   TEXT,
    error         TEXT,
    cost_usd      REAL NOT NULL DEFAULT 0,
    num_turns     INTEGER NOT NULL DEFAULT 0,
    queued_at     TEXT NOT NULL,
    started_at    TEXT,
    ended_at      TEXT
);
CREATE INDEX idx_routine_run_status ON routine_run(status);
CREATE INDEX idx_routine_run_context ON routine_run(context_key);
-- Dedup: one active run per routine × context.
CREATE UNIQUE INDEX idx_routine_run_active ON routine_run(routine_id, context_key)
    WHERE status IN ('queued', 'running');

-- The stream-json lines of a run, for the live transcript.
CREATE TABLE routine_run_event (
    run_id INTEGER NOT NULL REFERENCES routine_run(id) ON DELETE CASCADE,
    seq    INTEGER NOT NULL,
    ts     TEXT NOT NULL,
    kind   TEXT NOT NULL,
    json   TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);
