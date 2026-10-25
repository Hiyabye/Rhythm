#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod archive;
mod core;
mod extensions;
mod intelligence;
mod storage;
use extensions::{ExtensionInfo, Registry};
use intelligence::{Worker, WorkerStatus};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use storage::{Settings, StorageInfo, Store};
use tauri::Manager;

struct AppState {
    _instance_lock: std::fs::File,
    store: Arc<Mutex<Store>>,
    data_root: PathBuf,
    archive: Arc<archive::sync::Controller>,
    network: archive::Client,
    registry: Registry,
    worker: Arc<tokio::sync::Mutex<Worker>>,
    analysis: intelligence::analysis::Controller,
    models: intelligence::models::Controller,
    hardware: intelligence::models::Hardware,
    ai_gate: tokio::sync::Mutex<()>,
    maintenance_shutdown: Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Serialize)]
struct Snapshot {
    settings: Settings,
    extensions: Vec<ExtensionInfo>,
    storage: StorageInfo,
    worker: WorkerStatus,
}
fn storage_error(error: String) -> String {
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
    })
}
#[tauri::command]
async fn get_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    snapshot(&state).await
}
#[tauri::command]
async fn save_settings(
    state: tauri::State<'_, AppState>,
    settings: Settings,
) -> Result<Snapshot, String> {
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
#[tauri::command]
async fn set_extension(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Snapshot, String> {
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
    match state.worker.try_lock() {
        Ok(mut worker) => Ok(worker.refresh().await),
        Err(_) => Ok(runtime_status(&state)),
    }
}
fn main() {
    let app = tauri::Builder::default()
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
            let store =
                Store::open(&data_root.join("rhythm.sqlite3")).map_err(std::io::Error::other)?;
            store
                .catalog(&archive::definitions())
                .map_err(std::io::Error::other)?;
            let archive = archive::sync::Controller::new();
            let network = archive::client().map_err(std::io::Error::other)?;
            let executable = std::env::current_exe()?
                .parent()
                .ok_or("실행 파일 경로를 찾을 수 없습니다.")?
                .join(if cfg!(windows) {
                    "rhythm-ai-worker.exe"
                } else {
                    "rhythm-ai-worker"
                });
            let worker = tauri::async_runtime::block_on(async {
                let mut worker = Worker::launch(&executable);
                worker.refresh().await;
                worker
            });
            let hardware = intelligence::models::hardware(&data_root);
            let maintenance_shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.manage(AppState {
                _instance_lock: instance_lock,
                store: Arc::new(Mutex::new(store)),
                registry: Registry::new(archive.clone()),
                data_root,
                archive,
                network,
                worker: Arc::new(tokio::sync::Mutex::new(worker)),
                analysis: intelligence::analysis::Controller::default(),
                models: intelligence::models::Controller::default(),
                hardware,
                ai_gate: tokio::sync::Mutex::new(()),
                maintenance_shutdown: maintenance_shutdown.clone(),
            });
            tauri::async_runtime::spawn(intelligence::maintenance::run(
                app.handle().clone(),
                maintenance_shutdown,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_settings,
            set_extension,
            worker_status,
            intelligence::commands::ai_snapshot,
            intelligence::commands::ai_install,
            intelligence::commands::ai_cancel_download,
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
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            let Some(state) = handle.try_state::<AppState>() else {
                return; // A rejected second launch never opened the store or worker.
            };
            state
                .maintenance_shutdown
                .store(true, std::sync::atomic::Ordering::Release);
            state.registry.stop_all();
            state.analysis.stop();
            state.models.stop();
            tauri::async_runtime::block_on(async {
                state.archive.wait().await;
                state.analysis.wait().await;
                state.models.wait().await;
                state.worker.lock().await.shutdown().await;
            });
        }
    });
}
