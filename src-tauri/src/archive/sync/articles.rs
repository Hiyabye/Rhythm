//! Fetching one notice and committing it to the archive.
use super::*;

impl Controller {
    /// Fetches and archives one notice. Returns whether a new document was created.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn article<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        policy: &Robots,
        definition: &SourceDefinition,
        entry: &ListingEntry,
        existing_id: Option<i64>,
        historical: bool,
        resource_bytes: &mut u64,
    ) -> Result<bool, ArticleError> {
        let validators = with_store(store, |s| s.validators(&definition.id, &entry.external_id))?;
        let url = parser::detail_url(definition, entry)?;
        let response = fetch(
            transport,
            &url,
            validators,
            ARTICLE_LIMIT,
            &self.cancel,
            policy,
            network::Access::Source,
        )
        .await?;
        let (id, inserted) = match response {
            network::Fetch::NotModified => {
                let id = existing_id.ok_or("보관되지 않은 원문의 변경 없음 응답을 받았습니다.")?;
                with_store(store, |s| {
                    ensure_active(&self.cancel)?;
                    s.checked(&definition.id, &entry.external_id)
                })?;
                (id, false)
            }
            network::Fetch::Redirect(_) => {
                return Err("학교 서버 이동 응답을 처리하지 못했습니다.".into())
            }
            network::Fetch::Body {
                bytes,
                etag,
                last_modified,
                mime,
            } => {
                // A different content type is more likely a block or error page than
                // the notice itself, so it stays a transient failure.
                check_document_mime(definition, mime.as_deref(), false)?;
                let permanent = |reason: &str| ArticleError::Permanent {
                    reason: reason.into(),
                    response_sha256: format!("{:x}", Sha256::digest(&bytes)),
                };
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| permanent("공지 본문 문자 인코딩을 지원하지 않습니다."))?;
                let mut article =
                    parser::article(definition, entry, text).map_err(|e| permanent(&e))?;
                let viewer: Vec<String> = parser::viewer_pdf_resources(&article)
                    .into_iter()
                    .map(|r| r.url.clone())
                    .collect();
                // A viewer PDF this notice already holds yields the same body
                // (and content hash) as the archived one.
                if let (Some(id), false) = (existing_id, viewer.is_empty()) {
                    local_pdf_text(store, root, id, &mut article).await?;
                }
                let keep = match existing_id {
                    Some(id) if article.unsupported_body => with_store(store, |s| {
                        if s.has_readable_content(id)? {
                            // Keep the last good content (approved Sogang exception)
                            // unless the viewer now shows a PDF never seen before.
                            viewer.iter().try_fold(true, |all, url| {
                                Ok::<_, String>(all && s.knows_resource(id, url)?)
                            })
                        } else if viewer.is_empty() {
                            Ok(false)
                        } else {
                            // Unchanged except for the newly recognized viewer PDF:
                            // the body fill below adds it to the stored revision.
                            let mut previous = article.clone();
                            previous.resources.retain(|r| !viewer.contains(&r.url));
                            s.content_matches(id, &previous)
                        }
                    })?,
                    _ => false,
                };
                if keep {
                    // The notice was checked, so this is neither a failure nor a skip.
                    let id = existing_id.expect("kept content belongs to a document");
                    with_store(store, |s| {
                        ensure_active(&self.cancel)?;
                        s.checked(&definition.id, &entry.external_id)
                    })?;
                    (id, false)
                } else {
                    article.etag = etag;
                    article.last_modified = last_modified;
                    let (id, _) = with_store(store, |s| {
                        ensure_active(&self.cancel)?;
                        s.upsert_article(&definition.id, &article, historical)
                    })?;
                    (id, existing_id.is_none())
                }
            }
        };
        with_store(store, |s| {
            s.clear_skipped(&definition.id, &entry.external_id)
        })?;
        self.pending_resources(store, root, transport, policy, id, resource_bytes)
            .await?;
        if definition.id != "sogang-academic" || with_store(store, |s| s.has_readable_content(id))?
        {
            return Ok(inserted);
        }
        let download = Download {
            transport,
            policy,
            total: resource_bytes,
        };
        self.fill_pdf_body(store, root, Some(download), id).await?;
        Ok(inserted)
    }
}

/// Transient failures hold the page checkpoint so the next run retries them.
/// A permanent failure belongs to the fetched response and is recorded instead.
pub(super) enum ArticleError {
    Transient(String),
    Permanent {
        reason: String,
        response_sha256: String,
    },
}
impl From<String> for ArticleError {
    fn from(error: String) -> Self {
        Self::Transient(error)
    }
}
impl From<&str> for ArticleError {
    fn from(error: &str) -> Self {
        Self::Transient(error.into())
    }
}
pub(super) fn mark_seed_complete(store: &SharedStore, source: &str) -> Result<(), String> {
    with_store(store, |s| s.mark_seed_complete(source))
}
pub(super) fn persist(
    store: &SharedStore,
    source: &SourceInfo,
    status: &str,
    error: Option<&str>,
    success: bool,
) -> Result<(), String> {
    with_store(store, |s| {
        s.source_progress(
            &source.definition.id,
            status,
            source.next_page,
            source.history_complete,
            source.discovered,
            source.archived,
            source.failed,
            error,
            success,
        )
    })
}
pub(super) fn check_document_mime(
    source: &SourceDefinition,
    mime: Option<&str>,
    listing: bool,
) -> Result<(), String> {
    // The mechanical sitemap is XML; article pages must still be HTML so a wrong
    // content type stays a transient block-page reading.
    let expected = if source.id == "sogang-academic" {
        mime == Some("application/json")
    } else if source.family == "mechanical" && listing {
        matches!(
            mime,
            Some("text/html" | "application/xhtml+xml" | "application/xml" | "text/xml")
        )
    } else {
        matches!(mime, Some("text/html" | "application/xhtml+xml"))
    };
    if !expected {
        return Err("학교 서버가 예상한 공지 형식 대신 다른 자료를 반환했습니다.".into());
    }
    Ok(())
}
pub(super) async fn fetch<T: Transport>(
    transport: &T,
    url: &Url,
    validators: Option<(Option<String>, Option<String>)>,
    limit: usize,
    cancel: &AtomicBool,
    policy: &Robots,
    access: network::Access,
) -> Result<network::Fetch, String> {
    let mut target = url.clone();
    for redirects in 0..=3 {
        // Apply robots to every hop before any request, not only the original URL.
        // Origin equality also prevents sending validators to another source host.
        if !policy.permits(&target) {
            return Err("학교의 robots.txt 정책에 따라 수집을 중단했습니다.".into());
        }
        if policy.delay > 0.65 {
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs_f64(policy.delay))=>{},_=wait_cancel(cancel)=>return Err("동기화를 중단했습니다.".into())}
        }
        match transport
            .fetch(&target, validators.clone(), limit, cancel, access)
            .await?
        {
            network::Fetch::Redirect(next) => {
                if redirects == 3 {
                    return Err("학교 서버의 이동 경로가 너무 많습니다.".into());
                }
                network::validate(&next, access)?;
                target = next;
            }
            response => return Ok(response),
        }
    }
    unreachable!()
}
pub(super) async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
