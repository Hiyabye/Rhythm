//! The personalization commands; the methods are `App::personalization()`.
use crate::storage::personalization::*;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn personalization_get(
    state: State<'_, AppState>,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().snapshot().await
}
#[tauri::command]
pub async fn personalization_save_profile(
    state: State<'_, AppState>,
    profile: PersonalizationProfile,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().save_profile(profile).await
}
#[tauri::command]
pub async fn personalization_save_interest(
    state: State<'_, AppState>,
    interest: PersonalInterestInput,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().save_interest(interest).await
}
#[tauri::command]
pub async fn personalization_remove_interest(
    state: State<'_, AppState>,
    id: i64,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().remove_interest(id).await
}
#[tauri::command]
pub async fn personalization_set_learning(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().set_learning(enabled).await
}
#[tauri::command]
pub async fn personalization_reset(
    state: State<'_, AppState>,
) -> Result<PersonalizationSnapshot, String> {
    state.personalization().reset().await
}
#[tauri::command]
pub async fn personalization_get_feedback(
    state: State<'_, AppState>,
    document_id: i64,
) -> Result<Option<String>, String> {
    state.personalization().feedback(document_id).await
}
#[tauri::command]
pub async fn personalization_set_feedback(
    state: State<'_, AppState>,
    document_id: i64,
    feedback: Option<String>,
) -> Result<Option<String>, String> {
    state
        .personalization()
        .set_feedback(document_id, feedback)
        .await
}
#[tauri::command]
pub async fn personalization_recommendations(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<PersonalizationRecommendations, String> {
    state.personalization().recommendations(limit).await
}
