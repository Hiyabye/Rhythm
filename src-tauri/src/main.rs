mod archive;
mod backup;
mod core;
mod extensions;
mod intelligence;
mod notification_permission;
pub(crate) mod operation_gate;
mod personalization;
mod proactive;
mod storage;
mod updater;
use extensions::{ExtensionInfo, Registry};
use intelligence::{Worker, WorkerStatus};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use storage::{proactive::ProactiveSettings, Settings, StorageInfo, Store};
use tauri::Manager;

pub(crate) struct AppState {
    _instance_lock: core::DataDirLock,
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) data_root: PathBuf,
    lifecycle: Arc<dyn core::lifecycle::LifecyclePreferences>,
    tray_available: bool,
    launch_at_login: Mutex<()>,
    pub(crate) proactive: Arc<proactive::Scheduler>,
    pub(crate) archive: Arc<archive::sync::Controller>,
    network: archive::Client,
    registry: Registry,
    pub(crate) worker: Arc<tokio::sync::Mutex<Worker>>,
    pub(crate) analysis: Arc<intelligence::analysis::Controller>,
    pub(crate) models: intelligence::models::Controller,
    hardware: intelligence::models::Hardware,
    ai_gate: tokio::sync::Mutex<()>,
    pub(crate) ai_operation: intelligence::operations::Controller,
    maintenance_shutdown: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) operation_gate: Arc<operation_gate::AdmissionGate>,
}

pub(crate) struct AppMaintenanceGuard {
    _gate: operation_gate::MaintenanceGuard,
    proactive: Arc<proactive::Scheduler>,
    analysis: Arc<intelligence::analysis::Controller>,
}

impl Drop for AppMaintenanceGuard {
    fn drop(&mut self) {
        self.analysis.resume();
        if let Err(error) = self.proactive.start() {
            eprintln!("Proactive: 유지보수 후 스케줄러를 다시 시작하지 못했습니다: {error}");
        }
    }
}

struct MaintenancePreparation {
    reservation: Option<operation_gate::MaintenanceReservation>,
    proactive: Arc<proactive::Scheduler>,
    analysis: Arc<intelligence::analysis::Controller>,
    armed: bool,
}

impl Drop for MaintenancePreparation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.analysis.resume();
        let proactive = self.proactive.clone();
        tauri::async_runtime::spawn(async move {
            proactive.resume_after_quiesce().await;
        });
    }
}

impl AppState {
    pub(crate) async fn exclusive_maintenance(&self) -> Result<AppMaintenanceGuard, String> {
        let mut preparation = MaintenancePreparation {
            reservation: Some(self.operation_gate.close()?),
            proactive: self.proactive.clone(),
            analysis: self.analysis.clone(),
            armed: true,
        };
        self.archive.cancel();
        self.analysis.stop();
        self.models.stop();
        self.ai_operation.cancel();
        let ai_gate = self.ai_gate.lock().await;
        self.analysis.stop();
        self.models.stop();
        self.ai_operation.cancel();
        drop(ai_gate);
        self.proactive.quiesce().await;
        self.archive.wait().await;
        self.analysis.wait().await;
        self.models.wait().await;
        let gate = preparation
            .reservation
            .take()
            .expect("maintenance reservation remains until exclusive acquisition")
            .acquire()
            .await?;
        preparation.armed = false;
        Ok(AppMaintenanceGuard {
            _gate: gate,
            proactive: self.proactive.clone(),
            analysis: self.analysis.clone(),
        })
    }
}
#[derive(Serialize)]
struct Snapshot {
    settings: Settings,
    extensions: Vec<ExtensionInfo>,
    storage: StorageInfo,
    worker: WorkerStatus,
    proactive: ProactiveSettings,
}
fn storage_error(error: String) -> String {
    eprintln!("Storage: {error}");
    "로컬 데이터를 읽거나 저장하지 못했습니다. 저장 공간과 파일 권한을 확인해 주세요.".into()
}

/// Reads lifecycle preferences directly from persisted settings. Reading on
/// demand avoids a cache that could go stale when another command saves the
/// full proactive settings object.
struct StoredLifecycle(Arc<Mutex<Store>>);

