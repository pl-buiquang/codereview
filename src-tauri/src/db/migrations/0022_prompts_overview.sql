CREATE TABLE prompt_override (
    key        TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE board_overview (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    markdown      TEXT NOT NULL,
    generated_at  TEXT NOT NULL,
    model         TEXT,
    card_count    INTEGER NOT NULL DEFAULT 0,
    card_ids      TEXT NOT NULL DEFAULT '[]',
    input_tokens  INTEGER,
    output_tokens INTEGER,
    cost_usd      REAL
);
