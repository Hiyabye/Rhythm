//! The 열람실 commands: one-line wrappers over `App::studium()`. The mini
//! window's own controls are in `views.rs`.
use super::model::{AlertRuleInput, Snapshot};
use crate::AppState;

#[tauri::command]
pub(crate) async fn studium_save_alert(
    state: tauri::State<'_, AppState>,
    room_id: String,
    enabled: bool,
    kind: String,
    minimum: u32,
) -> Result<Snapshot, String> {
    let rule = AlertRuleInput {
        enabled,
        kind,
        minimum,
    };
    state.studium().save_alert(&room_id, &rule).await
}

#[tauri::command]
pub(crate) async fn studium_notification_activation(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    state.studium().notification_activation().await
}

#[tauri::command]
pub(crate) async fn studium_save_favorites(
    state: tauri::State<'_, AppState>,
    room_id: String,
    seat_numbers: Vec<u32>,
) -> Result<Snapshot, String> {
    state
        .studium()
        .save_favorites(&room_id, &seat_numbers)
        .await
}

#[tauri::command]
pub(crate) async fn studium_save_interval(
    state: tauri::State<'_, AppState>,
    interval_minutes: u32,
) -> Result<Snapshot, String> {
    state.studium().save_interval(interval_minutes).await
}

#[tauri::command]
pub(crate) async fn studium_automatic_start(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    state.studium().automatic_start().await
}

#[tauri::command]
pub(crate) async fn studium_automatic_pause(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    state.studium().automatic_pause().await
}

#[tauri::command]
pub(crate) async fn studium_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    state.studium().snapshot().await
}

/// The same read as `studium_snapshot`; windows poll it while a check runs.
#[tauri::command]
pub(crate) async fn studium_status(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    state.studium().snapshot().await
}

#[tauri::command]
pub(crate) async fn studium_save_preferences(
    state: tauri::State<'_, AppState>,
    room_ids: Vec<String>,
    view_room: String,
) -> Result<Snapshot, String> {
    state
        .studium()
        .save_preferences(&room_ids, &view_room)
        .await
}

#[tauri::command]
pub(crate) async fn studium_set_view_room(
    state: tauri::State<'_, AppState>,
    room_id: String,
) -> Result<Snapshot, String> {
    state.studium().set_view_room(&room_id).await
}

#[tauri::command]
pub(crate) async fn studium_start(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    state.studium().start().await
}

#[tauri::command]
pub(crate) async fn studium_cancel(
    state: tauri::State<'_, AppState>,
    job_id: u64,
) -> Result<Snapshot, String> {
    state.studium().cancel(job_id).await
}
