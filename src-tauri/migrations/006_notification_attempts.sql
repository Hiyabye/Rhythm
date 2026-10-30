-- Keep one timestamp per actual notification handoff intent. This ledger is
-- independent of bounded event history and survives history clearing/pruning.
CREATE TABLE proactive_delivery_attempts (
 document_id INTEGER NOT NULL,
 revision_id INTEGER NOT NULL,
 event_type TEXT NOT NULL CHECK(event_type IN ('new','revision')),
 attempt_number INTEGER NOT NULL CHECK(attempt_number BETWEEN 1 AND 3),
 attempted_at TEXT NOT NULL,
 PRIMARY KEY(document_id,revision_id,event_type,attempt_number)
) WITHOUT ROWID;
CREATE INDEX proactive_delivery_attempts_window ON proactive_delivery_attempts(attempted_at);
-- Schema 5 retained only each event's latest attempt timestamp. Reconstruct all
-- known attempts at that timestamp to avoid undercounting during the window;
-- historical spacing cannot be recovered, so this is intentionally conservative.
WITH RECURSIVE attempt_numbers(number) AS (
 SELECT 1
 UNION ALL SELECT number+1 FROM attempt_numbers WHERE number<3
)
INSERT INTO proactive_delivery_attempts(document_id,revision_id,event_type,attempt_number,attempted_at)
SELECT e.document_id,e.revision_id,e.event_type,n.number,e.last_attempt_at
FROM proactive_events e
JOIN attempt_numbers n ON n.number<=e.attempt_count
WHERE e.last_attempt_at IS NOT NULL;
-- Use a persisted nondecreasing UTC watermark for window aging. If the system
-- clock moves backwards, time pauses at the last observed maximum rather than
-- making pruned attempts relevant again after a restart.
CREATE TABLE proactive_delivery_clock (
 id INTEGER PRIMARY KEY CHECK(id=1),
 max_observed_at TEXT NOT NULL
);
INSERT INTO proactive_delivery_clock(id,max_observed_at)
SELECT 1,MAX(strftime('%Y-%m-%dT%H:%M:%fZ','now'),
              COALESCE((SELECT MAX(attempted_at) FROM proactive_delivery_attempts),
                       strftime('%Y-%m-%dT%H:%M:%fZ','now')));
PRAGMA user_version=6;
