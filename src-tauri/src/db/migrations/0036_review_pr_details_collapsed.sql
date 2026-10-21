ALTER TABLE review ADD COLUMN pr_details_collapsed INTEGER NOT NULL DEFAULT 0;
-- Reviews that already exist were opened with the details collapsed.
UPDATE review SET pr_details_collapsed = 1;
