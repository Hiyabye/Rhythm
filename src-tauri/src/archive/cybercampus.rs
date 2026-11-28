//! The Cybercampus commands; wrappers over `App::cybercampus()`.
use rhythm_core::{
    archive::cybercampus::Status,
    storage::cybercampus::{CybercampusCourse, CybercampusTask},
};

#[tauri::command]
pub async fn cybercampus_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    state.cybercampus().status().await
}

#[tauri::command]
pub async fn cybercampus_save_login(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    password: String,
) -> Result<Status, String> {
    state.cybercampus().save_login(&id, &password).await
}

#[tauri::command]
pub async fn cybercampus_forget_login(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    state.cybercampus().forget_login().await
}

#[tauri::command]
pub async fn cybercampus_courses(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CybercampusCourse>, String> {
    state.cybercampus().courses().await
}

#[tauri::command]
pub async fn cybercampus_tasks(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CybercampusTask>, String> {
    state.cybercampus().tasks().await
}
