mod notification;
pub mod scheduler;

use crate::{
    storage::{
        proactive::{ProactiveHistoryItem, ProactiveSchedulerStatus, ProactiveSettings},
        Store,
    },
    AppState,
};
use serde::Serialize;
use tauri::State;

pub use notification::NotificationActivation;
pub use scheduler::Scheduler;

#[derive(Clone, Debug, Serialize)]
pub struct ProactiveSnapshot {
    pub settings: ProactiveSettings,
    pub scheduler: ProactiveSchedulerStatus,
    pub history: Vec<ProactiveHistoryItem>,
    /// One-shot fallback for a notification click received before the UI subscribed.
    pub pending_activation: Option<NotificationActivation>,
    /// Tauri's desktop permission API reports a default rather than authorization.
    pub notification_permission: &'static str,
}

fn snapshot(
    store: &Store,
    pending_activation: Option<NotificationActivation>,
) -> Result<ProactiveSnapshot, String> {
    Ok(ProactiveSnapshot {
        settings: store.proactive_settings()?,
        scheduler: store.proactive_scheduler_status()?,
        history: store.proactive_history(50)?,
        pending_activation,
        notification_permission: "unknown",
    })
}

#[tauri::command]
pub async fn proactive_snapshot(state: State<'_, AppState>) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state
        .store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    snapshot(&store, state.proactive.take_activation()?)
}

#[tauri::command]
pub async fn proactive_save(
    state: State<'_, AppState>,
    settings: ProactiveSettings,
) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state
        .store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    let persisted_settings = store.proactive_settings()?;
    let settings = preserve_lifecycle_settings(settings, &persisted_settings);
    store.proactive_save_settings(&settings)?;
    snapshot(&store, state.proactive.take_activation()?)
}

fn preserve_lifecycle_settings(
    mut settings: ProactiveSettings,
    persisted: &ProactiveSettings,
) -> ProactiveSettings {
    settings.close_to_background = persisted.close_to_background;
    settings.launch_at_login = persisted.launch_at_login;
    settings.start_hidden = persisted.start_hidden;
    settings
}

#[tauri::command]
pub async fn proactive_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<ProactiveHistoryItem>, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state
        .store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    store.proactive_history(limit.unwrap_or(50))
}

#[tauri::command]
pub async fn proactive_clear_history(
    state: State<'_, AppState>,
) -> Result<ProactiveSnapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let store = state
        .store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    store.proactive_clear_history()?;
    snapshot(&store, state.proactive.take_activation()?)
}

#[cfg(test)]
mod tests;
