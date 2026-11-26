-- Feed notifications (#177): the SHA-256 of each revision's full body text
-- (`rhythm_feed::body_hash`). A feed revision holds only an excerpt, so this
-- is how notification discovery tells a body change from a metadata-only
-- revision. NULL on revisions archived before it; discovery then compares text.
ALTER TABLE document_revisions ADD COLUMN body_hash TEXT;
PRAGMA user_version=39;
