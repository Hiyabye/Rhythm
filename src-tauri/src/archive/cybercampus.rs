//! The Cybercampus commands; the connector itself is
//! `rhythm_core::archive::cybercampus`.
pub use rhythm_core::archive::cybercampus::*;
use rhythm_core::storage::{
    cybercampus::{CybercampusCourse, CybercampusTask},
    with_store,
};
use std::sync::atomic::AtomicBool;

fn status(state: &crate::AppState) -> Result<Status, String> {
    Status::read(&state.archive.cybercampus, &state.store)
}
#[tauri::command]
pub async fn cybercampus_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    let _lease = state.operation_gate.shared().await?;
    status(&state)
}
/// Checks the login against eClass and stores it only when the site accepts it.
#[tauri::command]
pub async fn cybercampus_save_login(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    password: String,
) -> Result<Status, String> {
    let _lease = state.operation_gate.shared().await?;
    let credentials = credentials::Credentials::new(&id, &password)?;
    if !with_store(&state.store, |s| s.enabled(crate::extensions::CYBERCAMPUS))? {
        return Err("사이버캠퍼스 확장 기능을 먼저 켜 주세요.".into());
    }
    if state.archive.status()?.running {
        return Err("동기화가 끝난 뒤 다시 시도해 주세요.".into());
    }
    let connector = &state.archive.cybercampus;
    connector
        .session
        .login(
            &client::Reqwest::new(crate::USER_AGENT),
            &credentials,
            &AtomicBool::new(false),
        )
        .await?;
    connector.vault.save(&credentials)?;
    status(&state)
}
/// Deletes the stored login and the in-memory session. Archived notices and
/// deadlines stay, like any other source's archive.
#[tauri::command]
pub async fn cybercampus_forget_login(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    let _lease = state.operation_gate.shared().await?;
    let connector = &state.archive.cybercampus;
    connector.vault.clear()?;
    connector.session.reset().await;
    status(&state)
}
/// The synced term's courses with their unread counts, for the sidebar. Unlike
/// `cybercampus_status` it does not read the credential store.
#[tauri::command]
pub async fn cybercampus_courses(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CybercampusCourse>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.cybercampus_courses())
}
#[tauri::command]
pub async fn cybercampus_tasks(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<CybercampusTask>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state.store, |s| s.cybercampus_tasks())
}
