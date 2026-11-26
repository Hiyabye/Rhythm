-- Dates the deterministic extractor (`archive::dates`) found in each notice's
-- current text. One extraction row per notice records which content and
-- extractor version produced its dates, so a changed body or a new extractor
-- version is re-extracted; a notice without dates still has its row.
CREATE TABLE notice_date_extractions (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 content_hash TEXT NOT NULL,
 version INTEGER NOT NULL
);
CREATE TABLE notice_dates (
 document_id INTEGER NOT NULL REFERENCES notice_date_extractions(document_id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL,
 day TEXT NOT NULL,
 time TEXT,
 label TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('deadline','start','date')),
 PRIMARY KEY(document_id, ordinal)
) WITHOUT ROWID;
CREATE INDEX notice_dates_day ON notice_dates(day, kind);
-- The calendar shows dates of saved notices; this adds every notice's deadlines.
ALTER TABLE settings ADD COLUMN calendar_all_deadlines INTEGER NOT NULL DEFAULT 0 CHECK(calendar_all_deadlines IN (0,1));
PRAGMA user_version=32;
