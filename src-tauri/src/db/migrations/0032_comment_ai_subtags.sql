ALTER TABLE comment ADD COLUMN tag_new TEXT
  CHECK (tag_new IN ('ai','ai:explain','ai:review','ai:codex','local'));
UPDATE comment SET tag_new = tag;
ALTER TABLE comment DROP COLUMN tag;
ALTER TABLE comment RENAME COLUMN tag_new TO tag;
