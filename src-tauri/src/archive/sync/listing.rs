//! Listing discovery: paging through a board, the incremental frontier,
//! the manual refresh and the skipped-notice and missing-resource retries.
use super::*;

impl Controller {
    /// Runs one board's manual refresh: new notices and recent edits first, then
    /// the unfinished backlog. Each pass persists its own progress, so a failure
    /// or cancellation between them keeps what the first pass saved.
    #[allow(clippy::too_many_arguments)] // Shares the job's transport and resource budget.
    pub(super) async fn refresh<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        current: &mut SourceInfo,
        budget: &Budget,
        resource_bytes: &mut u64,
    ) -> Result<(), String> {
        self.source(
            store,
            root,
            transport,
            current,
            "incremental",
            budget,
            resource_bytes,
        )
        .await?;
        if current.history_complete {
            return Ok(());
        }
        active(&self.cancel)?;
        self.source(
            store,
            root,
            transport,
            current,
            "historical",
            budget,
            resource_bytes,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)] // One source run shares a bounded transport and resource budget.
    pub(super) async fn source<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        current: &mut SourceInfo,
        mode: &str,
        budget: &Budget,
        resource_bytes: &mut u64,
    ) -> Result<(), String> {
        if mode == "skipped" {
            let definition = current.definition.clone();
            return self
                .retry_skipped(store, root, transport, &definition, resource_bytes)
                .await;
        }
        if mode == "resources" {
            let definition = current.definition.clone();
            return self
                .download_missing(store, root, transport, &definition, resource_bytes)
                .await;
        }
        let historical = mode == "historical";
        // The durable flag, not the current document count, decides whether the
        // initial bootstrap is still owed. A cancelled or failed seed leaves it
        // unset, so later incremental runs keep marking historical pages as
        // historical instead of reporting them as newly published notices.
        let seed = !historical && !with_store(store, |s| s.seed_complete(&current.definition.id))?;
        if historical && current.history_complete {
            persist(store, current, "completed", None, true)?;
            return Ok(());
        }
        let first_page = if historical { current.next_page } else { 1 };
        let definition = &current.definition.clone();
        let initial = parser::list_url(definition, first_page)?;
        let policy =
            Robots::load(transport, &initial, &self.cancel, network::Access::Source).await?;
        let mut seen = HashSet::new();
        let mut page_hashes = HashSet::new();
        let mut page = first_page;
        let mut processed = 0;
        loop {
            active(&self.cancel)?;
            if page > 100_000 {
                persist(
                    store,
                    current,
                    "limited",
                    Some(
                        "안전한 페이지 수 제한에 도달했습니다. 전체 이력 완료로 표시하지 않습니다.",
                    ),
                    false,
                )?;
                return Ok(());
            }
            self.update(
                Some(definition.id.clone()),
                mode,
                page,
                "이번 동기화의 목록과 원문을 보관하고 있습니다.",
            );
            let url = parser::list_url(definition, page)?;
            let response = fetch(
                transport,
                &url,
                None,
                ARTICLE_LIMIT,
                &self.cancel,
                &policy,
                network::Access::Source,
            )
            .await?;
            let listing = match response {
                network::Fetch::Body { bytes, mime, .. } => {
                    check_document_mime(definition, mime.as_deref(), true)?;
                    parser::listing(
                        definition,
                        page,
                        std::str::from_utf8(&bytes)
                            .map_err(|_| "공지 목록 문자 인코딩을 지원하지 않습니다.")?,
                    )?
                }
                network::Fetch::NotModified | network::Fetch::Redirect(_) => {
                    return Err(
                        "보관되지 않은 목록의 변경 없음 또는 이동 응답을 받았습니다.".into(),
                    )
                }
            };
            let pinned = listing.entries.iter().filter(|entry| entry.pinned).count();
            self.update(
                Some(definition.id.clone()),
                mode,
                page,
                &format!(
                    "{} / {} 페이지 · 고정 공지 {}건 포함 · 원문 보관 중",
                    page, listing.total_pages, pinned
                ),
            );
            let fingerprint = listing
                .entries
                .iter()
                .map(|e| e.external_id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            if listing.has_next && (!page_hashes.insert(fingerprint) || listing.entries.is_empty())
            {
                return Err("학교 목록이 반복되어 가져오기를 중단했습니다. 전체 이력을 완료하지 않았습니다.".into());
            }
            let mut errors = Vec::new();
            let mut known_regular_frontier = false;
            let mut all_regular_known = true;
            for entry in listing.entries {
                active(&self.cancel)?;
                let existing_id =
                    with_store(store, |s| s.document_id(&definition.id, &entry.external_id))?;
                let exists = existing_id.is_some();
                if !entry.pinned {
                    known_regular_frontier = true;
                    all_regular_known &= exists;
                }
                if !seen.insert(entry.external_id.clone()) {
                    continue;
                }
                if historical {
                    if let Some(id) = existing_id {
                        self.pending_resources(store, root, transport, &policy, id, resource_bytes)
                            .await?;
                        continue;
                    }
                }
                // A sitemap-backed listing carries the site's own change time.
                // Skip the request while the archived notice already matches it;
                // pending files still resume so a failed download can recover.
                if let (Some(id), Some(modified_at)) = (existing_id, entry.modified_at.as_deref()) {
                    if with_store(store, |s| {
                        s.listing_unchanged(&definition.id, &entry.external_id, modified_at)
                    })? {
                        active(&self.cancel)?;
                        self.pending_resources(store, root, transport, &policy, id, resource_bytes)
                            .await?;
                        continue;
                    }
                }
                if budget.articles.is_some_and(|max| processed >= max) {
                    if !errors.is_empty() {
                        return Err(format!(
                            "원문 {}건을 보관하지 못했습니다. 같은 페이지에서 이어집니다. {}",
                            errors.len(),
                            errors[0]
                        ));
                    }
                    persist(store,current,"limited",Some("개발 검증용 제한으로 일부만 가져왔습니다. 전체 이력은 아직 완료되지 않았습니다."),false)?;
                    return Ok(());
                }
                if !exists {
                    current.discovered += 1;
                }
                let result = self
                    .article(
                        store,
                        root,
                        transport,
                        &policy,
                        definition,
                        &entry,
                        existing_id,
                        historical || seed,
                        resource_bytes,
                    )
                    .await;
                processed += 1;
                match result {
                    Ok(inserted) => current.archived += u64::from(inserted),
                    Err(ArticleError::Permanent {
                        reason,
                        response_sha256,
                    }) => {
                        // Retrying the same response cannot succeed, so this notice
                        // must not hold the page checkpoint; it stays listed instead.
                        with_store(store, |s| {
                            ensure_active(&self.cancel)?;
                            s.record_skipped(
                                &definition.id,
                                &entry,
                                historical || seed,
                                &reason,
                                &response_sha256,
                            )
                        })?;
                    }
                    Err(ArticleError::Transient(error)) => {
                        active(&self.cancel)?;
                        current.failed += 1;
                        // The server asked to slow down; the rest of the page
                        // would only repeat the refusal.
                        if error == network::RATE_LIMITED {
                            return Err(error);
                        }
                        errors.push(error);
                    }
                }
                persist(store, current, "running", None, false)?;
            }
            // Never checkpoint beyond a page containing transient article failures.
            if !errors.is_empty() {
                return Err(format!(
                    "원문 {}건을 보관하지 못했습니다. 같은 페이지에서 이어집니다. {}",
                    errors.len(),
                    errors[0]
                ));
            }
            if historical {
                current.next_page = page + 1;
            }
            if !listing.has_next {
                if historical {
                    current.history_complete = true;
                }
                // Reaching the last page safely also established the initial
                // frontier, whether through the bounded seed or a manual import.
                if historical || seed {
                    mark_seed_complete(store, &definition.id)?;
                }
                persist(store, current, "completed", None, true)?;
                return Ok(());
            }
            // The three-page edit window must not truncate discovery of a larger
            // burst. Only regular records (never repeated pins) establish a frontier.
            if !historical && page >= 3 && (seed || (known_regular_frontier && all_regular_known)) {
                if seed {
                    mark_seed_complete(store, &definition.id)?;
                }
                persist(store, current, "completed", None, true)?;
                return Ok(());
            }
            if budget.pages.is_some_and(|max| page - first_page + 1 >= max) {
                persist(store,current,"limited",Some("개발 검증용 페이지 제한으로 중단했습니다. 이력은 이어서 가져올 수 있습니다."),false)?;
                return Ok(());
            }
            persist(store, current, "running", None, false)?;
            page += 1;
        }
    }
    /// Downloads the missing files of archived notices, newest first, until the
    /// job budget is spent; files left over stay pending for the next run.
    pub(super) async fn download_missing<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        definition: &SourceDefinition,
        resource_bytes: &mut u64,
    ) -> Result<(), String> {
        let documents = with_store(store, |s| s.documents_missing_resources(&definition.id))?;
        if documents.is_empty() {
            return Ok(());
        }
        let policy = Robots::load(
            transport,
            &parser::list_url(definition, 1)?,
            &self.cancel,
            network::Access::Source,
        )
        .await?;
        for (index, id) in documents.iter().enumerate() {
            active(&self.cancel)?;
            if *resource_bytes >= IMPORT_RESOURCES {
                break;
            }
            self.update(
                Some(definition.id.clone()),
                "resources",
                0,
                &format!(
                    "첨부 자료를 내려받고 있습니다. 공지 {} / {}건",
                    index + 1,
                    documents.len()
                ),
            );
            self.pending_resources(store, root, transport, &policy, *id, resource_bytes)
                .await?;
        }
        Ok(())
    }
    /// Re-fetches only the notices recorded as skipped. The board's sync status,
    /// checkpoint and last success are left as the last sync set them.
    pub(super) async fn retry_skipped<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        definition: &SourceDefinition,
        resource_bytes: &mut u64,
    ) -> Result<(), String> {
        let entries = with_store(store, |s| s.skipped_entries(&definition.id))?;
        if entries.is_empty() {
            return Ok(());
        }
        let policy = Robots::load(
            transport,
            &parser::list_url(definition, 1)?,
            &self.cancel,
            network::Access::Source,
        )
        .await?;
        let mut errors = Vec::new();
        for (index, (entry, historical)) in entries.iter().enumerate() {
            active(&self.cancel)?;
            self.update(
                Some(definition.id.clone()),
                "skipped",
                0,
                &format!(
                    "가져오지 못한 공지를 다시 확인하고 있습니다. {} / {}건",
                    index + 1,
                    entries.len()
                ),
            );
            let existing_id =
                with_store(store, |s| s.document_id(&definition.id, &entry.external_id))?;
            match self
                .article(
                    store,
                    root,
                    transport,
                    &policy,
                    definition,
                    entry,
                    existing_id,
                    *historical,
                    resource_bytes,
                )
                .await
            {
                Ok(_) => {}
                Err(ArticleError::Permanent {
                    reason,
                    response_sha256,
                }) => {
                    with_store(store, |s| {
                        ensure_active(&self.cancel)?;
                        s.record_skipped(
                            &definition.id,
                            entry,
                            *historical,
                            &reason,
                            &response_sha256,
                        )
                    })?;
                }
                Err(ArticleError::Transient(error)) => {
                    active(&self.cancel)?;
                    if error == network::RATE_LIMITED {
                        return Err(error);
                    }
                    errors.push(error);
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "가져오지 못한 공지 {}건을 다시 확인하지 못했습니다. {}",
                errors.len(),
                errors[0]
            ))
        }
    }
}
