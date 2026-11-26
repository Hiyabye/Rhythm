-- The public boards the feed mirror knew when it last synced (#80). Earlier
-- syncs skipped the records of a board the catalog gains later, so the next
-- sync backfills that board from the snapshot. An upgraded profile knew the
-- boards of its previous catalog: the catalog is refreshed after migrations.
CREATE TABLE feed_known_boards(source_id TEXT PRIMARY KEY) WITHOUT ROWID;
INSERT INTO feed_known_boards(source_id)
  SELECT id FROM sources WHERE supported=1 AND family<>'cybercampus';
PRAGMA user_version=42;
