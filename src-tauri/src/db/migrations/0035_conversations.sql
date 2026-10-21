-- Inbox Conversations tab. The first two tables are GitHub mirror facts
-- (disposable, ADR 0002); conversation_state is user state and points at the
-- mirror by node id with no enforced FK.

-- One row per notified PR/issue (GitHub keeps one notification per subject).
CREATE TABLE gh_notification (
    id              TEXT PRIMARY KEY,     -- notification thread id
    subject_type    TEXT NOT NULL,        -- PullRequest | Issue
    subject_node_id TEXT,                 -- gh_pr / gh_issue node id, once resolved
    repo            TEXT NOT NULL,        -- owner/name
    number          INTEGER NOT NULL,
    title           TEXT NOT NULL,
    reason          TEXT,
    unread          INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL,
    last_read_at    TEXT,
    fetched_at      TEXT NOT NULL
);
CREATE INDEX idx_gh_notification_subject ON gh_notification (subject_node_id);

-- Top-level comments + review bodies of a PR, or comments of an issue. No FK:
-- the parent is a gh_pr or a gh_issue.
CREATE TABLE gh_discussion (
    subject_id    TEXT PRIMARY KEY,
    gh_updated_at TEXT,                   -- the subject updatedAt these were fetched at
    comments_json TEXT NOT NULL,
    fetched_at    TEXT NOT NULL
);

-- id is a review thread node id, or 'discussion:<subject node id>'.
CREATE TABLE conversation_state (
    id           TEXT PRIMARY KEY,
    subject_id   TEXT NOT NULL,
    pinned_at    TEXT,
    last_seen_at TEXT
);

-- History that exists today is not "unread".
INSERT OR REPLACE INTO meta (key, value) VALUES ('conversations_seen_baseline', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
