-- Deletion detection (#166 D24; user decisions 2026-11-17). `missing_since` is
-- the first definitive "not found" since the notice was last fetched; a second
-- one at least 24 h later sets `removed_at`, the feed's tombstone. A later
-- successful fetch clears both.
ALTER TABLE notices ADD COLUMN missing_since TEXT;
ALTER TABLE notices ADD COLUMN removed_at TEXT;
-- Skipped notices are retried daily, up to a bounded number of attempts per
-- collector version, so a parser fix in a new version retries them all.
ALTER TABLE skipped ADD COLUMN attempts INTEGER NOT NULL DEFAULT 1;
ALTER TABLE skipped ADD COLUMN collector_version TEXT NOT NULL DEFAULT '';

PRAGMA user_version=3;
