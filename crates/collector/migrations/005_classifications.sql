-- Server categories for the feed (#166 O9): the category prototypes embedded
-- through Workers AI once per vector scope, in `rhythm_embed::prototype_inputs`
-- order, and each notice's nearest-prototype result for the vector it was made
-- from. A notice is classified again when its vector changes.
CREATE TABLE prototypes(
  scope TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  vector BLOB NOT NULL CHECK(length(vector) = 2048),
  PRIMARY KEY(scope, ordinal)
) WITHOUT ROWID;
CREATE TABLE classifications(
  notice_id INTEGER PRIMARY KEY REFERENCES notices(id) ON DELETE CASCADE,
  revision_hash TEXT NOT NULL,
  scope TEXT NOT NULL,
  category TEXT NOT NULL CHECK(category IN ('학사','수업','장학','연구','학과 소식',
    '공모전·대회','인턴·채용','교육·프로그램','행사·활동','생활·시설','기타')),
  score REAL NOT NULL CHECK(score >= -1.0 AND score <= 1.0),
  margin REAL NOT NULL CHECK(margin >= -2.0 AND margin <= 2.0),
  low_confidence INTEGER NOT NULL CHECK(low_confidence IN (0,1)),
  classified_at TEXT NOT NULL
);

PRAGMA user_version=5;
