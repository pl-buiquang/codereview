-- GitHub "inbox": items (PRs + issues) needing the user's attention, ported from
-- the gh-dashboard app. Items are keyed by their GitHub GraphQL node id and carry
-- the reasons they surfaced (assigned / mention / review-requested / authored /
-- team-review) in a side table. State is expressed via timestamp columns:
--   untracked_at  -> archived (hidden from inbox, searchable in archive)
--   closed_at     -> detected closed/merged (shown in the Closed bucket)
--   engaged_at    -> marked "done"/visited

CREATE TABLE items (
    id              TEXT PRIMARY KEY,                 -- GitHub GraphQL node id
    type            TEXT NOT NULL CHECK (type IN ('pr', 'issue')),
    number          INTEGER NOT NULL,
    repo            TEXT NOT NULL,                    -- 'owner/name'
    title           TEXT NOT NULL,
    url             TEXT NOT NULL,
    author_login    TEXT,
    author_avatar   TEXT,
    state           TEXT,                             -- open / closed / merged
    is_draft        INTEGER NOT NULL DEFAULT 0,
    body            TEXT,                             -- snippet of the issue/PR body
    latest_comment  TEXT,                             -- snippet of the last comment
    latest_actor    TEXT,                             -- who commented last
    updated_at      TEXT NOT NULL,                    -- ISO8601, from GitHub
    files_changed   INTEGER,
    additions       INTEGER,
    deletions       INTEGER,
    top_files_json  TEXT,                             -- JSON array of top-5 changed files
    ci_state        TEXT,                             -- success / failure / pending
    review_decision TEXT,
    untracked_at    TEXT,                             -- archived when set
    closed_at       TEXT,                             -- detected closed when set
    engaged_at      TEXT,                             -- marked done/visited when set
    first_seen_at   TEXT NOT NULL,
    last_refreshed  TEXT NOT NULL
);

CREATE INDEX idx_items_inbox ON items (untracked_at, updated_at DESC);
CREATE INDEX idx_items_closed ON items (closed_at DESC) WHERE closed_at IS NOT NULL;

-- Why an item is in the inbox. An item can have several reasons; `detail` carries
-- e.g. the 'org/team' for a team-review reason (empty string otherwise).
CREATE TABLE item_reasons (
    item_id TEXT NOT NULL REFERENCES items (id) ON DELETE CASCADE,
    reason  TEXT NOT NULL,
    detail  TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (item_id, reason, detail)
);

-- Small key/value store for inbox bookkeeping: last_refresh_at and the cached
-- viewer (login + team slugs, with a fetched-at timestamp for a 24h TTL).
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT
);
