//! The proactive commands; the scheduler, notification pipeline and todo
//! reminders are `rhythm_core::proactive`.
#[cfg(windows)]
pub(crate) use rhythm_core::notification::windows as windows_notification;
pub use rhythm_core::proactive::*;

use crate::{
    storage::proactive::{ProactiveHistoryItem, ProactiveSettings},
    AppState,
};
use tauri::State;

#[tauri::command]
pub async fn proactive_snapshot(state: State<'_, AppState>) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = scheduler::lock_store(&state.store)?;
    snapshot(&store, state.proactive.take_activation()?)
}

#[tauri::command]
pub async fn proactive_save(
    state: State<'_, AppState>,
    settings: ProactiveSettings,
) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = scheduler::lock_store(&state.store)?;
    let persisted_settings = store.proactive_settings()?;
    let settings = preserve_lifecycle_settings(settings, &persisted_settings);
    store.proactive_save_settings(&settings)?;
    snapshot(&store, state.proactive.take_activation()?)
}

#[tauri::command]
pub async fn proactive_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<ProactiveHistoryItem>, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = scheduler::lock_store(&state.store)?;
    store.proactive_history(limit.unwrap_or(50))
}

#[tauri::command]
pub async fn proactive_clear_history(
    state: State<'_, AppState>,
) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = scheduler::lock_store(&state.store)?;
    store.proactive_clear_history()?;
    snapshot(&store, state.proactive.take_activation()?)
}

/// One-shot fallback for a reminder click received before the UI subscribed to
/// `calendar://reminder-clicked`: the todo's day while the calendar is still on.
#[tauri::command]
pub async fn calendar_reminder_activation(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = scheduler::lock_store(&state.store)?;
    let day = state.proactive.take_reminder_activation()?;
    Ok(day.filter(|_| store.enabled(crate::extensions::CALENDAR).unwrap_or(false)))
}

#[cfg(all(test, windows))]
mod native_validation;
