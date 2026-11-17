-- Crawl state of one public board. Counters describe the current attempt;
-- next_page, history_complete and seed_complete are cumulative checkpoints.
CREATE TABLE boards(
  source_id TEXT PRIMARY KEY,
  status TEXT NOT NULL DEFAULT 'idle'
    CHECK(status IN ('idle','running','completed','failed','cancelled','limited')),
  next_page INTEGER NOT NULL DEFAULT 1 CHECK(next_page >= 1),
  history_complete INTEGER NOT NULL DEFAULT 0 CHECK(history_complete IN (0,1)),
  seed_complete INTEGER NOT NULL DEFAULT 0 CHECK(seed_complete IN (0,1)),
  discovered INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  failed INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  last_attempt_at TEXT,
  last_success_at TEXT,
  -- Board health for the manifest (#166 D19); reset by a successful run.
  consecutive_failures INTEGER NOT NULL DEFAULT 0
);

-- The current revision of each notice: metadata, plain body text and resource
-- references only. Originals and HTML are never kept (#166 O19).
CREATE TABLE notices(
  id INTEGER PRIMARY KEY,
  source_id TEXT NOT NULL REFERENCES boards(source_id),
  external_id TEXT NOT NULL,
  canonical_url TEXT NOT NULL,
  title TEXT NOT NULL,
  author TEXT,
  category TEXT,
  published_at TEXT NOT NULL,
  -- The site's own change time (sitemap lastmod or article metadata).
  modified_at TEXT,
  body_state TEXT NOT NULL CHECK(body_state IN ('normal','unsupported')),
  body_text TEXT NOT NULL,
  -- JSON array of {url, filename, kind}; references only, never downloaded here.
  resources TEXT NOT NULL,
  revision_hash TEXT NOT NULL,
  etag TEXT,
  last_modified TEXT,
  -- Found by the initial history import rather than as a new notice.
  historical INTEGER NOT NULL CHECK(historical IN (0,1)),
  first_seen_at TEXT NOT NULL,
  revised_at TEXT NOT NULL,
  checked_at TEXT NOT NULL,
  UNIQUE(source_id, external_id)
);

-- Notices whose fetched response cannot be archived (it does not parse or uses
-- an unsupported encoding). They do not hold the history checkpoint.
CREATE TABLE skipped(
  source_id TEXT NOT NULL REFERENCES boards(source_id),
  external_id TEXT NOT NULL,
  url TEXT NOT NULL,
  historical INTEGER NOT NULL CHECK(historical IN (0,1)),
  reason TEXT NOT NULL,
  response_sha256 TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  PRIMARY KEY(source_id, external_id)
);

PRAGMA user_version=1;
