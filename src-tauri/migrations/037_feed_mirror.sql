-- The app mirrors the signed notice feed (docs/FEED.md, #177) into the v1
-- archive tables instead of crawling public boards. A feed notice is a
-- document whose `revision_hash` is the feed's; until the user opens it,
-- `body_text` holds the feed's excerpt (`body_pending`), so search, ranking
-- and previews cover it unchanged. `from_feed` marks notices the feed has
-- described: only those leave when a snapshot no longer lists them, so a v1
-- notice the feed lacks stays (user decision, 2026-11-18). A saved notice the
-- source deleted keeps its row with `removed_at`.
ALTER TABLE documents ADD COLUMN revision_hash TEXT;
ALTER TABLE documents ADD COLUMN body_state TEXT CHECK(body_state IS NULL OR body_state IN ('normal','unsupported'));
ALTER TABLE documents ADD COLUMN body_pending INTEGER NOT NULL DEFAULT 0 CHECK(body_pending IN (0,1));
ALTER TABLE documents ADD COLUMN from_feed INTEGER NOT NULL DEFAULT 0 CHECK(from_feed IN (0,1));
ALTER TABLE documents ADD COLUMN removed_at TEXT;
ALTER TABLE document_revisions ADD COLUMN revision_hash TEXT;
-- The last applied seq and the `generated_at` of the last accepted manifest.
CREATE TABLE feed_state (
 id INTEGER PRIMARY KEY CHECK(id=1),
 cursor INTEGER CHECK(cursor IS NULL OR cursor>=0),
 generated_at TEXT
);
INSERT INTO feed_state(id) VALUES(1);
-- The feed's vector and category of each notice's current revision, separate
-- from the local model's tables (which keep Cybercampus notices).
CREATE TABLE feed_embeddings (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 scope TEXT NOT NULL,
 vector BLOB NOT NULL CHECK(length(vector)=1028),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
CREATE TABLE feed_classifications (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 scope TEXT NOT NULL,
 category TEXT NOT NULL CHECK(category IN ('학사','수업','장학','연구','학과 소식','공모전·대회','인턴·채용','교육·프로그램','행사·활동','생활·시설','기타')),
 score REAL NOT NULL CHECK(score>=-1.0 AND score<=1.0),
 margin REAL NOT NULL CHECK(margin>=-2.0 AND margin<=2.0),
 low_confidence INTEGER NOT NULL CHECK(low_confidence IN (0,1)),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
PRAGMA user_version=37;
