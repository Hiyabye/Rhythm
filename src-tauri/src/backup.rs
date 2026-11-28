//! Backup and restore commands; wrappers over `App::backup()`.
use rhythm_core::backup::{BackupPreview, RestoreOutcome};
use std::path::PathBuf;

#[tauri::command]
pub(crate) fn backup_restore_support(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    state.backup().restore_support()
}

#[tauri::command]
pub(crate) fn backup_cancel(
    state: tauri::State<'_, crate::AppState>,
    operation_id: String,
) -> Result<bool, String> {
    state.backup().cancel(&operation_id)
}

#[tauri::command]
pub(crate) async fn backup_create(
    state: tauri::State<'_, crate::AppState>,
    destination: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    state
        .backup()
        .create(PathBuf::from(destination), operation_id)
        .await
}

#[tauri::command]
pub(crate) async fn backup_inspect(
    state: tauri::State<'_, crate::AppState>,
    source: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    state
        .backup()
        .inspect(PathBuf::from(source), operation_id)
        .await
}

#[tauri::command]
pub(crate) fn backup_restore_outcome(
    state: tauri::State<'_, crate::AppState>,
) -> Option<RestoreOutcome> {
    state.backup().restore_outcome()
}

#[tauri::command]
pub(crate) async fn backup_restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    source: String,
    confirmed: bool,
    operation_id: String,
) -> Result<(), String> {
    state
        .backup()
        .restore(PathBuf::from(source), confirmed, operation_id, move || {
            app.exit(0)
        })
        .await
}
