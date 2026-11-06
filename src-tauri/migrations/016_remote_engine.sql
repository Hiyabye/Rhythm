-- Opt-in OpenAI-compatible remote engine. The local engine remains the default;
-- these columns only describe the user's choice. The API key is never stored
-- here: it lives in the OS credential store, keyed by the normalized origin.
ALTER TABLE ai_preferences ADD COLUMN engine TEXT NOT NULL DEFAULT 'local'
 CHECK(engine IN ('local','openai-compatible'));
ALTER TABLE ai_preferences ADD COLUMN remote_base_url TEXT;
ALTER TABLE ai_preferences ADD COLUMN remote_model TEXT;
ALTER TABLE ai_preferences ADD COLUMN remote_embedding_model TEXT;
ALTER TABLE ai_preferences ADD COLUMN remote_timeout_seconds INTEGER NOT NULL DEFAULT 120
 CHECK(remote_timeout_seconds BETWEEN 10 AND 600);
-- Consent is bound to the configured origin and is cleared when that origin changes.
ALTER TABLE ai_preferences ADD COLUMN remote_consent INTEGER NOT NULL DEFAULT 0
 CHECK(remote_consent IN (0,1));
-- Automatic remote work may cost money, so it is a separate opt-in with a daily cap.
ALTER TABLE ai_preferences ADD COLUMN remote_automatic INTEGER NOT NULL DEFAULT 0
 CHECK(remote_automatic IN (0,1));
ALTER TABLE ai_preferences ADD COLUMN remote_daily_limit INTEGER NOT NULL DEFAULT 20
 CHECK(remote_daily_limit BETWEEN 0 AND 200);
-- Remote classification sends notice titles and bodies to the same endpoint.
ALTER TABLE ai_preferences ADD COLUMN remote_embeddings INTEGER NOT NULL DEFAULT 0
 CHECK(remote_embeddings IN (0,1));

-- Automatic remote attempts admitted today (local calendar date). Direct user
-- requests are not counted; this is a cost guard, not a usage report.
CREATE TABLE ai_remote_usage (
 day TEXT PRIMARY KEY NOT NULL,
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0)
);
PRAGMA user_version=16;
