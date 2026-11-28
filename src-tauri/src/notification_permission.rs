//! The notification authorization commands, wrappers over
//! `App::proactive()`; the checks themselves are
//! `rhythm_core::notification::permission`.
pub(crate) use rhythm_core::notification::permission::notification_authorization_status;
use rhythm_core::notification::permission::{Authorization, NotificationConnectionResult};

use crate::AppState;
use tauri::State;

/// Checks native notification authorization without prompting.
#[tauri::command]
pub(crate) async fn notification_authorization(
    state: State<'_, AppState>,
) -> Result<Authorization, String> {
    state.proactive().authorization().await
}

/// Invoked only by an explicit user action: macOS asks for authorization,
/// Windows checks the installed connection and sends one test notification.
#[tauri::command]
pub(crate) async fn notification_request_permission(
    state: State<'_, AppState>,
) -> Result<NotificationConnectionResult, String> {
    state.proactive().request_permission().await
}
