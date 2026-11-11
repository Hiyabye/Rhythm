-- Only user rules, a rearm latch and bounded delivery capabilities/accounting.
-- No counts, availability, source times, response HTML or seat history.
CREATE TABLE studium_alert_rules(
  room_id TEXT PRIMARY KEY CHECK(room_id IN ('19','5','6','11','15','16','18')),
  enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
  kind TEXT NOT NULL DEFAULT 'remaining' CHECK(kind IN ('remaining','preferred')),
  minimum INTEGER NOT NULL DEFAULT 1 CHECK(typeof(minimum)='integer' AND minimum BETWEEN 1 AND CASE room_id
    WHEN '19' THEN 113 WHEN '5' THEN 102 WHEN '6' THEN 311 WHEN '11' THEN 101
    WHEN '15' THEN 207 WHEN '16' THEN 92 WHEN '18' THEN 174 END),
  version INTEGER NOT NULL DEFAULT 1 CHECK(typeof(version)='integer' AND version>=1),
  armed INTEGER NOT NULL DEFAULT 1 CHECK(armed IN (0,1)),
  CHECK(kind='remaining' OR minimum=1)
);
INSERT INTO studium_alert_rules(room_id) VALUES('19'),('5'),('6'),('11'),('15'),('16'),('18');
CREATE TABLE studium_alert_attempts(
  id INTEGER PRIMARY KEY,
  attempted_at TEXT NOT NULL
);
CREATE INDEX studium_alert_attempts_window ON studium_alert_attempts(attempted_at);
CREATE TABLE studium_alert_tokens(
  token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
  room_id TEXT NOT NULL REFERENCES studium_alert_rules(room_id),
  rule_version INTEGER NOT NULL CHECK(typeof(rule_version)='integer' AND rule_version>=1),
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX studium_alert_tokens_expiry ON studium_alert_tokens(expires_at);
PRAGMA user_version=29;
