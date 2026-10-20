-- One row per app-spawned `claude -p` run, written by the shared runner. review_id / target_id are
-- plain columns (no FK) so usage history outlives deleted reviews; reads LEFT JOIN.
CREATE TABLE ai_run (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at            TEXT NOT NULL,
    duration_ms           INTEGER NOT NULL DEFAULT 0,
    feature               TEXT NOT NULL,
    variant               TEXT,
    review_id             INTEGER,
    target_id             INTEGER,
    label                 TEXT NOT NULL DEFAULT '',
    requested_model       TEXT NOT NULL DEFAULT '',
    status                TEXT NOT NULL CHECK (status IN ('ok', 'error')),
    error                 TEXT,
    num_turns             INTEGER NOT NULL DEFAULT 0,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd              REAL NOT NULL DEFAULT 0,
    session_id            TEXT
);
CREATE INDEX idx_ai_run_started ON ai_run(started_at);
CREATE INDEX idx_ai_run_feature ON ai_run(feature);

CREATE TABLE ai_run_model (
    run_id                INTEGER NOT NULL REFERENCES ai_run(id) ON DELETE CASCADE,
    model                 TEXT NOT NULL,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd              REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, model)
);
