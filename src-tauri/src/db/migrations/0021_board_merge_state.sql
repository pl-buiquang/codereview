-- GitHub mergeStateStatus (BEHIND = head ref out of date) and native stack
-- membership, so the board can flag "update branch" vs "rebase stack".
ALTER TABLE board_card ADD COLUMN merge_state_status TEXT;
ALTER TABLE board_card ADD COLUMN stack_number INTEGER;
ALTER TABLE board_card ADD COLUMN stack_position INTEGER;
ALTER TABLE board_card ADD COLUMN viewer_can_update_branch INTEGER NOT NULL DEFAULT 0;