impl StoredLifecycle {
    fn settings(&self) -> Option<ProactiveSettings> {
        match self
            .0
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())
            .and_then(|store| store.proactive_settings().map_err(storage_error))
        {
            Ok(settings) => Some(settings),
            Err(error) => {
                eprintln!("Lifecycle: 백그라운드 설정을 읽지 못해 기본 동작을 사용합니다: {error}");
                None
            }
        }
    }
}

impl core::lifecycle::LifecyclePreferences for StoredLifecycle {
    fn close_to_background(&self) -> bool {
        self.settings()
            .is_some_and(|settings| settings.close_to_background)
    }

    fn start_hidden(&self) -> bool {
        self.settings()
            .is_some_and(|settings| settings.start_hidden)
    }
}

async fn snapshot(state: &AppState) -> Result<Snapshot, String> {
    let worker = runtime_status(state);
    let store = state
        .store
        .lock()
        .map_err(|_| "데이터 저장소에 접근할 수 없습니다.")?;
    Ok(Snapshot {
        settings: store.settings().map_err(storage_error)?,
        extensions: state.registry.list(&store).map_err(storage_error)?,
        storage: store.info().map_err(storage_error)?,
        worker,
        proactive: store.proactive_settings().map_err(storage_error)?,
    })
}
#[tauri::command]
async fn get_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    snapshot(&state).await
}
#[tauri::command]
async fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    {
        state
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?
            .save(&settings)
            .map_err(storage_error)?;
    }
    snapshot(&state).await
}

#[derive(Serialize)]
struct LifecycleStatus {
    close_to_background: bool,
    start_hidden: bool,
    launch_at_login: bool,
    /// Actual OS registration state. `null` means the OS query failed; it is
    /// never replaced with a guess from the persisted preference.
    autostart_enabled: Option<bool>,
    tray_available: bool,
}

fn save_proactive(
    state: &AppState,
    update: impl FnOnce(&mut ProactiveSettings),
) -> Result<ProactiveSettings, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())?;
    let mut settings = store.proactive_settings().map_err(storage_error)?;
    update(&mut settings);
    store
        .proactive_save_settings(&settings)
        .map_err(storage_error)
}

fn read_lifecycle_status(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<LifecycleStatus, String> {
    let settings = {
        let store = state
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.".to_string())?;
        store.proactive_settings().map_err(storage_error)?
    };
    Ok(LifecycleStatus {
        close_to_background: settings.close_to_background,
        start_hidden: settings.start_hidden,
        launch_at_login: settings.launch_at_login,
        autostart_enabled: match core::lifecycle::autostart_status(app) {
            Ok(status) => Some(status.enabled),
            Err(error) => {
                eprintln!("Lifecycle: 자동 실행 상태를 확인하지 못했습니다: {error}");
                None
            }
        },
        tray_available: state.tray_available,
    })
}

#[tauri::command]
async fn lifecycle_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    read_lifecycle_status(&app, state.inner())
}

#[tauri::command]
async fn lifecycle_set_close_to_background(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    if enabled && !state.tray_available {
        return Err(
            "시스템 트레이를 사용할 수 없어 창을 닫을 때 백그라운드로 전환할 수 없습니다.".into(),
        );
    }
    save_proactive(state.inner(), |settings| {
        settings.close_to_background = enabled
    })?;
    read_lifecycle_status(&app, state.inner())
}

#[tauri::command]
async fn lifecycle_set_start_hidden(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    save_proactive(state.inner(), |settings| settings.start_hidden = enabled)?;
    read_lifecycle_status(&app, state.inner())
}

