CREATE TABLE personalization_profile (
 id INTEGER PRIMARY KEY CHECK(id=1),
 university TEXT NOT NULL DEFAULT '',
 department TEXT NOT NULL DEFAULT '',
 study_level TEXT NOT NULL DEFAULT '',
 learning_enabled INTEGER NOT NULL DEFAULT 1 CHECK(learning_enabled IN (0,1))
);
INSERT INTO personalization_profile(id) VALUES(1);
CREATE TABLE personalization_interests (
 id INTEGER PRIMARY KEY,
 name TEXT NOT NULL,
 normalized TEXT NOT NULL UNIQUE,
 aliases_json TEXT NOT NULL DEFAULT '[]',
 priority INTEGER NOT NULL CHECK(priority BETWEEN 1 AND 5),
 polarity TEXT NOT NULL DEFAULT 'positive' CHECK(polarity IN ('positive','negative')),
 origin TEXT NOT NULL CHECK(origin IN ('explicit','learned')),
 evidence_count INTEGER NOT NULL DEFAULT 0 CHECK(evidence_count>=0),
 evidence_summary TEXT,
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE personalization_feedback (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 value TEXT NOT NULL CHECK(value IN ('interested','not_interested')),
 updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE personalization_activity (
 id INTEGER PRIMARY KEY CHECK(id=1),
 sequence INTEGER NOT NULL DEFAULT 0 CHECK(sequence>=0),
 recomputed_sequence INTEGER NOT NULL DEFAULT 0 CHECK(recomputed_sequence>=0),
 last_recomputed_at TEXT
);
INSERT INTO personalization_activity(id,sequence) VALUES(1,0);
CREATE TABLE personalization_evidence (
 topic TEXT NOT NULL,
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('saved','feedback')),
 activity_sequence INTEGER NOT NULL CHECK(activity_sequence>0),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(topic,document_id,kind)
);
CREATE TABLE personalization_suppressed (
 topic TEXT PRIMARY KEY,
 baseline_sequence INTEGER NOT NULL DEFAULT 0 CHECK(baseline_sequence>=0)
);
CREATE TABLE source_extensions (
 source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
 extension_id TEXT NOT NULL
);
INSERT INTO source_extensions(source_id,extension_id) SELECT id,'sogang_notices' FROM sources;
ALTER TABLE documents ADD COLUMN last_revision_at TEXT;
UPDATE documents
SET last_revision_at=(SELECT MAX(r.collected_at) FROM document_revisions r WHERE r.document_id=documents.id)
WHERE (SELECT COUNT(*) FROM document_revisions r WHERE r.document_id=documents.id)>1;
CREATE INDEX documents_updated_recent ON documents(last_revision_at DESC,id DESC) WHERE last_revision_at IS NOT NULL;
CREATE INDEX personalization_evidence_document ON personalization_evidence(document_id);
CREATE INDEX personalization_evidence_recent ON personalization_evidence(created_at,topic,document_id);
CREATE INDEX personalization_evidence_topic_sequence ON personalization_evidence(topic,activity_sequence);
CREATE INDEX documents_category_date ON documents(category COLLATE NOCASE,published_at DESC,id DESC) WHERE category IS NOT NULL;
CREATE INDEX documents_collection_current ON documents(collected_at DESC,id DESC) WHERE historical=0;
CREATE INDEX document_state_saved_recent ON document_state(saved_at DESC,document_id DESC) WHERE saved=1;
CREATE INDEX personalization_feedback_recent ON personalization_feedback(updated_at DESC,document_id DESC);
-- Preserve legacy settings.interests verbatim; import the initial value once.
WITH RECURSIVE split_interests(rest,name,ordinal) AS (
 SELECT replace(replace(replace(replace(interests,'·','|'),',','|'),';','|'),char(10),'|') || '|','',0
 FROM settings WHERE id=1
 UNION ALL
 SELECT substr(rest,instr(rest,'|')+1),
        trim(substr(rest,1,instr(rest,'|')-1),char(9)||char(13)||' '),
        ordinal+1
 FROM split_interests WHERE instr(rest,'|')>0
), raw_interests(name,normalized,ordinal) AS (
 SELECT trim(name),lower(trim(name)),ordinal FROM split_interests WHERE trim(name)<>''
), collapsed(name,normalized,ordinal) AS (
 SELECT name,normalized,ordinal FROM raw_interests
 UNION ALL
 SELECT replace(replace(replace(name,char(9),' '),char(13),' '),'  ',' '),
        replace(replace(replace(normalized,char(9),' '),char(13),' '),'  ',' '),
        ordinal
 FROM collapsed
 WHERE instr(name,'  ')>0 OR instr(normalized,'  ')>0
    OR instr(name,char(9))>0 OR instr(normalized,char(9))>0
    OR instr(name,char(13))>0 OR instr(normalized,char(13))>0
), canonical_interests(name,normalized,aliases_json,ordinal) AS (
 SELECT name,
  CASE
   WHEN normalized IN ('ai','인공지능') THEN 'ai'
   WHEN normalized IN ('장학','장학금','scholarship','scholarships') THEN 'scholarship'
   WHEN normalized IN ('인턴','인턴십','internship','internships') THEN 'internship'
   WHEN normalized IN ('연구','연구기회','연구 기회','research','research opportunity','research opportunities') THEN 'research'
   WHEN normalized IN ('분산 시스템','분산 컴퓨팅','분산컴퓨팅','distributed systems','distributed computing') THEN 'distributed systems'
   WHEN normalized IN ('시스템 프로그래밍','시스템프로그래밍','systems programming','system programming') THEN 'systems programming'
   WHEN normalized IN ('퀀트','퀀트 금융','퀀트금융','quant finance','quantitative finance') THEN 'quantitative finance'
   WHEN normalized IN ('인공지능 인프라','ai 인프라','ai infra','ai infrastructure') THEN 'ai infrastructure'
   WHEN normalized IN ('공모전','competition','competitions') THEN 'competition'
   ELSE normalized
  END,
  CASE
   WHEN normalized IN ('ai','인공지능') THEN '["AI","인공지능"]'
   WHEN normalized IN ('장학','장학금','scholarship','scholarships') THEN '["장학","장학금","scholarship","scholarships"]'
   WHEN normalized IN ('인턴','인턴십','internship','internships') THEN '["인턴","인턴십","internship","internships"]'
   WHEN normalized IN ('연구','연구기회','연구 기회','research','research opportunity','research opportunities') THEN '["연구","연구기회","연구 기회","research","research opportunity","research opportunities"]'
   WHEN normalized IN ('분산 시스템','분산 컴퓨팅','분산컴퓨팅','distributed systems','distributed computing') THEN '["분산 시스템","분산 컴퓨팅","분산컴퓨팅","distributed systems","distributed computing"]'
   WHEN normalized IN ('시스템 프로그래밍','시스템프로그래밍','systems programming','system programming') THEN '["시스템 프로그래밍","시스템프로그래밍","systems programming","system programming"]'
   WHEN normalized IN ('퀀트','퀀트 금융','퀀트금융','quant finance','quantitative finance') THEN '["퀀트","퀀트 금융","퀀트금융","quant finance","quantitative finance"]'
   WHEN normalized IN ('인공지능 인프라','ai 인프라','ai infra','ai infrastructure') THEN '["인공지능 인프라","AI 인프라","ai 인프라","AI infra","AI infrastructure"]'
   WHEN normalized IN ('공모전','competition','competitions') THEN '["공모전","competition","competitions"]'
   ELSE '[]'
  END,
 ordinal
 FROM collapsed
 WHERE instr(name,'  ')=0 AND instr(normalized,'  ')=0
   AND instr(name,char(9))=0 AND instr(normalized,char(9))=0
   AND instr(name,char(13))=0 AND instr(normalized,char(13))=0
)
INSERT INTO personalization_interests(name,normalized,aliases_json,priority,origin)
SELECT c.name,c.normalized,c.aliases_json,3,'explicit'
FROM canonical_interests c
WHERE c.ordinal=(SELECT MIN(previous.ordinal) FROM canonical_interests previous WHERE previous.normalized=c.normalized)
ON CONFLICT(normalized) DO NOTHING;
PRAGMA user_version=4;
