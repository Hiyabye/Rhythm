-- The published feed (docs/FEED.md). `feed` holds the next seq; nothing is
-- written here before the run's files are uploaded, and the manifest is
-- published only after the state that records them (user decision,
-- 2026-11-18), so a seq a client has seen is never reused.
CREATE TABLE feed(
  id INTEGER PRIMARY KEY CHECK(id = 1),
  next_seq INTEGER NOT NULL CHECK(next_seq >= 1)
);
INSERT INTO feed(id,next_seq) VALUES(1,1);
-- What each notice last published: the seq and a digest of each record, so
-- a run publishes only what changed. A removal clears the digests, so a
-- restored notice is published again.
CREATE TABLE feed_notices(
  notice_id INTEGER PRIMARY KEY REFERENCES notices(id) ON DELETE CASCADE,
  notice_seq INTEGER,
  notice_digest TEXT,
  embedding_seq INTEGER,
  embedding_digest TEXT,
  classification_seq INTEGER,
  classification_digest TEXT,
  removed_seq INTEGER
);
-- The records of run segments not yet compacted into a daily segment: the
-- record JSON and, for an embedding, its int8 vector.
CREATE TABLE feed_records(
  seq INTEGER PRIMARY KEY,
  record TEXT NOT NULL,
  vector BLOB CHECK(vector IS NULL OR length(vector) = 1028)
);
-- Every published file: listed while `retired_at` is NULL, deleted from R2 a
-- day after it is retired. `day` is the UTC day of a run or daily segment
-- and of a snapshot.
CREATE TABLE feed_files(
  records_path TEXT PRIMARY KEY,
  role TEXT NOT NULL CHECK(role IN ('run','daily','snapshot')),
  day TEXT NOT NULL,
  from_seq INTEGER NOT NULL,
  to_seq INTEGER NOT NULL,
  records_sha256 TEXT NOT NULL,
  records_size INTEGER NOT NULL,
  vectors_path TEXT,
  vectors_sha256 TEXT,
  vectors_size INTEGER,
  retired_at TEXT
);

PRAGMA user_version=6;
