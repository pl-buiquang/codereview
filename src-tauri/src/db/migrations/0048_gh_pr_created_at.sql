-- PR creation date for the review header; filled on the next sync of each PR.
ALTER TABLE gh_pr ADD COLUMN created_at TEXT;
