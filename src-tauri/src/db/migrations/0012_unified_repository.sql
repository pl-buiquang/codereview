-- Unify the repository model: replace the overloaded `path` column
-- (real-path OR "github:owner/name" sentinel) with a nullable `local_path`.
-- Adds a partial unique index on (remote_owner, remote_name) to prevent
-- duplicate rows for the same GitHub repo.
-- Deduplicates existing sentinel+local-clone pairs by re-pointing targets
-- to the local-clone row and deleting the sentinel.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE repository_new (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    local_path     TEXT UNIQUE,
    remote_owner   TEXT,
    remote_name    TEXT,
    default_branch TEXT,
    added_at       TEXT NOT NULL,
    CHECK (local_path IS NOT NULL OR (remote_owner IS NOT NULL AND remote_name IS NOT NULL))
);

-- Copy data, converting github: sentinels to local_path = NULL
INSERT INTO repository_new (id, local_path, remote_owner, remote_name, default_branch, added_at)
SELECT id,
       CASE WHEN path LIKE 'github:%' THEN NULL ELSE path END,
       remote_owner,
       remote_name,
       default_branch,
       added_at
FROM repository;

-- Re-point targets from a sentinel row to its local-clone counterpart
-- (only fires when both a sentinel and a real clone share the same remote).
UPDATE target
SET repo_id = (
    SELECT keeper.id
    FROM repository_new AS keeper
    INNER JOIN repository_new AS loser
        ON keeper.remote_owner = loser.remote_owner
       AND keeper.remote_name  = loser.remote_name
       AND keeper.local_path IS NOT NULL
       AND loser.local_path  IS NULL
    WHERE loser.id = target.repo_id
)
WHERE EXISTS (
    SELECT 1
    FROM repository_new AS loser
    INNER JOIN repository_new AS keeper
        ON keeper.remote_owner = loser.remote_owner
       AND keeper.remote_name  = loser.remote_name
       AND keeper.local_path IS NOT NULL
    WHERE loser.id = target.repo_id
      AND loser.local_path IS NULL
);

-- Delete the now-redundant sentinel rows
DELETE FROM repository_new
WHERE id IN (
    SELECT loser.id
    FROM repository_new AS loser
    WHERE loser.local_path IS NULL
      AND loser.remote_owner IS NOT NULL
      AND loser.remote_name  IS NOT NULL
      AND EXISTS (
          SELECT 1 FROM repository_new AS keeper
          WHERE keeper.remote_owner = loser.remote_owner
            AND keeper.remote_name  = loser.remote_name
            AND keeper.local_path IS NOT NULL
      )
);

DROP TABLE repository;
ALTER TABLE repository_new RENAME TO repository;

-- Recreate indexes dropped with the old table.
-- idx_repo_remote from 0005 is replaced by a UNIQUE variant.
CREATE UNIQUE INDEX idx_repo_remote_unique
    ON repository (remote_owner, remote_name)
    WHERE remote_owner IS NOT NULL AND remote_name IS NOT NULL;

COMMIT;

PRAGMA foreign_keys = ON;
