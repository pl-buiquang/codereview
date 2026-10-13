-- Kanban-style work tracker ("board"): a card per tracked GitHub issue or PR,
-- derived lanes (Backlog/In progress/Reviews/Submitted/Done), issue<->PR links,
-- and freeform metadata (plan/spec files, links, notes, Claude sessions).

CREATE TABLE board_card (
    id              TEXT PRIMARY KEY,                 -- GitHub GraphQL node id
    type            TEXT NOT NULL CHECK (type IN ('pr', 'issue')),
    repo            TEXT NOT NULL,                    -- 'owner/name'
    number          INTEGER NOT NULL,
    url             TEXT NOT NULL,
    title           TEXT NOT NULL,
    author_login    TEXT,
    state           TEXT,                             -- open / closed / merged
    is_draft        INTEGER NOT NULL DEFAULT 0,
    review_decision TEXT,                              -- APPROVED / CHANGES_REQUESTED / REVIEW_REQUIRED
    has_reviews     INTEGER NOT NULL DEFAULT 0,        -- any review submitted (for the "commented" badge)
    head_ref        TEXT,
    is_mine         INTEGER NOT NULL DEFAULT 0,
    pending_threads INTEGER,                           -- computed comment badge, cached from the last refresh
    updated_at      TEXT NOT NULL,
    closed_at       TEXT,
    tracked_at      TEXT NOT NULL,
    discarded_at    TEXT,                              -- soft-discarded: hidden, metadata kept, re-trackable
    archived_at     TEXT,                              -- purged from the Done lane
    last_refreshed  TEXT,
    UNIQUE (repo, number, type)
);

CREATE INDEX idx_board_card_open ON board_card (archived_at, discarded_at, state);

-- Issue <-> PR links, auto-detected (closingIssuesReferences / branch name) or
-- manually added. Only manual links can be removed by the user.
CREATE TABLE board_link (
    issue_id TEXT NOT NULL REFERENCES board_card (id) ON DELETE CASCADE,
    pr_id    TEXT NOT NULL REFERENCES board_card (id) ON DELETE CASCADE,
    source   TEXT NOT NULL CHECK (source IN ('auto', 'manual')),
    PRIMARY KEY (issue_id, pr_id)
);

-- Freeform metadata attached to a card: plan/spec files, links, notes, and
-- Claude Code session pointers added via the `cr` CLI.
CREATE TABLE board_meta (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id    TEXT NOT NULL REFERENCES board_card (id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('file', 'link', 'note', 'claude_session')),
    value      TEXT NOT NULL,                          -- path / url / note text / session id
    tag        TEXT,
    label      TEXT,
    cwd        TEXT,                                   -- claude_session only
    created_at TEXT NOT NULL
);

CREATE INDEX idx_board_meta_card ON board_meta (card_id);
