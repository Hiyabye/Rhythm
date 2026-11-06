use super::Store;
use crate::archive::model::*;
use rusqlite::{params, params_from_iter, types::Value, OptionalExtension, Row};
use sha2::{Digest, Sha256};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
const SOURCE_COLUMNS: &str = "id,name,organization,description,url,family,supported,subscribed,last_attempt,last_success,last_error,next_page,history_complete,status,discovered,archived,failed,(SELECT COUNT(*) FROM skipped_articles k WHERE k.source_id=sources.id)";
const ITEM_COLUMNS: &str = "d.id,d.source_id,s.name,d.title,d.author,d.category,d.published_at,d.collected_at,COALESCE(u.is_read,0),COALESCE(u.saved,0),d.historical,d.body_text,CASE WHEN EXISTS(SELECT 1 FROM resources a WHERE a.document_id=d.id AND a.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=d.id) AND a.status<>'downloaded') THEN 'missing_resources' ELSE 'archived' END";
const ITEM_FROM: &str = "documents d JOIN sources s ON s.id=d.source_id LEFT JOIN document_state u ON u.document_id=d.id";
const RESOURCE_COLUMNS: &str = "id,url,filename,kind,status,mime,bytes,local_path,sha256,error";

type Validators = (Option<String>, Option<String>);

