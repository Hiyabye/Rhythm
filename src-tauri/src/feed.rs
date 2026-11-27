//! The feed sync's commands; the sync itself is `rhythm_core::feed`.
pub use rhythm_core::feed::*;
use rhythm_core::storage::{feed::FeedStatus, with_store};

#[tauri::command]
pub async fn feed_status(state: tauri::State<'_, crate::AppState>) -> Result<FeedStatus, String> {
    state.feed.status(&state.store)
}

/// 지금 받기: syncs now, or waits for the sync already running.
#[tauri::command]
pub async fn feed_sync(state: tauri::State<'_, crate::AppState>) -> Result<FeedStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    if !with_store(&state.store, |s| s.enabled(crate::extensions::NOTICES))? {
        return Err("서강대학교 공지 기능이 꺼져 있습니다.".into());
    }
    if !state
        .feed
        .run(
            &state.store,
            &state.data_root,
            &transport::HttpsTransport::new(crate::USER_AGENT),
        )
        .await?
    {
        state.feed.wait().await;
    }
    state.feed.status(&state.store)
}
