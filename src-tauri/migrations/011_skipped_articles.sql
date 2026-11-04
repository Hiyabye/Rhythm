-- Notices whose fetched response was rejected as unparseable for that response
-- (not a network or server failure). The page checkpoint may advance past them;
-- a later fetch that archives the notice removes the row. `historical` keeps the
-- marking the original run would have used, so an explicit retry neither
-- announces an old notice nor hides a new one.
CREATE TABLE skipped_articles (
 source_id TEXT NOT NULL REFERENCES sources(id),
 external_id TEXT NOT NULL,
 url TEXT NOT NULL,
 historical INTEGER NOT NULL CHECK(historical IN (0,1)),
 reason TEXT NOT NULL,
 response_sha256 TEXT NOT NULL,
 skipped_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(source_id,external_id)
);
PRAGMA user_version=11;
