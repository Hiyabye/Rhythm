//! Sources and subscriptions, archive lists and search, the reader, 휴지통,
//! Cybercampus jobs and local resources.

use super::App;
use crate::{
    archive::{
        app_collected, catalog,
        cybercampus::client::Reqwest,
        model::{
            ArchivePage, ArchiveQuery, ArchiveSummary, DocumentDetail, SourceGroup, SourceInfo,
            SubscribeResult, SyncMode, SyncStatus,
        },
        sync::{self, with_store},
    },
    storage::notice_dates::NoticeDate,
};
use base64::Engine;
use std::{path::PathBuf, sync::Arc};
use url::Url;

/// The largest local image the reader receives inline.
const IMAGE_LIMIT: usize = 10 * 1024 * 1024;

pub struct Archive<'a>(pub(super) &'a App);

impl Archive<'_> {
    pub async fn sources(&self) -> Result<Vec<SourceInfo>, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.sources())
    }

    pub async fn subscribe(&self, id: &str, subscribed: bool) -> Result<SubscribeResult, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        with_store(&app.store, |s| {
            if !s.source_extension_enabled(id)? {
                return Err("이 정보원의 확장 기능을 먼저 켜 주세요.".into());
            }
            s.subscribe(id, subscribed)?;
            // Under the store lock, as job admission, so a running job and this
            // choice agree on whether it collects the source.
            if !subscribed {
                app.archive.cancel_source(id);
            }
            Ok(SubscribeResult {
                sources: s.sources()?,
                groups: s.source_groups()?,
            })
        })
    }

    pub async fn source_groups(&self) -> Result<Vec<SourceGroup>, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.source_groups())
    }

    /// Turns a settings group of public boards on or off.
    pub async fn set_group(&self, id: &str, enabled: bool) -> Result<SubscribeResult, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            if !s.enabled(crate::extensions::NOTICES)? {
                return Err("이 정보원의 확장 기능을 먼저 켜 주세요.".into());
            }
            s.set_source_group(id, enabled)?;
            Ok(SubscribeResult {
                sources: s.sources()?,
                groups: s.source_groups()?,
            })
        })
    }

    pub async fn summary(&self) -> Result<ArchiveSummary, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.archive_summary())
    }

    pub async fn query(&self, query: ArchiveQuery) -> Result<ArchivePage, String> {
        let store = self.0.store.clone();
        let _lease = self.0.operation_gate.shared().await?;
        tokio::task::spawn_blocking(move || with_store(&store, |s| s.archive_query(&query)))
            .await
            .map_err(|_| "로컬 검색 작업을 완료하지 못했습니다.")?
    }

    /// Relevance-ordered search. When the local embedding model can embed the
    /// query, notices with a similar meaning join the text matches; otherwise
    /// the ranking is text-only. The query never leaves the device.
    pub async fn ranked_query(&self, query: ArchiveQuery) -> Result<ArchivePage, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        crate::storage::search_terms(&query.query)?;
        let semantic = app
            .classifier
            .query_vector(&app.store, &app.models, &app.data_root, &query.query)
            .await;
        let store = app.store.clone();
        tokio::task::spawn_blocking(move || {
            with_store(&store, |s| {
                s.archive_ranked_query(
                    &query,
                    semantic
                        .as_ref()
                        .map(|(identity, vector)| (identity.as_str(), vector.as_slice())),
                )
            })
        })
        .await
        .map_err(|_| "로컬 검색 작업을 완료하지 못했습니다.")?
    }

    pub async fn document(
        &self,
        id: i64,
        revision_id: Option<i64>,
    ) -> Result<DocumentDetail, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.document(id, revision_id))
    }

    /// The dates found in one notice's text, for its reader. The reader is
    /// core, so no extension gates them.
    pub async fn document_dates(&self, id: i64) -> Result<Vec<NoticeDate>, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.notice_dates(id))
    }

    /// Fetches the body of an opened feed notice from the school; the reader
    /// shows the feed's excerpt meanwhile. Returns whether the school's text
    /// differs from the revision the feed described.
    pub async fn fetch_body(&self, id: i64) -> Result<bool, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        app.archive
            .bodies
            .fetch(&app.store, &app.data_root, &app.network, id)
            .await
    }

    /// Downloads the images and attachments of an opened notice that were not
    /// tried yet. The reader calls it once the body is here and reloads after.
    pub async fn fetch_resources(&self, id: i64) -> Result<(), String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        app.archive
            .bodies
            .resources(&app.store, &app.data_root, &app.network, id)
            .await
    }

    pub async fn set_state(
        &self,
        id: i64,
        is_read: Option<bool>,
        saved: Option<bool>,
    ) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.set_document_state(id, is_read, saved))
    }

    pub async fn trash(&self, ids: &[i64]) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.trash_documents(ids))
    }

    pub async fn restore(&self, ids: &[i64]) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.restore_documents(ids))
    }

    /// Permanently deletes the given trashed notices, or all of 휴지통
    /// without `ids`.
    pub async fn purge(&self, ids: Option<&[i64]>) -> Result<(), String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        let orphaned = with_store(&app.store, |s| s.purge_trashed(ids))?;
        crate::archive::remove_orphans(&app.data_root, orphaned).await;
        Ok(())
    }

    /// Starts a collection job. Only app-collected sources (Cybercampus) may be
    /// named; none means all of them. Public boards arrive through the feed.
    pub async fn start(
        &self,
        source_ids: Option<Vec<String>>,
        mode: SyncMode,
    ) -> Result<SyncStatus, String> {
        let app = self.0;
        let source_ids = collected_ids(source_ids)?;
        let lease = app.operation_gate.shared().await?;
        app.archive
            .start_tracked_admitted(
                app.store.clone(),
                app.data_root.clone(),
                Arc::new(Reqwest::new(app.user_agent)),
                Some(source_ids),
                mode,
                lease,
            )
            .map(|(status, _)| status)
    }

    pub async fn cancel(&self) -> Result<SyncStatus, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.0.archive.cancel();
        self.0.archive.status()
    }

    pub async fn status(&self) -> Result<SyncStatus, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.0.archive.status()
    }

    pub async fn retry_resource(&self, id: i64) -> Result<SyncStatus, String> {
        let app = self.0;
        let lease = app.operation_gate.shared().await?;
        app.archive.retry_admitted(
            app.store.clone(),
            app.data_root.clone(),
            app.network.clone(),
            Arc::new(Reqwest::new(app.user_agent)),
            id,
            lease,
        )
    }

    /// A downloaded image as a `data:` URI, after its hash and raster bytes
    /// are checked.
    pub async fn resource_image(&self, id: i64) -> Result<String, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        let (_, bytes) = sync::local_resource(&app.store, &app.data_root, id).await?;
        // Released archives may label an inline raster as an attachment when both
        // roles shared its URL. Local integrity, size and raster bytes are the boundary.
        image_data_uri(&bytes)
    }

    /// The path of a downloaded file whose hash matches, for the shell to
    /// reveal in the file manager. Never open or execute it.
    pub async fn resource_to_reveal(&self, id: i64) -> Result<PathBuf, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        let (resource, _) = sync::local_resource(&app.store, &app.data_root, id).await?;
        let local = resource.local_path.ok_or("로컬 자료 경로가 없습니다.")?;
        #[cfg(windows)]
        return crate::archive::windows_reveal::resource_path(&app.data_root, &local);
        #[cfg(not(windows))]
        Ok(app.data_root.join(local))
    }
}

