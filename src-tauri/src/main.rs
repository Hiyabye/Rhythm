#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod core;
mod extensions;
mod intelligence;
mod storage;
use extensions::{ExtensionInfo, Registry};
use intelligence::{Worker, WorkerStatus};
use serde::Serialize;
use std::sync::Mutex;
use storage::{Settings, StorageInfo, Store};
use tauri::Manager;

struct AppState {
    store: Mutex<Store>,
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
            let path = app.path().app_data_dir()?.join("rhythm.sqlite3");
            let store = Store::open(&path).map_err(std::io::Error::other)?;
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
                store: Mutex::new(store),
                registry: Registry::new(),
                worker: tokio::sync::Mutex::new(worker),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_settings,
            set_extension,
            worker_status
        ])
        .build(tauri::generate_context!())
        .expect("Rhythm을 시작하지 못했습니다. 로컬 저장소와 시스템 환경을 확인해 주세요.");
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            let state = handle.state::<AppState>();
            state.registry.stop_all();
            tauri::async_runtime::block_on(async {
                state.worker.lock().await.shutdown().await;
            });
        }
    });
}
