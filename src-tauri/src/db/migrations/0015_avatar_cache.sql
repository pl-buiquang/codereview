CREATE TABLE avatar_cache (
    email      TEXT PRIMARY KEY,
    data_url   TEXT,
    fetched_at INTEGER NOT NULL
);
