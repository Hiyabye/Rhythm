use super::{controller::CompletionHandle, model::Snapshot};
use crate::{
    archive::sync::{SharedStore, Transport},
    operation_gate::{AdmissionGate, AdmissionLease, MaintenanceReservation},
    storage::Store,
    AppState,
};
use std::sync::{atomic::AtomicBool, Arc};

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
    start_with_transport(
        &state.studium,
        &state.operation_gate,
        &state.maintenance_shutdown,
        &state.store,
        state.network.clone(),
    )
    .await
}

/// The IPC command and in-process callers use the same admission boundary.
/// Transport injection lets command tests observe requests without a live server.
pub(crate) async fn start_with_transport<T: Transport + 'static>(
    controller: &Arc<super::controller::Controller>,
    gate: &Arc<AdmissionGate>,
    shutdown: &AtomicBool,
    shared_store: &SharedStore,
    transport: T,
) -> Result<Snapshot, String> {
    let lease = gate.shared().await?;
    register(controller, gate, shutdown, shared_store, transport, lease)
}

fn register<T: Transport + 'static>(
    controller: &Arc<super::controller::Controller>,
    gate: &AdmissionGate,
    shutdown: &AtomicBool,
    shared_store: &SharedStore,
    transport: T,
    lease: AdmissionLease,
) -> Result<Snapshot, String> {
    let store = shared_store.lock().map_err(|_| "설정 잠금 오류")?;
    // Registration and cancellation share Store's boundary. A lease admitted
    // just before maintenance/exit must not register after their cancel pass.
    gate.ensure_open()?;
    if shutdown.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Rhythm을 종료하고 있습니다.".into());
    }
    if !store.enabled(crate::extensions::STUDIUM)? {
        return Err("열람실 확장 기능이 꺼져 있습니다.".into());
    }
    controller.start(
        store.studium_preferences()?,
        transport,
        shared_store.clone(),
        lease,
    )?;
    controller.snapshot(store.studium_preferences()?)
}

/// Cancellation and admission serialize on Store. The returned handle belongs
/// to the cancelled generation even if a later enable starts another job.
pub(crate) fn cancel_at_boundary(
    store: &SharedStore,
    controller: &super::controller::Controller,
) -> Result<Option<CompletionHandle>, String> {
    let _store = store.lock().map_err(|_| "설정 잠금 오류")?;
    controller.cancel();
    Ok(controller.completion())
}

/// Failure before maintenance preparation returns drops the reservation and
/// reopens admission; no scheduler or AI work has been stopped at this point.
pub(crate) fn reserve_maintenance(
    gate: &Arc<AdmissionGate>,
    store: &SharedStore,
    controller: &super::controller::Controller,
) -> Result<(MaintenanceReservation, Option<CompletionHandle>), String> {
    let reservation = gate.close()?;
    let completion = cancel_at_boundary(store, controller)?;
    Ok((reservation, completion))
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

#[cfg(test)]
mod tests;
