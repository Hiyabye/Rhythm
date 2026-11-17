-- Dates `rhythm_sources::dates` found in each notice's current title and body
-- (#166 O1); the feed ships them because it carries no full bodies.
-- `dates_version` is the extractor version that produced a notice's rows, NULL
-- before its first extraction; a notice without dates still records it.
ALTER TABLE notices ADD COLUMN dates_version INTEGER;
CREATE TABLE notice_dates(
  notice_id INTEGER NOT NULL REFERENCES notices(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  day TEXT NOT NULL,
  time TEXT,
  -- The last day of a multi-day `date`.
  end_day TEXT,
  end_time TEXT,
  label TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('deadline','start','date')),
  PRIMARY KEY(notice_id, ordinal)
) WITHOUT ROWID;

PRAGMA user_version=2;
