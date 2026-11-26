ALTER TABLE ai_preferences ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'auto'
 CHECK(execution_mode IN ('auto','cpu','gpu'));
PRAGMA user_version=8;
