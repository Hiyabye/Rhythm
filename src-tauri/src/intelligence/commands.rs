use super::{classification, models};
use crate::{
    storage::intelligence::{AiClassification, AiModel, AiPreferences, SimilarNotice},
    AppState,
};
use serde::Serialize;
use std::{path::PathBuf, sync::atomic::Ordering};

const EXIT_ERROR: &str = "앱을 종료하고 있습니다.";
fn check_open(state: &AppState) -> Result<(), String> {
    if state.maintenance_shutdown.load(Ordering::Acquire) {
        Err(EXIT_ERROR.into())
    } else {
        state.operation_gate.ensure_open()
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    catalog: Vec<models::CatalogEntry>,
    models: Vec<AiModel>,
    preferences: AiPreferences,
    hardware: models::Hardware,
    model_storage: super::model_storage::Status,
    busy: bool,
    classification: classification::Status,
}
fn with_store<T>(
    state: &AppState,
    f: impl FnOnce(&crate::Store) -> Result<T, String>,
) -> Result<T, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "저장소에 접근할 수 없습니다.")?;
    f(&store).map_err(crate::storage_error)
}

#[tauri::command]
pub async fn ai_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let (models, preferences) = with_store(&state, |s| Ok((s.ai_models()?, s.ai_preferences()?)))?;
    let initial_hardware = state.hardware.clone();
    let root = state.data_root.clone();
    let directory = preferences.model_directory.clone();
    let (hardware, model_storage) = tokio::task::spawn_blocking(move || {
        (
            models::refresh_disk_space(initial_hardware, &root),
            super::model_storage::status(&root, directory.as_deref()),
        )
    })
    .await
    .map_err(|_| "저장 공간 정보를 확인하지 못했습니다.")?;
    Ok(Snapshot {
        catalog: models::catalog(),
        models,
        preferences,
        hardware,
        model_storage,
        busy: state.models.busy(),
        classification: state.classifier.status(),
    })
}

