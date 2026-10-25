CREATE TABLE ai_models (
 id TEXT PRIMARY KEY NOT NULL,
 catalog_id TEXT,
 name TEXT NOT NULL,
 ownership TEXT NOT NULL CHECK(ownership IN ('managed','external')),
 path TEXT NOT NULL,
 sha256 TEXT,
 expected_sha256 TEXT,
 expected_bytes INTEGER CHECK(expected_bytes IS NULL OR expected_bytes>=0),
 bytes INTEGER NOT NULL DEFAULT 0 CHECK(bytes>=0),
 source_revision TEXT,
 source_filename TEXT,
 status TEXT NOT NULL CHECK(status IN ('pending','downloading','verifying','installed','failed','cancelled','missing')),
 downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK(downloaded_bytes>=0),
 resume_etag TEXT,
 error TEXT,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE UNIQUE INDEX ai_models_catalog ON ai_models(catalog_id) WHERE catalog_id IS NOT NULL;
CREATE UNIQUE INDEX ai_models_external_path ON ai_models(path) WHERE ownership='external';
CREATE TABLE ai_preferences (
 id INTEGER PRIMARY KEY CHECK(id=1),
 active_model_id TEXT REFERENCES ai_models(id) ON DELETE SET NULL,
 setup_completed INTEGER NOT NULL DEFAULT 0 CHECK(setup_completed IN (0,1)),
 processing_mode TEXT NOT NULL DEFAULT 'explicit' CHECK(processing_mode IN ('explicit','recent')),
 automatic_limit INTEGER NOT NULL DEFAULT 3 CHECK(automatic_limit BETWEEN 0 AND 20),
 max_tokens INTEGER NOT NULL DEFAULT 1024 CHECK(max_tokens BETWEEN 128 AND 4096)
);
INSERT INTO ai_preferences(id) VALUES(1);
CREATE TABLE ai_jobs (
 id INTEGER PRIMARY KEY,
 document_id INTEGER NOT NULL,
 revision_id INTEGER NOT NULL,
 task TEXT NOT NULL,
 model_sha256 TEXT NOT NULL,
 model_identity_json TEXT NOT NULL,
 prompt_version INTEGER NOT NULL CHECK(prompt_version>0),
 schema_version INTEGER NOT NULL CHECK(schema_version>0),
 configuration_hash TEXT NOT NULL,
 priority INTEGER NOT NULL DEFAULT 0,
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','completed','failed','cancelled','stale')),
 error TEXT,
 duration_ms INTEGER CHECK(duration_ms IS NULL OR duration_ms>=0),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX ai_jobs_active ON ai_jobs(document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash) WHERE status IN ('pending','running');
CREATE INDEX ai_jobs_queue ON ai_jobs(status,priority DESC,id);
CREATE TABLE ai_results (
 id INTEGER PRIMARY KEY,
 job_id INTEGER NOT NULL UNIQUE REFERENCES ai_jobs(id) ON DELETE CASCADE,
 document_id INTEGER NOT NULL,
 revision_id INTEGER NOT NULL,
 task TEXT NOT NULL,
 model_sha256 TEXT NOT NULL,
 model_identity_json TEXT NOT NULL,
 prompt_version INTEGER NOT NULL,
 schema_version INTEGER NOT NULL,
 configuration_hash TEXT NOT NULL,
 result_json TEXT NOT NULL,
 partial INTEGER NOT NULL CHECK(partial IN (0,1)),
 processed_chars INTEGER NOT NULL CHECK(processed_chars>=0),
 total_chars INTEGER NOT NULL CHECK(total_chars>=processed_chars),
 duration_ms INTEGER NOT NULL CHECK(duration_ms>=0),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
CREATE INDEX ai_results_cache ON ai_results(document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,id DESC);
CREATE INDEX ai_results_recent ON ai_results(id DESC);
PRAGMA user_version=3;
