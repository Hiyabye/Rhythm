//! The AI commands: one-line wrappers over `App::ai()`, except the folder
//! picker, whose native dialog belongs to the shell.
use crate::{
    storage::intelligence::{AiClassification, SimilarNotice},
    AppState,
};
use rhythm_core::app::{AiSnapshot, DuplicateNotices};

#[tauri::command]
pub async fn ai_snapshot(state: tauri::State<'_, AppState>) -> Result<AiSnapshot, String> {
    state.ai().snapshot().await
}

/// Asks for a folder and makes it the model folder. Core holds no admission
/// lease or AI gate while the dialog is open, so a backup, restore or update
/// never waits for it; the change is checked again after the pick.
#[tauri::command]
pub async fn ai_choose_model_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let initial = state.ai().model_directory().await?;
    let selected = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("AI 모델 설치 폴더 선택")
            .set_directory(initial)
            .blocking_pick_folder()
    })
    .await
    .map_err(|_| "폴더 선택 창을 열지 못했습니다.")?;
    let Some(selected) = selected else {
        return Ok(false);
    };
    let path = selected
        .into_path()
        .map_err(|_| "선택한 폴더 경로를 읽지 못했습니다.")?;
    state.ai().set_model_directory(path).await?;
    Ok(true)
}

#[tauri::command]
pub async fn ai_reset_model_directory(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.ai().reset_model_directory().await
}

#[tauri::command]
pub async fn ai_install(
    state: tauri::State<'_, AppState>,
    catalog_id: String,
) -> Result<(), String> {
    state.ai().install(catalog_id).await
}
#[tauri::command]
pub async fn ai_cancel_download(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.ai().cancel_download().await
}
#[tauri::command]
pub async fn ai_remove(
    state: tauri::State<'_, AppState>,
    id: String,
    confirmed: bool,
    keep_files: Option<bool>,
) -> Result<(), String> {
    state
        .ai()
        .remove(id, confirmed, keep_files.unwrap_or(false))
        .await
}
#[tauri::command]
pub async fn ai_classification_preferences(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    state.ai().set_classification(enabled).await
}

#[tauri::command]
pub async fn ai_classifications(
    state: tauri::State<'_, AppState>,
    document_ids: Vec<i64>,
) -> Result<Vec<AiClassification>, String> {
    state.ai().classifications(&document_ids).await
}

#[tauri::command]
pub async fn ai_classification(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Option<AiClassification>, String> {
    state.ai().classification(document_id, revision_id).await
}

#[tauri::command]
pub async fn ai_similar_notices(
    state: tauri::State<'_, AppState>,
    document_id: i64,
) -> Result<Vec<SimilarNotice>, String> {
    state.ai().similar_notices(document_id).await
}

#[tauri::command]
pub async fn ai_duplicate_notices(
    state: tauri::State<'_, AppState>,
    document_ids: Vec<i64>,
) -> Result<Vec<DuplicateNotices>, String> {
    state.ai().duplicate_notices(&document_ids).await
}
