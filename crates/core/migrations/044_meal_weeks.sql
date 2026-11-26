-- Weeks of the Bellarmine dormitory menu from the feed (format 1.3, #318),
-- read by the collector's OCR from the menu image; display only, never used
-- for ranking, recommendations or notifications. `menu` holds the validated
-- days and week-shared rows as JSON while `status` is 'ok' and is NULL for an
-- unreadable week. `feed_state.meals_known` is set once a complete sync has
-- mirrored menu weeks; until then a sync with a cursor first backfills them
-- from the snapshot and later segments, which an older version skipped. The
-- `meals` extension is new, so it starts off on every profile.
CREATE TABLE meal_weeks(
  week TEXT PRIMARY KEY,
  source_id TEXT NOT NULL,
  external_id TEXT NOT NULL,
  image_url TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('ok', 'unreadable')),
  low_resolution INTEGER NOT NULL CHECK(low_resolution IN (0, 1)),
  menu TEXT,
  CHECK((status = 'ok') = (menu IS NOT NULL))
) WITHOUT ROWID;
ALTER TABLE feed_state ADD COLUMN meals_known INTEGER NOT NULL DEFAULT 0
  CHECK(meals_known IN (0, 1));
INSERT INTO extension_preferences(extension_id,enabled) VALUES('meals',0)
ON CONFLICT(extension_id) DO NOTHING;
PRAGMA user_version=44;
