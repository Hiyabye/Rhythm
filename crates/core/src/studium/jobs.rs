//! Seat-check admission at the Store boundary: the manual and automatic starts
//! the commands and tests share, cancellation, the maintenance reservation and
//! the end of a disable.
use super::{
    controller::{CompletionHandle, Controller},
    model::Snapshot,
};
use crate::{
    archive::sync::Transport,
    operation_gate::{AdmissionGate, AdmissionLease, MaintenanceReservation},
    storage::{SharedStore, Store},
};
use std::sync::{atomic::AtomicBool, Arc};

pub fn snapshot_with_store(controller: &Controller, store: &Store) -> Result<Snapshot, String> {
    let mut value = controller.snapshot(store.studium_preferences()?)?;
    value.automatic.interval_minutes = store.studium_auto_interval()?;
    value.favorites = store.studium_favorites()?;
    value.alert_rules = store.studium_alert_rules()?;
    Ok(value)
}

pub async fn automatic_start_with_transport<T: Transport + Clone + 'static>(
    controller: &Arc<Controller>,
    gate: &Arc<AdmissionGate>,
    shutdown: &Arc<AtomicBool>,
    shared_store: &SharedStore,
    transport: T,
) -> Result<Snapshot, String> {
    let _lease = gate.shared().await?;
    let store = shared_store
        .lock()
        .map_err(|_| "설정에 접근하지 못했습니다.")?;
    gate.ensure_open()?;
    if shutdown.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Rhythm을 종료하고 있습니다.".into());
    }
    if !store.enabled(crate::extensions::STUDIUM)? {
        return Err("Studium 확장 기능이 꺼져 있습니다.".into());
    }
    store.studium_auto_interval()?;
    controller.start_automatic(
        shared_store.clone(),
        gate.clone(),
        shutdown.clone(),
        transport,
    )?;
    snapshot_with_store(controller, &store)
}

/// The IPC command and in-process callers use the same admission boundary.
/// Transport injection lets command tests observe requests without a live server.
pub async fn start_with_transport<T: Transport + 'static>(
    controller: &Arc<Controller>,
    gate: &Arc<AdmissionGate>,
    shutdown: &AtomicBool,
    shared_store: &SharedStore,
    transport: T,
) -> Result<Snapshot, String> {
    let lease = gate.shared().await?;
    register(controller, gate, shutdown, shared_store, transport, lease)
}

/// Registers a check whose admission `lease` the caller already holds.
pub fn register<T: Transport + 'static>(
    controller: &Arc<Controller>,
    gate: &AdmissionGate,
    shutdown: &AtomicBool,
    shared_store: &SharedStore,
    transport: T,
    lease: AdmissionLease,
) -> Result<Snapshot, String> {
    let store = shared_store
        .lock()
        .map_err(|_| "설정에 접근하지 못했습니다.")?;
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
    snapshot_with_store(controller, &store)
}

/// Cancellation and admission serialize on Store. The returned handle belongs
/// to the cancelled generation even if a later enable starts another job.
pub fn cancel_at_boundary(
    store: &SharedStore,
    controller: &Controller,
) -> Result<Option<CompletionHandle>, String> {
    let _store = store.lock().map_err(|_| "설정에 접근하지 못했습니다.")?;
    controller.cancel();
    Ok(controller.completion())
}

/// Failure before maintenance preparation returns drops the reservation and
/// reopens admission; no scheduler or AI work has been stopped at this point.
pub fn reserve_maintenance(
    gate: &Arc<AdmissionGate>,
    store: &SharedStore,
    controller: &Controller,
) -> Result<(MaintenanceReservation, Option<CompletionHandle>), String> {
    let reservation = gate.close()?;
    let _store = store.lock().map_err(|_| "설정에 접근하지 못했습니다.")?;
    controller.cancel_for_maintenance();
    Ok((reservation, controller.completion()))
}

/// Ends a Studium disable after the Store lock is released: waits for the
/// cancelled generation, then clears results unless a later enable won.
pub async fn finish_disable(
    shared_store: &SharedStore,
    studium: &Controller,
    completion: Option<CompletionHandle>,
) -> Result<(), String> {
    if let Some(completion) = completion {
        completion.wait().await;
    }
    let store = shared_store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.")?;
    // A later enable may have won while disable awaited. Never clear a newly
    // admitted generation's results in that case.
    if !store.enabled(crate::extensions::STUDIUM)? {
        studium.clear();
    }
    Ok(())
}
