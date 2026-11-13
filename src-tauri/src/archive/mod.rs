mod catalog;
pub mod cybercampus;
pub(crate) mod dates;
pub mod model;
pub(crate) mod network;
mod parser;
mod pdf;
pub mod sync;
mod tls;
#[cfg(windows)]
mod windows_reveal;

use crate::AppState;
use base64::Engine;
use model::*;
pub use network::Client;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
};
use sync::with_store;
use tauri::{Manager, State};
use url::Url;

/// Parses each distinct CSS selector once. Callers pass literals or values built
/// from a small fixed set, so the cache stays small and its entries live for the
/// whole process.
fn selector(value: &str) -> &'static scraper::Selector {
    static CACHE: LazyLock<Mutex<HashMap<String, &'static scraper::Selector>>> =
        LazyLock::new(Default::default);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = cache.get(value) {
        return found;
    }
    let parsed: &'static scraper::Selector = Box::leak(Box::new(
        scraper::Selector::parse(value).expect("static CSS selector"),
    ));
    cache.insert(value.to_owned(), parsed);
    parsed
}

pub fn definitions() -> Vec<SourceDefinition> {
    catalog::definitions()
}
pub fn groups() -> Vec<SourceGroup> {
    catalog::groups()
}
pub fn client() -> Result<Client, String> {
    network::Client::new()
}

