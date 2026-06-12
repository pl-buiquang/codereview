-- Add the 'published_pending' review status (review staged on GitHub as a
-- PENDING review, not yet submitted). SQLite cannot ALTER a CHECK constraint,
-- so rebuild the table. foreign_keys must be OFF around the DROP: comment and
-- file_view_state reference review(id) ON DELETE CASCADE, and DROP TABLE with
-- FKs on performs an implicit DELETE FROM that would fire those cascades.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE review_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id        INTEGER NOT NULL REFERENCES target(id) ON DELETE CASCADE,
    body             TEXT NOT NULL DEFAULT '',
    event            TEXT CHECK (event IN ('comment', 'approve', 'request_changes')),
    status           TEXT NOT NULL DEFAULT 'draft'
                     CHECK (status IN ('draft', 'published_pending', 'published')),
    published_at     TEXT,
    github_review_id INTEGER,
    last_exported_at TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

INSERT INTO review_new (id, target_id, body, event, status, published_at,
                        github_review_id, last_exported_at, created_at, updated_at)
    SELECT id, target_id, body, event, status, published_at,
           github_review_id, last_exported_at, created_at, updated_at
    FROM review;

DROP TABLE review;
ALTER TABLE review_new RENAME TO review;

-- Recreate the review index from 0001 (dropped with the old table). There are
-- no triggers and no other review indexes in migrations 0001-0008.
CREATE INDEX idx_review_target ON review(target_id);

COMMIT;

PRAGMA foreign_keys = ON;
