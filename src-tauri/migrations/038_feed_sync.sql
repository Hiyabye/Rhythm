-- Feed sync (#177): where the feed lives (the default base until a signed
-- `moved_to` moves it), the last successful check and how the last attempt
-- ended, and the board health the last accepted manifest reported.
ALTER TABLE feed_state ADD COLUMN base TEXT;
ALTER TABLE feed_state ADD COLUMN checked_at TEXT;
ALTER TABLE feed_state ADD COLUMN result TEXT CHECK(result IS NULL OR result IN ('ok','offline','failed','update_required'));
ALTER TABLE feed_state ADD COLUMN error TEXT;
CREATE TABLE feed_boards (
 source_id TEXT PRIMARY KEY,
 last_success_at TEXT,
 consecutive_failures INTEGER NOT NULL CHECK(consecutive_failures>=0),
 history_complete INTEGER NOT NULL CHECK(history_complete IN (0,1))
);
PRAGMA user_version=38;
