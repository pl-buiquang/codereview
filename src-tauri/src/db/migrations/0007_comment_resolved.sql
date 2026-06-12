-- When a comment thread was resolved (ISO-8601 UTC). NULL = unresolved.
-- Only meaningful on root comments (parent_id IS NULL); replies inherit the
-- root's state. Root-only is enforced by set_comment_resolved, not a CHECK.
ALTER TABLE comment ADD COLUMN resolved_at TEXT;
