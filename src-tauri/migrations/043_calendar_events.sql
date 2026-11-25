-- The academic calendar's events from the feed (format 1.2, #319): dates and
-- titles only, never used for ranking, recommendations or notifications.
-- `feed_state.events_known` is set once a complete sync has mirrored events;
-- until then a sync with a cursor first backfills them from the snapshot and
-- later segments, which an older version skipped. `settings.calendar_academic`
-- shows them in the calendar and the week pane (on by default, user decision).
CREATE TABLE calendar_events(
  calendar_id TEXT NOT NULL,
  uid TEXT NOT NULL,
  title TEXT NOT NULL,
  start_day TEXT NOT NULL,
  end_day TEXT NOT NULL CHECK(end_day >= start_day),
  PRIMARY KEY(calendar_id, uid)
) WITHOUT ROWID;
CREATE INDEX calendar_events_days ON calendar_events(start_day, end_day);
ALTER TABLE feed_state ADD COLUMN events_known INTEGER NOT NULL DEFAULT 0
  CHECK(events_known IN (0, 1));
ALTER TABLE settings ADD COLUMN calendar_academic INTEGER NOT NULL DEFAULT 1
  CHECK(calendar_academic IN (0, 1));
PRAGMA user_version=43;
