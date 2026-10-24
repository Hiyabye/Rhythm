CREATE TABLE sources (
 id TEXT PRIMARY KEY NOT NULL,
 name TEXT NOT NULL,
 organization TEXT NOT NULL,
 description TEXT NOT NULL,
 url TEXT NOT NULL,
 family TEXT NOT NULL,
 supported INTEGER NOT NULL CHECK(supported IN (0,1)),
 subscribed INTEGER NOT NULL DEFAULT 0 CHECK(subscribed IN (0,1)),
 last_attempt TEXT,
 last_success TEXT,
 previous_success TEXT,
 last_error TEXT,
 next_page INTEGER NOT NULL DEFAULT 1 CHECK(next_page>=1),
 history_complete INTEGER NOT NULL DEFAULT 0 CHECK(history_complete IN (0,1)),
 status TEXT NOT NULL DEFAULT 'idle',
 discovered INTEGER NOT NULL DEFAULT 0 CHECK(discovered>=0),
 archived INTEGER NOT NULL DEFAULT 0 CHECK(archived>=0),
 failed INTEGER NOT NULL DEFAULT 0 CHECK(failed>=0)
);
CREATE TABLE documents (
 id INTEGER PRIMARY KEY,
 source_id TEXT NOT NULL REFERENCES sources(id),
 external_id TEXT NOT NULL,
 canonical_url TEXT NOT NULL,
 title TEXT NOT NULL,
 author TEXT,
 category TEXT,
 published_at TEXT NOT NULL,
 modified_at TEXT,
 collected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 checked_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 body_html TEXT NOT NULL,
 body_text TEXT NOT NULL,
 content_hash TEXT NOT NULL,
 etag TEXT,
 last_modified TEXT,
 historical INTEGER NOT NULL CHECK(historical IN (0,1)),
 UNIQUE(source_id,external_id)
);
CREATE INDEX documents_source_date ON documents(source_id,published_at DESC,id DESC);
CREATE INDEX documents_date ON documents(published_at DESC,id DESC);
CREATE TABLE document_revisions (
 id INTEGER PRIMARY KEY,
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 canonical_url TEXT NOT NULL,
 title TEXT NOT NULL,
 author TEXT,
 category TEXT,
 published_at TEXT NOT NULL,
 modified_at TEXT,
 original TEXT NOT NULL,
 body_html TEXT NOT NULL,
 body_text TEXT NOT NULL,
 content_hash TEXT NOT NULL,
 collected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 UNIQUE(id,document_id)
);
CREATE INDEX revisions_document ON document_revisions(document_id,id DESC);
CREATE TABLE document_state (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
 is_read INTEGER NOT NULL DEFAULT 0 CHECK(is_read IN (0,1)),
 saved INTEGER NOT NULL DEFAULT 0 CHECK(saved IN (0,1)),
 read_at TEXT,
 saved_at TEXT
);
CREATE TABLE resources (
 id INTEGER PRIMARY KEY,
 document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
 revision_id INTEGER NOT NULL,
 url TEXT NOT NULL,
 filename TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('image','attachment')),
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','downloaded','failed','too_large','unsupported')),
 mime TEXT,
 bytes INTEGER CHECK(bytes IS NULL OR bytes>=0),
 local_path TEXT,
 sha256 TEXT,
 error TEXT,
 FOREIGN KEY(revision_id,document_id) REFERENCES document_revisions(id,document_id) ON DELETE CASCADE,
 UNIQUE(revision_id,url)
);
CREATE INDEX resources_document ON resources(document_id,revision_id);
CREATE VIRTUAL TABLE documents_fts USING fts5(title,body_text,content='documents',content_rowid='id',tokenize='trigram');
CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
 INSERT INTO documents_fts(rowid,title,body_text) VALUES(new.id,new.title,new.body_text);
END;
CREATE TRIGGER documents_ad AFTER DELETE ON documents BEGIN
 INSERT INTO documents_fts(documents_fts,rowid,title,body_text) VALUES('delete',old.id,old.title,old.body_text);
END;
CREATE TRIGGER documents_au AFTER UPDATE OF title,body_text ON documents BEGIN
 INSERT INTO documents_fts(documents_fts,rowid,title,body_text) VALUES('delete',old.id,old.title,old.body_text);
 INSERT INTO documents_fts(rowid,title,body_text) VALUES(new.id,new.title,new.body_text);
END;
PRAGMA user_version=2;
