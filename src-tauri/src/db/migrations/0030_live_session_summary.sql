CREATE TABLE live_session_summary (
    session_id    TEXT PRIMARY KEY,
    title         TEXT,
    body          TEXT NOT NULL,
    generated_at  TEXT NOT NULL,
    model         TEXT,
    mode          TEXT NOT NULL,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    cost_usd      REAL
);
