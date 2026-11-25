-- The academic calendar (#319): each calendar's health and its current
-- events. An event absent from a fetched calendar is marked missing, and
-- removed by a later fetch at least a day after (`calendar.rs`); an event that
-- ended before the calendar's earliest event aged out of its window and is
-- kept.
CREATE TABLE calendars(
  calendar_id TEXT PRIMARY KEY,
  last_success_at TEXT,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);
CREATE TABLE calendar_events(
  calendar_id TEXT NOT NULL REFERENCES calendars(calendar_id),
  uid TEXT NOT NULL,
  title TEXT NOT NULL,
  start_day TEXT NOT NULL,
  end_day TEXT NOT NULL CHECK(end_day >= start_day),
  first_seen_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  missing_since TEXT,
  removed_at TEXT,
  PRIMARY KEY(calendar_id, uid)
);
-- What each event last published, like `feed_notices`.
CREATE TABLE feed_events(
  calendar_id TEXT NOT NULL,
  uid TEXT NOT NULL,
  event_seq INTEGER,
  event_digest TEXT,
  removed_seq INTEGER,
  PRIMARY KEY(calendar_id, uid),
  FOREIGN KEY(calendar_id, uid) REFERENCES calendar_events(calendar_id, uid) ON DELETE CASCADE
);

PRAGMA user_version=8;
