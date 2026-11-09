-- AI profiles (harness + model + settings), the neutral routine access model, routine kinds and
-- per-chat profile pinning. The old Claude-specific routine columns (permission_mode, allowed_tools,
-- extra_args, system_prompt_mode) stay for one release and are no longer read; a later migration
-- drops them.

-- host_id has no foreign key: the `host` table arrives with the remote harness.
CREATE TABLE ai_profile (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    harness    TEXT NOT NULL DEFAULT 'claude',
    model      TEXT NOT NULL DEFAULT '',
    settings   TEXT NOT NULL DEFAULT '{}',   -- JSON object, harness-specific, never a secret
    host_id    INTEGER,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
-- At most one default profile.
CREATE UNIQUE INDEX idx_ai_profile_default ON ai_profile(is_default) WHERE is_default = 1;

-- The model is empty: the former chat model lived in the frontend settings and is imported once
-- (`ai_import_legacy_models`).
INSERT INTO ai_profile (name, harness, model, settings, is_default, created_at, updated_at)
VALUES ('Claude Code', 'claude', '', '{}', 1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- Routines: kind, profile, neutral access, per-harness overrides.
ALTER TABLE routine ADD COLUMN kind TEXT NOT NULL DEFAULT 'plain' CHECK (kind IN ('plain', 'system'));
ALTER TABLE routine ADD COLUMN profile_id INTEGER REFERENCES ai_profile(id) ON DELETE SET NULL;
ALTER TABLE routine ADD COLUMN access TEXT NOT NULL DEFAULT 'read_only' CHECK (access IN ('read_only', 'edit', 'full'));
ALTER TABLE routine ADD COLUMN harness_overrides TEXT NOT NULL DEFAULT '{}';   -- JSON, keyed by harness id

-- System routines are the ones with a Rust handler.
UPDATE routine SET kind = 'system' WHERE handler IS NOT NULL;
UPDATE routine SET access = CASE permission_mode
    WHEN 'bypassPermissions' THEN 'full'
    WHEN 'acceptEdits' THEN 'edit'
    ELSE 'read_only'
END;
UPDATE routine SET harness_overrides = json_object('claude', json_object(
    'permission_mode', permission_mode,
    'allowed_tools', json(CASE WHEN json_valid(allowed_tools) THEN allowed_tools ELSE '[]' END),
    'extra_args', json(CASE WHEN json_valid(extra_args) THEN extra_args ELSE '[]' END),
    'system_prompt_mode', system_prompt_mode
));

-- A chat pins the profile it started its session with.
ALTER TABLE chat ADD COLUMN profile_id INTEGER REFERENCES ai_profile(id) ON DELETE SET NULL;

-- Which harness ran (the usage log groups by it).
ALTER TABLE ai_run ADD COLUMN harness TEXT NOT NULL DEFAULT 'claude';
