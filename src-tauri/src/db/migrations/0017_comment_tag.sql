ALTER TABLE comment ADD COLUMN tag TEXT CHECK (tag IN ('ai','local'));

CREATE TABLE focus_comment (
  focus_id   INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  comment_id INTEGER NOT NULL REFERENCES comment(id) ON DELETE CASCADE,
  PRIMARY KEY (focus_id, comment_id)
);

ALTER TABLE focus_hunk DROP COLUMN note;
ALTER TABLE focus_hunk DROP COLUMN note_dismissed;
