//! Main-board notices whose whole body is the school's PDF viewer: the viewer
//! PDF is archived as an attachment and its text layer becomes the body.
use super::*;

/// Fills an archived notice's empty body from the text of its downloaded
/// viewer PDFs, re-parsing the stored response so older notices gain the PDF
/// reference. Used after an explicit retry downloads such a PDF.
pub(super) async fn fill_pdf_body(
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    id: i64,
) -> Result<(), String> {
    let (source, entry, original) = with_store(store, |s| {
        let (source, entry) = s.document_entry(id)?;
        Ok((s.source(&source)?.definition, entry, s.latest_original(id)?))
    })?;
    let Ok(mut article) = parser::article(&source, &entry, &original) else {
        return Ok(());
    };
    if parser::viewer_pdf_resources(&article).is_empty() {
        return Ok(());
    }
    with_store(store, |s| {
        ensure_active(cancel)?;
        s.attach_resources(id, &article.resources)
    })?;
    local_pdf_text(store, root, id, &mut article).await?;
    if !article.unsupported_body {
        with_store(store, |s| {
            ensure_active(cancel)?;
            if !s.fill_body(id, &article)? {
                s.upsert_article(&source.id, &article, false)?;
            }
            Ok(())
        })?;
    }
    Ok(())
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