/// Checks a notice's link before the shell opens it in the OS browser: only
/// HTTP(S) with a host and no credentials.
pub fn original_link(url: &str) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|_| "외부 링크 주소가 올바르지 않습니다.")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || url.len() > 8192
    {
        return Err("HTTP 또는 HTTPS 외부 링크만 열 수 있습니다.".into());
    }
    Ok(parsed)
}

/// The sources a start request may collect: every request names only
/// app-collected sources, and none means all of them.
fn collected_ids(ids: Option<Vec<String>>) -> Result<Vec<String>, String> {
    match ids {
        None => Ok(vec![catalog::CYBERCAMPUS_SOURCE_ID.to_string()]),
        Some(ids) if ids.iter().all(|id| app_collected(id)) => Ok(ids),
        Some(_) => Err(
            "학교 게시판 공지는 공지 피드로 받습니다. 공지 피드의 지금 받기를 눌러 주세요.".into(),
        ),
    }
}

fn image_data_uri(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > IMAGE_LIMIT {
        return Err("이 자료는 지원되는 로컬 이미지가 아닙니다.".into());
    }
    let mime = sync::image_mime(bytes).ok_or("안전한 로컬 이미지 형식이 아닙니다.")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_collects_only_cybercampus() {
        assert_eq!(collected_ids(None).unwrap(), ["cybercampus"]);
        assert_eq!(
            collected_ids(Some(vec!["cybercampus".into()])).unwrap(),
            ["cybercampus"]
        );
        for ids in [
            vec!["sogang-academic".to_string()],
            vec!["cybercampus".into(), "computing-updates".into()],
        ] {
            assert!(collected_ids(Some(ids)).unwrap_err().contains("공지 피드"));
        }
    }

    #[test]
    fn verified_raster_images_do_not_depend_on_legacy_resource_role() {
        let png = b"\x89PNG\r\n\x1a\nlocal raster";
        assert!(image_data_uri(png)
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(image_data_uri(b"%PDF-document").is_err());
        assert!(image_data_uri(b"<svg onload=\"alert(1)\"></svg>").is_err());
        assert!(image_data_uri(&vec![0; IMAGE_LIMIT + 1]).is_err());
    }

    #[test]
    fn only_plain_http_links_open() {
        assert!(original_link("https://www.sogang.ac.kr/notice/1").is_ok());
        assert!(original_link("http://example.com/").is_ok());
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:pass@example.com/",
            "mailto:someone@example.com",
            "data:text/html,<p>x</p>",
            "not a url",
        ] {
            assert!(original_link(url).is_err(), "{url}");
        }
        let long = format!("https://example.com/{}", "a".repeat(8192));
        assert!(original_link(&long).is_err());
    }
}
