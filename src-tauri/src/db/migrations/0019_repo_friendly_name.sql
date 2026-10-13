-- Optional user-set display name for a repository, shown instead of the
-- owner/name (or local folder name) wherever a repo is listed.
ALTER TABLE repository ADD COLUMN friendly_name TEXT;
