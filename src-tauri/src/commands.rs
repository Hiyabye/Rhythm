//! Tauri commands for settings, onboarding, lifecycle preferences, extensions,
//! the calendar and the menu. Startup and shared state stay in `main.rs`.
use crate::{
    core, extensions,
    storage::{
        calendar::{CalendarTodo, TodoInput},
        meals::Meals,
        notice_dates::{CalendarNoticeDates, NoticeDate},
    },
    AppState,
};
use rhythm_core::{app::Snapshot, storage::Settings};
use serde::Serialize;

#[tauri::command]
pub async fn get_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    state.settings().snapshot().await
}
#[tauri::command]
pub async fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<Snapshot, String> {
    state.settings().save(settings).await
}
#[tauri::command]
pub async fn complete_onboarding(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    state.settings().complete_onboarding().await
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

async fn read_lifecycle_status(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<LifecycleStatus, String> {
    let settings = state.settings().proactive().await?;
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
    read_lifecycle_status(&app, state.inner()).await
}

#[tauri::command]
pub async fn lifecycle_set_close_to_background(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    if enabled && !state.tray_available {
        return Err(
            "시스템 트레이를 사용할 수 없어 창을 닫을 때 백그라운드로 전환할 수 없습니다.".into(),
        );
    }
    state.settings().set_close_to_background(enabled).await?;
    read_lifecycle_status(&app, state.inner()).await
}

#[tauri::command]
pub async fn lifecycle_set_start_hidden(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    state.settings().set_start_hidden(enabled).await?;
    read_lifecycle_status(&app, state.inner()).await
}

#[tauri::command]
pub async fn lifecycle_set_launch_at_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<LifecycleStatus, String> {
    set_launch_at_login(&app, state.inner(), enabled).await?;
    read_lifecycle_status(&app, state.inner()).await
}

/// Changes the OS login item and the stored preference together, rolling the
/// OS state back when the preference cannot be saved.
async fn set_launch_at_login(
    app: &tauri::AppHandle,
    state: &AppState,
    enabled: bool,
) -> Result<(), String> {
    let _guard = state.launch_at_login.lock().await;
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
    if let Err(error) = state.settings().set_launch_at_login(enabled).await {
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
    // A login item is OS state the flag cannot gate, so it is removed first;
    // if that fails the extension stays on.
    if !enabled && id == extensions::PROACTIVE {
        set_launch_at_login(&app, state.inner(), false).await?;
    }
    state.settings().set_extension(&id, enabled).await
}

#[cfg(test)]
pub(crate) fn change_extension(
    registry: &extensions::Registry,
    shared_store: &crate::archive::sync::SharedStore,
    id: &str,
    enabled: bool,
) -> Result<Option<crate::studium::controller::CompletionHandle>, String> {
    registry
        .change(shared_store, id, enabled, || None::<()>)
        .map(|changed| changed.studium)
}
#[tauri::command]
pub async fn calendar_todos(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Vec<CalendarTodo>, String> {
    state.calendar().todos(&from, &to).await
}
#[tauri::command]
pub async fn calendar_notice_dates(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<CalendarNoticeDates, String> {
    state.calendar().notice_dates(&from, &to).await
}
#[tauri::command]
pub async fn calendar_set_all_deadlines(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    state.calendar().set_all_deadlines(enabled).await
}
#[tauri::command]
pub async fn calendar_set_academic(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    state.calendar().set_academic(enabled).await
}
#[tauri::command]
pub async fn meal_weeks(state: tauri::State<'_, AppState>) -> Result<Meals, String> {
    state.meals().weeks().await
}
#[tauri::command]
pub async fn notice_dates(
    state: tauri::State<'_, AppState>,
    document_id: i64,
) -> Result<Vec<NoticeDate>, String> {
    state.archive().document_dates(document_id).await
}
#[tauri::command]
pub async fn calendar_create_todo(
    state: tauri::State<'_, AppState>,
    todo: TodoInput,
) -> Result<CalendarTodo, String> {
    state.calendar().create_todo(&todo).await
}
#[tauri::command]
pub async fn calendar_update_todo(
    state: tauri::State<'_, AppState>,
    id: i64,
    todo: TodoInput,
) -> Result<CalendarTodo, String> {
    state.calendar().update_todo(id, &todo).await
}
#[tauri::command]
pub async fn calendar_set_todo_done(
    state: tauri::State<'_, AppState>,
    id: i64,
    done: bool,
) -> Result<CalendarTodo, String> {
    state.calendar().set_todo_done(id, done).await
}
#[tauri::command]
pub async fn calendar_delete_todo(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    state.calendar().delete_todo(id).await
}
#[tauri::command]
pub async fn calendar_reminder_activation(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    state.calendar().reminder_activation().await
}
