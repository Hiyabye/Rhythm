#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod archive;
mod backup;
mod commands;
pub(crate) use commands::storage_error;
mod core;
mod extensions;
mod intelligence;
mod notification_permission;
pub(crate) mod operation_gate;
mod personalization;
mod platform_support;
mod proactive;
mod storage;
mod studium;
mod updater;
use extensions::Registry;
use intelligence::Worker;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use storage::{proactive::ProactiveSettings, Store};
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
    pub(crate) studium: Arc<studium::controller::Controller>,
    pub(crate) studium_notifications: Arc<studium::notification::Notifications>,
    pub(crate) studium_views: Arc<studium::views::Views>,
    network: archive::Client,
    registry: Registry,
    pub(crate) worker: Arc<tokio::sync::Mutex<Worker>>,
    pub(crate) classifier: Arc<intelligence::classification::Controller>,
    pub(crate) analysis: Arc<intelligence::analysis::Controller>,
    pub(crate) remote: Arc<intelligence::remote::Remote>,
    pub(crate) models: intelligence::models::Controller,
    hardware: intelligence::models::Hardware,
    ai_gate: tokio::sync::Mutex<()>,
    pub(crate) ai_operation: intelligence::operations::Controller,
    maintenance_shutdown: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) operation_gate: Arc<operation_gate::AdmissionGate>,
    pub(crate) restore_outcome: Option<backup::RestoreOutcome>,
}

pub(crate) struct AppMaintenanceGuard {
    _gate: operation_gate::MaintenanceGuard,
    proactive: Arc<proactive::Scheduler>,
    studium_notifications: Arc<studium::notification::Notifications>,
    studium_views: Arc<studium::views::Views>,
    analysis: Arc<intelligence::analysis::Controller>,
    classifier_guard: Option<intelligence::classification::PauseGuard>,
}

impl AppMaintenanceGuard {
    /// Keeps maintenance closed and work stopped for the rest of the process.
    /// `RunEvent::Exit` stops the classifier by taking its batch guard, so an
    /// exit requested while this guard is held must release that guard first,
    /// or the main thread blocks forever and the window stops responding.
    pub(crate) fn hold_until_exit(mut self) {
        if let Some(classifier_guard) = self.classifier_guard.take() {
            classifier_guard.release_for_exit();
        }
        std::mem::forget(self);
    }
}

impl Drop for AppMaintenanceGuard {
    fn drop(&mut self) {
        drop(self.classifier_guard.take());
        self.analysis.resume();
        if let Err(error) = self.proactive.start() {
            eprintln!("Proactive: 유지보수 후 스케줄러를 다시 시작하지 못했습니다: {error}");
        }
        if let Err(error) = self.studium_notifications.start() {
            eprintln!("Studium: 유지보수 후 알림 수신을 다시 시작하지 못했습니다: {error}");
        }
        if let Err(error) = self.studium_views.start() {
            eprintln!("Studium: 표시 재개 실패: {error}");
        }
    }
}

struct MaintenancePreparation {
    reservation: Option<operation_gate::MaintenanceReservation>,
    proactive: Arc<proactive::Scheduler>,
    studium_notifications: Arc<studium::notification::Notifications>,
    studium_views: Arc<studium::views::Views>,
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
        let studium_notifications = self.studium_notifications.clone();
        let studium_views = self.studium_views.clone();
        tauri::async_runtime::spawn(async move {
            proactive.resume_after_quiesce().await;
            studium_notifications.resume_after_quiesce().await;
            studium_views.resume_after_quiesce().await;
        });
    }
}

