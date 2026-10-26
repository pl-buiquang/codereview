-- Per-item "viewed" for the Inbox badge, replacing the global inbox_new_seen_at
-- marker (bumped whenever the New tab was on screen). A New item counts in the
-- badge until it is marked viewed or triaged; a new or lapsed reason clears it.
ALTER TABLE inbox_entry ADD COLUMN viewed_at TEXT;

-- Backfill: what the old marker already counted as seen stays seen.
UPDATE inbox_entry
SET viewed_at = (SELECT value FROM meta WHERE key = 'inbox_new_seen_at')
WHERE state = 'new'
  AND first_seen_at <= (SELECT value FROM meta WHERE key = 'inbox_new_seen_at');

DELETE FROM meta WHERE key = 'inbox_new_seen_at';
