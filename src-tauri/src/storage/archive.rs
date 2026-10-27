use super::Store;
use crate::archive::model::*;
use rusqlite::{params, params_from_iter, types::Value, OptionalExtension, Row};
use sha2::{Digest, Sha256};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
const SOURCE_COLUMNS: &str = "id,name,organization,description,url,family,supported,subscribed,last_attempt,last_success,last_error,next_page,history_complete,status,discovered,archived,failed";
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
    })
}
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
        result
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
            || (article.body_text.trim().is_empty()
                && article.body_html.trim().is_empty()
                && article.resources.is_empty())
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
    pub(crate) fn archive_query(&self, query: &ArchiveQuery) -> Result<ArchivePage, String> {
        // SQLite LIKE/FTS treat NUL as a terminator even in bound values.
        if query.query.contains('\0') {
            return Err("검색어에 사용할 수 없는 문자가 포함되어 있습니다.".into());
        }
        if query.query.chars().count() > 512 {
            return Err("검색어는 512자 이내로 입력해 주세요.".into());
        }
        let terms: Vec<&str> = query.query.split_whitespace().collect();
        if terms.len() > 32 {
            return Err("검색 단어는 32개 이내로 입력해 주세요.".into());
        }
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
        let indexed: Vec<String> = terms
            .iter()
            .filter(|term| term.chars().count() >= 3)
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect();
        if !indexed.is_empty() {
            filters.push(
                "d.id IN (SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?)".into(),
            );
            values.push(indexed.join(" AND ").into());
        }
        // LIKE is an exact literal-substring check and handles one/two-character Korean queries.
        for term in &terms {
            filters.push("(d.title LIKE ? ESCAPE '\\' OR d.body_text LIKE ? ESCAPE '\\')".into());
            let escaped = format!(
                "%{}%",
                term.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            values.push(escaped.clone().into());
            values.push(escaped.into());
        }
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
        let mut categories_statement=self.connection.prepare("SELECT DISTINCT category FROM documents WHERE category IS NOT NULL AND category<>'' AND (?1 IS NULL OR source_id=?1) ORDER BY category").map_err(|e|e.to_string())?;
        let categories = categories_statement
            .query_map([query.source_id.as_deref()], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(ArchivePage {
            items,
            total,
            categories,
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
                let topics = super::personalization::topics(&title, category.as_deref());
                if !topics.is_empty() {
                    let sequence = activity_sequence
                        .ok_or_else(|| "저장 활동 순서를 확인할 수 없습니다.".to_string())?;
                    for topic in topics {
                        tx.execute(
                            "INSERT OR IGNORE INTO personalization_evidence(topic,document_id,kind,activity_sequence)
                             VALUES(?1,?2,'saved',?3)",
                            params![topic, id, sequence],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
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
