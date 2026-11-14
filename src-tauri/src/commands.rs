//! Tauri commands for settings, onboarding, lifecycle preferences, extensions and
//! the calendar. Startup and shared state stay in `main.rs`.
use crate::{
    archive::sync::with_store as locked,
    core,
    extensions::{self, ExtensionInfo},
    storage::{
        calendar::{CalendarTodo, TodoInput},
        notice_dates::{CalendarNoticeDates, NoticeDate},
        proactive::ProactiveSettings,
        Settings, StorageInfo,
    },
    AppState,
};
use serde::Serialize;
use tauri::Emitter;

#[derive(Serialize)]
pub struct Snapshot {
    settings: Settings,
    extensions: Vec<ExtensionInfo>,
    storage: StorageInfo,
    proactive: ProactiveSettings,
    onboarding_completed: bool,
}
pub(crate) fn storage_error(error: String) -> String {
    eprintln!("Storage: {error}");
    "로컬 데이터를 읽거나 저장하지 못했습니다. 저장 공간과 파일 권한을 확인해 주세요.".into()
}

async fn snapshot(state: &AppState) -> Result<Snapshot, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "데이터 저장소에 접근할 수 없습니다.")?;
    Ok(Snapshot {
        settings: store.settings().map_err(storage_error)?,
        extensions: state.registry.list(&store).map_err(storage_error)?,
        storage: store.info().map_err(storage_error)?,
        proactive: store.proactive_settings().map_err(storage_error)?,
        onboarding_completed: store.onboarding_completed().map_err(storage_error)?,
    })
}
#[tauri::command]
pub async fn get_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    snapshot(&state).await
}
#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?;
        store
            .save(&settings)
            .and_then(|()| store.auto_trash_old())
            .map_err(storage_error)?;
    }
    let _ = app.emit("rhythm://theme-changed", &settings.theme);
    let _ = app.emit("rhythm://clock-format-changed", &settings.clock_format);
    state.studium_views.refresh();
    snapshot(&state).await
}
/// Ends the first-run setup flow. A rerun from Settings ends the same way, so
/// completion is never cleared.
#[tauri::command]
pub async fn complete_onboarding(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    {
        state
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?
            .complete_onboarding()
            .map_err(storage_error)?;
    }
    snapshot(&state).await
}

#[derive(Serialize)]
pub struct LifecycleStatus {
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
pub async fn lifecycle_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    read_lifecycle_status(&app, state.inner())
}

#[tauri::command]
pub async fn lifecycle_set_close_to_background(
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
pub async fn lifecycle_set_start_hidden(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    save_proactive(state.inner(), |settings| settings.start_hidden = enabled)?;
    read_lifecycle_status(&app, state.inner())
}

#[tauri::command]
pub async fn lifecycle_set_launch_at_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    set_launch_at_login(&app, state.inner(), enabled)?;
    read_lifecycle_status(&app, state.inner())
}