impl AppState {
    pub(crate) async fn exclusive_maintenance(&self) -> Result<AppMaintenanceGuard, String> {
        let (reservation, studium_completion) = studium::commands::reserve_maintenance(
            &self.operation_gate,
            &self.store,
            &self.studium,
        )?;
        let mut preparation = MaintenancePreparation {
            reservation: Some(reservation),
            proactive: self.proactive.clone(),
            studium_notifications: self.studium_notifications.clone(),
            studium_views: self.studium_views.clone(),
            analysis: self.analysis.clone(),
            armed: true,
        };
        self.archive.cancel();
        self.analysis.stop();
        self.models.cancel();
        self.ai_operation.cancel();
        let ai_gate = self.ai_gate.lock().await;
        self.analysis.stop();
        self.models.cancel();
        self.ai_operation.cancel();
        drop(ai_gate);
        self.proactive.quiesce().await;
        self.studium_notifications.quiesce().await;
        self.studium_views.quiesce().await;
        self.archive.wait().await;
        if let Some(completion) = studium_completion {
            completion.wait().await;
        }
        self.analysis.wait().await;
        self.models.wait().await;
        let gate = preparation
            .reservation
            .take()
            .expect("maintenance reservation remains until exclusive acquisition")
            .acquire()
            .await?;
        preparation.armed = false;
        // A classification batch writes SQLite; exclude it from backup/restore
        // snapshots and wait for one already in flight before returning.
        let classifier_guard = self.classifier.quiesce().await;
        Ok(AppMaintenanceGuard {
            _gate: gate,
            proactive: self.proactive.clone(),
            studium_notifications: self.studium_notifications.clone(),
            studium_views: self.studium_views.clone(),
            analysis: self.analysis.clone(),
            classifier_guard: Some(classifier_guard),
        })
    }
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
            .and_then(|store| store.proactive_effective_settings().map_err(storage_error))
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

#[cfg(windows)]
#[derive(Default)]
struct NotificationLaunchQueue {
    ready: bool,
    pending: std::collections::VecDeque<String>,
}

#[cfg(windows)]
impl NotificationLaunchQueue {
    fn offer(&mut self, uri: String) -> Result<Option<String>, ()> {
        if self.ready {
            return Ok(Some(uri));
        }
        if self.pending.len() >= 64 {
            return Err(());
        }
        self.pending.push_back(uri);
        Ok(None)
    }

    fn mark_ready(&mut self) -> Vec<String> {
        self.ready = true;
        self.pending.drain(..).collect()
    }
}

#[cfg(windows)]
static NOTIFICATION_LAUNCHES: std::sync::OnceLock<Mutex<NotificationLaunchQueue>> =
    std::sync::OnceLock::new();

#[cfg(windows)]
fn notification_launches() -> &'static Mutex<NotificationLaunchQueue> {
    NOTIFICATION_LAUNCHES.get_or_init(|| Mutex::new(NotificationLaunchQueue::default()))
}

#[cfg(windows)]
fn spawn_notification_activation(app: &tauri::AppHandle, uri: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(state) = app.try_state::<AppState>() {
            let result = if studium::notification::token_from_uri(&uri).is_some() {
                state
                    .studium_notifications
                    .handle_windows_activation(&uri)
                    .await
            } else {
                state.proactive.handle_windows_activation(&uri).await
            };
            if let Err(error) = result {
                eprintln!("알림을 열지 못했습니다: {error}");
            }
        } else {
            eprintln!("알림을 처리할 앱 상태를 사용할 수 없습니다.");
        }
    });
}

fn dispatch_notification_arguments(app: &tauri::AppHandle, arguments: &[String]) -> bool {
    #[cfg(windows)]
    if let Some(uri) = arguments.iter().find(|value| {
        value.starts_with("rhythm-win://notification/")
            || value.starts_with(studium::notification::TOKEN_PREFIX)
    }) {
        if proactive::windows_notification::token_from_uri(uri).is_none()
            && studium::notification::token_from_uri(uri).is_none()
        {
            eprintln!("올바르지 않은 알림 연결을 거부했습니다.");
            return true;
        }
        // Readiness and enqueue are guarded together, preventing a notification
        // arriving at the end of setup from falling between the two paths.
        let action = notification_launches()
            .lock()
            .map(|mut queue| queue.offer(uri.clone()));
        match action {
            Ok(Ok(Some(uri))) => spawn_notification_activation(app, uri),
            Ok(Ok(None)) => {}
            _ => eprintln!("대기 중인 알림이 너무 많거나 상태를 확인할 수 없습니다."),
        }
        return true;
    }
    let _ = (app, arguments);
    false
}

