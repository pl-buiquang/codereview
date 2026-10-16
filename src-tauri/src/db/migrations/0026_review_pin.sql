-- The pin (the commits a review was written against) moves from the shared
-- target to each review, so reopening a target never moves an older review.
-- target.base_sha/head_sha are no longer read (dropped with the GitHub mirror).
ALTER TABLE review ADD COLUMN base_sha TEXT;
ALTER TABLE review ADD COLUMN head_sha TEXT;
UPDATE review SET
    base_sha = (SELECT t.base_sha FROM target t WHERE t.id = review.target_id),
    head_sha = (SELECT t.head_sha FROM target t WHERE t.id = review.target_id);
