-- "Archive and forget": an archived entry with forgotten_at set never returns to
-- New, even when discovery finds a new reason. Cleared by any later state change
-- (restore, track, discard). A column rather than an archive_cause value because
-- the CHECK on archive_cause can't change without rebuilding the table.
ALTER TABLE inbox_entry ADD COLUMN forgotten_at TEXT;
