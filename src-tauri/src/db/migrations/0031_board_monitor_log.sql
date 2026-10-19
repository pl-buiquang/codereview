CREATE TABLE board_monitor_log (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    at      TEXT NOT NULL,
    card_id TEXT NOT NULL,
    repo    TEXT NOT NULL,
    number  INTEGER NOT NULL,
    kind    TEXT NOT NULL,
    detail  TEXT NOT NULL
);
CREATE INDEX board_monitor_log_at ON board_monitor_log (id DESC);
