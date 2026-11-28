//! The in-app scheduler (Cybercampus source tasks), the notice notification
//! pipeline and todo reminders. The UI's calls are `App::proactive()`.
mod notification;
mod reminders;
pub mod scheduler;

use crate::storage::{
    proactive::{ProactiveHistoryItem, ProactiveSchedulerStatus, ProactiveSettings},
    Store,
};
use serde::Serialize;

pub use notification::NotificationActivation;
#[cfg(any(windows, test))]
pub use reminders::{token_from_uri as reminder_token_from_uri, token_prefix as reminder_prefix};
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

pub fn snapshot(
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

/// Saving the notification settings never changes the lifecycle ones, which
/// have their own commands.
pub fn preserve_lifecycle_settings(
    mut settings: ProactiveSettings,
    persisted: &ProactiveSettings,
) -> ProactiveSettings {
    settings.close_to_background = persisted.close_to_background;
    settings.launch_at_login = persisted.launch_at_login;
    settings.start_hidden = persisted.start_hidden;
    settings
}

#[cfg(test)]
mod tests;