fn source_row(row: &Row<'_>) -> rusqlite::Result<SourceInfo> {
    Ok(SourceInfo {
        definition: SourceDefinition {
            id: row.get(0)?,
            name: row.get(1)?,
            organization: row.get(2)?,
            description: row.get(3)?,
            url: row.get(4)?,
            family: row.get(5)?,
            supported: row.get(6)?,
        },
        subscribed: row.get(7)?,
        last_attempt: row.get(8)?,
        last_success: row.get(9)?,
        last_error: row.get(10)?,
        next_page: row.get(11)?,
        history_complete: row.get(12)?,
        status: row.get(13)?,
        discovered: row.get::<_, i64>(14)? as u64,
        archived: row.get::<_, i64>(15)? as u64,
        failed: row.get::<_, i64>(16)? as u64,
        skipped_count: row.get::<_, i64>(17)? as u64,
        skipped: Vec::new(),
    })
}
/// Bounds the notices listed per source; the count covers all of them.
const SKIPPED_LISTED: i64 = 20;
fn item_row(row: &Row<'_>) -> rusqlite::Result<DocumentItem> {
    Ok(DocumentItem {
        id: row.get(0)?,
        source_id: row.get(1)?,
        source_name: row.get(2)?,
        title: row.get(3)?,
        author: row.get(4)?,
        category: row.get(5)?,
        published_at: row.get(6)?,
        collected_at: row.get(7)?,
        is_read: row.get(8)?,
        saved: row.get(9)?,
        historical: row.get(10)?,
        snippet: row.get(11)?,
        content_status: row.get(12)?,
    })
}
fn resource_row(row: &Row<'_>) -> rusqlite::Result<ResourceRecord> {
    Ok(ResourceRecord {
        id: row.get(0)?,
        url: row.get(1)?,
        filename: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        mime: row.get(5)?,
        bytes: row.get::<_, Option<i64>>(6)?.map(|bytes| bytes as u64),
        local_path: row.get(7)?,
        sha256: row.get(8)?,
        error: row.get(9)?,
    })
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn public_url(value: &str) -> bool {
    web_url(value, false)
}
fn web_url(value: &str, allow_http: bool) -> bool {
    value.len() <= 8192
        && url::Url::parse(value).is_ok_and(|url| {
            (url.scheme() == "https" || (allow_http && url.scheme() == "http"))
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}
fn require_changed(changed: usize) -> Result<(), String> {
    if changed == 0 {
        Err("보관된 항목을 찾을 수 없습니다.".into())
    } else {
        Ok(())
    }
}
fn meaningful_hash(article: &ArticleInput) -> Result<String, String> {
    let resources: Vec<_> = article
        .resources
        .iter()
        .map(|r| (&r.url, &r.filename, &r.kind))
        .collect();
    let encoded = serde_json::to_vec(&(
        &article.canonical_url,
        &article.title,
        &article.author,
        &article.category,
        &article.published_at,
        &article.body_html,
        &article.body_text,
        resources,
    ))
    .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
/// Reciprocal rank fusion constant; 60 is the value from the original paper.
const RRF_K: f64 = 60.0;
/// Text matches fused with semantic hits; later text matches keep text order.
const RANKED_TEXT_HEAD: usize = 200;
/// Semantic hits per search. Measured on the 18 synthetic fixtures with
/// Qwen3-Embedding 0.6B (docs/AI_EVALUATION.md): keeping hits within 0.08 of
/// the best score and above 0.20 kept a relevant notice for every query.
const SEARCH_LIMIT: usize = 20;
const SEARCH_MIN_SCORE: f32 = 0.20;
const SEARCH_MARGIN: f32 = 0.08;

/// Validated whitespace-separated search terms.
pub(crate) fn search_terms(query: &str) -> Result<Vec<&str>, String> {
    // SQLite LIKE/FTS treat NUL as a terminator even in bound values.
    if query.contains('\0') {
        return Err("검색어에 사용할 수 없는 문자가 포함되어 있습니다.".into());
    }
    if query.chars().count() > 512 {
        return Err("검색어는 512자 이내로 입력해 주세요.".into());
    }
    let terms: Vec<&str> = query.split_whitespace().collect();
    if terms.len() > 32 {
        return Err("검색 단어는 32개 이내로 입력해 주세요.".into());
    }
    Ok(terms)
}
/// Trigram FTS needs three characters; shorter terms rely on LIKE alone.
fn fts_expression(terms: &[&str]) -> Option<String> {
    let indexed: Vec<String> = terms
        .iter()
        .filter(|term| term.chars().count() >= 3)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    (!indexed.is_empty()).then(|| indexed.join(" AND "))
}
fn like_pattern(term: &str) -> String {
    format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}
/// LIKE is an exact literal-substring check and handles one/two-character Korean queries.
fn push_like_filters(terms: &[&str], filters: &mut Vec<String>, values: &mut Vec<Value>) {
    for term in terms {
        filters.push("(d.title LIKE ? ESCAPE '\\' OR d.body_text LIKE ? ESCAPE '\\')".into());
        let escaped = like_pattern(term);
        values.push(escaped.clone().into());
        values.push(escaped.into());
    }
}
fn id_list(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}
fn excerpt(text: &str, terms: &[&str]) -> String {
    let folded = text.to_lowercase();
    let at = terms
        .iter()
        .filter_map(|term| folded.find(&term.to_lowercase()))
        .min()
        .unwrap_or(0);
    let index = folded[..at].chars().count().saturating_sub(48);
    let mut result: String = text.chars().skip(index).take(240).collect();
    if index > 0 {
        result.insert(0, '…');
    }
    if text.chars().count() > index + 240 {
        result.push('…');
    }
    result
}

impl Store {
    pub(crate) fn catalog(
        &self,
        definitions: &[SourceDefinition],
    ) -> Result<Vec<SourceInfo>, String> {
        self.catalog_for_extension("sogang_notices", definitions)
    }
    pub(crate) fn catalog_for_extension(
        &self,
        extension_id: &str,
        definitions: &[SourceDefinition],
    ) -> Result<Vec<SourceInfo>, String> {
        if !valid_id(extension_id) {
            return Err("확장 정보원 정의를 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        for definition in definitions {
            if !valid_id(&definition.id)
                || definition.name.trim().is_empty()
                || !public_url(&definition.url)
            {
                return Err("정보원 정의를 확인해 주세요.".into());
            }
            transaction.execute("INSERT INTO sources(id,name,organization,description,url,family,supported) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,organization=excluded.organization,description=excluded.description,url=excluded.url,family=excluded.family,supported=excluded.supported",params![definition.id,definition.name,definition.organization,definition.description,definition.url,definition.family,definition.supported]).map_err(|e|e.to_string())?;
            transaction
                .execute(
                    "INSERT INTO source_extensions(source_id,extension_id) VALUES(?1,?2)
                 ON CONFLICT(source_id) DO UPDATE SET extension_id=excluded.extension_id",
                    params![definition.id, extension_id],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        self.sources()
    }
    pub(crate) fn sources(&self) -> Result<Vec<SourceInfo>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {SOURCE_COLUMNS} FROM sources ORDER BY organization,name,id"
            ))
            .map_err(|e| e.to_string())?;
        let result = statement
            .query_map([], source_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        let mut sources = result?;
        for source in sources.iter_mut().filter(|s| s.skipped_count > 0) {
            source.skipped = self.skipped_articles(&source.definition.id, SKIPPED_LISTED)?;
        }
        Ok(sources)
    }
    fn skipped_articles(&self, source: &str, limit: i64) -> Result<Vec<SkippedArticle>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT k.external_id,k.url,k.reason,k.skipped_at,EXISTS(
                    SELECT 1 FROM documents d WHERE d.source_id=k.source_id AND d.external_id=k.external_id)
                 FROM skipped_articles k WHERE k.source_id=?1
                 ORDER BY k.skipped_at DESC,k.external_id LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![source, limit], |row| {
                Ok(SkippedArticle {
                    external_id: row.get(0)?,
                    url: row.get(1)?,
                    reason: row.get(2)?,
                    skipped_at: row.get(3)?,
                    archived: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    /// Documents of a source whose current revision has a file to (re)download,
    /// newest first. Terminal states (unsupported, too large) are not retried.
    pub(crate) fn documents_missing_resources(&self, source: &str) -> Result<Vec<i64>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT d.id FROM documents d WHERE d.source_id=?1 AND EXISTS(
                    SELECT 1 FROM resources r WHERE r.document_id=d.id
                      AND r.revision_id=(SELECT MAX(id) FROM document_revisions WHERE document_id=d.id)
                      AND r.status IN ('pending','failed'))
                 ORDER BY d.published_at DESC,d.id DESC",
            )
            .map_err(|e| e.to_string())?;
        let ids = statement
            .query_map([source], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        ids
    }
    /// Every skipped notice of a source, with the historical marking to retry it under.
    pub(crate) fn skipped_entries(
        &self,
        source: &str,
    ) -> Result<Vec<(ListingEntry, bool)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT external_id,url,historical FROM skipped_articles
                 WHERE source_id=?1 ORDER BY skipped_at,external_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([source], |row| {
                Ok((
                    ListingEntry {
                        external_id: row.get(0)?,
                        url: row.get(1)?,
                        pinned: false,
                        modified_at: None,
                    },
                    row.get(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    /// Records a notice whose response was rejected for its content. A repeated
    /// rejection keeps the first skip time and the original historical marking.
    pub(crate) fn record_skipped(
        &self,
        source: &str,
        entry: &ListingEntry,
        historical: bool,
        reason: &str,
        response_sha256: &str,
    ) -> Result<(), String> {
        if !self.source(source)?.definition.supported
            || !valid_id(&entry.external_id)
            || !public_url(&entry.url)
            || reason.trim().is_empty()
            || reason.len() > 1024
            || response_sha256.len() != 64
        {
            return Err("가져오지 못한 공지 정보를 확인해 주세요.".into());
        }
        self.connection
            .execute(
                "INSERT INTO skipped_articles(source_id,external_id,url,historical,reason,response_sha256)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(source_id,external_id) DO UPDATE SET
                   url=excluded.url,reason=excluded.reason,response_sha256=excluded.response_sha256",
                params![source, entry.external_id, entry.url, historical, reason, response_sha256],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(crate) fn clear_skipped(&self, source: &str, external: &str) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM skipped_articles WHERE source_id=?1 AND external_id=?2",
                params![source, external],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(crate) fn source(&self, id: &str) -> Result<SourceInfo, String> {
        self.connection
            .query_row(
                &format!("SELECT {SOURCE_COLUMNS} FROM sources WHERE id=?1"),
                [id],
                source_row,
            )
            .map_err(|_| "등록된 정보원을 찾을 수 없습니다.".into())
    }
    pub(crate) fn subscribe(&self, id: &str, subscribed: bool) -> Result<(), String> {
        let source = self.source(id)?;
        if subscribed && !source.definition.supported {
            return Err("아직 지원하지 않는 정보원입니다.".into());
        }
        self.connection
            .execute(
                "UPDATE sources SET subscribed=?2 WHERE id=?1",
                params![id, subscribed],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn source_progress(
        &self,
        id: &str,
        status: &str,
        next_page: u32,
        history_complete: bool,
        discovered: u64,
        archived: u64,
        failed: u64,
        error: Option<&str>,
        success: bool,
    ) -> Result<(), String> {
        if next_page == 0
            || ![
                "idle",
                "running",
                "completed",
                "failed",
                "cancelled",
                "interrupted",
                "limited",
            ]
            .contains(&status)
            || [discovered, archived, failed]
                .iter()
                .any(|n| *n > i64::MAX as u64)
        {
            return Err("동기화 진행 상태를 확인해 주세요.".into());
        }
        require_changed(self.connection.execute(&format!("UPDATE sources SET status=?2,next_page=?3,history_complete=?4,discovered=?5,archived=?6,failed=?7,last_error=?8,last_attempt=CASE WHEN ?2='running' AND status<>'running' THEN {NOW} ELSE last_attempt END,previous_success=CASE WHEN ?9 THEN last_success ELSE previous_success END,last_success=CASE WHEN ?9 THEN {NOW} ELSE last_success END WHERE id=?1"),params![id,status,next_page,history_complete,discovered as i64,archived as i64,failed as i64,error,success]).map_err(|e|e.to_string())?)
    }
    pub(crate) fn reset_interrupted(&self) -> Result<(), String> {
        self.connection.execute("UPDATE sources SET status='interrupted',last_error='이전 가져오기가 중단되었습니다. 이어서 가져올 수 있습니다.' WHERE status='running'",[]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub(crate) fn document_id(&self, source: &str, external: &str) -> Result<Option<i64>, String> {
        self.connection
            .query_row(
                "SELECT id FROM documents WHERE source_id=?1 AND external_id=?2",
                params![source, external],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub(crate) fn has_readable_content(&self, document_id: i64) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT d.body_html<>'' OR d.body_text<>'' OR EXISTS(
                    SELECT 1 FROM resources r
                    WHERE r.document_id=d.id
                      AND r.revision_id=(SELECT MAX(id) FROM document_revisions WHERE document_id=d.id)
                      AND r.kind='image'
                 )
                 FROM documents d WHERE d.id=?1",
                [document_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
    #[cfg(test)]
    pub(crate) fn latest_original(&self, document_id: i64) -> Result<String, String> {
        self.connection
            .query_row(
                "SELECT original FROM document_revisions
                 WHERE document_id=?1 ORDER BY id DESC LIMIT 1",
                [document_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub(crate) fn validators(
        &self,
        source: &str,
        external: &str,
    ) -> Result<Option<Validators>, String> {
        self.connection
            .query_row(
                "SELECT etag,last_modified FROM documents WHERE source_id=?1 AND external_id=?2",
                params![source, external],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    /// True when the archived notice already carries the listing's own change
    /// timestamp. A sitemap-backed family uses this to skip a request instead of
    /// revalidating every article on each incremental run.
    pub(crate) fn listing_unchanged(
        &self,
        source: &str,
        external: &str,
        modified_at: &str,
    ) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT modified_at FROM documents WHERE source_id=?1 AND external_id=?2",
                params![source, external],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|stored| stored.flatten().is_some_and(|value| value == modified_at))
            .map_err(|e| e.to_string())
    }
    pub(crate) fn checked(&self, source: &str, external: &str) -> Result<(), String> {
        require_changed(self.connection.execute(&format!("UPDATE documents SET checked_at={NOW} WHERE source_id=?1 AND external_id=?2"),params![source,external]).map_err(|e|e.to_string())?)
    }
    pub(crate) fn upsert_article(
        &self,
        source_id: &str,
        article: &ArticleInput,
        historical: bool,
    ) -> Result<(i64, bool), String> {
        let source = self.source(source_id)?;
        let empty_body = article.body_text.trim().is_empty() && article.body_html.trim().is_empty();
        let invalid_unsupported_body = article.unsupported_body
            && (source_id != "sogang-academic"
                || !empty_body
                || article.resources.iter().any(|r| r.kind == "image"));
        if !source.definition.supported
            || !valid_id(&article.external_id)
            || article.title.trim().is_empty()
            || article.title.len() > 16_384
            || article.original.len() > 8 * 1024 * 1024
            || article.body_html.len() > 8 * 1024 * 1024
            || article.body_text.len() > 8 * 1024 * 1024
            || !public_url(&article.canonical_url)
            || article.published_at.trim().is_empty()
            || article.resources.len() > 256
            || invalid_unsupported_body
            || (empty_body && article.resources.is_empty() && !article.unsupported_body)
        {
            return Err("공지의 식별자, 본문 또는 크기를 확인해 주세요.".into());
        }
        for resource in &article.resources {
            // Retaining a safe reference is not permission to fetch it. The
            // transport separately enforces HTTPS and exact approved hosts.
            if !web_url(&resource.url, true)
                || resource.filename.len() > 4096
                || !["image", "attachment"].contains(&resource.kind.as_str())
            {
                return Err("첨부 자료의 주소나 종류를 확인해 주세요.".into());
            }
        }
        let hash = meaningful_hash(article)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let existing: Option<(i64, String)> = transaction
            .query_row(
                "SELECT id,content_hash FROM documents WHERE source_id=?1 AND external_id=?2",
                params![source_id, article.external_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((id, old_hash)) = &existing {
            if *old_hash == hash {
                transaction.execute(&format!("UPDATE documents SET checked_at={NOW},etag=?2,last_modified=?3,modified_at=?4 WHERE id=?1"),params![id,article.etag,article.last_modified,article.modified_at]).map_err(|e|e.to_string())?;
                transaction.commit().map_err(|e| e.to_string())?;
                return Ok((*id, false));
            }
        }
        transaction.execute(&format!("INSERT INTO documents(source_id,external_id,canonical_url,title,author,category,published_at,modified_at,body_html,body_text,content_hash,etag,last_modified,historical) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(source_id,external_id) DO UPDATE SET canonical_url=excluded.canonical_url,title=excluded.title,author=excluded.author,category=excluded.category,published_at=excluded.published_at,modified_at=excluded.modified_at,body_html=excluded.body_html,body_text=excluded.body_text,content_hash=excluded.content_hash,etag=excluded.etag,last_modified=excluded.last_modified,checked_at={NOW}"),params![source_id,article.external_id,article.canonical_url,article.title,article.author,article.category,article.published_at,article.modified_at,article.body_html,article.body_text,hash,article.etag,article.last_modified,historical]).map_err(|e|e.to_string())?;
        let id: i64 = transaction
            .query_row(
                "SELECT id FROM documents WHERE source_id=?1 AND external_id=?2",
                params![source_id, article.external_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        transaction.execute("INSERT INTO document_revisions(document_id,canonical_url,title,author,category,published_at,modified_at,original,body_html,body_text,content_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![id,article.canonical_url,article.title,article.author,article.category,article.published_at,article.modified_at,article.original,article.body_html,article.body_text,hash]).map_err(|e|e.to_string())?;
        let revision_id = transaction.last_insert_rowid();
        if existing.is_some() {
            transaction
                .execute(
                    "UPDATE documents SET last_revision_at=(
                        SELECT collected_at FROM document_revisions WHERE id=?2
                     ) WHERE id=?1",
                    params![id, revision_id],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO document_state(document_id) VALUES(?1)",
                [id],
            )
            .map_err(|e| e.to_string())?;
        for resource in &article.resources {
            transaction.execute("INSERT OR IGNORE INTO resources(document_id,revision_id,url,filename,kind) VALUES(?1,?2,?3,?4,?5)",params![id,revision_id,resource.url,resource.filename,resource.kind]).map_err(|e|e.to_string())?;
            // Reuse bytes only for the same URL and role; image and attachment
            // access have different validation limits. This avoids a download,
            // not a remote-byte recheck; local access still verifies integrity.
            transaction.execute("UPDATE resources SET (status,mime,bytes,local_path,sha256,error)=(SELECT status,mime,bytes,local_path,sha256,NULL FROM resources prior WHERE prior.document_id=?1 AND prior.url=?3 AND prior.kind=?4 AND prior.status='downloaded' AND prior.revision_id<>?2 ORDER BY prior.id DESC LIMIT 1) WHERE document_id=?1 AND revision_id=?2 AND url=?3 AND EXISTS(SELECT 1 FROM resources prior WHERE prior.document_id=?1 AND prior.url=?3 AND prior.kind=?4 AND prior.status='downloaded' AND prior.revision_id<>?2)",params![id,revision_id,resource.url,resource.kind]).map_err(|e|e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok((id, true))
    }
    /// Source, category and state filters; text conditions are added separately.
    fn archive_filters(&self, query: &ArchiveQuery) -> Result<(Vec<String>, Vec<Value>), String> {
        let mut filters = vec!["1=1".to_string()];
        let mut values: Vec<Value> = Vec::new();
        if let Some(source) = &query.source_id {
            self.source(source)?;
            filters.push("d.source_id=?".into());
            values.push(source.clone().into());
        }
        if let Some(category) = &query.category {
            filters.push("d.category=?".into());
            values.push(category.clone().into());
        }
        if query.saved_only {
            filters.push("COALESCE(u.saved,0)=1".into());
        }
        if query.unread_only {
            filters.push("COALESCE(u.is_read,0)=0".into());
        }
        Ok((filters, values))
    }
    pub(crate) fn archive_query(&self, query: &ArchiveQuery) -> Result<ArchivePage, String> {
        let terms = search_terms(&query.query)?;
        let (mut filters, mut values) = self.archive_filters(query)?;
        if let Some(expression) = fts_expression(&terms) {
            filters.push(
                "d.id IN (SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?)".into(),
            );
            values.push(expression.into());
        }
        push_like_filters(&terms, &mut filters, &mut values);
        let condition = filters.join(" AND ");
        let total: u64 = self
            .connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {ITEM_FROM} WHERE {condition}"),
                params_from_iter(values.iter()),
                |r| r.get::<_, i64>(0).map(|count| count as u64),
            )
            .map_err(|e| e.to_string())?;
        let order = if query.ascending { "ASC" } else { "DESC" };
        let limit = query.limit.unwrap_or(30).clamp(1, 100);
        let sql=format!("SELECT {ITEM_COLUMNS} FROM {ITEM_FROM} WHERE {condition} ORDER BY d.published_at {order},d.id {order} LIMIT ? OFFSET ?");
        values.push(i64::from(limit).into());
        values.push(i64::from(query.offset).into());
        let mut statement = self.connection.prepare(&sql).map_err(|e| e.to_string())?;
        let mut items = statement
            .query_map(params_from_iter(values.iter()), item_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for item in &mut items {
            item.snippet = excerpt(&item.snippet, &terms);
        }
        Ok(ArchivePage {
            items,
            total,
            categories: self.archive_categories(query.source_id.as_deref())?,
            meaning_only: Vec::new(),
            semantic: false,
        })
    }
    fn archive_categories(&self, source_id: Option<&str>) -> Result<Vec<String>, String> {
        let mut categories_statement=self.connection.prepare("SELECT DISTINCT category FROM documents WHERE category IS NOT NULL AND category<>'' AND (?1 IS NULL OR source_id=?1) ORDER BY category").map_err(|e|e.to_string())?;
        let categories = categories_statement
            .query_map([source_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(categories)
    }
    /// Text matches in relevance order: title hits first, then FTS bm25 when an
    /// indexed term exists, then newest. `extra` is a trusted SQL condition
    /// built from integer IDs only.
    fn ranked_text_sql(
        &self,
        query: &ArchiveQuery,
        terms: &[&str],
        extra: &str,
    ) -> Result<(String, Vec<Value>, String, Vec<Value>), String> {
        let mut from = ITEM_FROM.to_string();
        let mut values = Vec::new();
        let fts = fts_expression(terms);
        if let Some(expression) = &fts {
            from.push_str(" JOIN (SELECT rowid AS fts_id,bm25(documents_fts) AS fts_rank FROM documents_fts WHERE documents_fts MATCH ?) f ON f.fts_id=d.id");
            values.push(expression.clone().into());
        }
        let (mut filters, filter_values) = self.archive_filters(query)?;
        values.extend(filter_values);
        push_like_filters(terms, &mut filters, &mut values);
        filters.push(extra.to_owned());
        let mut order_values = Vec::new();
        let title_hits = terms
            .iter()
            .map(|term| {
                order_values.push(like_pattern(term).into());
                "(d.title LIKE ? ESCAPE '\\')"
            })
            .collect::<Vec<_>>()
            .join("+");
        let order = format!(
            "{title_hits} DESC,{}d.published_at DESC,d.id DESC",
            if fts.is_some() { "f.fts_rank," } else { "" }
        );
        Ok((
            format!("{from} WHERE {}", filters.join(" AND ")),
            values,
            order,
            order_values,
        ))
    }
    fn ranked_text_ids(
        &self,
        query: &ArchiveQuery,
        terms: &[&str],
        extra: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<i64>, String> {
        let (source, mut values, order, order_values) =
            self.ranked_text_sql(query, terms, extra)?;
        values.extend(order_values);
        values.push((limit as i64).into());
        values.push((offset as i64).into());
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT d.id FROM {source} ORDER BY {order} LIMIT ? OFFSET ?"
            ))
            .map_err(|e| e.to_string())?;
        let ids = statement
            .query_map(params_from_iter(values.iter()), |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(ids)
    }
    /// Latest-revision vectors of one embedding model that pass the
    /// non-text filters, newest `SIMILARITY_WINDOW` first, ranked by cosine
    /// to the query. A hit is kept only near the best one (see the constants).
    fn semantic_ids(
        &self,
        query: &ArchiveQuery,
        model_sha256: &str,
        vector: &[f32],
    ) -> Result<Vec<i64>, String> {
        if !super::intelligence::valid_hash(model_sha256) {
            return Err("검색 모델 식별자가 올바르지 않습니다.".into());
        }
        let (filters, filter_values) = self.archive_filters(query)?;
        let mut values: Vec<Value> = vec![model_sha256.to_owned().into()];
        values.extend(filter_values);
        values.push((super::intelligence::SIMILARITY_WINDOW as i64).into());
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT d.id,e.dim,e.vector FROM {ITEM_FROM}
                 JOIN notice_embeddings e ON e.document_id=d.id AND e.model_sha256=?
                   AND e.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=d.id)
                 WHERE {} ORDER BY d.published_at DESC,d.id DESC LIMIT ?",
                filters.join(" AND ")
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // A damaged stored vector only drops that notice from the semantic side.
        let mut scored = rows
            .into_iter()
            .filter_map(|(id, dim, bytes)| {
                let stored = super::intelligence::decode_embedding(dim, &bytes).ok()?;
                let score = super::intelligence::cosine(vector, &stored)?;
                score.is_finite().then_some((id, score))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
        let Some(best) = scored.first().map(|(_, score)| *score) else {
            return Ok(Vec::new());
        };
        let floor = SEARCH_MIN_SCORE.max(best - SEARCH_MARGIN);
        Ok(scored
            .into_iter()
            .take_while(|(_, score)| *score >= floor)
            .take(SEARCH_LIMIT)
            .map(|(id, _)| id)
            .collect())
    }
    /// Search results in relevance order. Reciprocal rank fusion merges the
    /// first `RANKED_TEXT_HEAD` text matches with semantic hits when a query
    /// vector is given; later text matches follow in text order, so no text
    /// match is ever dropped. Without a query this is the date-ordered list.
    pub(crate) fn archive_ranked_query(
        &self,
        query: &ArchiveQuery,
        semantic: Option<(&str, &[f32])>,
    ) -> Result<ArchivePage, String> {
        let terms = search_terms(&query.query)?;
        if terms.is_empty() {
            return self.archive_query(query);
        }
        let text_head = self.ranked_text_ids(query, &terms, "1=1", RANKED_TEXT_HEAD, 0)?;
        let text_total: u64 = {
            let (source, values, _, _) = self.ranked_text_sql(query, &terms, "1=1")?;
            self.connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {source}"),
                    params_from_iter(values.iter()),
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())? as u64
        };
        let semantic_hits = match semantic {
            Some((model_sha256, vector)) => self.semantic_ids(query, model_sha256, vector)?,
            None => Vec::new(),
        };
        let semantic_text = if semantic_hits.is_empty() {
            Vec::new()
        } else {
            self.ranked_text_ids(
                query,
                &terms,
                &format!("d.id IN ({})", id_list(&semantic_hits)),
                semantic_hits.len(),
                0,
            )?
        };
        let mut fused = std::collections::HashMap::<i64, f64>::new();
        for list in [&text_head, &semantic_hits] {
            for (rank, id) in list.iter().enumerate() {
                *fused.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
            }
        }
        let mut head = fused.into_iter().collect::<Vec<_>>();
        head.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
        let head = head.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let text_in_head = text_head
            .iter()
            .chain(&semantic_text)
            .collect::<std::collections::HashSet<_>>()
            .len() as u64;
        let total = head.len() as u64 + text_total.saturating_sub(text_in_head);
        let limit = query.limit.unwrap_or(30).clamp(1, 100) as usize;
        let offset = query.offset as usize;
        let mut ids = head
            .iter()
            .skip(offset)
            .take(limit)
            .copied()
            .collect::<Vec<_>>();
        if !head.is_empty() && ids.len() < limit && offset + ids.len() >= head.len() {
            ids.extend(self.ranked_text_ids(
                query,
                &terms,
                &format!("d.id NOT IN ({})", id_list(&head)),
                limit - ids.len(),
                offset + ids.len() - head.len(),
            )?);
        }
        let mut items = if ids.is_empty() {
            Vec::new()
        } else {
            let mut statement = self
                .connection
                .prepare(&format!(
                    "SELECT {ITEM_COLUMNS} FROM {ITEM_FROM} WHERE d.id IN ({})",
                    id_list(&ids)
                ))
                .map_err(|e| e.to_string())?;
            let mut found = statement
                .query_map([], item_row)
                .map_err(|e| e.to_string())?
                .map(|item| item.map(|item| (item.id, item)))
                .collect::<Result<std::collections::HashMap<_, _>, _>>()
                .map_err(|e| e.to_string())?;
            ids.iter()
                .filter_map(|id| found.remove(id))
                .collect::<Vec<_>>()
        };
        for item in &mut items {
            item.snippet = excerpt(&item.snippet, &terms);
        }
        Ok(ArchivePage {
            items,
            total,
            categories: self.archive_categories(query.source_id.as_deref())?,
            meaning_only: semantic_hits
                .into_iter()
                .filter(|id| !semantic_text.contains(id))
                .collect(),
            semantic: semantic.is_some(),
        })
    }
    pub(crate) fn archive_summary(&self) -> Result<ArchiveSummary, String> {
        self.connection.query_row("SELECT COUNT(*),COALESCE(SUM(COALESCE(u.is_read,0)=0),0),COALESCE(SUM(COALESCE(u.saved,0)=1),0),COALESCE(SUM(d.historical=0 AND d.collected_at>=COALESCE(s.previous_success,'1970')),0),(SELECT COALESCE(SUM(bytes),0) FROM (SELECT MAX(bytes) bytes FROM resources WHERE status='downloaded' GROUP BY local_path)) FROM documents d JOIN sources s ON s.id=d.source_id LEFT JOIN document_state u ON u.document_id=d.id",[],|r|Ok(ArchiveSummary {documents:r.get::<_,i64>(0)? as u64,unread:r.get::<_,i64>(1)? as u64,saved:r.get::<_,i64>(2)? as u64,new_since_sync:r.get::<_,i64>(3)? as u64,resource_bytes:r.get::<_,i64>(4)? as u64})).map_err(|e|e.to_string())
    }
    pub(crate) fn document(
        &self,
        id: i64,
        revision_id: Option<i64>,
    ) -> Result<DocumentDetail, String> {
        let mut item = self
            .connection
            .query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM {ITEM_FROM} WHERE d.id=?1"),
                [id],
                item_row,
            )
            .map_err(|_| "보관된 공지를 찾을 수 없습니다.".to_string())?;
        let mut statement=self.connection.prepare("SELECT id,collected_at,content_hash FROM document_revisions WHERE document_id=?1 ORDER BY id DESC").map_err(|e|e.to_string())?;
        let revisions = statement
            .query_map([id], |r| {
                Ok(RevisionInfo {
                    id: r.get(0)?,
                    collected_at: r.get(1)?,
                    content_hash: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let selected = revision_id
            .or_else(|| revisions.first().map(|r| r.id))
            .ok_or("보관된 버전을 찾을 수 없습니다.")?;
        let (canonical_url,body_html,body_text): (String, String, String)=self.connection.query_row("SELECT canonical_url,body_html,body_text,title,author,category,published_at FROM document_revisions WHERE id=?1 AND document_id=?2",params![selected,id],|r| {
            item.title=r.get(3)?; item.author=r.get(4)?; item.category=r.get(5)?; item.published_at=r.get(6)?;
            Ok((r.get(0)?,r.get(1)?,r.get(2)?))
        }).map_err(|_|"이 공지에 해당하는 보관 버전을 찾을 수 없습니다.".to_string())?;
        let mut resource_statement=self.connection.prepare(&format!("SELECT {RESOURCE_COLUMNS} FROM resources WHERE document_id=?1 AND revision_id=?2 ORDER BY id")).map_err(|e|e.to_string())?;
        let resources = resource_statement
            .query_map(params![id, selected], resource_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        item.content_status = if resources.iter().all(|r| r.status == "downloaded") {
            "archived"
        } else {
            "missing_resources"
        }
        .into();
        item.snippet = excerpt(&body_text, &[]);
        Ok(DocumentDetail {
            item,
            canonical_url,
            body_html,
            body_text,
            revision_id: selected,
            revisions,
            resources,
        })
    }
    pub(crate) fn set_document_state(
        &self,
        id: i64,
        is_read: Option<bool>,
        saved: Option<bool>,
    ) -> Result<(), String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let was_saved = if saved.is_some() {
            tx.query_row(
                "SELECT saved FROM document_state WHERE document_id=?1",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("보관된 항목을 찾을 수 없습니다.")?
        } else {
            false
        };
        let saved_changed = saved.is_some_and(|value| value != was_saved);
        let activity_sequence = if saved_changed {
            Some(super::personalization::next_activity_sequence(&tx)?)
        } else {
            None
        };
        require_changed(
            tx.execute(
                &format!(
                    "UPDATE document_state SET
                     is_read=COALESCE(?2,is_read),
                     saved=COALESCE(?3,saved),
                     read_at=CASE WHEN ?2 THEN {NOW} WHEN ?2=0 THEN NULL ELSE read_at END,
                     saved_at=CASE WHEN ?3=1 AND saved=0 THEN {NOW}
                                   WHEN ?3=0 THEN NULL ELSE saved_at END
                     WHERE document_id=?1"
                ),
                params![id, is_read, saved],
            )
            .map_err(|e| e.to_string())?,
        )?;
        if saved == Some(false) {
            tx.execute(
                "DELETE FROM personalization_evidence WHERE document_id=?1 AND kind='saved'",
                [id],
            )
            .map_err(|e| e.to_string())?;
        } else if saved == Some(true) && !was_saved {
            let learning_enabled: bool = tx
                .query_row(
                    "SELECT learning_enabled FROM personalization_profile WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if learning_enabled {
                let (title, category): (String, Option<String>) = tx
                    .query_row(
                        "SELECT title,category FROM documents WHERE id=?1",
                        [id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|e| e.to_string())?;
                let sequence = activity_sequence
                    .ok_or_else(|| "저장 활동 순서를 확인할 수 없습니다.".to_string())?;
                super::personalization::insert_evidence(
                    &tx,
                    id,
                    "saved",
                    sequence,
                    &title,
                    category.as_deref(),
                )?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        if saved_changed {
            self.recompute_learned()?;
        }
        Ok(())
    }
    pub(crate) fn resource(&self, id: i64) -> Result<ResourceRecord, String> {
        self.connection
            .query_row(
                &format!("SELECT {RESOURCE_COLUMNS} FROM resources WHERE id=?1"),
                [id],
                resource_row,
            )
            .map_err(|_| "보관된 첨부 자료를 찾을 수 없습니다.".into())
    }
    pub(crate) fn pending_resources(
        &self,
        document_id: i64,
    ) -> Result<Vec<ResourceRecord>, String> {
        let mut statement=self.connection.prepare(&format!("SELECT {RESOURCE_COLUMNS} FROM resources WHERE document_id=?1 AND revision_id=(SELECT MAX(id) FROM document_revisions WHERE document_id=?1) AND status<>'downloaded' ORDER BY id")).map_err(|e|e.to_string())?;
        let result = statement
            .query_map([document_id], resource_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        result
    }
    /// Inserts one resource row for deterministic tests, outside the sync path.
    #[cfg(test)]
    pub(crate) fn add_resource_for_test(
        &self,
        document_id: i64,
        revision_id: i64,
        url: &str,
        filename: &str,
        kind: &str,
    ) -> Result<i64, String> {
        self.connection
            .execute(
                "INSERT INTO resources(document_id,revision_id,url,filename,kind) VALUES(?1,?2,?3,?4,?5)",
                params![document_id, revision_id, url, filename, kind],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.connection.last_insert_rowid())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resource_result(
        &self,
        id: i64,
        status: &str,
        mime: Option<&str>,
        bytes: Option<u64>,
        local_path: Option<&str>,
        hash: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), String> {
        if ![
            "pending",
            "downloaded",
            "failed",
            "too_large",
            "unsupported",
        ]
        .contains(&status)
            || bytes.is_some_and(|n| n > i64::MAX as u64)
            || (status == "downloaded" && (local_path.is_none() || hash.is_none()))
            || local_path.is_some_and(|path| {
                path.is_empty()
                    || path.contains(['\\', ':'])
                    || std::path::Path::new(path).is_absolute()
                    || std::path::Path::new(path)
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
            })
        {
            return Err("첨부 자료의 저장 상태나 경로를 확인해 주세요.".into());
        }
        require_changed(self.connection.execute("UPDATE resources SET status=?2,mime=?3,bytes=?4,local_path=?5,sha256=?6,error=?7 WHERE id=?1",params![id,status,mime,bytes.map(|value|value as i64),local_path,hash,error]).map_err(|e|e.to_string())?)
    }
}
