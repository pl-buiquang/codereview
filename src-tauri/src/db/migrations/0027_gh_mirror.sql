-- GitHub mirror (ADR 0002): one disposable copy of GitHub facts that Inbox,
-- Board and Review all read. User tables keep only their own state and point
-- at the mirror by node id with no enforced FK. `items` becomes `inbox_entry`
-- and `board_card` keeps only tracking state; both lose their fact columns.
PRAGMA foreign_keys = OFF;

BEGIN;

-- Repositories ---------------------------------------------------------------

ALTER TABLE repository ADD COLUMN registered_at TEXT;
UPDATE repository SET registered_at = added_at;

-- Repos only known through an inbox item or a board card become auto-added rows.
INSERT INTO repository (remote_owner, remote_name, added_at, registered_at)
SELECT DISTINCT substr(repo, 1, instr(repo, '/') - 1), substr(repo, instr(repo, '/') + 1),
       strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), NULL
FROM (SELECT repo FROM items UNION SELECT repo FROM board_card) AS src
WHERE instr(repo, '/') > 1
  AND NOT EXISTS (
      SELECT 1 FROM repository r
      WHERE LOWER(r.remote_owner || '/' || r.remote_name) = LOWER(src.repo)
  )
ORDER BY 1, 2;

-- Mirror tables ----------------------------------------------------------------

CREATE TABLE gh_user (
    login      TEXT PRIMARY KEY,
    avatar_url TEXT,
    fetched_at TEXT NOT NULL
);

CREATE TABLE gh_pr (
    node_id         TEXT PRIMARY KEY,
    repo_id         INTEGER NOT NULL REFERENCES repository(id) ON DELETE CASCADE,
    number          INTEGER NOT NULL,
    title           TEXT NOT NULL,
    url             TEXT NOT NULL,
    author_login    TEXT,
    state           TEXT,                 -- open / closed / merged
    is_draft        INTEGER NOT NULL DEFAULT 0,
    body_snippet    TEXT,                 -- plain-text snippet, from every fetch
    body            TEXT,                 -- full markdown, only from the detail fetch
    base_ref        TEXT,
    head_ref        TEXT,
    head_sha        TEXT,                 -- the LIVE head
    review_decision TEXT,
    review_count    INTEGER,
    additions       INTEGER,
    deletions       INTEGER,
    changed_files   INTEGER,
    latest_comment  TEXT,
    latest_actor    TEXT,
    gh_updated_at   TEXT,                 -- change key
    closed_at       TEXT,
    fetched_at      TEXT NOT NULL,
    UNIQUE (repo_id, number)
);

CREATE TABLE gh_pr_status (
    pr_id                    TEXT PRIMARY KEY REFERENCES gh_pr(node_id) ON DELETE CASCADE,
    head_sha                 TEXT,        -- the head these facts are valid for
    ci_state                 TEXT,        -- statusCheckRollup.state, GitHub casing
    checks_json              TEXT,        -- detail fetch only
    mergeable                TEXT,
    merge_state_status       TEXT,
    viewer_can_update_branch INTEGER NOT NULL DEFAULT 0,
    auto_merge_method        TEXT,
    auto_merge_by            TEXT,
    labels_json              TEXT,        -- detail fetch only
    latest_reviews_json      TEXT,        -- detail fetch only
    stack_number             INTEGER,
    stack_position           INTEGER,
    fetched_at               TEXT NOT NULL
);

CREATE TABLE gh_pr_threads (
    pr_id         TEXT PRIMARY KEY REFERENCES gh_pr(node_id) ON DELETE CASCADE,
    gh_updated_at TEXT,                   -- the PR updatedAt these were fetched at
    pending_count INTEGER,
    threads_json  TEXT,
    fetched_at    TEXT
);

CREATE TABLE gh_pr_file (
    pr_id     TEXT NOT NULL REFERENCES gh_pr(node_id) ON DELETE CASCADE,
    filename  TEXT NOT NULL,
    additions INTEGER NOT NULL,
    deletions INTEGER NOT NULL,
    changes   INTEGER NOT NULL,
    head_sha  TEXT,                       -- the head this list was fetched at
    PRIMARY KEY (pr_id, filename)
);

CREATE TABLE gh_issue (
    node_id        TEXT PRIMARY KEY,
    repo_id        INTEGER NOT NULL REFERENCES repository(id) ON DELETE CASCADE,
    number         INTEGER NOT NULL,
    title          TEXT NOT NULL,
    url            TEXT NOT NULL,
    author_login   TEXT,
    state          TEXT,
    body_snippet   TEXT,
    latest_comment TEXT,
    latest_actor   TEXT,
    gh_updated_at  TEXT,
    closed_at      TEXT,
    fetched_at     TEXT NOT NULL,
    UNIQUE (repo_id, number)
);

-- Backfill (newest updated_at wins between items and board_card) -------------

INSERT OR IGNORE INTO gh_user (login, avatar_url, fetched_at)
SELECT author_login, MAX(author_avatar), MAX(last_refreshed)
FROM items WHERE author_login IS NOT NULL GROUP BY author_login;
INSERT OR IGNORE INTO gh_user (login, avatar_url, fetched_at)
SELECT DISTINCT author_login, NULL, COALESCE(last_refreshed, tracked_at)
FROM board_card WHERE author_login IS NOT NULL;

