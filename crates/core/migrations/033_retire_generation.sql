-- AI notice summaries and the OpenAI-compatible remote engine are retired
-- (#226). Their jobs, results and remote-attempt ledger are deleted; restoring
-- an older backup runs this again. Installed generation models stay in
-- ai_models until the user removes them, so their bytes are never deleted here.
-- Every profile at schema 32 has these tables; IF EXISTS also lets upgrade
-- tests that roll a current profile back to an older version run this again.
DROP TABLE IF EXISTS ai_results;
DROP TABLE IF EXISTS ai_jobs;
DROP TABLE IF EXISTS ai_remote_usage;
-- Keep only the embedding choices. SQLite cannot drop constrained columns in
-- place, so the single-row table is rebuilt as 014 did.
CREATE TABLE ai_preferences_033 (
 id INTEGER PRIMARY KEY CHECK(id=1),
 execution_mode TEXT NOT NULL DEFAULT 'auto' CHECK(execution_mode IN ('auto','cpu','gpu')),
 model_directory TEXT,
 embedding_model_id TEXT REFERENCES ai_models(id) ON DELETE SET NULL,
 classification_enabled INTEGER NOT NULL DEFAULT 1 CHECK(classification_enabled IN (0,1))
);
INSERT INTO ai_preferences_033(id,execution_mode,model_directory,embedding_model_id,classification_enabled)
 SELECT id,execution_mode,model_directory,embedding_model_id,classification_enabled FROM ai_preferences;
DROP TABLE ai_preferences;
ALTER TABLE ai_preferences_033 RENAME TO ai_preferences;
-- Removing a model deletes its vectors, so a vector under a hash no model row
-- carries came from the remote embedding endpoint (its derived identity).
DELETE FROM notice_classifications WHERE model_sha256 NOT IN (SELECT sha256 FROM ai_models WHERE sha256 IS NOT NULL);
DELETE FROM notice_embeddings WHERE model_sha256 NOT IN (SELECT sha256 FROM ai_models WHERE sha256 IS NOT NULL);
PRAGMA user_version=33;
