-- Months after publication when a notice moves to 휴지통 by itself; 0 never.
ALTER TABLE settings ADD COLUMN auto_trash_months INTEGER NOT NULL DEFAULT 0 CHECK(auto_trash_months IN (0,1,3,6,12,24));
PRAGMA user_version=26;
