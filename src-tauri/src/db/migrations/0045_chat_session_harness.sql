-- The harness the chat's session lives in. A session can only be resumed by the harness that
-- started it, so a chat whose profile now resolves to another harness (a pinned profile deleted,
-- the default changed) starts a new session. NULL: Claude Code, which every earlier chat used.
ALTER TABLE chat ADD COLUMN session_harness TEXT;
