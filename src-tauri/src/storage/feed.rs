//! The feed mirror (docs/FEED.md, #177): applies one decoded records file of
//! the signed feed to the archive tables. A feed notice is a `documents` row
//! whose `revision_hash` is the feed's; until it is opened its `body_text` is
//! the feed's excerpt (`body_pending`). Each new feed revision is a revision
//! row without an original, so resources, notification events and the
//! feed's vectors attach to it as they did in v1.
//!
//! Records the app cannot use (an unknown board, Cybercampus, invalid fields)
//! are skipped and counted rather than failing the file: the file is signed,
//! so a bad record is a publisher bug, and refusing the file would stall the
//! mirror for good.
use super::{
    archive::{check_article, meaningful_hash, public_url, valid_id, web_url},
    notice_dates, proactive, Store,
};
use crate::archive::model::{ArticleInput, ListingEntry, ResourceRecord};
use rhythm_embed::CATEGORIES;
use rhythm_feed::{
    record::{BodyState, Classification, Embedding, Entry, Notice, Record, ResourceKind},
    vector, Revision, VECTOR_BYTES,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashSet;

pub(crate) mod state;
pub(crate) use state::FeedResult;
pub use state::FeedStatus;

/// Bounds a record must stay within; the feed's own limits are smaller.
const MAX_TEXT: usize = 16_384;
const MAX_RESOURCES: usize = 256;
const MAX_DATES: usize = 64;

/// What one applied file changed.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Applied {
    /// Notices inserted or given a new revision.
    pub revised: usize,
    /// Notices deleted or, if saved, marked removed.
    pub removed: usize,
    pub skipped: usize,
    /// Resource files no remaining notice uses; the caller deletes them after
    /// releasing the store lock.
    pub orphaned: Vec<String>,
}

