CREATE TABLE focus (
  id          INTEGER PRIMARY KEY,
  review_id   INTEGER NOT NULL UNIQUE REFERENCES review(id) ON DELETE CASCADE,
  head_sha    TEXT,
  base_sha    TEXT,
  level       TEXT NOT NULL DEFAULT 'focus'
              CHECK (level IN ('off','thorough','focus','critical')),
  summary     TEXT,
  model       TEXT,
  status      TEXT NOT NULL CHECK (status IN ('generating','ready','failed')),
  error       TEXT,
  planned_at  TEXT,
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE focus_hunk (
  focus_id       INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  hunk_id        TEXT NOT NULL,
  file_path      TEXT NOT NULL,
  old_start      INTEGER NOT NULL,
  new_start      INTEGER NOT NULL,
  tier           TEXT NOT NULL CHECK (tier IN ('critical','review','skim','trivial')),
  source         TEXT NOT NULL CHECK (source IN ('rule','ai','default')),
  reason         TEXT,
  note           TEXT,
  note_dismissed INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (focus_id, hunk_id)
);

CREATE TABLE focus_hunk_override (
  focus_id  INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  hunk_id   TEXT NOT NULL,
  override  TEXT NOT NULL CHECK (override IN ('show','hide')),
  PRIMARY KEY (focus_id, hunk_id)
);

CREATE TABLE focus_file_override (
  focus_id  INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  override  TEXT NOT NULL CHECK (override IN ('show','hide')),
  PRIMARY KEY (focus_id, file_path)
);