INSERT INTO gh_pr (node_id, repo_id, number, title, url, author_login, state, is_draft,
                   body_snippet, review_decision, additions, deletions, changed_files,
                   latest_comment, latest_actor, gh_updated_at, fetched_at)
SELECT i.id, r.id, i.number, i.title, i.url, i.author_login, i.state, i.is_draft,
       i.body, i.review_decision, i.additions, i.deletions, i.files_changed,
       i.latest_comment, i.latest_actor, i.updated_at, i.last_refreshed
FROM items i JOIN repository r ON r.id = (SELECT MIN(r2.id) FROM repository r2
                              WHERE LOWER(r2.remote_owner || '/' || r2.remote_name) = LOWER(i.repo))
WHERE i.type = 'pr';

INSERT INTO gh_pr (node_id, repo_id, number, title, url, author_login, state, is_draft,
                   head_ref, review_decision, review_count, gh_updated_at, closed_at, fetched_at)
SELECT b.id, r.id, b.number, b.title, b.url, b.author_login, b.state, b.is_draft,
       b.head_ref, b.review_decision, b.has_reviews, b.updated_at, b.closed_at,
       COALESCE(b.last_refreshed, b.tracked_at)
FROM board_card b JOIN repository r ON r.id = (SELECT MIN(r2.id) FROM repository r2
                              WHERE LOWER(r2.remote_owner || '/' || r2.remote_name) = LOWER(b.repo))
WHERE b.type = 'pr'
ON CONFLICT (node_id) DO UPDATE SET
    head_ref     = excluded.head_ref,
    review_count = excluded.review_count,
    closed_at    = excluded.closed_at,
    title           = CASE WHEN excluded.gh_updated_at > gh_pr.gh_updated_at THEN excluded.title ELSE gh_pr.title END,
    state           = CASE WHEN excluded.gh_updated_at > gh_pr.gh_updated_at THEN excluded.state ELSE gh_pr.state END,
    is_draft        = CASE WHEN excluded.gh_updated_at > gh_pr.gh_updated_at THEN excluded.is_draft ELSE gh_pr.is_draft END,
    review_decision = CASE WHEN excluded.gh_updated_at > gh_pr.gh_updated_at THEN excluded.review_decision ELSE gh_pr.review_decision END,
    gh_updated_at   = MAX(excluded.gh_updated_at, gh_pr.gh_updated_at),
    fetched_at      = MAX(excluded.fetched_at, gh_pr.fetched_at);

-- Refs a review target already knows fill the gaps (base_ref is never on a card).
UPDATE gh_pr SET
    base_ref = COALESCE(base_ref, (SELECT t.base_ref FROM target t
                                   WHERE t.kind = 'github_pr' AND t.repo_id = gh_pr.repo_id
                                     AND t.github_pr_number = gh_pr.number)),
    head_ref = COALESCE(head_ref, (SELECT t.head_ref FROM target t
                                   WHERE t.kind = 'github_pr' AND t.repo_id = gh_pr.repo_id
                                     AND t.github_pr_number = gh_pr.number));

INSERT INTO gh_pr_status (pr_id, ci_state, fetched_at)
SELECT id, UPPER(ci_state), last_refreshed FROM items
WHERE type = 'pr' AND id IN (SELECT node_id FROM gh_pr);

INSERT INTO gh_pr_status (pr_id, ci_state, mergeable, merge_state_status, viewer_can_update_branch,
                          auto_merge_method, stack_number, stack_position, fetched_at)
SELECT b.id, b.ci_status, b.mergeable, b.merge_state_status, b.viewer_can_update_branch,
       b.auto_merge, b.stack_number, b.stack_position, COALESCE(b.last_refreshed, b.tracked_at)
FROM board_card b
WHERE b.type = 'pr' AND b.id IN (SELECT node_id FROM gh_pr)
ON CONFLICT (pr_id) DO UPDATE SET
    ci_state                 = COALESCE(excluded.ci_state, gh_pr_status.ci_state),
    mergeable                = excluded.mergeable,
    merge_state_status       = excluded.merge_state_status,
    viewer_can_update_branch = excluded.viewer_can_update_branch,
    auto_merge_method        = excluded.auto_merge_method,
    stack_number             = excluded.stack_number,
    stack_position           = excluded.stack_position;

INSERT INTO gh_pr_threads (pr_id, gh_updated_at, pending_count)
SELECT id, updated_at, pending_threads FROM board_card
WHERE type = 'pr' AND pending_threads IS NOT NULL AND id IN (SELECT node_id FROM gh_pr);

INSERT OR IGNORE INTO gh_pr_file (pr_id, filename, additions, deletions, changes)
SELECT i.id, json_extract(f.value, '$.path'), json_extract(f.value, '$.additions'),
       json_extract(f.value, '$.deletions'), json_extract(f.value, '$.changes')
FROM items i, json_each(i.top_files_json) f
WHERE i.type = 'pr' AND json_valid(i.top_files_json) AND i.id IN (SELECT node_id FROM gh_pr);

