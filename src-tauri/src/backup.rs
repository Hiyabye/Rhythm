//! Backup and restore commands; the logic lives in `rhythm_core::backup`.
pub(crate) use rhythm_core::backup::*;
use std::path::{Path, PathBuf};

#[tauri::command]
pub(crate) fn backup_restore_support(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    restore_support(&state.data_root)
}

#[tauri::command]
pub(crate) fn backup_cancel(operation_id: String) -> Result<bool, String> {
    cancel(&operation_id)
}

#[tauri::command]
pub(crate) async fn backup_create(
    state: tauri::State<'_, crate::AppState>,
    destination: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    let operation = begin(operation_id)?;
    let maintenance = state.exclusive_maintenance().await?;
    let root = state.data_root.clone();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let _maintenance = maintenance;
        operation.create(&store, &root, &PathBuf::from(destination))
    })
    .await
    .map_err(|_| "백업 작업을 마치지 못했습니다.".to_owned())?
}

#[tauri::command]
pub(crate) async fn backup_inspect(
    source: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    let operation = begin(operation_id)?;
    tokio::task::spawn_blocking(move || operation.inspect(Path::new(&source)))
        .await
        .map_err(|_| "백업 파일을 검사하지 못했습니다.".to_owned())?
}

/// The restore this launch finalized, if any; it stays available for the
/// whole session so a remounted window still reports it.
#[tauri::command]
pub(crate) fn backup_restore_outcome(
    state: tauri::State<'_, crate::AppState>,
) -> Option<RestoreOutcome> {
    state.restore_outcome.clone()
}

#[tauri::command]
pub(crate) async fn backup_restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    source: String,
    confirmed: bool,
    operation_id: String,
) -> Result<(), String> {
    if !confirmed {
        return Err("복원을 시작하려면 확인이 필요합니다.".into());
    }
    let operation = begin(operation_id)?;
    restore_support(&state.data_root)?;
    let maintenance = state.exclusive_maintenance().await?;
    let root = state.data_root.clone();
    tokio::task::spawn_blocking(move || {
        operation.restore(&root, Path::new(&source), move || {
            // Keep maintenance closed until exit (see `Operation::restore`).
            maintenance.hold_until_exit();
            app.exit(0);
        })
    })
    .await
    .map_err(|_| "복원 준비를 마치지 못했습니다.".to_owned())??;
    Ok(())
}
