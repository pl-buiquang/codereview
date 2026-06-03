-- Initial schema for the codereview internal model.

CREATE TABLE repository (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    path           TEXT NOT NULL UNIQUE,
    remote_owner   TEXT,
    remote_name    TEXT,
    default_branch TEXT,
    added_at       TEXT NOT NULL
);

-- The thing being reviewed: a real GitHub PR or a local "virtual PR".
-- Reusable across multiple reviews.
CREATE TABLE target (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id          INTEGER NOT NULL REFERENCES repository(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('github_pr', 'local')),
    github_pr_number INTEGER,
    title            TEXT NOT NULL,
    base_ref         TEXT NOT NULL,
    head_ref         TEXT NOT NULL,
    base_sha         TEXT,
    head_sha         TEXT,
    three_dot        INTEGER NOT NULL DEFAULT 1,
    created_at       TEXT NOT NULL
);

-- The user-managed review unit. Many reviews can target the same `target`.
CREATE TABLE review (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id        INTEGER NOT NULL REFERENCES target(id) ON DELETE CASCADE,
    body             TEXT NOT NULL DEFAULT '',
    event            TEXT CHECK (event IN ('comment', 'approve', 'request_changes')),
    status           TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published')),
    published_at     TEXT,
    github_review_id INTEGER,
    last_exported_at TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

-- An inline or general comment belonging to one review.
CREATE TABLE comment (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    review_id         INTEGER NOT NULL REFERENCES review(id) ON DELETE CASCADE,
    file_path         TEXT NOT NULL,
    side              TEXT NOT NULL DEFAULT 'RIGHT' CHECK (side IN ('LEFT', 'RIGHT')),
    line              INTEGER NOT NULL,
    start_line        INTEGER,
    diff_hunk         TEXT,
    body              TEXT NOT NULL,
    parent_id         INTEGER REFERENCES comment(id) ON DELETE CASCADE,
    anchored_head_sha TEXT,
    github_comment_id INTEGER,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE INDEX idx_target_repo    ON target(repo_id);
CREATE INDEX idx_review_target  ON review(target_id);
CREATE INDEX idx_comment_review ON comment(review_id);
