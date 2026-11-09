-- Harness-neutral form of every routine run event, next to the raw line (`json`, the source of
-- truth). NULL for rows written before this migration: the UI falls back to the Claude parser.
-- JSON: {"v":1,"events":[{"type":"text|tool_call|tool_result|final|other",..}]}
ALTER TABLE routine_run_event ADD COLUMN norm TEXT;

-- Which harness ran the routine run (shown in the run header). Every earlier run was Claude Code.
ALTER TABLE routine_run ADD COLUMN harness TEXT NOT NULL DEFAULT 'claude';
