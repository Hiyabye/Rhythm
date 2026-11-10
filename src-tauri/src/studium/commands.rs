use super::model::Snapshot;
use crate::{storage::Store, AppState};

fn snapshot(state: &AppState, store: &Store) -> Result<Snapshot, String> {
    state.studium.snapshot(store.studium_preferences()?)
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
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_start(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    // Registration and cancellation share Store's boundary. A lease admitted
    // just before maintenance/exit must not register after their cancel pass.
    state.operation_gate.ensure_open()?;
    if state
        .maintenance_shutdown
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("Rhythm을 종료하고 있습니다.".into());
    }
    if !store.enabled(crate::extensions::STUDIUM)? {
        return Err("열람실 확장 기능이 꺼져 있습니다.".into());
    }
    state.studium.start(
        store.studium_preferences()?,
        state.network.clone(),
        state.store.clone(),
        lease,
    )?;
    snapshot(&state, &store)
}

#[tauri::command]
pub(crate) async fn studium_cancel(
    state: tauri::State<'_, AppState>,
    job_id: u64,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state.store.lock().map_err(|_| "설정 잠금 오류")?;
    state.studium.cancel_job(job_id)?;
    snapshot(&state, &store)
}
