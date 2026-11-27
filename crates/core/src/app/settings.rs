//! General settings, first-run setup, lifecycle preferences and extensions.

use super::App;
use crate::{
    extensions::{self, ExtensionInfo},
    shell::Event,
    storage::{self, proactive::ProactiveSettings, StorageInfo},
};
use serde::Serialize;

/// What the shell shows on every page: settings, extensions and setup state.
#[derive(Serialize)]
pub struct Snapshot {
    pub settings: storage::Settings,
    pub extensions: Vec<ExtensionInfo>,
    pub storage: StorageInfo,
    pub proactive: ProactiveSettings,
    pub onboarding_completed: bool,
}

/// Logs a storage error and returns the message the UI shows for it.
pub fn storage_error(error: String) -> String {
    eprintln!("Storage: {error}");
    "로컬 데이터를 읽거나 저장하지 못했습니다. 저장 공간과 파일 권한을 확인해 주세요.".into()
}

pub struct Settings<'a>(pub(super) &'a App);

impl Settings<'_> {
    pub async fn snapshot(&self) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.read_snapshot()
    }

    /// Saves the general settings and tells the UI the theme and clock format.
    pub async fn save(&self, settings: storage::Settings) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        {
            let store = self
                .0
                .store
                .lock()
                .map_err(|_| "저장소에 접근할 수 없습니다.")?;
            store
                .save(&settings)
                .and_then(|()| store.auto_trash_old())
                .map_err(storage_error)?;
        }
        self.0.ui.emit(Event::ThemeChanged(settings.theme));
        self.0
            .ui
            .emit(Event::ClockFormatChanged(settings.clock_format));
        self.0.refresh_views();
        self.read_snapshot()
    }

    /// Ends the first-run setup flow. A rerun from Settings ends the same way,
    /// so completion is never cleared.
    pub async fn complete_onboarding(&self) -> Result<Snapshot, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.0
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?
            .complete_onboarding()
            .map_err(storage_error)?;
        self.read_snapshot()
    }

    /// The stored lifecycle preferences. Whether the shell can honor them
    /// (a tray, the OS login item) is the shell's to check.
    pub async fn proactive(&self) -> Result<ProactiveSettings, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.read_proactive()
    }

    pub async fn set_close_to_background(&self, enabled: bool) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.update_proactive(|settings| settings.close_to_background = enabled)
    }

    pub async fn set_start_hidden(&self, enabled: bool) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.update_proactive(|settings| settings.start_hidden = enabled)
    }

    /// Records the launch-at-login choice. The shell changes the OS login
    /// item first and rolls it back when this fails.
    pub async fn set_launch_at_login(&self, enabled: bool) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.update_proactive(|settings| settings.launch_at_login = enabled)
    }

    /// Turns an extension on or off and waits for the work it stops. Before
    /// turning `proactive` off the shell removes its OS login item, which no
    /// stored flag can gate; if that fails it leaves the extension on.
    pub async fn set_extension(&self, id: &str, enabled: bool) -> Result<Snapshot, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        let changed = app.registry.change(&app.store, id, enabled, || {
            app.views().and_then(|views| views.capture_close())
        })?;
        if !enabled {
            let view_result = match (changed.captured, app.views()) {
                (Some(ticket), Some(views)) => views.clone().close(ticket).await,
                _ => Ok(()),
            };
            if let Some(completion) = changed.notifications {
                completion.wait().await;
            }
            match id {
                extensions::NOTICES => {
                    app.archive.wait().await;
                    app.archive.bodies.wait().await;
                }
                extensions::CYBERCAMPUS => app.archive.wait().await,
                extensions::STUDIUM => {
                    crate::studium::jobs::finish_disable(&app.store, &app.studium, changed.studium)
                        .await?;
                }
                // The classifier sees the extension off on its next pass and
                // stops its process; wake it instead of waiting for the timer.
                extensions::AI => app.classifier.wake(),
                _ => {}
            }
            view_result?;
        } else if matches!(id, extensions::STUDIUM | extensions::PROACTIVE) {
            app.studium_notifications.resume_after_quiesce().await;
        }
        app.refresh_views();
        self.read_snapshot()
    }

    fn read_snapshot(&self) -> Result<Snapshot, String> {
        let store = self
            .0
            .store
            .lock()
            .map_err(|_| "데이터 저장소에 접근할 수 없습니다.")?;
        Ok(Snapshot {
            settings: store.settings().map_err(storage_error)?,
            extensions: self.0.registry.list(&store).map_err(storage_error)?,
            storage: store.info().map_err(storage_error)?,
            proactive: store.proactive_settings().map_err(storage_error)?,
            onboarding_completed: store.onboarding_completed().map_err(storage_error)?,
        })
    }

    fn read_proactive(&self) -> Result<ProactiveSettings, String> {
        self.0
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())?
            .proactive_settings()
            .map_err(storage_error)
    }

    fn update_proactive(&self, update: impl FnOnce(&mut ProactiveSettings)) -> Result<(), String> {
        let store = self
            .0
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())?;
        let mut settings = store.proactive_settings().map_err(storage_error)?;
        update(&mut settings);
        store
            .proactive_save_settings(&settings)
            .map(|_| ())
            .map_err(storage_error)
    }
}
