use super::{
    jobs::{automatic_start_with_transport, snapshot_with_store, start_with_transport},
    model::Snapshot,
};
use crate::{storage::Store, AppState};

fn snapshot(state: &AppState, store: &Store) -> Result<Snapshot, String> {
    snapshot_with_store(&state.studium, store)
}

#[tauri::command]
pub(crate) async fn studium_save_alert(
    state: tauri::State<'_, AppState>,
    room_id: String,
    enabled: bool,
    kind: String,
    minimum: u32,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    store.studium_save_alert_rule(
        &room_id,
        &super::model::AlertRuleInput {
            enabled,
            kind,
            minimum,
        },
    )?;
    // Rule editing invalidates old candidates but never observes old results.
    state.studium_views.refresh();
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_notification_activation(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let _lease = state.operation_gate.shared().await?;
    state.studium_notifications.take_activation()
}

#[tauri::command]
pub(crate) async fn studium_save_favorites(
    state: tauri::State<'_, AppState>,
    room_id: String,
    seat_numbers: Vec<u32>,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    let value = save_favorites_with_store(&state.studium, &store, &room_id, &seat_numbers)?;
    state.studium_views.refresh();
    Ok(value)
}

fn save_favorites_with_store(
    controller: &super::controller::Controller,
    store: &Store,
    room_id: &str,
    seat_numbers: &[u32],
) -> Result<Snapshot, String> {
    store.studium_save_favorites(room_id, seat_numbers)?;
    snapshot_with_store(controller, store)
}

#[tauri::command]
pub(crate) async fn studium_save_interval(
    state: tauri::State<'_, AppState>,
    interval_minutes: u32,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    store.studium_save_auto_interval(interval_minutes)?;
    // Saving changes the next cycle's interval, never wakes the current timer.
    state.studium_views.refresh();
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_automatic_start(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    let value = automatic_start_with_transport(
        &state.studium,
        &state.operation_gate,
        &state.maintenance_shutdown,
        &state.store,
        state.network.clone(),
    )
    .await?;
    state.studium_views.refresh();
    Ok(value)
}

#[tauri::command]
pub(crate) async fn studium_automatic_pause(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let completion = {
        let _store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
        state.studium.pause_automatic()
    };
    if let Some(completion) = completion {
        completion.wait().await;
    }
    state.studium_views.refresh();
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_status(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    studium_snapshot(state).await
}

#[tauri::command]
pub(crate) async fn studium_save_preferences(
    state: tauri::State<'_, AppState>,
    room_ids: Vec<String>,
    view_room: String,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    state.studium.ensure_idle()?;
    store.studium_save_preferences(&room_ids, &view_room)?;
    state.studium_views.refresh();
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_set_view_room(
    state: tauri::State<'_, AppState>,
    room_id: String,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    store.studium_set_view_room(&room_id)?;
    state.studium_views.refresh();
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_start(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    start_with_transport(
        &state.studium,
        &state.operation_gate,
        &state.maintenance_shutdown,
        &state.store,
        state.network.clone(),
    )
    .await
}

#[tauri::command]
pub(crate) async fn studium_cancel(
    state: tauri::State<'_, AppState>,
    job_id: u64,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    state.studium.cancel_job(job_id)?;
    state.studium_views.refresh();
    snapshot(&state, &store)
}

#[cfg(test)]
mod tests;
