-- Per-comment publish override. NULL follows the tag: untagged comments are
-- published, AI-tagged (`ai`, `ai:*`) ones are not. 1 publishes an AI comment
-- (it keeps its tag), 0 keeps an untagged one local. A `local` tag always wins.
ALTER TABLE comment ADD COLUMN publish INTEGER;
