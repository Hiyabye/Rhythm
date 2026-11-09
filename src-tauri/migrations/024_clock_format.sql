-- How the app shows times: 12-hour (오전/오후) or 24-hour. Stored times stay HH:MM.
ALTER TABLE settings ADD COLUMN clock_format TEXT NOT NULL DEFAULT '12h' CHECK(clock_format IN ('12h','24h'));
PRAGMA user_version=24;
