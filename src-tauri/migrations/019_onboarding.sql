-- First-run setup flow. Existing profiles already chose their settings, so the
-- default marks them done; `migrate` clears it for a profile created from scratch.
ALTER TABLE settings ADD COLUMN onboarding_completed INTEGER NOT NULL DEFAULT 1 CHECK(onboarding_completed IN (0,1));
PRAGMA user_version=19;
