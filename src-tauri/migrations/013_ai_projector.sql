-- Optional vision projector (mmproj) for an installed model. Separate row so a
-- text-only install stays usable and removal of the model removes it too.
CREATE TABLE ai_projectors (
 model_id TEXT PRIMARY KEY NOT NULL REFERENCES ai_models(id) ON DELETE CASCADE,
 path TEXT NOT NULL,
 sha256 TEXT,
 expected_sha256 TEXT NOT NULL,
 expected_bytes INTEGER NOT NULL CHECK(expected_bytes>0),
 bytes INTEGER NOT NULL DEFAULT 0 CHECK(bytes>=0),
 status TEXT NOT NULL CHECK(status IN ('pending','downloading','verifying','installed','failed','cancelled','missing')),
 downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK(downloaded_bytes>=0),
 resume_etag TEXT,
 error TEXT,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
PRAGMA user_version=13;