#[tauri::command]
pub async fn archive_sources(state: State<'_, AppState>) -> Result<Vec<SourceInfo>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.sources())
}
#[tauri::command]
pub async fn archive_subscribe(
    state: State<'_, AppState>,
    id: String,
    subscribed: bool,
) -> Result<SubscribeResult, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| {
        if !s.source_extension_enabled(&id)? {
            return Err("이 정보원의 확장 기능을 먼저 켜 주세요.".into());
        }
        s.subscribe(&id, subscribed)?;
        // Under the store lock, as job admission, so a running job and this
        // choice agree on whether it collects the board.
        let joined = if subscribed {
            state.archive.join(s.source(&id)?)
        } else {
            state.archive.cancel_source(&id);
            false
        };
        Ok(SubscribeResult {
            sources: s.sources()?,
            groups: s.source_groups()?,
            joined,
        })
    })
}
#[tauri::command]
pub async fn archive_source_groups(state: State<'_, AppState>) -> Result<Vec<SourceGroup>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.source_groups())
}
/// Turns a settings group on or off. `joined` is whether a running refresh of
/// every board collects a board the group turned back on.
#[tauri::command]
pub async fn archive_set_group(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<SubscribeResult, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| {
        if !s.enabled(crate::extensions::NOTICES)? {
            return Err("이 정보원의 확장 기능을 먼저 켜 주세요.".into());
        }
        // Under the store lock, as `archive_subscribe`, so a running job and
        // the group agree on which boards it collects.
        let mut joined = false;
        for source in s.set_source_group(&id, enabled)? {
            if source.subscribed {
                joined |= state.archive.join(source);
            } else {
                state.archive.cancel_source(&source.definition.id);
            }
        }
        Ok(SubscribeResult {
            sources: s.sources()?,
            groups: s.source_groups()?,
            joined,
        })
    })
}
#[tauri::command]
pub async fn archive_summary(state: State<'_, AppState>) -> Result<ArchiveSummary, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.archive_summary())
}
#[tauri::command]
pub async fn archive_query(
    state: State<'_, AppState>,
    query: ArchiveQuery,
) -> Result<ArchivePage, String> {
    let store = state.store.clone();
    let _lease = state.operation_gate.shared().await?;
    tauri::async_runtime::spawn_blocking(move || with_store(&store, |s| s.archive_query(&query)))
        .await
        .map_err(|_| "로컬 검색 작업을 완료하지 못했습니다.")?
}
/// Relevance-ordered search. When the local embedding model can embed the
/// query, notices with a similar meaning join the text matches; otherwise the
/// ranking is text-only. The query never leaves the device.
#[tauri::command]
pub async fn archive_ranked_query(
    state: State<'_, AppState>,
    query: ArchiveQuery,
) -> Result<ArchivePage, String> {
    let _lease = state.operation_gate.shared().await?;
    crate::storage::search_terms(&query.query)?;
    let semantic = state.classifier.query_vector(&state, &query.query).await;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
#[tauri::command]
pub async fn archive_document(
    state: State<'_, AppState>,
    id: i64,
    revision_id: Option<i64>,
) -> Result<DocumentDetail, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.document(id, revision_id))
}
#[tauri::command]
pub async fn archive_state(
    state: State<'_, AppState>,
    id: i64,
    is_read: Option<bool>,
    saved: Option<bool>,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.set_document_state(id, is_read, saved))
}
#[tauri::command]
pub async fn archive_trash(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.trash_documents(&ids))
}
#[tauri::command]
pub async fn archive_restore(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.restore_documents(&ids))
}
/// Permanently deletes the given trashed notices, or all of 휴지통 without `ids`.
#[tauri::command]
pub async fn archive_purge(
    state: State<'_, AppState>,
    ids: Option<Vec<i64>>,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let orphaned = with_store(&state.store, |s| s.purge_trashed(ids.as_deref()))?;
    // The rows are gone, so a file left behind is only unused disk space.
    for path in orphaned {
        let _ = tokio::fs::remove_file(state.data_root.join(path)).await;
    }
    Ok(())
}
/// Moves old notices to 휴지통 at launch and then hourly while the app is open,
/// so a notice that crosses the chosen age moves without a sync or a settings
/// change. Sync jobs and saving settings also run it.
pub async fn run_auto_trash(app: tauri::AppHandle, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(_lease) = state.operation_gate.shared().await {
                if let Err(error) = with_store(&state.store, |s| s.auto_trash_old()) {
                    eprintln!("Archive: 오래된 공지를 휴지통으로 옮기지 못했습니다: {error}");
                }
            }
        }
        tokio::time::sleep(AUTO_TRASH_WAKE).await;
    }
}
const AUTO_TRASH_WAKE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Extracts dates for notices archived before the dates table existed or by an
/// older extractor, once per launch. New and changed notices are extracted when
/// they are written. Batches release the store between them so commands and
/// sync are not held up.
pub async fn run_notice_dates(app: tauri::AppHandle, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let Ok(lease) = state.operation_gate.shared().await else {
            return;
        };
        match with_store(&state.store, |s| s.refresh_notice_dates(NOTICE_DATES_BATCH)) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                eprintln!("Archive: 공지 날짜를 정리하지 못했습니다: {error}");
                return;
            }
        }
        drop(lease);
        tokio::task::yield_now().await;
    }
}
const NOTICE_DATES_BATCH: i64 = 200;
#[tauri::command]
pub async fn archive_start(
    state: State<'_, AppState>,
    source_ids: Option<Vec<String>>,
    mode: String,
) -> Result<SyncStatus, String> {
    let lease = state.operation_gate.shared().await?;
    state
        .archive
        .start_tracked_admitted(
            state.store.clone(),
            state.data_root.clone(),
            state.network.clone(),
            source_ids,
            mode,
            lease,
        )
        .map(|(status, _)| status)
}
#[tauri::command]
pub async fn archive_cancel(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    state.archive.cancel();
    state.archive.status()
}
#[tauri::command]
pub async fn archive_status(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    state.archive.status()
}
#[tauri::command]
pub async fn archive_retry_resource(
    state: State<'_, AppState>,
    id: i64,
) -> Result<SyncStatus, String> {
    let lease = state.operation_gate.shared().await?;
    state.archive.retry_admitted(
        state.store.clone(),
        state.data_root.clone(),
        state.network.clone(),
        id,
        lease,
    )
}
#[tauri::command]
pub async fn archive_resource_image(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    let _lease = state.operation_gate.shared().await?;
    let (_, bytes) = sync::local_resource(&state.store, &state.data_root, id).await?;
    // Released archives may label an inline raster as an attachment when both
    // roles shared its URL. Local integrity, size and raster bytes are the boundary.
    image_data_uri(&bytes)
}
fn image_data_uri(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > 10 * 1024 * 1024 {
        return Err("이 자료는 지원되는 로컬 이미지가 아닙니다.".into());
    }
    let mime = sync::image_mime(bytes).ok_or("안전한 로컬 이미지 형식이 아닙니다.")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
#[tauri::command]
pub async fn archive_reveal_resource(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let (resource, _) = sync::local_resource(&state.store, &state.data_root, id).await?;
    // Reveal only an integrity-checked archive file; never execute/open its contents.
    #[cfg(windows)]
    {
        let path = windows_reveal::resource_path(
            &state.data_root,
            resource
                .local_path
                .as_deref()
                .ok_or("로컬 자료 경로가 없습니다.")?,
        )?;
        let root = state.data_root.clone();
        tokio::task::spawn_blocking(move || windows_reveal::reveal(&root, &path))
            .await
            .map_err(|_| {
                "저장 폴더를 열지 못했습니다. 데이터 저장 위치를 확인해 주세요.".to_string()
            })?
    }
    #[cfg(not(windows))]
    tauri_plugin_opener::reveal_item_in_dir(
        state
            .data_root
            .join(resource.local_path.ok_or("로컬 자료 경로가 없습니다.")?),
    )
    .map_err(|_| "저장 폴더를 열지 못했습니다. 데이터 저장 위치를 확인해 주세요.".into())
}
#[tauri::command]
pub async fn archive_open_original(url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|_| "외부 링크 주소가 올바르지 않습니다.")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || url.len() > 8192
    {
        return Err("HTTP 또는 HTTPS 외부 링크만 열 수 있습니다.".into());
    }
    tauri_plugin_opener::open_url(parsed.as_str(), None::<&str>)
        .map_err(|_| "기본 브라우저에서 외부 링크를 열지 못했습니다.".into())
}

#[cfg(test)]
mod image_tests {
    use super::*;
    #[test]
    fn verified_raster_images_do_not_depend_on_legacy_resource_role() {
        let png = b"\x89PNG\r\n\x1a\nlocal raster";
        assert!(image_data_uri(png)
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(image_data_uri(b"%PDF-document").is_err());
        assert!(image_data_uri(b"<svg onload=\"alert(1)\"></svg>").is_err());
        assert!(image_data_uri(&vec![0; 10 * 1024 * 1024 + 1]).is_err());
    }
}

#[cfg(test)]
mod selector_tests {
    use super::selector;

    #[test]
    fn a_selector_is_parsed_once_and_shared() {
        let first = selector(".board-list-title a");
        assert!(std::ptr::eq(first, selector(".board-list-title a")));
        assert!(!std::ptr::eq(first, selector(".board-list-date")));
        let built = format!("input[name={}]", "idx");
        assert!(std::ptr::eq(selector(&built), selector("input[name=idx]")));
    }
}
