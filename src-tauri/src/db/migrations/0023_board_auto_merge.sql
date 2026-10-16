-- Merge method of an enabled GitHub auto-merge (SQUASH/MERGE/REBASE); NULL = off.
ALTER TABLE board_card ADD COLUMN auto_merge TEXT;
