-- Board metadata gains the `ai_session` kind: a pointer to a session of a non-Claude harness,
-- tagged with the harness id. Existing `claude_session` rows stay as they are and are read as
-- `ai_session` of harness `claude`. SQLite can't alter a CHECK, so the table is rebuilt.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE board_meta_new (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id    TEXT NOT NULL REFERENCES board_card (id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('file', 'link', 'note', 'claude_session', 'ai_session')),
    value      TEXT NOT NULL,                          -- path / url / note text / session id
    tag        TEXT,
    label      TEXT,
    cwd        TEXT,                                   -- sessions only
    created_at TEXT NOT NULL,
    harness    TEXT                                    -- ai_session only (NULL reads as claude)
);

INSERT INTO board_meta_new (id, card_id, kind, value, tag, label, cwd, created_at)
    SELECT id, card_id, kind, value, tag, label, cwd, created_at FROM board_meta;

DROP TABLE board_meta;
ALTER TABLE board_meta_new RENAME TO board_meta;
CREATE INDEX idx_board_meta_card ON board_meta (card_id);

COMMIT;

PRAGMA foreign_keys = ON;
