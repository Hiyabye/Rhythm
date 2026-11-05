-- Optional idle backlog analysis (`processing_mode='backlog'`). SQLite cannot
-- widen a CHECK constraint in place, so the single-row preferences table is
-- rebuilt with the same columns and constraints.
CREATE TABLE ai_preferences_014 (
 id INTEGER PRIMARY KEY CHECK(id=1),
 active_model_id TEXT REFERENCES ai_models(id) ON DELETE SET NULL,
 setup_completed INTEGER NOT NULL DEFAULT 0 CHECK(setup_completed IN (0,1)),
 processing_mode TEXT NOT NULL DEFAULT 'explicit' CHECK(processing_mode IN ('explicit','recent','backlog')),
 automatic_limit INTEGER NOT NULL DEFAULT 3 CHECK(automatic_limit BETWEEN 0 AND 20),
 max_tokens INTEGER NOT NULL DEFAULT 1024 CHECK(max_tokens BETWEEN 128 AND 4096),
 execution_mode TEXT NOT NULL DEFAULT 'auto' CHECK(execution_mode IN ('auto','cpu','gpu')),
 model_directory TEXT
);
INSERT INTO ai_preferences_014(id,active_model_id,setup_completed,processing_mode,automatic_limit,max_tokens,execution_mode,model_directory)
 SELECT id,active_model_id,setup_completed,processing_mode,automatic_limit,max_tokens,execution_mode,model_directory FROM ai_preferences;
DROP TABLE ai_preferences;
ALTER TABLE ai_preferences_014 RENAME TO ai_preferences;
-- Candidate queries test "any attempt for this revision"; index that lookup.
CREATE INDEX IF NOT EXISTS ai_jobs_revision ON ai_jobs(document_id,revision_id,status);
PRAGMA user_version=14;
