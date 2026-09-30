-- databaseId of the GitHub thread's root comment this draft reply targets.
-- NULL for normal local comments. Non-null means this comment is a draft reply
-- to an existing GitHub PR review thread; publish_draft_replies posts it via
-- the replies API and then deletes the local row.
ALTER TABLE comment ADD COLUMN github_thread_root_id INTEGER;