INSERT INTO gh_issue (node_id, repo_id, number, title, url, author_login, state, body_snippet,
                      latest_comment, latest_actor, gh_updated_at, fetched_at)
SELECT i.id, r.id, i.number, i.title, i.url, i.author_login, i.state, i.body,
       i.latest_comment, i.latest_actor, i.updated_at, i.last_refreshed
FROM items i JOIN repository r ON r.id = (SELECT MIN(r2.id) FROM repository r2
                              WHERE LOWER(r2.remote_owner || '/' || r2.remote_name) = LOWER(i.repo))
WHERE i.type = 'issue';

INSERT INTO gh_issue (node_id, repo_id, number, title, url, author_login, state,
                      gh_updated_at, closed_at, fetched_at)
SELECT b.id, r.id, b.number, b.title, b.url, b.author_login, b.state, b.updated_at,
       b.closed_at, COALESCE(b.last_refreshed, b.tracked_at)
FROM board_card b JOIN repository r ON r.id = (SELECT MIN(r2.id) FROM repository r2
                              WHERE LOWER(r2.remote_owner || '/' || r2.remote_name) = LOWER(b.repo))
WHERE b.type = 'issue'
ON CONFLICT (node_id) DO UPDATE SET
    closed_at     = excluded.closed_at,
    title         = CASE WHEN excluded.gh_updated_at > gh_issue.gh_updated_at THEN excluded.title ELSE gh_issue.title END,
    state         = CASE WHEN excluded.gh_updated_at > gh_issue.gh_updated_at THEN excluded.state ELSE gh_issue.state END,
    gh_updated_at = MAX(excluded.gh_updated_at, gh_issue.gh_updated_at);

-- Inbox: triage state only. `closed_at` is when the inbox noticed the item
-- closed (its Closed bucket), not GitHub's closedAt. -------------------------

CREATE TABLE inbox_entry (
    id            TEXT PRIMARY KEY,       -- gh_pr / gh_issue node id (soft ref)
    type          TEXT NOT NULL CHECK (type IN ('pr', 'issue')),
    first_seen_at TEXT NOT NULL,
    untracked_at  TEXT,
    engaged_at    TEXT,
    closed_at     TEXT
);

INSERT INTO inbox_entry (id, type, first_seen_at, untracked_at, engaged_at, closed_at)
SELECT id, type, first_seen_at, untracked_at, engaged_at, closed_at FROM items;

CREATE TABLE item_reasons_new (
    item_id TEXT NOT NULL REFERENCES inbox_entry (id) ON DELETE CASCADE,
    reason  TEXT NOT NULL,
    detail  TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (item_id, reason, detail)
);
INSERT INTO item_reasons_new SELECT item_id, reason, detail FROM item_reasons;
DROP TABLE item_reasons;
DROP TABLE items;
ALTER TABLE item_reasons_new RENAME TO item_reasons;

CREATE INDEX idx_inbox_entry_open ON inbox_entry (untracked_at, closed_at);

-- Board: tracking state only (board_link / board_meta keep referencing id). --

CREATE TABLE board_card_new (
    id           TEXT PRIMARY KEY,        -- gh_pr / gh_issue node id (soft ref)
    type         TEXT NOT NULL CHECK (type IN ('pr', 'issue')),
    tracked_at   TEXT NOT NULL,
    discarded_at TEXT,
    archived_at  TEXT
);
INSERT INTO board_card_new (id, type, tracked_at, discarded_at, archived_at)
SELECT id, type, tracked_at, discarded_at, archived_at FROM board_card;
DROP TABLE board_card;
ALTER TABLE board_card_new RENAME TO board_card;

CREATE INDEX idx_board_card_open ON board_card (archived_at, discarded_at);

-- Targets: name what is reviewed. PR facts come from gh_pr; title/refs stay as
-- the label shown until the mirror has the PR. The pin moved to review (0026).

CREATE TABLE target_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id          INTEGER NOT NULL REFERENCES repository(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('github_pr', 'local')),
    github_pr_number INTEGER,
    gh_pr_id         TEXT,                -- soft ref to gh_pr
    title            TEXT NOT NULL,
    base_ref         TEXT NOT NULL,
    head_ref         TEXT NOT NULL,
    three_dot        INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);
INSERT INTO target_new (id, repo_id, kind, github_pr_number, gh_pr_id, title, base_ref, head_ref,
                        three_dot, created_at)
SELECT t.id, t.repo_id, t.kind, t.github_pr_number,
       (SELECT p.node_id FROM gh_pr p WHERE t.kind = 'github_pr' AND p.repo_id = t.repo_id
                                       AND p.number = t.github_pr_number),
       t.title, t.base_ref, t.head_ref, t.three_dot, t.created_at
FROM target t;
DROP TABLE target;
ALTER TABLE target_new RENAME TO target;

CREATE INDEX idx_target_repo ON target(repo_id);
CREATE INDEX idx_target_gh_pr ON target(gh_pr_id) WHERE gh_pr_id IS NOT NULL;

COMMIT;

PRAGMA foreign_keys = ON;
