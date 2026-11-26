-- Dedicated local embedding model and revision-scoped notice classification.
-- Existing models are generation models; persist the role so a catalog entry can
-- be retired without changing behavior for an installed copy.
ALTER TABLE ai_models ADD COLUMN kind TEXT NOT NULL DEFAULT 'generation' CHECK(kind IN ('generation','embedding'));
ALTER TABLE ai_preferences ADD COLUMN embedding_model_id TEXT REFERENCES ai_models(id) ON DELETE SET NULL;
ALTER TABLE ai_preferences ADD COLUMN classification_enabled INTEGER NOT NULL DEFAULT 1 CHECK(classification_enabled IN (0,1));

-- One unit-length vector per document revision and embedding model. The vector
-- is kept so a prototype/config change reclassifies without re-embedding.
CREATE TABLE notice_embeddings (
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 model_sha256 TEXT NOT NULL,
 dim INTEGER NOT NULL CHECK(dim>0),
 vector BLOB NOT NULL,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(document_id,revision_id,model_sha256),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);

-- Classification is derived from the vector plus the prototype/scoring config.
-- A model or config change creates a new row instead of overwriting history.
CREATE TABLE notice_classifications (
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 model_sha256 TEXT NOT NULL,
 config_hash TEXT NOT NULL,
 category TEXT NOT NULL CHECK(category IN ('학사','장학','수업·교육','연구','공모전·대회','인턴·채용','행사·활동','기타')),
 score REAL NOT NULL CHECK(score>=-1.0 AND score<=1.0),
 margin REAL NOT NULL CHECK(margin>=-2.0 AND margin<=2.0),
 low_confidence INTEGER NOT NULL CHECK(low_confidence IN (0,1)),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(document_id,revision_id,model_sha256,config_hash),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
CREATE INDEX notice_classifications_revision ON notice_classifications(document_id,revision_id);
PRAGMA user_version=15;
