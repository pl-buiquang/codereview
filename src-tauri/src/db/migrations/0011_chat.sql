CREATE TABLE chat (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    review_id           INTEGER NOT NULL UNIQUE REFERENCES review(id) ON DELETE CASCADE,
    model               TEXT NOT NULL,
    session_id          TEXT,
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost_usd      REAL NOT NULL DEFAULT 0,
    worktree_path       TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE chat_message (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id       INTEGER NOT NULL REFERENCES chat(id) ON DELETE CASCADE,
    role          TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content       TEXT NOT NULL,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    cost_usd      REAL,
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_chat_message_chat ON chat_message(chat_id);

ALTER TABLE review ADD COLUMN chat_collapsed INTEGER NOT NULL DEFAULT 1;
