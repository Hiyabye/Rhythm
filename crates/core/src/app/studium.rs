//! The 열람실 page of the `studium` extension: targets, the display room, manual
//! and automatic seat checks, favorites and alert rules. Only [`Studium::start`]
//! and a session started by [`Studium::automatic_start`] send school requests.
//! Seat results stay in memory.

use super::{App, Snapshot as AppSnapshot};
use crate::{
    storage::Store,
    studium::{
        jobs::{automatic_start_with_transport, snapshot_with_store, start_with_transport},
        model::{AlertRuleInput, Snapshot},
    },
};

pub struct Studium<'a>(pub(super) &'a App);

impl Studium<'_> {
    /// The settings and the in-memory results. A local read: it starts no
    /// check.
    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            store.studium_on()?;
            self.read(store)
        })
    }

    // The store refuses each settings write below while the extension is off.

    /// Replaces the rooms to check. Refused while a check is running.
    pub async fn save_preferences(
        &self,
        room_ids: &[String],
        view_room: &str,
    ) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            self.0.studium.ensure_idle()?;
            store.studium_save_preferences(room_ids, view_room)?;
            self.0.refresh_views();
            self.read(store)
        })
    }

    pub async fn set_view_room(&self, room_id: &str) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            store.studium_set_view_room(room_id)?;
            self.0.refresh_views();
            self.read(store)
        })
    }

    /// Starts one manual check of the selected rooms.
    pub async fn start(&self) -> Result<Snapshot, String> {
        let app = self.0;
        start_with_transport(
            &app.studium,
            &app.operation_gate,
            &app.maintenance_shutdown,
            &app.store,
            app.network.clone(),
        )
        .await
    }

    /// Cancels the check `job_id`. Stopping is never refused for the
    /// extension being off.
    pub async fn cancel(&self, job_id: u64) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            self.0.studium.cancel_job(job_id)?;
            self.0.refresh_views();
            self.read(store)
        })
    }

    pub async fn save_interval(&self, interval_minutes: u32) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            store.studium_save_auto_interval(interval_minutes)?;
            // Saving changes the next cycle's interval, never wakes the current timer.
            self.0.refresh_views();
            self.read(store)
        })
    }

    /// Starts the automatic session, which checks at the saved interval until
    /// it is paused, Studium turns off or Rhythm quits.
    pub async fn automatic_start(&self) -> Result<Snapshot, String> {
        let app = self.0;
        let value = automatic_start_with_transport(
            &app.studium,
            &app.operation_gate,
            &app.maintenance_shutdown,
            &app.store,
            app.network.clone(),
        )
        .await?;
        app.refresh_views();
        Ok(value)
    }

    /// Ends the automatic session and waits for its timer. A manual check
    /// keeps running. Like [`Studium::cancel`], never refused.
    pub async fn automatic_pause(&self) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        let completion = self.locked(|_| Ok(self.0.studium.pause_automatic()))?;
        if let Some(completion) = completion {
            completion.wait().await;
        }
        self.0.refresh_views();
        self.locked(|store| self.read(store))
    }

    pub async fn save_favorites(
        &self,
        room_id: &str,
        seat_numbers: &[u32],
    ) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            store.studium_save_favorites(room_id, seat_numbers)?;
            self.0.refresh_views();
            self.read(store)
        })
    }

    pub async fn save_alert(
        &self,
        room_id: &str,
        rule: &AlertRuleInput,
    ) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.locked(|store| {
            store.studium_save_alert_rule(room_id, rule)?;
            // Rule editing invalidates old candidates but never observes old results.
            self.0.refresh_views();
            self.read(store)
        })
    }

    /// One-shot fallback for a seat-alert click that arrived before the UI
    /// subscribed to [`Event::StudiumClicked`](crate::shell::Event): the room
    /// to show.
    pub async fn notification_activation(&self) -> Result<Option<String>, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.0.studium_notifications.take_activation()
    }

    fn read(&self, store: &Store) -> Result<Snapshot, String> {
        snapshot_with_store(&self.0.studium, store)
    }

    fn locked<T>(&self, operation: impl FnOnce(&Store) -> Result<T, String>) -> Result<T, String> {
        let store = self
            .0
            .store
            .lock()
            .map_err(|_| "설정에 접근하지 못했습니다.")?;
        operation(&store)
    }
}

/// What the Studium mini window may call: the two snapshots it draws from and
/// the seat-check controls it shows. It is a separate, narrower type so that a
/// shell hands the mini window no backup, extension, credential or settings
/// method. Its window controls stay with the shell that owns the window.
pub struct StudiumMini<'a>(pub(super) &'a App);

impl StudiumMini<'_> {
    /// Settings and extensions, for the theme, the clock format and whether
    /// Studium is still on.
    pub async fn app_snapshot(&self) -> Result<AppSnapshot, String> {
        self.0.settings().snapshot().await
    }

    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        self.0.studium().snapshot().await
    }

    pub async fn set_view_room(&self, room_id: &str) -> Result<Snapshot, String> {
        self.0.studium().set_view_room(room_id).await
    }

    pub async fn start(&self) -> Result<Snapshot, String> {
        self.0.studium().start().await
    }

    pub async fn cancel(&self, job_id: u64) -> Result<Snapshot, String> {
        self.0.studium().cancel(job_id).await
    }
}