#[cfg(windows)]
fn notification_launches_ready(app: &tauri::AppHandle) {
    let pending = match notification_launches().lock() {
        Ok(mut queue) => queue.mark_ready(),
        Err(_) => {
            eprintln!("대기 중인 알림을 확인할 수 없습니다.");
            return;
        }
    };
    // Before the WebView listener exists, the existing snapshot carries one
    // activation. Preserve the user's latest queued selection and leave older
    // tokens unconsumed; no lock crosses dispatch or an async boundary.
    if let Some(uri) = pending.into_iter().last() {
        spawn_notification_activation(app, uri);
    }
}

#[cfg(all(test, windows))]
mod notification_launch_queue_tests {
    use super::NotificationLaunchQueue;
    #[test]
    fn bounded_queue_drains_once_and_later_clicks_dispatch_immediately() {
        let mut queue = NotificationLaunchQueue::default();
        for i in 0..64 {
            assert_eq!(queue.offer(i.to_string()), Ok(None));
        }
        assert_eq!(queue.offer("overflow".into()), Err(()));
        let pending = queue.mark_ready();
        assert_eq!(pending, (0..64).map(|i| i.to_string()).collect::<Vec<_>>());
        assert!(queue.mark_ready().is_empty());
        assert_eq!(queue.offer("later".into()), Ok(Some("later".into())));
    }
}