#[tauri::command]
async fn lifecycle_set_launch_at_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    let _guard = state
        .launch_at_login
        .lock()
        .map_err(|_| "자동 실행 설정 잠금을 획득하지 못했습니다.".to_string())?;
    let previous = core::lifecycle::autostart_status(&app)?;
    let changed = previous.enabled != enabled;
    if changed {
        core::lifecycle::set_autostart(&app, enabled)?;
    }
    if let Err(error) = save_proactive(state.inner(), |settings| settings.launch_at_login = enabled)
    {
        if changed {
            return match core::lifecycle::set_autostart(&app, previous.enabled) {
                Ok(_) => Err(format!("{error} 자동 실행은 이전 상태로 되돌렸습니다.")),
                Err(rollback) => {
                    eprintln!("Lifecycle: 자동 실행 롤백 실패: {rollback}");
                    Err(format!(
                        "{error} {rollback} 자동 실행 상태를 직접 확인해 주세요."
                    ))
                }
            };
        }
        return Err(error);
    }
    read_lifecycle_status(&app, state.inner())
}
#[tauri::command]
async fn set_extension(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?;
        state.registry.set(&store, &id, enabled)?;
    }
    if !enabled {
        state.archive.wait().await;
    }
    snapshot(&state).await
}
fn runtime_status(state: &AppState) -> WorkerStatus {
    state
        .worker
        .try_lock()
        .map(|mut w| w.status())
        .unwrap_or(WorkerStatus {
            state: "busy".into(),
            runtime_available: true,
            model: None,
            detail: Some("로컬 AI 작업을 처리하고 있습니다.".into()),
        })
}
#[tauri::command]
async fn worker_status(state: tauri::State<'_, AppState>) -> Result<WorkerStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    match state.worker.try_lock() {
        Ok(mut worker) => Ok(worker.refresh().await),
        Err(_) => Ok(runtime_status(&state)),
    }
}
fn main() {
    let mut builder = tauri::Builder::default()
        // The single-instance check runs before windows, the data lock and every
        // store/worker initialization. A second process exits in plugin setup.
        .plugin(tauri_plugin_single_instance::init(
            |app, arguments, _cwd| {
                core::lifecycle::handle_second_instance(
                    app,
                    core::lifecycle::LaunchMode::from_args(arguments),
                );
            },
        ))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Rhythm")
                .arg(core::lifecycle::AUTOSTART_ARG)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init());
    if let Some(key) = option_env!("RHYTHM_UPDATER_PUBKEY").filter(|key| !key.trim().is_empty()) {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().pubkey(key).build());
    }
    let app = builder
        .setup(|app| {
            core::install_menu(app.handle())?;
            let data_root = app.path().app_data_dir()?;
            let instance_lock = match core::lock_data_dir(&data_root) {
                Ok(lock) => lock,
                Err(error) => {
                    // Tauri panics for an Err returned from setup. An already-owned
                    // data directory is an ordinary rejected launch, not a crash.
                    eprintln!("{error}");
                    app.handle().exit(1);
                    return Ok(());
                }
            };
            backup::recover_pending_restore(&data_root).map_err(std::io::Error::other)?;
            let store =
                Store::open(&data_root.join("rhythm.sqlite3")).map_err(std::io::Error::other)?;
            store
                .catalog(&archive::definitions())
                .map_err(std::io::Error::other)?;
            let store = Arc::new(Mutex::new(store));
            let tray_available = match core::lifecycle::install_tray(app.handle()) {
                Ok(()) => true,
                Err(error) => {
                    // Close-to-background stays unavailable rather than hiding the
                    // only window with no way to restore it.
                    eprintln!("Lifecycle: 시스템 트레이를 만들지 못했습니다: {error}");
                    false
                }
            };
            let lifecycle: Arc<dyn core::lifecycle::LifecyclePreferences> =
                Arc::new(StoredLifecycle(store.clone()));
            let launch = core::lifecycle::LaunchMode::current();
            // A hidden login launch requires the tray as its restore path.
            // Without it reveal the window without taking focus; manual
            // `--hidden` launches retain their explicit behavior.
            if !launch.start_hidden(lifecycle.start_hidden(), tray_available) {
                if launch.autostart {
                    core::lifecycle::reveal_main_window(app.handle())?;
                } else {
                    core::lifecycle::show_main_window(app.handle())?;
                }
            }
            let operation_gate = Arc::new(operation_gate::AdmissionGate::default());
            let archive = archive::sync::Controller::new();
            let network = archive::client().map_err(std::io::Error::other)?;
            let proactive = proactive::Scheduler::new(
                store.clone(),
                archive.clone(),
                network.clone(),
                data_root.clone(),
                app.handle().clone(),
                operation_gate.clone(),
            )
            .map_err(std::io::Error::other)?;
            let executable = std::env::current_exe()?
                .parent()
                .ok_or("실행 파일 경로를 찾을 수 없습니다.")?
                .join("rhythm-ai-worker");
            let worker = tauri::async_runtime::block_on(async {
                let mut worker = Worker::launch(&executable);
                worker.refresh().await;
                worker
            });
            let hardware = intelligence::models::hardware(&data_root);
            let maintenance_shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.manage(updater::State::default());
            app.manage(AppState {
                launch_at_login: Mutex::new(()),
                _instance_lock: instance_lock,
                store,
                data_root,
                lifecycle,
                tray_available,
                proactive: Arc::new(proactive),
                registry: Registry::new(archive.clone()),
                archive,
                network,
                worker: Arc::new(tokio::sync::Mutex::new(worker)),
                analysis: Arc::new(intelligence::analysis::Controller::default()),
                models: intelligence::models::Controller::default(),
                hardware,
                ai_gate: tokio::sync::Mutex::new(()),
                ai_operation: intelligence::operations::Controller::default(),
                maintenance_shutdown: maintenance_shutdown.clone(),
                operation_gate,
            });
            tauri::async_runtime::spawn(intelligence::maintenance::run(
                app.handle().clone(),
                maintenance_shutdown,
            ));
            if let Err(error) = app.state::<AppState>().proactive.start() {
                // Proactive operation is optional. A scheduler start failure must
                // not take away archive, AI or settings access.
                eprintln!("Proactive: 스케줄러를 시작하지 못했습니다: {error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backup::backup_create,
            backup::backup_inspect,
            backup::backup_restore,
            backup::backup_cancel,
            updater::updater_check,
            updater::updater_install,
            updater::updater_restart,
            personalization::personalization_get,
            personalization::personalization_save_profile,
            personalization::personalization_save_interest,
            personalization::personalization_remove_interest,
            personalization::personalization_set_learning,
            personalization::personalization_reset,
            personalization::personalization_get_feedback,
            personalization::personalization_set_feedback,
            personalization::personalization_recommendations,
            notification_permission::notification_authorization,
            notification_permission::notification_request_permission,
            get_snapshot,
            save_settings,
            lifecycle_status,
            lifecycle_set_close_to_background,
            lifecycle_set_start_hidden,
            lifecycle_set_launch_at_login,
            proactive::proactive_snapshot,
            proactive::proactive_save,
            proactive::proactive_history,
            proactive::proactive_clear_history,
            set_extension,
            worker_status,
            intelligence::commands::ai_snapshot,
            intelligence::commands::ai_install,
            intelligence::commands::ai_cancel_download,
            intelligence::commands::ai_cancel_operation,
            intelligence::commands::ai_diagnostics,
            intelligence::commands::ai_register,
            intelligence::commands::ai_select,
            intelligence::commands::ai_load,
            intelligence::commands::ai_unload,
            intelligence::commands::ai_restart,
            intelligence::commands::ai_remove,
            intelligence::commands::ai_preferences,
            intelligence::commands::ai_analyze,
            intelligence::commands::ai_cancel,
            intelligence::commands::ai_results,
            intelligence::commands::ai_document_jobs,
            intelligence::commands::ai_recent,
            archive::archive_sources,
            archive::archive_subscribe,
            archive::archive_summary,
            archive::archive_query,
            archive::archive_document,
            archive::archive_state,
            archive::archive_start,
            archive::archive_cancel,
            archive::archive_status,
            archive::archive_retry_resource,
            archive::archive_resource_image,
            archive::archive_reveal_resource,
            archive::archive_open_original
        ])
        .build(tauri::generate_context!())
        .expect("Rhythm을 시작하지 못했습니다. 로컬 저장소와 시스템 환경을 확인해 주세요.");
    app.run(|handle, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            let Some(state) = handle.try_state::<AppState>() else {
                return;
            };
            let close_to_background = state.tray_available && state.lifecycle.close_to_background();
            core::lifecycle::handle_main_close(handle, &label, &api, close_to_background);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => core::lifecycle::handle_reopen(handle, has_visible_windows),
        tauri::RunEvent::Exit => {
            let Some(state) = handle.try_state::<AppState>() else {
                return; // A rejected second launch never opened the store or worker.
            };
            state
                .maintenance_shutdown
                .store(true, std::sync::atomic::Ordering::Release);
            state.registry.stop_all();
            state.ai_operation.cancel();
            state.analysis.stop();
            state.models.stop();
            tauri::async_runtime::block_on(async {
                state.proactive.shutdown().await;
                state.archive.wait().await;
                state.analysis.wait().await;
                state.models.wait().await;
                state.worker.lock().await.shutdown().await;
            });
        }
        _ => {}
    });
}
