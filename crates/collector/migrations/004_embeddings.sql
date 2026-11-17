-- Workers AI embeddings (#166 D25, O19b): an f16 unit vector (1,024 dims,
-- little-endian) of each notice's current revision. `scope` names the model
-- and the input contract (`rhythm_embed::config_hash`); a vector of another
-- scope or an older revision is embedded again.
CREATE TABLE embeddings(
  notice_id INTEGER PRIMARY KEY REFERENCES notices(id) ON DELETE CASCADE,
  revision_hash TEXT NOT NULL,
  scope TEXT NOT NULL,
  vector BLOB NOT NULL CHECK(length(vector) = 2048),
  embedded_at TEXT NOT NULL
);
-- Neurons Workers AI reported per UTC day (its free allowance resets at
-- 00:00 UTC) and the day's last embedding error.
CREATE TABLE ai_usage(
  day TEXT PRIMARY KEY,
  neurons REAL NOT NULL DEFAULT 0,
  last_error TEXT
);
-- The drift canary (D27): reference vectors of the committed canary texts,
-- recorded by the first check, and one check per UTC day.
CREATE TABLE canary(
  ordinal INTEGER PRIMARY KEY,
  text_sha256 TEXT NOT NULL,
  vector BLOB NOT NULL CHECK(length(vector) = 2048),
  recorded_at TEXT NOT NULL
);
CREATE TABLE canary_checks(
  day TEXT PRIMARY KEY,
  state TEXT NOT NULL CHECK(state IN ('recorded','normal','warning','stopped')),
  min_cosine REAL,
  checked_at TEXT NOT NULL
);

PRAGMA user_version=4;
