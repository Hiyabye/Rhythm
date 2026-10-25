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
    worker: tokio::sync::Mutex<Worker>,
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
    let worker = state.worker.lock().await.status();
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
#[tauri::command]
async fn worker_status(state: tauri::State<'_, AppState>) -> Result<WorkerStatus, String> {
    Ok(state.worker.lock().await.refresh().await)
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
            app.manage(AppState {
                _instance_lock: instance_lock,
                store: Arc::new(Mutex::new(store)),
                registry: Registry::new(archive.clone()),
                data_root,
                archive,
                network,
                worker: tokio::sync::Mutex::new(worker),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_settings,
            set_extension,
            worker_status,
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
            state.registry.stop_all();
            tauri::async_runtime::block_on(async {
                state.archive.wait().await;
                state.worker.lock().await.shutdown().await;
            });
        }
    });
}
