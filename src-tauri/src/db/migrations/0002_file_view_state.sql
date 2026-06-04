-- Per-(review, file) "viewed"/collapsed state for the diff view. Purely a UI
-- convenience so collapsing a file survives closing and reopening the review.
CREATE TABLE file_view_state (
    review_id  INTEGER NOT NULL REFERENCES review(id) ON DELETE CASCADE,
    file_path  TEXT NOT NULL,
    viewed     INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (review_id, file_path)
);
