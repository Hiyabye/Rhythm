-- Public boards' "new since sync" count (#274) compares a feed notice's first
-- sighting with the manifest accepted before the latest one. Starting it at
-- the current manifest counts nothing until the next new manifest arrives.
ALTER TABLE feed_state ADD COLUMN previous_generated_at TEXT;
UPDATE feed_state SET previous_generated_at=generated_at;
PRAGMA user_version=41;
