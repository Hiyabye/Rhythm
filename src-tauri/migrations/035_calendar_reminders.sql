-- Optional per-todo reminders (#189). `reminder_minutes` is how long before the
-- todo's local time (09:00 of its day for an untimed todo) to notify;
-- `reminder_sent` latches once that reminder was handed to the OS and is cleared
-- when the day, time or reminder changes. Attempts share the notification rate
-- window; tokens are hashed, expiring, one-use click capabilities.
ALTER TABLE calendar_todos ADD COLUMN reminder_minutes INTEGER
  CHECK(reminder_minutes IS NULL OR reminder_minutes IN (0,10,30,60,1440));
ALTER TABLE calendar_todos ADD COLUMN reminder_sent INTEGER NOT NULL DEFAULT 0
  CHECK(reminder_sent IN (0,1));
CREATE TABLE calendar_reminder_attempts(
  id INTEGER PRIMARY KEY,
  attempted_at TEXT NOT NULL
);
CREATE INDEX calendar_reminder_attempts_window ON calendar_reminder_attempts(attempted_at);
CREATE TABLE calendar_reminder_tokens(
  token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
  todo_id INTEGER NOT NULL REFERENCES calendar_todos(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX calendar_reminder_tokens_expiry ON calendar_reminder_tokens(expires_at);
PRAGMA user_version=35;