fn check_storage_change(state: &AppState) -> Result<(), String> {
    check_open(state)?;
    if state.models.busy() {
        return Err("진행 중인 모델 설치를 완료하거나 취소한 뒤 설치 폴더를 변경해 주세요.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn ai_choose_model_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_storage_change(&state)?;
    let initial = with_store(&state, |s| Ok(s.ai_preferences()?.model_directory))?
        .map(PathBuf::from)
        .unwrap_or_else(|| state.data_root.clone());
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
    check_storage_change(&state)?;
    let path = selected
        .into_path()
        .map_err(|_| "선택한 폴더 경로를 읽지 못했습니다.")?;
    let directory =
        tokio::task::spawn_blocking(move || super::model_storage::prepare_selection(&path))
            .await
            .map_err(|_| "설치 폴더를 확인하지 못했습니다.")??;
    check_storage_change(&state)?;
    with_store(&state, |s| s.ai_save_model_directory(Some(&directory)))?;
    Ok(true)
}

#[tauri::command]
pub async fn ai_reset_model_directory(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_storage_change(&state)?;
    with_store(&state, |s| s.ai_save_model_directory(None))
}

#[tauri::command]
pub async fn ai_install(
    state: tauri::State<'_, AppState>,
    catalog_id: String,
) -> Result<(), String> {
    let lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    state.models.start_admitted(
        state.store.clone(),
        state.data_root.clone(),
        catalog_id,
        lease,
    )
}
#[tauri::command]
pub async fn ai_cancel_download(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    state.models.cancel();
    Ok(())
}
#[tauri::command]
pub async fn ai_remove(
    state: tauri::State<'_, AppState>,
    id: String,
    confirmed: bool,
    keep_files: Option<bool>,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    if !confirmed {
        return Err("모델 삭제를 확인해 주세요.".into());
    }
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.models.busy() {
        return Err("진행 중인 모델 설치를 먼저 취소하거나 완료해 주세요.".into());
    }
    let kind = with_store(&state, |s| Ok(s.ai_model(&id)?.map(|model| model.kind)))?;
    let _classifier_paused = if kind.as_deref() == Some("embedding") {
        // Windows cannot unlink a file mapped by the embedding process. Hold
        // classification paused until inventory and file removal finish.
        let guard = state.classifier.quiesce().await;
        state.classifier.stop_worker().await?;
        Some(guard)
    } else {
        None
    };
    with_store(&state, |s| {
        let mut preferences = s.ai_preferences()?;
        if preferences.embedding_model_id.as_deref() == Some(&id) {
            preferences.embedding_model_id = None;
            s.ai_save_preferences(&preferences)?;
        }
        Ok(())
    })?;
    if keep_files == Some(true) {
        models::remove_with_policy(
            state.store.clone(),
            state.data_root.clone(),
            id,
            confirmed,
            true,
        )
        .await
    } else {
        models::remove(state.store.clone(), state.data_root.clone(), id, confirmed).await
    }
}
/// Changes the Windows execution mode. Model folder, model selection and the
/// classification switch each have their own command.
#[tauri::command]
pub async fn ai_preferences(
    state: tauri::State<'_, AppState>,
    preferences: AiPreferences,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let old = with_store(&state, |s| s.ai_preferences())?;
    if old.model_directory != preferences.model_directory {
        return Err("모델 설치 폴더는 폴더 선택 기능으로 변경해 주세요.".into());
    }
    if old.embedding_model_id != preferences.embedding_model_id
        || old.classification_enabled != preferences.classification_enabled
    {
        return Err("공지 분류 설정은 분류 설정 기능으로 변경해 주세요.".into());
    }
    if old.execution_mode == preferences.execution_mode {
        return Ok(());
    }
    if !super::valid_execution_mode(&preferences.execution_mode) {
        return Err("AI 실행 장치를 확인해 주세요.".into());
    }
    // Keep the batch paused through reap, configuration and persistence so the
    // old backend cannot restart between these steps.
    let _paused = state.classifier.quiesce().await;
    check_open(&state)?;
    state
        .classifier
        .set_execution_mode(&preferences.execution_mode)
        .await?;
    if let Err(error) = with_store(&state, |s| s.ai_save_preferences(&preferences)) {
        state
            .classifier
            .set_execution_mode(&old.execution_mode)
            .await?;
        return Err(error);
    }
    Ok(())
}
/// The only command that changes automatic classification. It quiesces the
/// running batch so the saved preference and the worker cannot disagree.
#[tauri::command]
pub async fn ai_classification_preferences(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let _paused = state.classifier.quiesce().await;
    let result = with_store(&state, |store| {
        let mut preferences = store.ai_preferences()?;
        preferences.classification_enabled = enabled;
        store.ai_save_preferences(&preferences)
    });
    state.classifier.invalidate();
    result
}

/// SHA-256 of the selected, installed embedding model. Turning automatic
/// classification off keeps existing rows visible, so the switch is not checked.
fn embedding_model_sha(state: &AppState) -> Result<Option<String>, String> {
    with_store(state, |store| {
        let Some(id) = store.ai_preferences()?.embedding_model_id else {
            return Ok(None);
        };
        Ok(store
            .ai_model(&id)?
            .filter(|model| model.kind == "embedding" && model.status == "installed")
            .and_then(|model| model.sha256))
    })
}

/// Latest-revision classifications for the visible page. Returns nothing when
/// no embedding model is selected or classification is disabled.
#[tauri::command]
pub async fn ai_classifications(
    state: tauri::State<'_, AppState>,
    document_ids: Vec<i64>,
) -> Result<Vec<AiClassification>, String> {
    if document_ids.len() > 200 || document_ids.iter().any(|id| *id <= 0) {
        return Err("분류 조회 범위를 확인해 주세요.".into());
    }
    let Some(sha) = embedding_model_sha(&state)? else {
        return Ok(Vec::new());
    };
    with_store(&state, |store| {
        store.ai_classifications_for_documents(&document_ids, &sha, classification::config_hash())
    })
}

/// Revision-scoped classification for the reader. Older revisions without a
/// stored row return `None`; they are classified only through the latest-revision
/// background pass.
#[tauri::command]
pub async fn ai_classification(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Option<AiClassification>, String> {
    if document_id <= 0 || revision_id <= 0 {
        return Err("분류 조회 식별자가 올바르지 않습니다.".into());
    }
    let Some(sha) = embedding_model_sha(&state)? else {
        return Ok(None);
    };
    with_store(&state, |store| {
        store.ai_classification(
            document_id,
            revision_id,
            &sha,
            classification::config_hash(),
        )
    })
}

/// Similar notices for the reader, from stored vectors only (no model call).
/// Returns nothing without a selected, installed embedding model.
#[tauri::command]
pub async fn ai_similar_notices(
    state: tauri::State<'_, AppState>,
    document_id: i64,
) -> Result<Vec<SimilarNotice>, String> {
    if document_id <= 0 {
        return Err("비슷한 공지 조회 식별자가 올바르지 않습니다.".into());
    }
    let Some(sha) = embedding_model_sha(&state)? else {
        return Ok(Vec::new());
    };
    with_store(&state, |store| {
        Ok(store
            .ai_similar_notices(&[document_id], &sha, 5)?
            .pop()
            .map(|(_, similar)| similar)
            .unwrap_or_default())
    })
}

/// Likely cross-posts of each visible notice, for grouping list rows. Only
/// documents with at least one duplicate are returned.
#[tauri::command]
pub async fn ai_duplicate_notices(
    state: tauri::State<'_, AppState>,
    document_ids: Vec<i64>,
) -> Result<Vec<DuplicateNotices>, String> {
    if document_ids.len() > 200 || document_ids.iter().any(|id| *id <= 0) {
        return Err("비슷한 공지 조회 범위를 확인해 주세요.".into());
    }
    let Some(sha) = embedding_model_sha(&state)? else {
        return Ok(Vec::new());
    };
    with_store(&state, |store| {
        Ok(store
            .ai_similar_notices(&document_ids, &sha, 1)?
            .into_iter()
            .filter_map(|(document_id, similar)| {
                let duplicates = similar
                    .into_iter()
                    .filter(|notice| notice.duplicate)
                    .collect::<Vec<_>>();
                (!duplicates.is_empty()).then_some(DuplicateNotices {
                    document_id,
                    duplicates,
                })
            })
            .collect())
    })
}

#[derive(Serialize)]
pub struct DuplicateNotices {
    document_id: i64,
    duplicates: Vec<SimilarNotice>,
}
