-- The notice categories were revised: 수업·교육 splits into 수업 and
-- 교육·프로그램, and 학과 소식 and 생활·시설 are new. SQLite cannot change a
-- CHECK in place, so the table is rebuilt as 015 created it. 수업·교육 stays
-- allowed for rows under the previous config hash, which are kept as history;
-- the classifier writes the new set from stored vectors without re-embedding.
-- The rebuild also runs again when an upgrade test rolls a profile back.
CREATE TABLE notice_classifications_036 (
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 model_sha256 TEXT NOT NULL,
 config_hash TEXT NOT NULL,
 category TEXT NOT NULL CHECK(category IN ('학사','수업','장학','연구','학과 소식','공모전·대회','인턴·채용','교육·프로그램','행사·활동','생활·시설','기타','수업·교육')),
 score REAL NOT NULL CHECK(score>=-1.0 AND score<=1.0),
 margin REAL NOT NULL CHECK(margin>=-2.0 AND margin<=2.0),
 low_confidence INTEGER NOT NULL CHECK(low_confidence IN (0,1)),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(document_id,revision_id,model_sha256,config_hash),
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE
);
INSERT INTO notice_classifications_036 SELECT document_id,revision_id,model_sha256,config_hash,category,score,margin,low_confidence,created_at FROM notice_classifications;
DROP TABLE notice_classifications;
ALTER TABLE notice_classifications_036 RENAME TO notice_classifications;
CREATE INDEX notice_classifications_revision ON notice_classifications(document_id,revision_id);
PRAGMA user_version=36;
