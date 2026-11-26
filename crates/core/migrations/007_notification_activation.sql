-- Tokens are random bearer capabilities. Only their SHA-256 digests are kept.
-- Tokens are short-lived and bound to the exact event/attempt batch.
CREATE TABLE notification_activation_tokens (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 attempts_json TEXT NOT NULL,
 expires_at TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX notification_activation_expiry ON notification_activation_tokens(expires_at);
PRAGMA user_version=7;
