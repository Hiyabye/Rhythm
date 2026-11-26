-- A multi-day notice date (`행사 11. 15.~11. 16.`) is one row from `day`
-- through `end_day`; a range that ends in a deadline stays a start row and a
-- deadline row. Rows extracted before this keep NULL until the extractor's
-- version bump re-extracts them.
ALTER TABLE notice_dates ADD COLUMN end_day TEXT;
ALTER TABLE notice_dates ADD COLUMN end_time TEXT;
PRAGMA user_version=34;
