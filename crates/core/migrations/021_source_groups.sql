-- Boards are listed under nested settings groups (서강대학교 > 공과대학 >
-- 기계공학과). `selected` is the user's own pick; `subscribed` becomes the
-- effective subscription: selected, supported and every enclosing group on.
-- The catalog seeds group names and membership on every open; a group the
-- catalog no longer names is never deleted here. Upgraded profiles keep every
-- current subscription because every group starts on.
CREATE TABLE source_groups (
 id TEXT PRIMARY KEY,
 name TEXT NOT NULL,
 parent_id TEXT REFERENCES source_groups(id),
 position INTEGER NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1))
);
ALTER TABLE sources ADD COLUMN selected INTEGER NOT NULL DEFAULT 0 CHECK(selected IN (0,1));
ALTER TABLE sources ADD COLUMN group_id TEXT;
UPDATE sources SET selected=subscribed;
PRAGMA user_version=21;
