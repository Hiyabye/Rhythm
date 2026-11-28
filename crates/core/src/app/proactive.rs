//! The `proactive` extension's 알림·백그라운드 pane: scheduler and notification
//! settings, notification history, and the operating system's notification
//! authorization. The lifecycle switches it also shows are the shell's
//! (`core::lifecycle` in the Tauri app).

use super::App;
use crate::{
    extensions::PROACTIVE,
    notification::permission::{self, Authorization, NotificationConnectionResult},
    proactive::{preserve_lifecycle_settings, snapshot, ProactiveSnapshot},
    storage::{
        proactive::{ProactiveHistoryItem, ProactiveSettings},
        with_store, Store,
    },
};

const OFF: &str = "알림·백그라운드 확장 기능이 꺼져 있습니다.";

pub struct Proactive<'a>(pub(super) &'a App);

impl Proactive<'_> {
    /// The settings, scheduler state and recent history, with a notice
    /// click that arrived before the UI subscribed (taken once).
    pub async fn snapshot(&self) -> Result<ProactiveSnapshot, String> {
        self.with_on(|s, app| snapshot(s, app.proactive.take_activation()?))
            .await
    }

    /// Saves the notification and scheduler settings. The lifecycle ones keep
    /// their stored values: they have their own commands.
    pub async fn save(&self, settings: ProactiveSettings) -> Result<ProactiveSnapshot, String> {
        self.with_on(move |s, app| {
            let persisted_settings = s.proactive_settings()?;
            let settings = preserve_lifecycle_settings(settings, &persisted_settings);
            s.proactive_save_settings(&settings)?;
            snapshot(s, app.proactive.take_activation()?)
        })
        .await
    }

    pub async fn history(&self, limit: Option<usize>) -> Result<Vec<ProactiveHistoryItem>, String> {
        self.with_on(move |s, _| s.proactive_history(limit.unwrap_or(50)))
            .await
    }

    /// Clears the shown history; rate accounting and dedupe stay.
    pub async fn clear_history(&self) -> Result<ProactiveSnapshot, String> {
        self.with_on(|s, app| {
            s.proactive_clear_history()?;
            snapshot(s, app.proactive.take_activation()?)
        })
        .await
    }

    /// The operating system's notification authorization, without prompting.
    pub async fn authorization(&self) -> Result<Authorization, String> {
        self.with_on(|_, _| Ok(())).await?;
        Ok(permission::notification_authorization_status().await)
    }

    /// Only for an explicit user action: macOS asks for authorization, Windows
    /// checks the installed connection and sends one test notification. No
    /// lease is held while the system prompt is open, so backup, restore and
    /// updates never wait for it.
    pub async fn request_permission(&self) -> Result<NotificationConnectionResult, String> {
        self.with_on(|_, _| Ok(())).await?;
        permission::request_permission().await
    }

    /// Runs `operation` under a shared lease and the Store lock, refused while
    /// the extension is off.
    async fn with_on<T>(
        &self,
        operation: impl FnOnce(&Store, &App) -> Result<T, String>,
    ) -> Result<T, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        with_store(&app.store, |s| {
            if !s.enabled(PROACTIVE)? {
                return Err(OFF.into());
            }
            operation(s, app)
        })
    }
}
