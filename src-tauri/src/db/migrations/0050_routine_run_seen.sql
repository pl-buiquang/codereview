-- When the user looked at a finished run (selected it in Sessions → Routines, or marked all seen).
-- NULL on a finished run that notifies (failed / timed out / needs attention, or a successful AI
-- review) means it still shows in the sidebar badge. Runs that already ended count as seen.
ALTER TABLE routine_run ADD COLUMN seen_at TEXT;
UPDATE routine_run SET seen_at = COALESCE(ended_at, queued_at) WHERE status NOT IN ('queued', 'running');