/// Changes the OS login item and the stored preference together, rolling the
/// OS state back when the preference cannot be saved.
fn set_launch_at_login(
    app: &tauri::AppHandle,
    state: &AppState,
    enabled: bool,
) -> Result<(), String> {
    let _guard = state
        .launch_at_login
        .lock()
        .map_err(|_| "자동 실행 설정 잠금을 획득하지 못했습니다.".to_string())?;
    let previous = core::lifecycle::autostart_status(app)?;
    let changed = previous.enabled != enabled;
    #[cfg(windows)]
    let autostart_snapshot = if changed {
        Some(core::lifecycle::autostart_snapshot(app)?)
    } else {
        None
    };
    if changed {
        core::lifecycle::set_autostart(app, enabled)?;
    }
    if let Err(error) = save_proactive(state, |settings| settings.launch_at_login = enabled) {
        if changed {
            #[cfg(windows)]
            let rollback = core::lifecycle::restore_autostart(
                autostart_snapshot
                    .as_ref()
                    .expect("snapshot captured before changing autostart"),
            );
            #[cfg(not(windows))]
            let rollback = core::lifecycle::set_autostart(app, previous.enabled).map(|_| ());
            return match rollback {
                Ok(()) => Err(format!("{error} 자동 실행은 이전 상태로 되돌렸습니다.")),
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
    Ok(())
}
#[tauri::command]
pub async fn set_extension(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    // A login item is OS state the flag cannot gate, so it is removed first;
    // if that fails the extension stays on.
    if !enabled && id == extensions::PROACTIVE {
        set_launch_at_login(&app, state.inner(), false)?;
    }
    let (studium_completion, notification_completion, view_ticket) =
        change_extension_with_notifications(
            &state.registry,
            &state.store,
            &state.studium,
            &id,
            enabled,
        )?;
    if !enabled {
        let view_result = match view_ticket {
            Some(ticket) => state.studium_views.disable(ticket).await,
            None => Ok(()),
        };
        if let Some(completion) = notification_completion {
            completion.wait().await;
        }
        match id.as_str() {
            extensions::NOTICES | extensions::CYBERCAMPUS => state.archive.wait().await,
            extensions::STUDIUM => {
                finish_studium_disable(&state.store, &state.studium, studium_completion).await?;
            }
            // The classifier sees the extension off on its next pass and
            // stops its process; wake it instead of waiting for the timer.
            extensions::AI => state.classifier.wake(),
            _ => {}
        }
        view_result?;
    } else if matches!(id.as_str(), extensions::STUDIUM | extensions::PROACTIVE) {
        state.studium_notifications.resume_after_quiesce().await;
    }
    state.studium_views.refresh();
    snapshot(&state).await
}

#[cfg(test)]
pub(crate) fn change_extension(
    registry: &extensions::Registry,
    shared_store: &crate::archive::sync::SharedStore,
    studium: &crate::studium::controller::Controller,
    id: &str,
    enabled: bool,
) -> Result<Option<crate::studium::controller::CompletionHandle>, String> {
    change_extension_with_notifications(registry, shared_store, studium, id, enabled)
        .map(|(completion, _, _)| completion)
}

type ExtensionCompletions = (
    Option<crate::studium::controller::CompletionHandle>,
    Option<crate::studium::notification::Completion>,
    Option<crate::studium::views::CloseTicket>,
);

fn change_extension_with_notifications(
    registry: &extensions::Registry,
    shared_store: &crate::archive::sync::SharedStore,
    studium: &crate::studium::controller::Controller,
    id: &str,
    enabled: bool,
) -> Result<ExtensionCompletions, String> {
    let store = shared_store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.")?;
    registry.set(&store, id, enabled)?;
    let notifications = if !enabled && matches!(id, extensions::STUDIUM | extensions::PROACTIVE) {
        studium.cancel_notifications()
    } else {
        None
    };
    let view_ticket = if !enabled && id == extensions::STUDIUM {
        studium.capture_views_close()
    } else {
        None
    };
    Ok((
        if !enabled && id == extensions::STUDIUM {
            studium.completion()
        } else {
            None
        },
        notifications,
        view_ticket,
    ))
}

pub(crate) async fn finish_studium_disable(
    shared_store: &crate::archive::sync::SharedStore,
    studium: &crate::studium::controller::Controller,
    completion: Option<crate::studium::controller::CompletionHandle>,
) -> Result<(), String> {
    if let Some(completion) = completion {
        completion.wait().await;
    }
    let store = shared_store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.")?;
    // A later enable may have won while disable awaited. Never clear a newly
    // admitted generation's results in that case.
    if !store.enabled(extensions::STUDIUM)? {
        studium.clear();
    }
    Ok(())
}
#[tauri::command]
pub async fn calendar_todos(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Vec<CalendarTodo>, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_todos(&from, &to))
}
/// Dates found in notice text for the calendar range: saved notices' dates, plus
/// every notice's deadlines when the user turned that on.
#[tauri::command]
pub async fn calendar_notice_dates(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<CalendarNoticeDates, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_notice_dates(&from, &to))
}
#[tauri::command]
pub async fn calendar_set_all_deadlines(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.set_calendar_all_deadlines(enabled))
}
#[tauri::command]
pub async fn notice_dates(
    state: tauri::State<'_, AppState>,
    document_id: i64,
) -> Result<Vec<NoticeDate>, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.notice_dates(document_id))
}
#[tauri::command]
pub async fn calendar_create_todo(
    state: tauri::State<'_, AppState>,
    todo: TodoInput,
) -> Result<CalendarTodo, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_create_todo(&todo))
}
#[tauri::command]
pub async fn calendar_update_todo(
    state: tauri::State<'_, AppState>,
    id: i64,
    todo: TodoInput,
) -> Result<CalendarTodo, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_update_todo(id, &todo))
}
#[tauri::command]
pub async fn calendar_set_todo_done(
    state: tauri::State<'_, AppState>,
    id: i64,
    done: bool,
) -> Result<CalendarTodo, String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_set_todo_done(id, done))
}
#[tauri::command]
pub async fn calendar_delete_todo(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    locked(&state.store, |s| s.calendar_delete_todo(id))
}
