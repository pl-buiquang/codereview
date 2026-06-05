-- A comment is either anchored to specific line(s) ('line') or attached to the
-- whole file ('file'). File-level comments ignore side/line, which keep their
-- column defaults (side='RIGHT', line=0).
ALTER TABLE comment ADD COLUMN subject_type TEXT NOT NULL DEFAULT 'line'
    CHECK (subject_type IN ('line', 'file'));
