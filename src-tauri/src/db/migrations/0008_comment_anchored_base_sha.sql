-- The base/merge-base SHA a LEFT-side comment's line numbers are valid against
-- (the old side of the diff the user was looking at). NULL on RIGHT comments and
-- on rows created before this column existed; NULL is treated as "anchored".
ALTER TABLE comment ADD COLUMN anchored_base_sha TEXT;
