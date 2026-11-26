-- The 공과대학 e뉴스레터 identity (eng-newsletter) is retired from the catalog
-- (#93). It was never subscribable, so a profile cannot hold notices for it;
-- dropping the row keeps it out of the board list. A source that does hold
-- archived notices or skipped entries stays, so its stored data still opens.
DELETE FROM sources
WHERE id='eng-newsletter'
  AND NOT EXISTS (SELECT 1 FROM documents d WHERE d.source_id=sources.id)
  AND NOT EXISTS (SELECT 1 FROM skipped_articles k WHERE k.source_id=sources.id);
PRAGMA user_version=17;
