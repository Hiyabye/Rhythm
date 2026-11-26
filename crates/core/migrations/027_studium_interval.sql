-- Automatic checks remain session-only. Persist only the chosen interval.
ALTER TABLE studium_preferences
ADD COLUMN interval_minutes INTEGER NOT NULL DEFAULT 5
CHECK(interval_minutes BETWEEN 2 AND 60);
PRAGMA user_version=27;
