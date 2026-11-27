#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod archive;
mod backup;
mod calendar_export;
mod commands;
pub(crate) use rhythm_core::app::storage_error;
mod core;
mod extensions;
mod feed;
mod intelligence;
mod notification_permission;
pub(crate) use rhythm_core::operation_gate;
mod personalization;
mod platform_support;
mod proactive;
mod shell;
use rhythm_core::storage;
mod studium;
mod updater;
use rhythm_core::app::{App, OpenError};
use std::sync::{Arc, Mutex};
use storage::{proactive::ProactiveSettings, Store};
use tauri::Manager;

/// The User-Agent of the app's own requests (models, feed): `Rhythm/<version>`.
/// `rhythm-core` cannot name it itself; its package version is 0.0.0.
pub(crate) const USER_AGENT: &str = concat!("Rhythm/", env!("CARGO_PKG_VERSION"));

/// The core app plus what only this Tauri shell owns: tray, login item and
/// the native Studium mini window. It derefs to the core `App`, so commands
/// still reach core handles as `state.store`, `state.settings()` and so on.
pub(crate) struct AppState {
    core: App,
    lifecycle: Arc<dyn core::lifecycle::LifecyclePreferences>,
    tray_available: bool,
    /// Serializes login-item changes, which span an OS call and a store write.
    launch_at_login: tokio::sync::Mutex<()>,
    pub(crate) studium_views: Arc<studium::views::Views>,
}

impl std::ops::Deref for AppState {
    type Target = App;
    fn deref(&self) -> &App {
        &self.core
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
            } else if uri.starts_with(proactive::reminder_prefix()) {
                state.proactive.handle_windows_reminder(&uri).await
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
    if let Some(uri) = {
        let identity = core::windows_identity::current();
        arguments.iter().find(|value| {
            value.starts_with(identity.notification_prefix.as_str())
                || value.starts_with(identity.studium_prefix.as_str())
                || value.starts_with(identity.reminder_prefix.as_str())
        })
    } {
        if proactive::windows_notification::token_from_uri(uri).is_none()
            && studium::notification::token_from_uri(uri).is_none()
            && proactive::reminder_token_from_uri(uri).is_none()
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

/// Appended to a failed restore recovery error in its dialog.
const RESTORE_FAILED_NOTE: &str =
    "\n\n기존 파일은 그대로 두었습니다. 원인을 확인한 뒤 다시 실행해 주세요.";
const START_FAILED_NOTE: &str =
    "\n\n저장된 데이터는 그대로 두었습니다. 원인을 확인한 뒤 다시 실행해 주세요.";

/// Routes SIGTERM, SIGINT and SIGHUP (`kill`, Ctrl-C, logout) through the
/// normal exit, so `RunEvent::Exit` stops jobs and reaps the embedding server
/// (#273). SIGKILL and crashes are left to `intelligence::stop_orphan` at the
/// next launch.
#[cfg(unix)]
async fn quit_on_signal(app: tauri::AppHandle) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut interrupt), Ok(mut hangup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        eprintln!("종료 신호 처리를 설정하지 못했습니다.");
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = interrupt.recv() => {}
        _ = hangup.recv() => {}
    }
    app.exit(0);
}

fn main() {
    let context = tauri::generate_context!();
    #[cfg(windows)]
    {
        core::windows_identity::init(
            &context.config().identifier,
            context.config().product_name.as_deref(),
        );
        if let Err(error) = proactive::windows_notification::initialize_process_identity() {
            eprintln!("Windows 알림 프로세스 식별자를 설정하지 못했습니다: {error}");
        }
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
            let opened = App::open(rhythm_core::app::Config {
                identifier: app.config().identifier.clone(),
                user_agent: USER_AGENT,
                runtime: tauri::async_runtime::handle().inner().clone(),
                shell: Arc::new(shell::AppShell(app.handle().clone())),
            });
            let core = match opened {
                Ok(core) => core,
                Err(OpenError::Locked(error)) => {
                    // Tauri panics for an Err returned from setup. An already-owned
                    // data directory is an ordinary rejected launch, not a crash.
                    eprintln!("{error}");
                    app.handle().exit(1);
                    return Ok(());
                }
                Err(OpenError::Restore(error)) => {
                    // The journal and every previous file stay in place, so a later
                    // launch retries. Explain instead of panicking out of setup.
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("{error}{RESTORE_FAILED_NOTE}"))
                        .title("백업 복원을 마무리하지 못했습니다")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
                Err(OpenError::Failed(error)) => {
                    // The store could not be opened (a damaged or newer database,
                    // no disk space, no permission). Nothing was changed; explain
                    // instead of panicking out of setup with no message.
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("{error}{START_FAILED_NOTE}"))
                        .title("Rhythm을 시작하지 못했습니다")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
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
                Arc::new(StoredLifecycle(core.store.clone()));
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
            let studium_views = studium::views::Views::new(
                core.store.clone(),
                Arc::downgrade(&core.studium),
                core.operation_gate.clone(),
                app.handle().clone(),
            );
            shell::forward_events(app.handle().clone(), core.events());
            app.manage(updater::State::default());
            app.manage(AppState {
                core,
                lifecycle,
                tray_available,
                launch_at_login: tokio::sync::Mutex::new(()),
                studium_views,
            });
            let state = app.state::<AppState>();
            state.start(Some(state.studium_views.clone()));
            dispatch_notification_arguments(
                app.handle(),
                &std::env::args_os()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            );
            #[cfg(windows)]
            notification_launches_ready(app.handle());
            tauri::async_runtime::spawn(updater::run_auto_check(
                app.handle().clone(),
                state.maintenance_shutdown.clone(),
            ));
            #[cfg(unix)]
            tauri::async_runtime::spawn(quit_on_signal(app.handle().clone()));
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
            commands::calendar_todos,
            commands::calendar_notice_dates,
            commands::calendar_set_all_deadlines,
            commands::calendar_set_academic,
            commands::meal_weeks,
            commands::notice_dates,
            commands::calendar_create_todo,
            commands::calendar_update_todo,
            commands::calendar_set_todo_done,
            commands::calendar_delete_todo,
            calendar_export::calendar_export,
            proactive::calendar_reminder_activation,
            intelligence::commands::ai_snapshot,
            intelligence::commands::ai_choose_model_directory,
            intelligence::commands::ai_reset_model_directory,
            intelligence::commands::ai_install,
            intelligence::commands::ai_cancel_download,
            intelligence::commands::ai_remove,
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
            archive::archive_fetch_body,
            archive::archive_fetch_resources,
            archive::archive_state,
            archive::archive_trash,
            archive::archive_restore,
            archive::archive_purge,
            archive::archive_start,
            archive::archive_cancel,
            archive::archive_status,
            feed::feed_status,
            feed::feed_sync,
            archive::archive_retry_resource,
            archive::archive_resource_image,
            archive::archive_reveal_resource,
            archive::archive_open_original,
            archive::cybercampus::cybercampus_status,
            archive::cybercampus::cybercampus_save_login,
            archive::cybercampus::cybercampus_forget_login,
            archive::cybercampus::cybercampus_courses,
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
            tauri::async_runtime::block_on(state.shutdown());
        }
        _ => {}
    });
}
