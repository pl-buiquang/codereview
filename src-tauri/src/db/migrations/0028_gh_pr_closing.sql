-- Issues a PR closes (JSON array of node ids), so the board can rebuild its
-- auto issue<->PR links from the mirror instead of from one sync's fetches.
-- NULL = not fetched since this column existed; the sync probe refetches it.
ALTER TABLE gh_pr ADD COLUMN closing_issue_ids TEXT;
