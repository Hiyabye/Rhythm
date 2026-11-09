-- User-made calendar todos. `day` and `time` are local wall-clock values (a
-- todo is on a date the user picked, not a UTC instant); only the bookkeeping
-- timestamps are UTC. The calendar extension is new, so it starts off on every
-- profile.
CREATE TABLE calendar_todos(
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  day TEXT NOT NULL,
  time TEXT,
  memo TEXT NOT NULL DEFAULT '',
  done INTEGER NOT NULL DEFAULT 0 CHECK(done IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX calendar_todos_day ON calendar_todos(day, time);
INSERT INTO extension_preferences(extension_id,enabled) VALUES('calendar',0)
ON CONFLICT(extension_id) DO NOTHING;
PRAGMA user_version=22;
