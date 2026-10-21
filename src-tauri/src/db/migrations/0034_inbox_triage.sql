-- Inbox as a triage queue: one `state` replaces the engaged_at / untracked_at /
-- closed_at timestamps (left in place, no longer read).
--   new      untriaged, listed on the Inbox's New tab
--   tracked  put on the Board (or opened as a review); out of the queue
--   archived dismissed by hand, closed on GitHub, or discarded from the Board
ALTER TABLE inbox_entry ADD COLUMN state TEXT NOT NULL DEFAULT 'new'
    CHECK (state IN ('new', 'tracked', 'archived'));
ALTER TABLE inbox_entry ADD COLUMN state_at TEXT;
ALTER TABLE inbox_entry ADD COLUMN archive_cause TEXT
    CHECK (archive_cause IN ('dismissed', 'closed', 'discarded'));

-- When discovery last found this reason; a New item none of whose reasons were
-- seen in the last complete discovery is "stale".
ALTER TABLE item_reasons ADD COLUMN last_seen_at TEXT;
UPDATE item_reasons SET last_seen_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now');

-- GitHub's author type, for the Inbox's bot filter.
ALTER TABLE gh_user ADD COLUMN is_bot INTEGER NOT NULL DEFAULT 0;

-- Backfill. My own PRs are on the Board already, never queued.
UPDATE inbox_entry SET state = 'archived', archive_cause = 'dismissed'
WHERE type = 'pr' AND id IN (SELECT item_id FROM item_reasons WHERE reason = 'author');

UPDATE inbox_entry
SET state = 'archived',
    archive_cause = CASE WHEN untracked_at IS NULL AND engaged_at IS NULL THEN 'closed' ELSE 'dismissed' END
WHERE untracked_at IS NOT NULL OR engaged_at IS NOT NULL OR closed_at IS NOT NULL;

UPDATE inbox_entry SET state = 'tracked', archive_cause = NULL
WHERE id IN (SELECT id FROM board_card WHERE discarded_at IS NULL);

UPDATE inbox_entry
SET state_at = COALESCE(untracked_at, closed_at, engaged_at, first_seen_at)
WHERE state != 'new';

DROP INDEX idx_inbox_entry_open;
CREATE INDEX idx_inbox_entry_state ON inbox_entry (state, state_at);
