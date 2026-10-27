use crate::storage::{personalization::*, Store};
use crate::AppState;
use std::sync::{Arc, Mutex};
use tauri::State;
async fn call<T: Send + 'static>(
    store: Arc<Mutex<Store>>,
    f: impl FnOnce(&Store) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = store
            .lock()
            .map_err(|_| "로컬 저장소에 접근할 수 없습니다.".to_string())?;
        f(&guard)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn personalization_get(
    state: State<'_, AppState>,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), |s| s.personalization_snapshot()).await
}
#[tauri::command]
pub async fn personalization_save_profile(
    state: State<'_, AppState>,
    profile: PersonalizationProfile,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), move |s| {
        s.personalization_save_profile(&profile)
    })
    .await
}
#[tauri::command]
pub async fn personalization_save_interest(
    state: State<'_, AppState>,
    interest: PersonalInterestInput,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), move |s| {
        s.personalization_save_interest(&interest)
    })
    .await
}
#[tauri::command]
pub async fn personalization_remove_interest(
    state: State<'_, AppState>,
    id: i64,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), move |s| {
        s.personalization_remove_interest(id)
    })
    .await
}
#[tauri::command]
pub async fn personalization_set_learning(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), move |s| {
        s.personalization_set_learning(enabled)
    })
    .await
}
#[tauri::command]
pub async fn personalization_reset(
    state: State<'_, AppState>,
) -> Result<PersonalizationSnapshot, String> {
    call(state.store.clone(), |s| s.personalization_reset()).await
}
#[tauri::command]
pub async fn personalization_get_feedback(
    state: State<'_, AppState>,
    document_id: i64,
) -> Result<Option<String>, String> {
    call(state.store.clone(), move |s| {
        s.personalization_feedback(document_id)
    })
    .await
}
#[tauri::command]
pub async fn personalization_set_feedback(
    state: State<'_, AppState>,
    document_id: i64,
    feedback: Option<String>,
) -> Result<Option<String>, String> {
    call(state.store.clone(), move |s| {
        s.personalization_set_feedback(document_id, feedback.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn personalization_recommendations(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<PersonalizationRecommendations, String> {
    call(state.store.clone(), move |s| {
        s.personalization_recommendations(limit)
    })
    .await
}
