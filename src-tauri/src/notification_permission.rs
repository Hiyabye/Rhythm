//! The notification authorization commands; the checks themselves are
//! `rhythm_core::notification::permission`.
pub(crate) use rhythm_core::notification::permission::notification_authorization_status;
use rhythm_core::notification::permission::{request_permission, NotificationConnectionResult};

/// Checks native notification authorization without prompting.
#[tauri::command]
pub(crate) async fn notification_authorization() -> String {
    notification_authorization_status().await
}

/// Invoked only by an explicit user action: macOS asks for authorization,
/// Windows checks the installed connection and sends one test notification.
#[tauri::command]
pub(crate) async fn notification_request_permission() -> Result<NotificationConnectionResult, String>
{
    request_permission().await
}
