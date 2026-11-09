-- Notices the user moved to 휴지통. Their content stays until the user deletes
-- them permanently; until then lists, search, recommendations, notifications
-- and automatic AI work leave them out.
CREATE TABLE trashed_documents (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 trashed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
-- Permanently deleted notices. Sync never fetches or archives one again.
CREATE TABLE deleted_notices (
 source_id TEXT NOT NULL REFERENCES sources(id),
 external_id TEXT NOT NULL,
 deleted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(source_id,external_id)
);
PRAGMA user_version=25;