impl Store {
    /// The last seq applied, or `None` before the first sync.
    pub(crate) fn feed_cursor(&self) -> Result<Option<u64>, String> {
        self.connection
            .query_row("SELECT cursor FROM feed_state WHERE id=1", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map(|cursor| cursor.map(|c| c as u64))
            .map_err(|e| e.to_string())
    }

    /// Applies a snapshot through `through_seq` (FEED.md "Applying the feed",
    /// step 4). Notices from the feed that it no longer lists were removed;
    /// v1 notices the feed never described stay (user decision, 2026-11-18).
    /// Its vectors and categories replace every earlier one, and nothing it
    /// carries becomes a notification candidate, and the notices it adds
    /// start read: a first sync is a backfill, not news.
    pub(crate) fn feed_apply_snapshot(
        &self,
        through_seq: u64,
        entries: &[Entry],
        vectors: &[u8],
    ) -> Result<Applied, String> {
        let vectors = chunks(vectors)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        transaction
            .execute_batch("DELETE FROM feed_embeddings; DELETE FROM feed_classifications;")
            .map_err(|e| e.to_string())?;
        let mut applied = Applied::default();
        let mut listed: HashSet<(String, String)> = HashSet::new();
        for entry in entries {
            if let Some(Record::Notice(notice)) = &entry.record {
                listed.insert((notice.source_id.clone(), notice.external_id.clone()));
            }
            apply(&transaction, entry, &vectors, true, &mut applied)?;
        }
        let held: Vec<(i64, String, String)> = transaction
            .prepare("SELECT id,source_id,external_id FROM documents WHERE from_feed=1")
            .and_then(|mut statement| {
                statement
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect()
            })
            .map_err(|e| e.to_string())?;
        let now: String = transaction
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        for (id, source, external) in held {
            if !listed.contains(&(source, external)) {
                remove(&transaction, id, &now, &mut applied)?;
            }
        }
        proactive::advance_baseline(&transaction)?;
        self.finish(transaction, through_seq, applied)
    }

    /// Applies a segment from `from_seq` to `to_seq`, skipping the seqs the
    /// cursor already covers. The cursor must lie inside it or at its start.
    /// `quiet` (a sync that began from the snapshot) keeps its notices out of
    /// notifications and unread counts too, so a first sync never notifies.
    pub(crate) fn feed_apply_segment(
        &self,
        from_seq: u64,
        to_seq: u64,
        entries: &[Entry],
        vectors: &[u8],
        quiet: bool,
    ) -> Result<Applied, String> {
        let cursor = self
            .feed_cursor()?
            .ok_or("피드를 처음 받을 때는 스냅샷이 필요합니다.")?;
        if cursor + 1 < from_seq || cursor >= to_seq {
            return Err("받은 피드 구간이 마지막으로 반영한 위치와 이어지지 않습니다.".into());
        }
        let vectors = chunks(vectors)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let mut applied = Applied::default();
        for entry in entries.iter().filter(|e| e.seq > cursor) {
            apply(&transaction, entry, &vectors, quiet, &mut applied)?;
        }
        if quiet {
            proactive::advance_baseline(&transaction)?;
        }
        self.finish(transaction, to_seq, applied)
    }

    fn finish(
        &self,
        transaction: Transaction,
        cursor: u64,
        mut applied: Applied,
    ) -> Result<Applied, String> {
        transaction
            .execute(
                "UPDATE feed_state SET cursor=?1 WHERE id=1",
                [cursor as i64],
            )
            .map_err(|e| e.to_string())?;
        applied.orphaned = unused(&transaction, std::mem::take(&mut applied.orphaned))?;
        transaction.commit().map_err(|e| e.to_string())?;
        if applied.removed > 0 {
            self.recompute_learned()?;
        }
        Ok(applied)
    }
}

/// The vectors file as whole vectors, each checked as the feed defines them.
fn chunks(bytes: &[u8]) -> Result<Vec<&[u8; VECTOR_BYTES]>, String> {
    vector::decode(bytes).map_err(|e| e.to_string())?;
    Ok(bytes.as_chunks::<VECTOR_BYTES>().0.iter().collect())
}

fn apply(
    transaction: &Connection,
    entry: &Entry,
    vectors: &[&[u8; VECTOR_BYTES]],
    quiet: bool,
    applied: &mut Applied,
) -> Result<(), String> {
    let applied_one = match &entry.record {
        // A record kind of a newer minor version.
        None => return Ok(()),
        Some(Record::Notice(notice)) => upsert(transaction, notice, quiet, applied)?,
        Some(Record::Embedding(embedding)) => embed(transaction, embedding, vectors)?,
        Some(Record::Classification(classification)) => classify(transaction, classification)?,
        Some(Record::Remove(removal)) => {
            match feed_document(transaction, &removal.source_id, &removal.external_id)? {
                Some(id) => {
                    remove(transaction, id, &removal.removed_at, applied)?;
                    true
                }
                // Unknown here, or a board the app does not mirror.
                None => known_board(transaction, &removal.source_id)?,
            }
        }
    };
    if !applied_one {
        applied.skipped += 1;
    }
    Ok(())
}

/// A public board in the app's catalog; Cybercampus never comes from the feed.
fn known_board(connection: &Connection, source_id: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE id=?1 AND supported=1 AND family<>'cybercampus')",
            [source_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
}

fn feed_document(
    connection: &Connection,
    source_id: &str,
    external_id: &str,
) -> Result<Option<i64>, String> {
    if !known_board(connection, source_id)? {
        return Ok(None);
    }
    connection
        .query_row(
            "SELECT id FROM documents WHERE source_id=?1 AND external_id=?2",
            params![source_id, external_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
}

fn valid_notice(notice: &Notice) -> bool {
    valid_id(&notice.external_id)
        && is_sha256(&notice.revision_hash)
        && !notice.title.trim().is_empty()
        && notice.title.len() <= MAX_TEXT
        && notice.author.as_ref().is_none_or(|a| a.len() <= MAX_TEXT)
        && notice.category.as_ref().is_none_or(|c| c.len() <= MAX_TEXT)
        && public_url(&notice.canonical_url)
        && !notice.published_at.trim().is_empty()
        && notice.published_at.len() <= 64
        && notice.excerpt.len() <= MAX_TEXT
        && notice.body_hash.as_deref().is_none_or(is_sha256)
        && notice.resources.len() <= MAX_RESOURCES
        && notice
            .resources
            .iter()
            .all(|r| web_url(&r.url, true) && r.filename.len() <= 4096)
        && notice.dates.len() <= MAX_DATES
        && notice
            .dates
            .iter()
            .all(|d| d.label.len() <= 512 && d.day.len() <= 32)
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn kind_name(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Attachment => "attachment",
        ResourceKind::Image => "image",
    }
}

/// Applies a `notice.upsert`; `false` when it was skipped. A notice it adds
/// starts read when it arrives `quiet` or from the collector's history
/// import: neither is news, so neither floods the unread count. Notices
/// already here keep their read state.
fn upsert(
    connection: &Connection,
    notice: &Notice,
    quiet: bool,
    applied: &mut Applied,
) -> Result<bool, String> {
    if !known_board(connection, &notice.source_id)? || !valid_notice(notice) {
        return Ok(false);
    }
    // A notice the user deleted permanently is never archived again.
    let deleted: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM deleted_notices WHERE source_id=?1 AND external_id=?2)",
            params![notice.source_id, notice.external_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if deleted {
        return Ok(false);
    }
    let body_state = match notice.body_state {
        BodyState::Normal => "normal",
        BodyState::Unsupported => "unsupported",
    };
    let existing: Option<(i64, Option<String>, bool)> = connection
        .query_row(
            "SELECT id,revision_hash,from_feed FROM documents WHERE source_id=?1 AND external_id=?2",
            params![notice.source_id, notice.external_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let id = match existing {
        Some((id, hash, from_feed)) => {
            let same = match hash {
                Some(hash) => hash == notice.revision_hash,
                // A v1 notice whose archived revision is the one the feed
                // describes keeps its body and revision.
                None => !from_feed && v1_revision_hash(connection, id)? == notice.revision_hash,
            };
            if same {
                connection
                    .execute(
                        "UPDATE documents SET revision_hash=?2,body_state=?3,modified_at=?4,from_feed=1,removed_at=NULL WHERE id=?1",
                        params![id, notice.revision_hash, body_state, notice.modified_at],
                    )
                    .map_err(|e| e.to_string())?;
                connection
                    .execute(
                        "UPDATE document_revisions SET revision_hash=?2,body_hash=COALESCE(body_hash,?3)
                         WHERE id=(SELECT MAX(id) FROM document_revisions WHERE document_id=?1)",
                        params![id, notice.revision_hash, notice.body_hash],
                    )
                    .map_err(|e| e.to_string())?;
                // The feed's dates stand in only while the body is not here.
                let pending: bool = connection
                    .query_row(
                        "SELECT body_pending FROM documents WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if pending {
                    notice_dates::store_feed(
                        connection,
                        id,
                        &notice.revision_hash,
                        notice.dates_version,
                        &notice.dates,
                    )?;
                }
                return Ok(true);
            }
            connection
                .execute(
                    "UPDATE documents SET canonical_url=?2,title=?3,author=?4,category=?5,published_at=?6,
                       modified_at=?7,body_html='',body_text=?8,content_hash=?9,revision_hash=?9,
                       body_state=?10,body_pending=1,from_feed=1,removed_at=NULL,last_revision_at=?11,
                       checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     WHERE id=?1",
                    params![
                        id,
                        notice.canonical_url,
                        notice.title,
                        notice.author,
                        notice.category,
                        notice.published_at,
                        notice.modified_at,
                        notice.excerpt,
                        notice.revision_hash,
                        body_state,
                        notice.revised_at
                    ],
                )
                .map_err(|e| e.to_string())?;
            id
        }
        None => {
            connection
                .execute(
                    "INSERT INTO documents(source_id,external_id,canonical_url,title,author,category,
                       published_at,modified_at,collected_at,body_html,body_text,content_hash,historical,
                       revision_hash,body_state,body_pending,from_feed)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'',?10,?11,?12,?11,?13,1,1)",
                    params![
                        notice.source_id,
                        notice.external_id,
                        notice.canonical_url,
                        notice.title,
                        notice.author,
                        notice.category,
                        notice.published_at,
                        notice.modified_at,
                        notice.first_seen_at,
                        notice.excerpt,
                        notice.revision_hash,
                        notice.historical,
                        body_state
                    ],
                )
                .map_err(|e| e.to_string())?;
            let id = connection.last_insert_rowid();
            connection
                .execute(
                    "INSERT OR IGNORE INTO document_state(document_id,is_read) VALUES(?1,?2)",
                    params![id, quiet || notice.historical],
                )
                .map_err(|e| e.to_string())?;
            id
        }
    };
    connection
        .execute(
            "INSERT INTO document_revisions(document_id,canonical_url,title,author,category,published_at,
               modified_at,original,body_html,body_text,content_hash,revision_hash,body_hash)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'','',?8,?9,?9,?10)",
            params![
                id,
                notice.canonical_url,
                notice.title,
                notice.author,
                notice.category,
                notice.published_at,
                notice.modified_at,
                notice.excerpt,
                notice.revision_hash,
                notice.body_hash
            ],
        )
        .map_err(|e| e.to_string())?;
    let revision_id = connection.last_insert_rowid();
    for resource in &notice.resources {
        insert_resource(
            connection,
            id,
            revision_id,
            &resource.url,
            &resource.filename,
            kind_name(resource.kind),
        )?;
    }
    notice_dates::store_feed(
        connection,
        id,
        &notice.revision_hash,
        notice.dates_version,
        &notice.dates,
    )?;
    applied.revised += 1;
    Ok(true)
}

/// The feed's `revision_hash` of a v1 notice's latest archived revision.
/// The crawler archived `document_id`'s body (`upsert_article`). For a notice
/// the feed described, that body counts as opened: the date backfill reads it
/// and the revision carries the feed hash of what was archived, so a later
/// feed record of the same revision keeps it (#177; the crawler leaves public
/// boards in PR 8).
pub(super) fn adopt_crawled(
    connection: &Connection,
    document_id: i64,
    unsupported: bool,
) -> Result<(), String> {
    let feed: bool = connection
        .query_row(
            "SELECT revision_hash IS NOT NULL FROM documents WHERE id=?1",
            [document_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !feed {
        return Ok(());
    }
    let hash = v1_revision_hash(connection, document_id)?;
    connection
        .execute(
            "UPDATE documents SET revision_hash=?2,body_pending=0,body_state=?3 WHERE id=?1",
            params![
                document_id,
                hash,
                if unsupported { "unsupported" } else { "normal" }
            ],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE document_revisions SET revision_hash=?2 WHERE id=(SELECT MAX(id) FROM document_revisions WHERE document_id=?1)",
            params![document_id, hash],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Adds a reference to a revision. As `upsert_article`: bytes already
/// downloaded for the same URL and role are reused; local access still
/// verifies their integrity.
fn insert_resource(
    connection: &Connection,
    document_id: i64,
    revision_id: i64,
    url: &str,
    filename: &str,
    kind: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR IGNORE INTO resources(document_id,revision_id,url,filename,kind) VALUES(?1,?2,?3,?4,?5)",
            params![document_id, revision_id, url, filename, kind],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE resources SET (status,mime,bytes,local_path,sha256,error)=(SELECT status,mime,bytes,local_path,sha256,NULL FROM resources prior WHERE prior.document_id=?1 AND prior.url=?3 AND prior.kind=?4 AND prior.status='downloaded' AND prior.revision_id<>?2 ORDER BY prior.id DESC LIMIT 1) WHERE document_id=?1 AND revision_id=?2 AND url=?3 AND status<>'downloaded' AND EXISTS(SELECT 1 FROM resources prior WHERE prior.document_id=?1 AND prior.url=?3 AND prior.kind=?4 AND prior.status='downloaded' AND prior.revision_id<>?2)",
            params![document_id, revision_id, url, kind],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The given resource files that no remaining row uses. Files are
/// content-addressed, so another notice may still use one.
fn unused(connection: &Connection, paths: Vec<String>) -> Result<Vec<String>, String> {
    let mut orphaned = Vec::new();
    for path in paths {
        let used: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM resources WHERE local_path=?1)",
                [&path],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !used && !orphaned.contains(&path) {
            orphaned.push(path);
        }
    }
    Ok(orphaned)
}

/// The feed's `revision_hash` of a fetched article, as the collector computes it.
pub(crate) fn article_revision_hash(article: &ArticleInput) -> String {
    let resources: Vec<(&str, &str, &str)> = article
        .resources
        .iter()
        .map(|r| (r.url.as_str(), r.filename.as_str(), r.kind.as_str()))
        .collect();
    rhythm_feed::revision_hash(&Revision {
        canonical_url: &article.canonical_url,
        title: &article.title,
        author: article.author.as_deref(),
        category: article.category.as_deref(),
        published_at: &article.published_at,
        body_text: &article.body_text,
        resources: &resources,
    })
}

/// What filling an opened notice's body changed.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Filled {
    /// The school's text is not the revision the feed described.
    pub revised: bool,
    /// Resource files no remaining notice uses, as in `Applied`.
    pub orphaned: Vec<String>,
}

impl Store {
    /// The source and listing identity of a notice whose body has not been
    /// fetched yet, or `None` when there is nothing to fetch: the body is
    /// here, or the source deleted the notice.
    pub(crate) fn pending_body(
        &self,
        document_id: i64,
    ) -> Result<Option<(String, ListingEntry)>, String> {
        let pending: Option<bool> = self
            .connection
            .query_row(
                "SELECT body_pending=1 AND removed_at IS NULL FROM documents WHERE id=?1",
                [document_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match pending {
            None => Err("보관된 공지를 찾을 수 없습니다.".into()),
            Some(false) => Ok(None),
            Some(true) => self.document_entry(document_id).map(Some),
        }
    }

    /// The source, listing identity and not yet attempted (`pending`)
    /// resources of an opened notice's latest revision, or `None` when there
    /// is nothing to download: the body has not arrived (its fetch brings
    /// the final references), the source deleted the notice, or every file
    /// was tried. Failed files wait for an explicit retry.
    pub(crate) fn pending_downloads(
        &self,
        document_id: i64,
    ) -> Result<Option<(String, ListingEntry, Vec<ResourceRecord>)>, String> {
        let ready: Option<bool> = self
            .connection
            .query_row(
                "SELECT body_pending=0 AND removed_at IS NULL FROM documents WHERE id=?1",
                [document_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match ready {
            None => return Err("보관된 공지를 찾을 수 없습니다.".into()),
            Some(false) => return Ok(None),
            Some(true) => {}
        }
        let pending: Vec<ResourceRecord> = self
            .pending_resources(document_id)?
            .into_iter()
            .filter(|r| r.status == "pending")
            .collect();
        if pending.is_empty() {
            return Ok(None);
        }
        let (source, entry) = self.document_entry(document_id)?;
        Ok(Some((source, entry, pending)))
    }

    /// Fills the latest revision of an opened feed notice in place with the
    /// article fetched from the school (#177). It stays the same revision,
    /// so freshness and notifications are unchanged. With `compare` the
    /// article's `revision_hash` decides whether the school's text is the
    /// revision the feed described; without it (a PDF-viewer notice, whose
    /// body is the PDF's text) the feed's `revision_hash` and `body_hash` are
    /// kept, so its next revision compares with what the collector saw. A
    /// revised text takes its own hashes, so the feed's later record of that
    /// revision keeps this body. Nothing changes when the body arrived
    /// meanwhile (another open, the crawler) or the notice left the feed.
    pub(crate) fn fill_feed_body(
        &self,
        document_id: i64,
        article: &ArticleInput,
        compare: bool,
    ) -> Result<Filled, String> {
        let (source_id, entry) = self.document_entry(document_id)?;
        check_article(&source_id, article)?;
        if article.external_id != entry.external_id {
            return Err("공지의 식별자, 본문 또는 크기를 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT revision_hash FROM documents WHERE id=?1 AND body_pending=1 AND removed_at IS NULL",
                [document_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(current) = current else {
            return Ok(Filled::default());
        };
        let fetched = article_revision_hash(article);
        let revised = compare && fetched != current;
        let revision_hash = if revised { fetched } else { current };
        let content_hash = meaningful_hash(article)?;
        let revision: i64 = transaction
            .query_row(
                "SELECT MAX(id) FROM document_revisions WHERE document_id=?1",
                [document_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE document_revisions SET canonical_url=?2,title=?3,author=?4,category=?5,
                   published_at=?6,modified_at=?7,original=?8,body_html=?9,body_text=?10,
                   content_hash=?11,revision_hash=?12,body_hash=COALESCE(?13,body_hash)
                 WHERE id=?1",
                params![
                    revision,
                    article.canonical_url,
                    article.title,
                    article.author,
                    article.category,
                    article.published_at,
                    article.modified_at,
                    article.original,
                    article.body_html,
                    article.body_text,
                    content_hash,
                    revision_hash,
                    compare.then(|| rhythm_feed::body_hash(&article.body_text))
                ],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE documents SET canonical_url=?2,title=?3,author=?4,category=?5,published_at=?6,
                   modified_at=?7,body_html=?8,body_text=?9,content_hash=?10,revision_hash=?11,
                   body_state=?12,body_pending=0,etag=?13,last_modified=?14,
                   checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id=?1",
                params![
                    document_id,
                    article.canonical_url,
                    article.title,
                    article.author,
                    article.category,
                    article.published_at,
                    article.modified_at,
                    article.body_html,
                    article.body_text,
                    content_hash,
                    revision_hash,
                    if article.unsupported_body {
                        "unsupported"
                    } else {
                        "normal"
                    },
                    article.etag,
                    article.last_modified
                ],
            )
            .map_err(|e| e.to_string())?;
        // The revision's references become the fetched ones. Unchanged URLs
        // keep their rows and download state.
        let urls: HashSet<&str> = article.resources.iter().map(|r| r.url.as_str()).collect();
        let stale: Vec<(i64, Option<String>)> = transaction
            .prepare("SELECT id,url,local_path FROM resources WHERE revision_id=?1")
            .and_then(|mut statement| {
                statement
                    .query_map([revision], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get(2)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(_, url, _)| !urls.contains(url.as_str()))
            .map(|(id, _, path)| (id, path))
            .collect();
        let mut filled = Filled {
            revised,
            orphaned: Vec::new(),
        };
        for (id, path) in stale {
            transaction
                .execute("DELETE FROM resources WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
            filled.orphaned.extend(path);
        }
        for resource in &article.resources {
            insert_resource(
                &transaction,
                document_id,
                revision,
                &resource.url,
                &resource.filename,
                &resource.kind,
            )?;
        }
        // Local vectors of the excerpt are recomputed from the body. The
        // feed's vector stays until the feed describes a newer revision.
        for table in ["notice_classifications", "notice_embeddings"] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE document_id=?1 AND revision_id=?2"),
                    params![document_id, revision],
                )
                .map_err(|e| e.to_string())?;
        }
        notice_dates::extract(&transaction, document_id)?;
        filled.orphaned = unused(&transaction, std::mem::take(&mut filled.orphaned))?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(filled)
    }
}

fn v1_revision_hash(connection: &Connection, document_id: i64) -> Result<String, String> {
    let (url, title, author, category, published, body): (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT canonical_url,title,author,category,published_at,body_text FROM document_revisions
             WHERE document_id=?1 ORDER BY id DESC LIMIT 1",
            [document_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String)> = connection
        .prepare(
            "SELECT url,filename,kind FROM resources WHERE revision_id=(
               SELECT MAX(id) FROM document_revisions WHERE document_id=?1) ORDER BY id",
        )
        .and_then(|mut statement| {
            statement
                .query_map([document_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .map_err(|e| e.to_string())?;
    let resources: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(u, f, k)| (u.as_str(), f.as_str(), k.as_str()))
        .collect();
    Ok(rhythm_feed::revision_hash(&Revision {
        canonical_url: &url,
        title: &title,
        author: author.as_deref(),
        category: category.as_deref(),
        published_at: &published,
        body_text: &body,
        resources: &resources,
    }))
}

/// The document and revision a derived record describes, while its
/// `revision_hash` is the notice's current one.
fn current_revision(
    connection: &Connection,
    source_id: &str,
    external_id: &str,
    revision_hash: &str,
) -> Result<Option<(i64, i64)>, String> {
    let Some(id) = feed_document(connection, source_id, external_id)? else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT d.id,(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=d.id)
             FROM documents d WHERE d.id=?1 AND d.revision_hash=?2",
            params![id, revision_hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())
}

fn embed(
    connection: &Connection,
    embedding: &Embedding,
    vectors: &[&[u8; VECTOR_BYTES]],
) -> Result<bool, String> {
    let vector = vectors.get(embedding.vector as usize);
    let target = current_revision(
        connection,
        &embedding.source_id,
        &embedding.external_id,
        &embedding.revision_hash,
    )?;
    let (Some(vector), Some((id, revision)), true) = (vector, target, embedding.scope.len() <= 512)
    else {
        return Ok(false);
    };
    connection
        .execute(
            "INSERT INTO feed_embeddings(document_id,revision_id,scope,vector) VALUES(?1,?2,?3,?4)
             ON CONFLICT(document_id) DO UPDATE SET revision_id=excluded.revision_id,
               scope=excluded.scope,vector=excluded.vector",
            params![id, revision, embedding.scope, vector.as_slice()],
        )
        .map_err(|e| e.to_string())?;
    Ok(true)
}

fn classify(connection: &Connection, classification: &Classification) -> Result<bool, String> {
    let valid = CATEGORIES.contains(&classification.category.as_str())
        && (-1.0..=1.0).contains(&classification.score)
        && (-2.0..=2.0).contains(&classification.margin)
        && classification.scope.len() <= 512;
    let target = current_revision(
        connection,
        &classification.source_id,
        &classification.external_id,
        &classification.revision_hash,
    )?;
    let (true, Some((id, revision))) = (valid, target) else {
        return Ok(false);
    };
    connection
        .execute(
            "INSERT INTO feed_classifications(document_id,revision_id,scope,category,score,margin,low_confidence)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(document_id) DO UPDATE SET revision_id=excluded.revision_id,scope=excluded.scope,
               category=excluded.category,score=excluded.score,margin=excluded.margin,
               low_confidence=excluded.low_confidence",
            params![
                id,
                revision,
                classification.scope,
                classification.category,
                classification.score,
                classification.margin,
                classification.low_confidence
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// A notice deleted at its source: removed with everything derived from it,
/// unless saved, which keeps its row and cached body with `removed_at`.
fn remove(
    connection: &Connection,
    document_id: i64,
    removed_at: &str,
    applied: &mut Applied,
) -> Result<(), String> {
    let saved: bool = connection
        .query_row(
            "SELECT COALESCE((SELECT saved FROM document_state WHERE document_id=?1),0)",
            [document_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if saved {
        let changed = connection
            .execute(
                "UPDATE documents SET removed_at=?2 WHERE id=?1 AND removed_at IS NULL",
                params![document_id, removed_at],
            )
            .map_err(|e| e.to_string())?;
        applied.removed += changed;
        return Ok(());
    }
    let paths: Vec<String> = connection
        .prepare("SELECT DISTINCT local_path FROM resources WHERE document_id=?1 AND local_path IS NOT NULL")
        .and_then(|mut statement| statement.query_map([document_id], |r| r.get(0))?.collect())
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "DELETE FROM skipped_articles WHERE EXISTS(SELECT 1 FROM documents d WHERE d.id=?1
               AND d.source_id=skipped_articles.source_id AND d.external_id=skipped_articles.external_id)",
            [document_id],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute("DELETE FROM documents WHERE id=?1", [document_id])
        .map_err(|e| e.to_string())?;
    applied.orphaned.extend(paths);
    applied.removed += 1;
    Ok(())
}
