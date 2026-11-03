-- Automatic update checks (user decision: on by default). The timestamp is the
-- last successful check, so a failed or offline check retries at the next wake.
ALTER TABLE settings ADD COLUMN update_check INTEGER NOT NULL DEFAULT 1 CHECK(update_check IN (0,1));
ALTER TABLE settings ADD COLUMN update_checked_at TEXT;
PRAGMA user_version=10;
