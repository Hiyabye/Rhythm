//! The proactive commands, wrappers over `App::proactive()`; the scheduler,
//! notification pipeline and todo reminders are `rhythm_core::proactive`.
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
    state.proactive().snapshot().await
}

#[tauri::command]
pub async fn proactive_save(
    state: State<'_, AppState>,
    settings: ProactiveSettings,
) -> Result<ProactiveSnapshot, String> {
    state.proactive().save(settings).await
}

#[tauri::command]
pub async fn proactive_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<ProactiveHistoryItem>, String> {
    state.proactive().history(limit).await
}

#[tauri::command]
pub async fn proactive_clear_history(
    state: State<'_, AppState>,
) -> Result<ProactiveSnapshot, String> {
    state.proactive().clear_history().await
}

#[cfg(all(test, windows))]
mod native_validation;
