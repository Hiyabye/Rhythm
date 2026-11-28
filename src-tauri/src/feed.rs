//! The feed sync's commands; wrappers over `App::feed()`.
use rhythm_core::storage::feed::FeedStatus;

#[tauri::command]
pub async fn feed_status(state: tauri::State<'_, crate::AppState>) -> Result<FeedStatus, String> {
    state.feed().status().await
}

/// 지금 받기: syncs now, or waits for the sync already running.
#[tauri::command]
pub async fn feed_sync(state: tauri::State<'_, crate::AppState>) -> Result<FeedStatus, String> {
    state.feed().sync().await
}