fn main() {
    let context = tauri::generate_context!();
    #[cfg(windows)]
    if let Err(error) = proactive::windows_notification::initialize_process_identity() {
        eprintln!("Windows 알림 프로세스 식별자를 설정하지 못했습니다: {error}");
    }
    let mut builder = tauri::Builder::default()
        // The single-instance check runs before windows, the data lock and every
        // store/worker initialization. A second process exits in plugin setup.
        .plugin(tauri_plugin_single_instance::init(
            |app, arguments, _cwd| {
                if dispatch_notification_arguments(app, &arguments) {
                    return;
                }
                core::lifecycle::handle_second_instance(
                    app,
                    core::lifecycle::LaunchMode::from_args(arguments),
                );
            },
        ))
        .plugin(tauri_plugin_dialog::init());
    // Windows registers login startup directly in the per-user Run key
    // (`core::windows_autostart`); macOS keeps the LaunchAgent plugin. Both
    // name the entry after the bundle identifier, so a validation build never
    // writes or deletes the installed app's login registration.
    #[cfg(not(windows))]
    {
        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(core::lifecycle::autostart_name(
                    &context.config().identifier,
                ))
                .arg(core::lifecycle::AUTOSTART_ARG)
                .build(),
        );
    }
    if let Some(key) = option_env!("RHYTHM_UPDATER_PUBKEY").filter(|key| !key.trim().is_empty()) {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().pubkey(key).build());
    }
    let app = builder
        .setup(|app| {
            core::install_menu(app.handle())?;
            // Windows keeps the archive and model weights in the non-roaming
            // profile; on macOS both resolve to Application Support.
            #[cfg(windows)]
            let default_root = app.path().app_local_data_dir()?;
            #[cfg(not(windows))]
            let default_root = app.path().app_data_dir()?;
            let data_root = if app.config().identifier.ends_with(".validation") {
                std::env::var_os("RHYTHM_VALIDATION_DATA_DIR")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .unwrap_or(default_root)
            } else {
                default_root
            };
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
            let restore_outcome = match backup::recover_pending_restore(&data_root) {
                Ok(outcome) => outcome,
                Err(error) => {
                    // The journal and every previous file stay in place, so a later
                    // launch retries. Explain instead of panicking out of setup.
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("{error}\n\n기존 파일은 보존했습니다. 오류 원인을 확인한 뒤 다시 실행해 주세요."))
                        .title("백업 복원을 마무리하지 못했습니다")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
            backup::remove_stale_backup_snapshots(&data_root);
            let store =
                Store::open(&data_root.join("rhythm.sqlite3")).map_err(std::io::Error::other)?;
            if restore_outcome.is_some() {
                store
                    .reset_remote_after_restore()
                    .map_err(std::io::Error::other)?;
            }
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
            let studium = studium::controller::Controller::new();
            let studium_notifications = studium::notification::Notifications::new(
                store.clone(), Arc::downgrade(&studium), operation_gate.clone(), app.handle().clone(),
            );
            studium.attach_notifications(&studium_notifications);
            let studium_views = studium::views::Views::new(store.clone(), Arc::downgrade(&studium), operation_gate.clone(), app.handle().clone());
            studium.attach_views(&studium_views);
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
            // Bundled llama.cpp builds; the private runtime directory holds the
            // server's socket or per-launch access key.
            let llama = app.path().resource_dir().map_err(std::io::Error::other)?.join("llama");
            let runtime = data_root.join("run");
            // The embedding process needs its own socket/key directory; sharing
            // `run/` would let either server remove the other's files.
            let classifier = Arc::new(intelligence::classification::Controller::new(
                &llama,
                &runtime.join("embedding"),
            ));
            let worker = tauri::async_runtime::block_on(async {
                let mode = store.lock().map_err(|_| std::io::Error::other("설정 잠금 오류"))?
                    .ai_preferences().map_err(std::io::Error::other)?.execution_mode;
                let mut worker = Worker::launch_configured(&llama, &runtime, &mode);
                worker.refresh().await;
                Ok::<_, std::io::Error>(worker)
            })?;
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
                registry: Registry::new(archive.clone(), studium.clone()),
                studium,
                studium_notifications: studium_notifications.clone(),
                studium_views: studium_views.clone(),
                archive,
                network,
                worker: Arc::new(tokio::sync::Mutex::new(worker)),
                classifier,
                analysis: Arc::new(intelligence::analysis::Controller::default()),
                remote: Arc::new(intelligence::remote::Remote::native()),
                models: intelligence::models::Controller::default(),
                hardware,
                ai_gate: tokio::sync::Mutex::new(()),
                ai_operation: intelligence::operations::Controller::default(),
                maintenance_shutdown: maintenance_shutdown.clone(),
                operation_gate,
                restore_outcome,
            });
            if let Err(error) = studium_notifications.start() {
                eprintln!("Studium: 알림 수신을 시작하지 못했습니다: {error}");
            }
            if let Err(error) = studium_views.start() { eprintln!("Studium: 표시 수신 시작 실패: {error}"); }
            dispatch_notification_arguments(app.handle(), &std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>());
            #[cfg(windows)]
            notification_launches_ready(app.handle());
            tauri::async_runtime::spawn(updater::run_auto_check(
                app.handle().clone(),
                maintenance_shutdown.clone(),
            ));
            tauri::async_runtime::spawn(intelligence::classification::run(
                app.handle().clone(),
                maintenance_shutdown.clone(),
            ));
            tauri::async_runtime::spawn(archive::run_auto_trash(
                app.handle().clone(),
                maintenance_shutdown.clone(),
            ));
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
            studium::commands::studium_snapshot,
            studium::commands::studium_status,
            studium::commands::studium_save_preferences,
            studium::commands::studium_set_view_room,
            studium::commands::studium_start,
            studium::commands::studium_cancel,
            studium::commands::studium_save_interval,
            studium::commands::studium_save_favorites,
            studium::commands::studium_save_alert,
            studium::commands::studium_notification_activation,
            studium::views::studium_open_mini,
            studium::views::studium_close_mini,
            studium::views::studium_mini_status,
            studium::views::studium_mini_set_on_top,
            studium::views::studium_mini_show_main,
            studium::commands::studium_automatic_start,
            studium::commands::studium_automatic_pause,
            backup::backup_create,
            backup::backup_inspect,
            backup::backup_restore,
            backup::backup_cancel,
            backup::backup_restore_outcome,
            backup::backup_restore_support,
            platform_support::platform_capabilities,
            updater::updater_configured,
            updater::updater_check,
            updater::updater_install,
            updater::updater_restart,
            updater::updater_auto_check,
            updater::updater_set_auto_check,
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
            commands::get_snapshot,
            commands::save_settings,
            commands::complete_onboarding,
            commands::lifecycle_status,
            commands::lifecycle_set_close_to_background,
            commands::lifecycle_set_start_hidden,
            commands::lifecycle_set_launch_at_login,
            proactive::proactive_snapshot,
            proactive::proactive_save,
            proactive::proactive_history,
            proactive::proactive_clear_history,
            commands::set_extension,
            commands::worker_status,
            commands::calendar_todos,
            commands::calendar_create_todo,
            commands::calendar_update_todo,
            commands::calendar_set_todo_done,
            commands::calendar_delete_todo,
            intelligence::commands::ai_snapshot,
            intelligence::commands::ai_choose_model_directory,
            intelligence::commands::ai_reset_model_directory,
            intelligence::commands::ai_install,
            intelligence::commands::ai_install_projector,
            intelligence::commands::ai_remove_projector,
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
            intelligence::commands::ai_remote_status,
            intelligence::commands::ai_remote_key,
            intelligence::commands::ai_remote_key_clear,
            intelligence::commands::ai_remote_test,
            intelligence::commands::ai_analyze,
            intelligence::commands::ai_cancel,
            intelligence::commands::ai_results,
            intelligence::commands::ai_document_jobs,
            intelligence::commands::ai_recent,
            intelligence::commands::ai_classifications,
            intelligence::commands::ai_classification,
            intelligence::commands::ai_similar_notices,
            intelligence::commands::ai_duplicate_notices,
            intelligence::commands::ai_classification_preferences,
            archive::archive_sources,
            archive::archive_subscribe,
            archive::archive_source_groups,
            archive::archive_set_group,
            archive::archive_summary,
            archive::archive_query,
            archive::archive_ranked_query,
            archive::archive_document,
            archive::archive_state,
            archive::archive_trash,
            archive::archive_restore,
            archive::archive_purge,
            archive::archive_start,
            archive::archive_cancel,
            archive::archive_status,
            archive::archive_retry_resource,
            archive::archive_resource_image,
            archive::archive_reveal_resource,
            archive::archive_open_original,
            archive::cybercampus::cybercampus_status,
            archive::cybercampus::cybercampus_save_login,
            archive::cybercampus::cybercampus_forget_login,
            archive::cybercampus::cybercampus_tasks
        ])
        .build(context)
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
        tauri::RunEvent::Reopen { .. } => core::lifecycle::handle_reopen(handle),
        tauri::RunEvent::Exit => {
            let Some(state) = handle.try_state::<AppState>() else {
                return; // A rejected second launch never opened the store or worker.
            };
            state.studium_views.save_exit_geometry();
            state
                .maintenance_shutdown
                .store(true, std::sync::atomic::Ordering::Release);
            // Serialize cancellation against a start already admitted by the
            // gate but not yet registered. Starts recheck shutdown under Store.
            let studium_completion =
                studium::commands::cancel_at_boundary(&state.store, &state.studium)
                    .ok()
                    .flatten();
            state.registry.stop_all();
            state.ai_operation.cancel();
            state.analysis.stop();
            state.models.cancel();
            tauri::async_runtime::block_on(async {
                state.studium_views.shutdown().await;
                state.studium_notifications.shutdown().await;
                state.proactive.shutdown().await;
                state.archive.wait().await;
                if let Some(completion) = studium_completion {
                    completion.wait().await;
                } else {
                    // A poisoned Store prevented capture, but Registry still
                    // signalled cancellation; exit must join that job as well.
                    state.studium.wait().await;
                }
                state.analysis.wait().await;
                state.models.wait().await;
                state.classifier.shutdown().await;
                state.worker.lock().await.shutdown().await;
            });
        }
        _ => {}
    });
}
