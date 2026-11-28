//! General settings, first-run setup, lifecycle preferences and extensions.

use super::App;
use crate::{
    extensions::{self, ExtensionInfo, Work},
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

/// The lifecycle preferences the shell applies; see [`Settings::lifecycle`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lifecycle {
    /// Closing the main window keeps the app running in the background
    /// (only when the shell has a tray to restore it from).
    pub close_to_background: bool,
    /// An OS login launch stays hidden.
    pub start_hidden: bool,
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

    /// The two lifecycle switches as they apply now (`proactive` off turns
    /// both off). Read on demand, so a save from another call is seen at once;
    /// an unreadable store falls back to the default behavior.
    pub fn lifecycle(&self) -> Lifecycle {
        match self
            .0
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())
            .and_then(|store| store.proactive_effective_settings().map_err(storage_error))
        {
            Ok(settings) => Lifecycle {
                close_to_background: settings.close_to_background,
                start_hidden: settings.start_hidden,
            },
            Err(error) => {
                eprintln!("Lifecycle: 백그라운드 설정을 읽지 못해 기본 동작을 사용합니다: {error}");
                Lifecycle::default()
            }
        }
    }

    /// Whether the automatic update check is on. The check itself is the
    /// shell's (the updater).
    pub fn update_check(&self) -> Result<bool, String> {
        self.0
            .store
            .lock()
            .map_err(|_| "설정 저장소를 사용할 수 없습니다.".to_string())?
            .update_check_enabled()
    }

    pub fn set_update_check(&self, enabled: bool) -> Result<bool, String> {
        let store = self
            .0
            .store
            .lock()
            .map_err(|_| "설정 저장소를 사용할 수 없습니다.".to_string())?;
        store.set_update_check(enabled)?;
        store.update_check_enabled()
    }

    /// Whether an automatic update check is due; false when it is off or the
    /// store cannot be read.
    pub fn update_check_due(&self) -> bool {
        self.0
            .store
            .lock()
            .ok()
            .and_then(|store| store.update_check_due().ok())
            .unwrap_or(false)
    }

    /// Records a finished automatic check, so the next one waits a day.
    pub fn record_update_check(&self) {
        if let Ok(store) = self.0.store.lock() {
            let _ = store.record_update_check();
        }
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
        // Starts the loops of an extension turned on and joins those of one
        // turned off. First, so that a failure below cannot skip it.
        app.settle_loops().await;
        if !enabled {
            let view_result = match (changed.captured, app.views()) {
                (Some(ticket), Some(views)) => views.clone().close(ticket).await,
                _ => Ok(()),
            };
            if let Some(completion) = changed.notifications {
                completion.wait().await;
            }
            let extension = extensions::declaration(id).ok_or("등록되지 않은 확장 기능입니다.")?;
            let mut studium = changed.studium;
            for work in extension.settles {
                match work {
                    Work::Archive => app.archive.wait_owned_by(id).await,
                    Work::Bodies => app.archive.bodies.wait().await,
                    Work::Studium => {
                        crate::studium::jobs::finish_disable(
                            &app.store,
                            &app.studium,
                            studium.take(),
                        )
                        .await?;
                    }
                    Work::Feed => {}
                }
            }
            view_result?;
        } else if extensions::declaration(id).is_some_and(|e| e.feeds_studium_alerts()) {
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
