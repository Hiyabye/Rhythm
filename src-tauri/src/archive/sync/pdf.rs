//! Main-board notices whose whole body is the school's PDF viewer: the viewer
//! PDF is archived as an attachment and its text layer becomes the body.
use super::*;

/// Viewer PDFs of older archived notices one job may download.
const BACKFILL_DOWNLOADS: usize = 10;

/// The network a body fill may use to download a missing viewer PDF.
pub(super) struct Download<'a, T> {
    pub(super) transport: &'a T,
    pub(super) policy: &'a Robots,
    pub(super) total: &'a AtomicU64,
}

impl Board<'_> {
    /// Fills an archived notice's empty body from the text of its viewer PDFs,
    /// re-parsing the stored response so older notices gain the PDF reference.
    /// Missing PDFs are downloaded only with `download`. Returns whether a
    /// download was attempted.
    pub(super) async fn fill_pdf_body<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        download: Option<Download<'_, T>>,
        id: i64,
    ) -> Result<bool, String> {
        let (source, entry, original) = with_store(store, |s| {
            let (source, entry) = s.document_entry(id)?;
            Ok((s.source(&source)?.definition, entry, s.latest_original(id)?))
        })?;
        let Ok(mut article) = parser::article(&source, &entry, &original) else {
            return Ok(false);
        };
        let viewer: Vec<String> = parser::viewer_pdf_resources(&article)
            .into_iter()
            .map(|r| r.url.clone())
            .collect();
        if viewer.is_empty() {
            return Ok(false);
        }
        let pending = with_store(store, |s| {
            ensure_active(self.cancel)?;
            s.attach_resources(id, &article.resources)?;
            Ok(s.pending_resources(id)?
                .iter()
                .any(|r| r.status == "pending" && viewer.contains(&r.url)))
        })?;
        let downloaded = pending && download.is_some();
        if let (true, Some(download)) = (pending, download) {
            self.pending_resources(
                store,
                root,
                download.transport,
                download.policy,
                id,
                download.total,
            )
            .await?;
        }
        local_pdf_text(store, root, id, &mut article).await?;
        if !article.unsupported_body {
            with_store(store, |s| {
                ensure_active(self.cancel)?;
                if !s.fill_body(id, &article)? {
                    s.upsert_article(&source.id, &article, false)?;
                }
                Ok(())
            })?;
        }
        Ok(downloaded)
    }
    /// Fills older notices archived before viewer PDFs were recognized, or
    /// whose PDF arrived later through a retry or the bulk download.
    pub(super) async fn backfill_pdf_bodies<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        definition: &SourceDefinition,
        total: &AtomicU64,
    ) -> Result<(), String> {
        if definition.id != "sogang-academic" {
            return Ok(());
        }
        let candidates = with_store(store, |s| s.pdf_viewer_candidates(&definition.id))?;
        if candidates.is_empty() {
            return Ok(());
        }
        let policy = Robots::load(
            transport,
            &parser::list_url(definition, 1)?,
            self.cancel,
            network::Access::Source,
        )
        .await?;
        let mut downloads = 0;
        for (id, _) in candidates {
            active(self.cancel)?;
            let download = if downloads < BACKFILL_DOWNLOADS
                && total.load(Ordering::Acquire) < IMPORT_RESOURCES
            {
                Some(Download {
                    transport,
                    policy: &policy,
                    total,
                })
            } else {
                None
            };
            if self.fill_pdf_body(store, root, download, id).await? {
                downloads += 1;
            }
        }
        Ok(())
    }
}

/// Makes the text of the document's downloaded viewer PDFs the article body.
/// Local copies are verified first; a missing or textless PDF adds nothing.
pub(super) async fn local_pdf_text(
    store: &SharedStore,
    root: &Path,
    id: i64,
    article: &mut ArticleInput,
) -> Result<(), String> {
    let mut texts = Vec::new();
    for resource in parser::viewer_pdf_resources(article) {
        let Some(local) = with_store(store, |s| s.downloaded_attachment(id, &resource.url))? else {
            continue;
        };
        let Ok((_, bytes)) = local_resource(store, root, local).await else {
            continue;
        };
        let text = tokio::task::spawn_blocking(move || super::super::pdf::text(&bytes))
            .await
            .map_err(|_| "PDF 본문을 읽지 못했습니다.")?;
        if let Some(text) = text {
            texts.push((resource.filename.clone(), text));
        }
    }
    parser::with_pdf_text(article, &texts);
    Ok(())
}
