pub mod cybercampus;
#[cfg(windows)]
use rhythm_core::archive::windows_reveal;
pub use rhythm_core::archive::{model, sync};
#[cfg(test)]
pub use rhythm_core::archive::{network, Client};

use crate::AppState;
use model::*;
use tauri::State;

#[cfg(test)]
pub fn client() -> Result<Client, String> {
    network::Client::new(concat!(
        "Rhythm/",
        env!("CARGO_PKG_VERSION"),
        " (local university notice archive)"
    ))
}

#[tauri::command]
pub async fn archive_sources(state: State<'_, AppState>) -> Result<Vec<SourceInfo>, String> {
    state.archive().sources().await
}
#[tauri::command]
pub async fn archive_subscribe(
    state: State<'_, AppState>,
    id: String,
    subscribed: bool,
) -> Result<SubscribeResult, String> {
    state.archive().subscribe(&id, subscribed).await
}
#[tauri::command]
pub async fn archive_source_groups(state: State<'_, AppState>) -> Result<Vec<SourceGroup>, String> {
    state.archive().source_groups().await
}
#[tauri::command]
pub async fn archive_set_group(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<SubscribeResult, String> {
    state.archive().set_group(&id, enabled).await
}
#[tauri::command]
pub async fn archive_summary(state: State<'_, AppState>) -> Result<ArchiveSummary, String> {
    state.archive().summary().await
}
#[tauri::command]
pub async fn archive_query(
    state: State<'_, AppState>,
    query: ArchiveQuery,
) -> Result<ArchivePage, String> {
    state.archive().query(query).await
}
#[tauri::command]
pub async fn archive_ranked_query(
    state: State<'_, AppState>,
    query: ArchiveQuery,
) -> Result<ArchivePage, String> {
    state.archive().ranked_query(query).await
}
#[tauri::command]
pub async fn archive_document(
    state: State<'_, AppState>,
    id: i64,
    revision_id: Option<i64>,
) -> Result<DocumentDetail, String> {
    state.archive().document(id, revision_id).await
}
#[tauri::command]
pub async fn archive_fetch_body(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    state.archive().fetch_body(id).await
}
#[tauri::command]
pub async fn archive_fetch_resources(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.archive().fetch_resources(id).await
}
#[tauri::command]
pub async fn archive_state(
    state: State<'_, AppState>,
    id: i64,
    is_read: Option<bool>,
    saved: Option<bool>,
) -> Result<(), String> {
    state.archive().set_state(id, is_read, saved).await
}
#[tauri::command]
pub async fn archive_trash(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    state.archive().trash(&ids).await
}
#[tauri::command]
pub async fn archive_restore(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    state.archive().restore(&ids).await
}
#[tauri::command]
pub async fn archive_purge(
    state: State<'_, AppState>,
    ids: Option<Vec<i64>>,
) -> Result<(), String> {
    state.archive().purge(ids.as_deref()).await
}
#[tauri::command]
pub async fn archive_start(
    state: State<'_, AppState>,
    source_ids: Option<Vec<String>>,
    mode: String,
) -> Result<SyncStatus, String> {
    // A malformed mode is refused here with the controller's message.
    state.archive().start(source_ids, mode.parse()?).await
}
#[tauri::command]
pub async fn archive_cancel(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    state.archive().cancel().await
}
#[tauri::command]
pub async fn archive_status(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    state.archive().status().await
}
#[tauri::command]
pub async fn archive_retry_resource(
    state: State<'_, AppState>,
    id: i64,
) -> Result<SyncStatus, String> {
    state.archive().retry_resource(id).await
}
#[tauri::command]
pub async fn archive_resource_image(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    state.archive().resource_image(id).await
}
/// Reveals an integrity-checked archive file in the file manager; never
/// executes or opens its contents.
#[tauri::command]
pub async fn archive_reveal_resource(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let path = state.archive().resource_to_reveal(id).await?;
    #[cfg(windows)]
    {
        let root = state.data_root.clone();
        tokio::task::spawn_blocking(move || windows_reveal::reveal(&root, &path))
            .await
            .map_err(|_| {
                "저장 폴더를 열지 못했습니다. 데이터 저장 위치를 확인해 주세요.".to_string()
            })?
    }
    #[cfg(not(windows))]
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|_| "저장 폴더를 열지 못했습니다. 데이터 저장 위치를 확인해 주세요.".into())
}
/// Opens a notice's original page in the OS browser after core checks it.
#[tauri::command]
pub async fn archive_open_original(url: String) -> Result<(), String> {
    let parsed = rhythm_core::app::original_link(&url)?;
    tauri_plugin_opener::open_url(parsed.as_str(), None::<&str>)
        .map_err(|_| "기본 브라우저에서 외부 링크를 열지 못했습니다.".into())
}
