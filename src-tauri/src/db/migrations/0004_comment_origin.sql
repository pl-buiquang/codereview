-- Where a comment was authored: 'diff' = against a diff hunk (publishable as a
-- GitHub inline review comment); 'file_view' = in the full-file pane against an
-- absolute head-file line (may sit outside any hunk, so it folds into the review
-- body on publish/export rather than posting inline).
ALTER TABLE comment ADD COLUMN origin TEXT NOT NULL DEFAULT 'diff'
    CHECK (origin IN ('diff', 'file_view'));
