-- CI status (head commit's statusCheckRollup) and merge status (conflicts)
-- for kanban board PR cards.
ALTER TABLE board_card ADD COLUMN ci_status TEXT;
ALTER TABLE board_card ADD COLUMN mergeable TEXT;
