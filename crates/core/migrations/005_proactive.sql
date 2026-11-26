-- v0.5.0 proactive operation. Every behavior is opt-in: autostart, hidden
-- startup, background collection and notifications default to off. Existing
-- archive rows and historical backfill sit outside the notification baseline,
-- so an upgrade never replays old notices. Only identities are stored here;
-- notice content stays in documents/document_revisions.
CREATE TABLE proactive_settings (
 id INTEGER PRIMARY KEY CHECK(id=1),
 close_to_background INTEGER NOT NULL DEFAULT 0 CHECK(close_to_background IN (0,1)),
 launch_at_login INTEGER NOT NULL DEFAULT 0 CHECK(launch_at_login IN (0,1)),
 start_hidden INTEGER NOT NULL DEFAULT 0 CHECK(start_hidden IN (0,1)),
 scheduler_enabled INTEGER NOT NULL DEFAULT 0 CHECK(scheduler_enabled IN (0,1)),
 scheduler_paused INTEGER NOT NULL DEFAULT 0 CHECK(scheduler_paused IN (0,1)),
 scheduler_interval_minutes INTEGER NOT NULL DEFAULT 0 CHECK(scheduler_interval_minutes=0 OR scheduler_interval_minutes BETWEEN 15 AND 720),
 scheduler_retry_minutes INTEGER NOT NULL DEFAULT 15 CHECK(scheduler_retry_minutes BETWEEN 5 AND 120),
 notifications_enabled INTEGER NOT NULL DEFAULT 0 CHECK(notifications_enabled IN (0,1)),
 notification_sensitivity TEXT NOT NULL DEFAULT 'important' CHECK(notification_sensitivity IN ('important','interested','all')),
 quiet_hours_enabled INTEGER NOT NULL DEFAULT 0 CHECK(quiet_hours_enabled IN (0,1)),
 -- Local wall-clock minutes since midnight. The scheduler interprets these in
 -- the user's local timezone; all other timestamps are UTC.
 quiet_hours_start_minutes INTEGER NOT NULL DEFAULT 1320 CHECK(quiet_hours_start_minutes BETWEEN 0 AND 1439),
 quiet_hours_end_minutes INTEGER NOT NULL DEFAULT 420 CHECK(quiet_hours_end_minutes BETWEEN 0 AND 1439),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT INTO proactive_settings(id) VALUES(1);
CREATE TABLE proactive_scheduler (
 id INTEGER PRIMARY KEY CHECK(id=1),
 last_run_at TEXT,
 last_outcome TEXT CHECK(last_outcome IS NULL OR last_outcome IN ('success','partial','skipped','failure','cancelled')),
 last_error TEXT,
 consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK(consecutive_failures>=0),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT INTO proactive_scheduler(id) VALUES(1);
CREATE TABLE proactive_tasks (
 task_id TEXT PRIMARY KEY NOT NULL,
 interval_minutes INTEGER NOT NULL CHECK(interval_minutes BETWEEN 5 AND 1440),
 enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
 next_run_at TEXT,
 last_run_at TEXT,
 last_outcome TEXT CHECK(last_outcome IS NULL OR last_outcome IN ('success','failure','skipped','cancelled')),
 last_error TEXT,
 consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK(consecutive_failures>=0),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX proactive_tasks_due ON proactive_tasks(next_run_at,task_id) WHERE enabled=1 AND next_run_at IS NOT NULL;
-- Ceiling of already archived revisions at migration time. A revision at or
-- below it never produces a notification, even if a source is re-fetched.
CREATE TABLE proactive_baseline (
 id INTEGER PRIMARY KEY CHECK(id=1),
 baseline_revision_id INTEGER NOT NULL DEFAULT 0 CHECK(baseline_revision_id>=0),
 initialized_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT INTO proactive_baseline(id,baseline_revision_id)
SELECT 1,COALESCE((SELECT MAX(id) FROM document_revisions),0);
-- Durable dedupe ledger. Independent of history retention and of
-- proactive_events pruning, so clearing history never re-notifies a document.
CREATE TABLE proactive_dedupe (
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 event_type TEXT NOT NULL CHECK(event_type IN ('new','revision')),
 detected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(document_id,revision_id,event_type),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TABLE proactive_events (
 id INTEGER PRIMARY KEY,
 document_id INTEGER NOT NULL,
 revision_id INTEGER NOT NULL,
 event_type TEXT NOT NULL CHECK(event_type IN ('new','revision')),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','claimed','attempted','deferred','delivered','suppressed','failed')),
 attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count>=0),
 claimed_at TEXT,
 last_attempt_at TEXT,
 deferred_until TEXT,
 delivered_at TEXT,
 error TEXT,
 reason TEXT,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 UNIQUE(document_id,revision_id,event_type),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
CREATE INDEX proactive_events_outstanding ON proactive_events(state,deferred_until,claimed_at,id);
CREATE INDEX proactive_events_terminal ON proactive_events(state,updated_at DESC,id DESC);
-- Durable seed-bootstrap frontier for the incremental collector. A legacy
-- interrupted seed may have left historical rows with history_complete=0, so
-- only a completed historical import or a non-historical frontier counts.
ALTER TABLE sources ADD COLUMN seed_complete INTEGER NOT NULL DEFAULT 0 CHECK(seed_complete IN (0,1));
UPDATE sources SET seed_complete=1
WHERE history_complete=1
   OR EXISTS (SELECT 1 FROM documents d WHERE d.source_id=sources.id AND d.historical=0);
PRAGMA user_version=5;
