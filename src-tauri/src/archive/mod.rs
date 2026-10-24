mod catalog;
pub mod model;
mod network;
mod parser;
pub mod sync;
mod tls;

use crate::AppState;
use base64::Engine;
use model::*;
pub use network::Client;
use sync::with_store;
use tauri::State;
use url::Url;

pub fn definitions() -> Vec<SourceDefinition> {
    catalog::definitions()
}
pub fn client() -> Result<Client, String> {
    network::Client::new()
}

#[tauri::command]
pub async fn archive_sources(state: State<'_, AppState>) -> Result<Vec<SourceInfo>, String> {
    with_store(&state.store, |s| s.sources())
}
#[tauri::command]
pub async fn archive_subscribe(
    state: State<'_, AppState>,
    id: String,
    subscribed: bool,
) -> Result<Vec<SourceInfo>, String> {
    with_store(&state.store, |s| {
        if !s.enabled("sogang_notices")? {
            return Err("서강대학교 공지 확장 기능을 먼저 켜 주세요.".into());
        }
        s.subscribe(&id, subscribed)?;
        if !subscribed {
            state.archive.cancel();
        }
        s.sources()
    })
}
#[tauri::command]
pub async fn archive_summary(state: State<'_, AppState>) -> Result<ArchiveSummary, String> {
    with_store(&state.store, |s| s.archive_summary())
}
#[tauri::command]
pub async fn archive_query(
    state: State<'_, AppState>,
    query: ArchiveQuery,
) -> Result<ArchivePage, String> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || with_store(&store, |s| s.archive_query(&query)))
        .await
        .map_err(|_| "로컬 검색 작업을 완료하지 못했습니다.")?
}
#[tauri::command]
pub async fn archive_document(
    state: State<'_, AppState>,
    id: i64,
    revision_id: Option<i64>,
) -> Result<DocumentDetail, String> {
    with_store(&state.store, |s| s.document(id, revision_id))
}
#[tauri::command]
pub async fn archive_state(
    state: State<'_, AppState>,
    id: i64,
    is_read: Option<bool>,
    saved: Option<bool>,
) -> Result<(), String> {
    with_store(&state.store, |s| s.set_document_state(id, is_read, saved))
}
#[tauri::command]
pub async fn archive_start(
    state: State<'_, AppState>,
    source_ids: Option<Vec<String>>,
    mode: String,
) -> Result<SyncStatus, String> {
    state.archive.start(
        state.store.clone(),
        state.data_root.clone(),
        state.network.clone(),
        source_ids,
        mode,
    )
}
#[tauri::command]
pub async fn archive_cancel(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    state.archive.cancel();
    state.archive.status()
}
#[tauri::command]
pub async fn archive_status(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    state.archive.status()
}
#[tauri::command]
pub async fn archive_retry_resource(
    state: State<'_, AppState>,
    id: i64,
) -> Result<SyncStatus, String> {
    state.archive.retry(
        state.store.clone(),
        state.data_root.clone(),
        state.network.clone(),
        id,
    )
}
#[tauri::command]
pub async fn archive_resource_image(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    let (resource, bytes) = sync::local_resource(&state.store, &state.data_root, id).await?;
    if resource.kind != "image" || bytes.len() > 10 * 1024 * 1024 {
        return Err("이 자료는 지원되는 로컬 이미지가 아닙니다.".into());
    }
    let mime = sync::image_mime(&bytes).ok_or("안전한 로컬 이미지 형식이 아닙니다.")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
#[tauri::command]
pub async fn archive_reveal_resource(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let (resource, _) = sync::local_resource(&state.store, &state.data_root, id).await?;
    // Reveal only an integrity-checked archive file; never execute/open its contents.
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
