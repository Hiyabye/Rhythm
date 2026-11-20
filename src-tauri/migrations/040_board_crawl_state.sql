-- The app no longer crawls public boards (#178); they arrive through the feed.
-- Clears what only that crawler kept for them: the result of its last attempt,
-- skipped notices and scheduler tasks. Cybercampus, which the app still
-- collects, keeps all of it. Nothing reads a public board's checkpoints
-- (`next_page`, `history_complete`, `seed_complete`) any more, so they stay;
-- `last_success`/`previous_success` stay because the "last checked" time and
-- the new-since-sync count still read them.
UPDATE sources SET status='idle',last_attempt=NULL,last_error=NULL,discovered=0,archived=0,failed=0
WHERE id<>'cybercampus';
DELETE FROM skipped_articles WHERE source_id<>'cybercampus';
DELETE FROM proactive_tasks WHERE task_id<>'source:cybercampus';
PRAGMA user_version=40;
