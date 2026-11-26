-- The preference only affects new downloads. Each managed model retains the
-- directory it was created in so interrupted downloads resume in that location.
ALTER TABLE ai_preferences ADD COLUMN model_directory TEXT;
ALTER TABLE ai_models ADD COLUMN storage_directory TEXT;
PRAGMA user_version=9;
