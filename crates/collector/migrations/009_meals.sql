-- The dormitory menu (#318). `meal_images` is each menu image the collector
-- offered to the OCR step: `pending` until a result of `ocr_version` arrives,
-- then `ok` or `unreadable`. An image read at the current OCR version is never
-- fetched again.
CREATE TABLE meal_images(
  image_url TEXT PRIMARY KEY,
  source_id TEXT NOT NULL,
  external_id TEXT NOT NULL,
  -- The week's Monday, from the notice title.
  week TEXT NOT NULL,
  published_at TEXT NOT NULL,
  -- Of the bytes the collector fetched; a result must name this hash.
  image_sha256 TEXT NOT NULL,
  ocr_version TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending','ok','unreadable')),
  reason TEXT,
  offered_at TEXT NOT NULL,
  processed_at TEXT
);
-- The current week as the feed publishes it: from the newest post of that
-- week whose image was read, `ok` or `unreadable`.
CREATE TABLE meal_weeks(
  week TEXT PRIMARY KEY,
  source_id TEXT NOT NULL,
  external_id TEXT NOT NULL,
  published_at TEXT NOT NULL,
  -- The `meal.week` record (docs/FEED.md), JSON.
  record TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
-- What each week last published, like `feed_events`.
CREATE TABLE feed_meals(
  week TEXT PRIMARY KEY REFERENCES meal_weeks(week) ON DELETE CASCADE,
  meal_seq INTEGER NOT NULL,
  meal_digest TEXT NOT NULL
);

PRAGMA user_version=9;
