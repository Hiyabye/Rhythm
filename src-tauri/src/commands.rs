//! Tauri commands for settings, onboarding, lifecycle preferences, extensions and
//! the worker status. Startup and shared state stay in `main.rs`.
use crate::{
    archive::sync::with_store as locked,
    core,
    extensions::{self, ExtensionInfo},
    intelligence::WorkerStatus,
    storage::{
        calendar::{CalendarTodo, TodoInput},
        proactive::ProactiveSettings,
        Settings, StorageInfo,
    },
    AppState,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct Snapshot {
    settings: Settings,
    extensions: Vec<ExtensionInfo>,
    storage: StorageInfo,
    worker: WorkerStatus,
    proactive: ProactiveSettings,
    onboarding_completed: bool,
}
pub(crate) fn storage_error(error: String) -> String {
    eprintln!("Storage: {error}");
    "로컬 데이터를 읽거나 저장하지 못했습니다. 저장 공간과 파일 권한을 확인해 주세요.".into()
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
    let studium_completion =
        change_extension(&state.registry, &state.store, &state.studium, &id, enabled)?;
    if !enabled {
        match id.as_str() {
            extensions::NOTICES | extensions::CYBERCAMPUS => state.archive.wait().await,
            extensions::STUDIUM => {
                finish_studium_disable(&state.store, &state.studium, studium_completion).await?;
            }
            extensions::AI => {
                crate::intelligence::commands::stop_for_disabled_extension(state.inner()).await?
            }
            _ => {}
        }
    }
    snapshot(&state).await
}

pub(crate) fn change_extension(
    registry: &extensions::Registry,
    shared_store: &crate::archive::sync::SharedStore,
    studium: &crate::studium::controller::Controller,
    id: &str,
    enabled: bool,
) -> Result<Option<crate::studium::controller::CompletionHandle>, String> {
    let store = shared_store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.")?;
    registry.set(&store, id, enabled)?;
    Ok(if !enabled && id == extensions::STUDIUM {
        studium.completion()
    } else {
        None
    })
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
pub async fn worker_status(state: tauri::State<'_, AppState>) -> Result<WorkerStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    match state.worker.try_lock() {
        Ok(mut worker) => Ok(worker.refresh().await),
        Err(_) => Ok(runtime_status(&state)),
    }
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
