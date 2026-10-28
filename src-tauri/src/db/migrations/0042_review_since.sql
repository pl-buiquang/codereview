-- A review can cover only what changed since an earlier review's pin (an
-- interdiff). `base_sha`/`head_sha` stay the real PR pin; these record the
-- earlier pin the diff starts from. NULL = the whole PR.
ALTER TABLE review ADD COLUMN since_base_sha TEXT;
ALTER TABLE review ADD COLUMN since_head_sha TEXT;
