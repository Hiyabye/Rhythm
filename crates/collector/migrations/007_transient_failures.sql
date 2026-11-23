-- Notices whose fetch failed transiently (timeout, dropped connection, server
-- error) on consecutive runs. Such a failure holds the page, so a notice that
-- fails the same way on every run would stop its board for good; after a few
-- runs it moves to `skipped` instead (user decision 2026-11-23). A successful
-- fetch, a skip or a "not found" removes the row.
CREATE TABLE transient_failures(
  source_id TEXT NOT NULL REFERENCES boards(source_id),
  external_id TEXT NOT NULL,
  runs INTEGER NOT NULL,
  last_failed_at TEXT NOT NULL,
  PRIMARY KEY(source_id, external_id)
);

PRAGMA user_version=7;
