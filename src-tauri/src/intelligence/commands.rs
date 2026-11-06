use super::{analysis, classification, models, operations, remote, WorkerStatus};
use crate::{
    storage::intelligence::{
        AiClassification, AiJob, AiModel, AiPreferences, AiProjector, AiResult,
    },
    AppState,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

async fn exiting(shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
const EXIT_ERROR: &str = operations::EXITING;
/// Opt-in idle backlog work must not block an explicit user action. A
/// user-requested analysis is never cancelled here; its error stays as before.
async fn yield_backlog(state: &AppState) -> Result<(), String> {
    state
        .analysis
        .cancel_automatic(&state.store)
        .await
        .map(|_| ())
}
fn check_open(state: &AppState) -> Result<(), String> {
    if state.maintenance_shutdown.load(Ordering::Acquire) {
        Err(EXIT_ERROR.into())
    } else {
        state.operation_gate.ensure_open()
    }
}

// All commands that reset the process share the exit boundary. Checking only
// before waiting for the worker lock could restart it after exit cleanup.
async fn reset_worker(
    worker: &tokio::sync::Mutex<super::Worker>,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let mut worker = tokio::select! {
        biased;
        _ = exiting(shutdown) => return Err(EXIT_ERROR.into()),
        worker = worker.lock() => worker,
    };
    let result = tokio::select! {
        biased;
        _ = exiting(shutdown) => None,
        result = worker.unload() => Some(result),
    };
    match result {
        Some(result) => result.map(|_| ()),
        None => {
            worker.stop().await;
            Err(EXIT_ERROR.into())
        }
    }
}

async fn stop_inventory_worker(
    worker: &tokio::sync::Mutex<super::Worker>,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let mut worker = tokio::select! {
        biased;
        _ = exiting(shutdown) => return Err(EXIT_ERROR.into()),
        worker = worker.lock() => worker,
    };
    tokio::select! {
        biased;
        _ = exiting(shutdown) => Err(EXIT_ERROR.into()),
        result = worker.stop_for_inventory() => result,
    }
}

#[derive(Serialize)]
pub struct Snapshot {
    catalog: Vec<models::CatalogEntry>,
    models: Vec<AiModel>,
    projectors: Vec<AiProjector>,
    preferences: AiPreferences,
    jobs: Vec<AiJob>,
    runtime: WorkerStatus,
    hardware: models::Hardware,
    model_storage: super::model_storage::Status,
    busy: bool,
    operation: Option<operations::Status>,
    progress: Option<analysis::Diagnostics>,
    analysis_contract: AnalysisContract,
    classification: classification::Status,
    /// Cached credential-store presence. `None` until `ai_remote_status` reads it.
    remote_key_present: Option<bool>,
    remote_usage_today: u32,
    remote_host: Option<String>,
}
#[derive(Serialize)]
pub struct AnalysisContract {
    prompt_version: u32,
    schema_version: u32,
    configuration_hash: Option<String>,
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
fn selected(state: &AppState) -> Result<AiModel, String> {
    with_store(state, |store| {
        let id = store
            .ai_preferences()?
            .active_model_id
            .ok_or("AI 모델을 먼저 선택해 주세요.")?;
        store
            .ai_model(&id)?
            .filter(|m| m.status == "installed")
            .ok_or("설치된 AI 모델을 찾을 수 없습니다.".into())
    })
}
async fn verify_operation_file(
    file: PathBuf,
    hash: String,
    bytes: u64,
    shutdown: &AtomicBool,
    operation: &operations::Operation,
) -> Result<PathBuf, String> {
    operation.phase("verifying");
    operation.check(shutdown)?;
    let copy = file.clone();
    let cancelled = operation.cancel.clone();
    let mut checked = tokio::task::spawn_blocking(move || {
        models::verify_file(&copy, Some((&hash, bytes)), &cancelled)
    });
    let checked = tokio::select! {
        biased;
        error = operation.interrupted(shutdown) => {
            // Blocking work is cooperative: keep ownership/gate until it stops.
            let _ = checked.await;
            return Err(error);
        }
        checked = &mut checked => checked,
    };
    // Verification may observe cancellation before the polling select branch.
    // After joining, normalize either interleaving to the owning operation's
    // cancellation/exit result rather than leaking the download-specific error.
    operation.check(shutdown)?;
    checked.map_err(|_| "모델 검증 작업을 완료하지 못했습니다.")??;
    Ok(file)
}
async fn verified(
    state: &AppState,
    model: &AiModel,
    operation: &operations::Operation,
) -> Result<PathBuf, String> {
    verify_operation_file(
        super::model_storage::resolve_model_path(&state.data_root, model)?,
        model.sha256.clone().ok_or("모델 무결성 정보가 없습니다.")?,
        model.bytes,
        &state.maintenance_shutdown,
        operation,
    )
    .await
}

enum WorkerAction<'a> {
    StopForInventory,
    Load(&'a Path),
    Setup(&'a Path),
}
async fn operation_worker(
    worker: &tokio::sync::Mutex<super::Worker>,
    shutdown: &AtomicBool,
    operation: &operations::Operation,
    action: WorkerAction<'_>,
) -> Result<(), String> {
    operation.phase("waiting");
    let mut worker = tokio::select! {
        biased;
        error = operation.interrupted(shutdown) => return Err(error),
        worker = worker.lock() => worker,
    };
    let result = tokio::select! {
        biased;
        error = operation.interrupted(shutdown) => Err(error),
        result = async {
            match action {
                WorkerAction::StopForInventory => {
                    operation.phase("stopping");
                    worker.stop_for_inventory().await
                }
                WorkerAction::Load(file) | WorkerAction::Setup(file) => {
                    operation.phase("loading");
                    worker.load(file).await?;
                    operation.check(shutdown)?;
                    if matches!(action, WorkerAction::Setup(_)) {
                        operation.phase("setup");
                        let text = worker.infer(file, "다음은 로컬 AI 설치 점검용 데이터입니다. {\"title\":\"설치 점검\",\"body\":\"로컬 모델 설치가 완료되었습니다.\"} 내용을 한국어 한 문장으로 요약하세요.", 96).await?;
                        if text.trim().is_empty() || text.len() > 8192 {
                            return Err("모델의 실제 추론 점검에 실패했습니다.".into());
                        }
                    }
                    Ok(())
                }
            }
        } => result,
    };
    if let Err(error) = operation.check(shutdown) {
        // Never let a cancelled serial request leave a response for the next owner.
        operation.phase("cancelling");
        worker.stop().await;
        return Err(error);
    }
    result
}
#[tauri::command]
pub async fn ai_install_projector(
    state: tauri::State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    state.models.start_projector_admitted(
        state.store.clone(),
        state.data_root.clone(),
        model_id,
        lease,
    )
}

#[tauri::command]
pub async fn ai_remove_projector(
    state: tauri::State<'_, AppState>,
    model_id: String,
    confirmed: bool,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if !confirmed {
        return Err("이미지 인식 모듈 삭제를 확인해 주세요.".into());
    }
    yield_backlog(&state).await?;
    if state.analysis.running() || state.models.busy() {
        return Err("진행 중인 AI 작업을 먼저 취소하거나 완료해 주세요.".into());
    }
    models::remove_projector(&state.store, &state.data_root, &model_id, false).await
}

#[tauri::command]
pub async fn ai_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
    let _lease = state.operation_gate.shared().await?;
    let runtime = state
        .worker
        .try_lock()
        .map(|mut w| w.status())
        .unwrap_or(WorkerStatus {
            state: "busy".into(),
            runtime_available: true,
            model: None,
            detail: Some("로컬 AI 작업을 처리하고 있습니다.".into()),
        });
    let (models, projectors, preferences, jobs) = with_store(&state, |s| {
        let mut jobs = s.ai_active_jobs()?;
        jobs.extend(s.ai_jobs(100)?);
        jobs.sort_by_key(|j| std::cmp::Reverse(j.id));
        jobs.dedup_by_key(|j| j.id);
        Ok((
            s.ai_models()?,
            s.ai_projectors()?,
            s.ai_preferences()?,
            jobs,
        ))
    })?;
    let configuration_hash = if preferences.engine == "openai-compatible" {
        remote::GenerationConfig::from_preferences(&preferences)
            .ok()
            .and_then(|config| analysis::key_for_remote(0, 0, &config, preferences.max_tokens).ok())
            .map(|key| key.configuration_hash)
    } else {
        preferences
            .active_model_id
            .as_ref()
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .and_then(|model| {
                analysis::key_for_mode(
                    0,
                    0,
                    model,
                    preferences.max_tokens,
                    &preferences.execution_mode,
                )
                .ok()
            })
            .map(|key| key.configuration_hash)
    };
    let remote_host = preferences
        .remote_base_url
        .as_deref()
        .and_then(|value| remote::Endpoint::parse(value).ok())
        .map(|endpoint| endpoint.host().to_owned());
    let remote_key_present = preferences
        .remote_base_url
        .as_deref()
        .and_then(|value| remote::Endpoint::parse(value).ok())
        .and_then(|endpoint| state.remote.cached_present(endpoint.origin()));
    let remote_usage_today = with_store(&state, |store| store.ai_remote_usage_today())?;
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
    let operation = state.ai_operation.status();
    Ok(Snapshot {
        catalog: models::catalog(),
        models,
        projectors,
        preferences,
        jobs,
        runtime,
        hardware,
        model_storage,
        busy: state.models.busy() || state.analysis.running() || operation.is_some(),
        operation,
        progress: state.analysis.progress(),
        analysis_contract: AnalysisContract {
            prompt_version: analysis::PROMPT_VERSION,
            schema_version: analysis::SCHEMA_VERSION,
            configuration_hash,
        },
        classification: state.classifier.status(),
        remote_key_present,
        remote_usage_today,
        remote_host,
    })
}

fn remote_origin(state: &AppState) -> Result<remote::Endpoint, String> {
    let preferences = with_store(state, |store| store.ai_preferences())?;
    remote::Endpoint::parse(
        preferences
            .remote_base_url
            .as_deref()
            .ok_or("원격 API 주소를 먼저 입력해 주세요.")?,
    )
}

/// Reads credential-store presence once per Settings mount. The key itself
/// never crosses the command boundary.
#[tauri::command]
pub async fn ai_remote_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    check_open(&state)?;
    let endpoint = remote_origin(&state)?;
    let remote = state.remote.clone();
    let origin = endpoint.origin().to_owned();
    tokio::task::spawn_blocking(move || remote.present(&origin))
        .await
        .map_err(|_| "API 키 상태를 확인하지 못했습니다.".to_string())?
}

#[tauri::command]
pub async fn ai_remote_key(state: tauri::State<'_, AppState>, key: String) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let endpoint = remote_origin(&state)?;
    let remote = state.remote.clone();
    let origin = endpoint.origin().to_owned();
    let result = tokio::task::spawn_blocking(move || remote.set_key(&origin, &key))
        .await
        .map_err(|_| "API 키를 저장하지 못했습니다.".to_string())?;
    state.classifier.invalidate();
    result
}

#[tauri::command]
pub async fn ai_remote_key_clear(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let endpoint = remote_origin(&state)?;
    let remote = state.remote.clone();
    let origin = endpoint.origin().to_owned();
    let result = tokio::task::spawn_blocking(move || remote.clear_key(&origin))
        .await
        .map_err(|_| "API 키를 삭제하지 못했습니다.".to_string())?;
    state.classifier.invalidate();
    result
}

/// One tiny explicit request. It contains no notice text or personal data.
#[tauri::command]
pub async fn ai_remote_test(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let preferences = with_store(&state, |store| store.ai_preferences())?;
    if !preferences.remote_consent {
        return Err("원격 AI 사용에 먼저 동의해 주세요.".into());
    }
    let config = remote::GenerationConfig::from_preferences(&preferences)?;
    let remote = state.remote.clone();
    let origin = config.endpoint.origin().to_owned();
    let key = tokio::task::spawn_blocking(move || remote.key(&origin))
        .await
        .map_err(|_| "API 키를 확인하지 못했습니다.".to_string())??
        .ok_or("원격 AI API 키를 저장해 주세요.".to_string())?;
    let cancellation = analysis::Cancellation::default();
    state.remote.test(&config, &key, &cancellation).await
}
async fn check_storage_change(state: &AppState) -> Result<(), String> {
    check_open(state)?;
    yield_backlog(state).await?;
    if state.models.busy() || state.analysis.running() || state.ai_operation.status().is_some() {
        return Err("진행 중인 AI 작업을 완료하거나 취소한 뒤 설치 폴더를 변경해 주세요.".into());
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
    check_storage_change(&state).await?;
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
    check_storage_change(&state).await?;
    let path = selected
        .into_path()
        .map_err(|_| "선택한 폴더 경로를 읽지 못했습니다.")?;
    let directory =
        tokio::task::spawn_blocking(move || super::model_storage::prepare_selection(&path))
            .await
            .map_err(|_| "설치 폴더를 확인하지 못했습니다.")??;
    check_storage_change(&state).await?;
    with_store(&state, |s| s.ai_save_model_directory(Some(&directory)))?;
    Ok(true)
}

#[tauri::command]
pub async fn ai_reset_model_directory(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_storage_change(&state).await?;
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
pub async fn ai_register(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("분석이 끝난 뒤 모델을 등록해 주세요.".into());
    }
    // Re-registration can change bytes at the same canonical path. A path-only
    // loaded model must never be cached under the replacement's artifact hash.
    let operation = state.ai_operation.begin("register")?;
    operation_worker(
        &state.worker,
        &state.maintenance_shutdown,
        &operation,
        WorkerAction::StopForInventory,
    )
    .await?;
    operation.phase("verifying");
    let mut registration = Box::pin(models::register(
        state.store.clone(),
        PathBuf::from(path),
        operation.cancel.clone(),
    ));
    tokio::select! {
        biased;
        error = operation.interrupted(&state.maintenance_shutdown) => {
            // Join the cooperative verifier before admitting a replacement operation.
            let _ = registration.await;
            Err(error)
        }
        result = &mut registration => {
            if result.is_err() {
                operation.check(&state.maintenance_shutdown)?;
            }
            result.map(|_| ())
        },
    }
}
#[tauri::command]
pub async fn ai_select(
    state: tauri::State<'_, AppState>,
    id: Option<String>,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    yield_backlog(&state).await?;
    if state.analysis.running() {
        return Err("분석을 취소하거나 완료한 뒤 모델을 변경해 주세요.".into());
    }
    let mut prefs = with_store(&state, |s| s.ai_preferences())?;
    if prefs.active_model_id == id {
        return Ok(());
    }
    if let Some(id) = &id {
        with_store(&state, |s| s.ai_model(id))?
            .filter(|m| m.status == "installed")
            .ok_or("설치된 모델을 선택해 주세요.")?;
    }
    stop_inventory_worker(&state.worker, &state.maintenance_shutdown).await?;
    prefs.active_model_id = id;
    with_store(&state, |s| s.ai_save_preferences(&prefs))
}
#[tauri::command]
pub async fn ai_load(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    yield_backlog(&state).await?;
    if state.analysis.running() {
        return Err("분석이 완료된 뒤 다시 시도해 주세요.".into());
    }
    let operation = state.ai_operation.begin("load")?;
    let model = selected(&state)?;
    let file = verified(&state, &model, &operation).await?;
    operation_worker(
        &state.worker,
        &state.maintenance_shutdown,
        &operation,
        WorkerAction::Load(&file),
    )
    .await
}
/// Unload and restart both reap the process; the next request starts a fresh one.
async fn reap_idle_worker(state: &AppState) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(state)?;
    yield_backlog(state).await?;
    if state.analysis.running() {
        return Err("먼저 진행 중인 분석을 취소해 주세요.".into());
    }
    reset_worker(&state.worker, &state.maintenance_shutdown).await
}
#[tauri::command]
pub async fn ai_unload(state: tauri::State<'_, AppState>) -> Result<(), String> {
    reap_idle_worker(&state).await
}
#[tauri::command]
pub async fn ai_restart(state: tauri::State<'_, AppState>) -> Result<(), String> {
    reap_idle_worker(&state).await
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
    yield_backlog(&state).await?;
    if state.analysis.running() || state.models.busy() {
        return Err("진행 중인 AI 작업을 먼저 취소하거나 완료해 주세요.".into());
    }
    let previous_preferences = with_store(&state, |s| s.ai_preferences())?;
    let mut prefs = previous_preferences.clone();
    // Unload even if an earlier selection was cleared, so no mapped model file is deleted.
    stop_inventory_worker(&state.worker, &state.maintenance_shutdown).await?;
    let kind = with_store(&state, |s| Ok(s.ai_model(&id)?.map(|model| model.kind)))?;
    let _classifier_paused = if kind.as_deref() == Some("embedding") {
        // Windows cannot unlink a file mapped by the embedding process. Hold
        // classification paused until inventory and file removal finish.
        let guard = state.classifier.quiesce().await;
        state.classifier.stop_worker().await;
        Some(guard)
    } else {
        None
    };
    if prefs.active_model_id.as_deref() == Some(&id) {
        prefs.active_model_id = None;
        with_store(&state, |s| s.ai_save_preferences(&prefs))?;
    }
    if prefs.embedding_model_id.as_deref() == Some(&id) {
        prefs.embedding_model_id = None;
        with_store(&state, |s| s.ai_save_preferences(&prefs))?;
    }
    let result = if keep_files == Some(true) {
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
    };
    if result.is_err() && prefs.active_model_id != previous_preferences.active_model_id {
        // A refused deletion retains selection, but partially completed deletion
        // can leave truthful noninstalled inventory. Never reselect missing bytes.
        let still_installed = with_store(&state, |s| {
            Ok(previous_preferences
                .active_model_id
                .as_deref()
                .map(|id| s.ai_model(id))
                .transpose()?
                .flatten()
                .is_some_and(|model| model.status == "installed"))
        })?;
        if still_installed {
            with_store(&state, |s| s.ai_save_preferences(&previous_preferences))?;
        }
    }
    result
}
#[tauri::command]
pub async fn ai_preferences(
    state: tauri::State<'_, AppState>,
    mut preferences: AiPreferences,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let old = with_store(&state, |s| s.ai_preferences())?;
    if preferences
        .remote_base_url
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        preferences.remote_base_url = None;
    }
    if let Some(value) = preferences.remote_base_url.as_deref() {
        preferences.remote_base_url = Some(remote::Endpoint::parse(value)?.base().to_owned());
    }
    let old_base = old
        .remote_base_url
        .as_deref()
        .and_then(|value| remote::Endpoint::parse(value).ok())
        .map(|endpoint| endpoint.base().to_owned());
    if old_base != preferences.remote_base_url {
        // Consent and automatic work are bound to the address that was shown
        // to the user; an address change requires a fresh explicit decision.
        preferences.remote_consent = false;
        preferences.remote_automatic = false;
        preferences.remote_embeddings = false;
    }
    if preferences.engine != "openai-compatible" {
        // These only authorize network/cost work for the remote engine. Require
        // an explicit re-opt-in when the remote engine is selected again.
        preferences.remote_automatic = false;
        preferences.remote_embeddings = false;
    } else if preferences.remote_consent {
        remote::GenerationConfig::from_preferences(&preferences)?;
        if preferences.remote_automatic && preferences.remote_daily_limit == 0 {
            return Err("자동 원격 분석의 하루 한도를 1 이상으로 설정해 주세요.".into());
        }
        if preferences.remote_embeddings {
            remote::EmbeddingConfig::from_preferences(&preferences)?;
        }
    }
    if old.engine != preferences.engine && state.analysis.running() {
        return Err("분석을 취소하거나 완료한 뒤 AI 엔진을 변경해 주세요.".into());
    }
    if old.model_directory != preferences.model_directory {
        return Err("모델 설치 폴더는 폴더 선택 기능으로 변경해 주세요.".into());
    }
    if old.active_model_id != preferences.active_model_id {
        return Err("모델 선택 기능으로 활성 모델을 변경해 주세요.".into());
    }
    if old.embedding_model_id != preferences.embedding_model_id
        || old.classification_enabled != preferences.classification_enabled
    {
        return Err("공지 분류 설정은 분류 설정 기능으로 변경해 주세요.".into());
    }
    if old.execution_mode != preferences.execution_mode {
        if !super::valid_execution_mode(&preferences.execution_mode) {
            return Err("AI 실행 장치를 확인해 주세요.".into());
        }
        yield_backlog(&state).await?;
        if state.analysis.running() {
            return Err("분석을 취소하거나 완료한 뒤 실행 장치를 변경해 주세요.".into());
        }
        if old.setup_completed != preferences.setup_completed {
            return Err("실행 장치를 먼저 저장한 뒤 AI 설정을 완료해 주세요.".into());
        }
        let mut worker = tokio::select! {
            biased;
            _ = exiting(&state.maintenance_shutdown) => return Err(EXIT_ERROR.into()),
            worker = state.worker.lock() => worker,
        };
        // Keep ownership through reap, configuration and persistence; a background
        // refresh cannot restart the old backend between these steps.
        worker.stop_for_inventory().await?;
        check_open(&state)?;
        worker.set_execution_mode(&preferences.execution_mode)?;
        if let Err(error) = with_store(&state, |s| s.ai_save_preferences(&preferences)) {
            worker.set_execution_mode(&old.execution_mode)?;
            return Err(error);
        }
        state.classifier.invalidate();
        return Ok(());
    }
    if preferences.setup_completed && !old.setup_completed {
        yield_backlog(&state).await?;
        if state.analysis.running() {
            return Err("분석이 끝난 뒤 설정을 완료해 주세요.".into());
        }
        let operation = state.ai_operation.begin("setup")?;
        let model = selected(&state)?;
        let file = verified(&state, &model, &operation).await?;
        operation_worker(
            &state.worker,
            &state.maintenance_shutdown,
            &operation,
            WorkerAction::Setup(&file),
        )
        .await?;
        operation.check(&state.maintenance_shutdown)?;
        // Persist completion while the operation still owns admission/status.
        let result = with_store(&state, |s| s.ai_save_preferences(&preferences));
        if result.is_ok() {
            state.classifier.invalidate();
        }
        return result;
    }
    let result = with_store(&state, |s| s.ai_save_preferences(&preferences));
    if result.is_ok() {
        state.classifier.invalidate();
    }
    result
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

#[tauri::command]
pub async fn ai_analyze(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
    regenerate: bool,
) -> Result<analysis::StartResponse, String> {
    let lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let preferences = with_store(&state, |store| store.ai_preferences())?;
    if preferences.engine == "openai-compatible" {
        if !preferences.remote_consent {
            return Err("원격 AI 사용에 먼저 동의해 주세요.".into());
        }
        let config = remote::GenerationConfig::from_preferences(&preferences)?;
        let remote = state.remote.clone();
        let origin = config.endpoint.origin().to_owned();
        let present = tokio::task::spawn_blocking(move || remote.present(&origin))
            .await
            .map_err(|_| "API 키 상태를 확인하지 못했습니다.".to_string())??;
        if !present {
            return Err("원격 AI API 키를 저장해 주세요.".into());
        }
    }
    state.analysis.start_admitted(
        state.store.clone(),
        state.worker.clone(),
        state.data_root.clone(),
        document_id,
        revision_id,
        regenerate,
        analysis::DIRECT_PRIORITY,
        lease,
        Some(state.remote.clone()),
    )
}
#[tauri::command]
pub async fn ai_cancel(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    state.analysis.cancel(&state.store, id)
}
#[tauri::command]
pub async fn ai_results(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Vec<AiResult>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state, |s| {
        s.ai_document_results(document_id, Some(revision_id), 20)
    })
}
#[tauri::command]
pub async fn ai_document_jobs(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Vec<AiJob>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state, |s| s.ai_document_jobs(document_id, revision_id, 20))
}
#[tauri::command]
pub async fn ai_cancel_operation(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    state.ai_operation.cancel();
    Ok(())
}
#[tauri::command]
pub async fn ai_diagnostics(
    state: tauri::State<'_, AppState>,
    job_id: i64,
) -> Result<Option<analysis::Diagnostics>, String> {
    let _lease = state.operation_gate.shared().await?;
    Ok(state.analysis.diagnostics(job_id))
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

#[tauri::command]
pub async fn ai_recent(state: tauri::State<'_, AppState>) -> Result<Vec<AiResult>, String> {
    let _lease = state.operation_gate.shared().await?;
    with_store(&state, |s| s.ai_recent_results(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, sync::Arc, time::Duration};
    use tokio::{sync::Mutex, time::timeout};

    /// Minimal valid GGUF header declaring the `qwen3` architecture.
    fn qwen3_header() -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(20u64.to_le_bytes());
        bytes.extend(b"general.architecture");
        bytes.extend(8u32.to_le_bytes());
        bytes.extend(5u64.to_le_bytes());
        bytes.extend(b"qwen3");
        bytes
    }

    #[tokio::test]
    async fn unavailable_worker_allows_external_inventory_changes_without_deleting_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("external.gguf");
        let mut bytes = qwen3_header();
        bytes.extend([0u8; 256]);
        std::fs::write(&file, &bytes).unwrap();
        let store = Arc::new(std::sync::Mutex::new(
            crate::Store::open(&directory.path().join("rhythm.sqlite3")).unwrap(),
        ));
        let worker = Mutex::new(super::super::Worker::launch(
            &directory.path().join("nonexistent-worker.exe"),
        ));
        let shutdown = AtomicBool::new(false);
        let controller = operations::Controller::default();
        let operation = controller.begin("register").unwrap();
        operation_worker(
            &worker,
            &shutdown,
            &operation,
            WorkerAction::StopForInventory,
        )
        .await
        .unwrap();
        let model = models::register(store.clone(), file.clone(), operation.cancel.clone())
            .await
            .unwrap();
        drop(operation);
        assert_eq!(model.ownership, "external");
        assert!(worker.lock().await.process_id().is_none());

        // Selection and removal share the same process-release boundary.
        stop_inventory_worker(&worker, &shutdown).await.unwrap();
        let mut preferences = store.lock().unwrap().ai_preferences().unwrap();
        preferences.active_model_id = Some(model.id.clone());
        store
            .lock()
            .unwrap()
            .ai_save_preferences(&preferences)
            .unwrap();
        stop_inventory_worker(&worker, &shutdown).await.unwrap();
        preferences.active_model_id = None;
        store
            .lock()
            .unwrap()
            .ai_save_preferences(&preferences)
            .unwrap();
        models::remove(
            store.clone(),
            directory.path().into(),
            model.id.clone(),
            true,
        )
        .await
        .unwrap();
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert!(worker.lock().await.process_id().is_none());
    }

    #[tokio::test]
    async fn inventory_stop_reaps_running_worker_without_relaunching() {
        let (_directory, worker) = running_worker().await;
        let worker = Mutex::new(worker);
        assert!(worker.lock().await.process_id().is_some());
        timeout(
            Duration::from_secs(5),
            stop_inventory_worker(&worker, &AtomicBool::new(false)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(worker.lock().await.process_id().is_none());
    }

    /// A worker whose fake server is already serving a loaded model.
    async fn running_worker() -> (tempfile::TempDir, super::super::Worker) {
        let directory = tempfile::tempdir().unwrap();
        let (executable, model) =
            crate::intelligence::test_worker::fake_engine(directory.path(), "ok");
        let mut worker = super::super::Worker::launch(&executable);
        worker.load(&model).await.unwrap();
        (directory, worker)
    }

    /// The fake server hangs once in load or in the setup request, writing
    /// `entered` first; later requests proceed.
    fn delayed_model_worker(
        setup: bool,
    ) -> (tempfile::TempDir, super::super::Worker, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let (executable, model) =
            crate::intelligence::test_worker::fake_engine(directory.path(), "ok");
        let control = if setup { "chat-delay" } else { "load-delay" };
        std::fs::write(directory.path().join(control), b"").unwrap();
        let marker = directory.path().join("entered");
        let worker = super::super::Worker::launch(&executable);
        (directory, worker, marker, model)
    }

    #[tokio::test]
    async fn command_owned_load_and_setup_cancel_reap_and_clear_status() {
        for setup in [false, true] {
            for exit in [false, true] {
                let (_directory, child, marker, model) = delayed_model_worker(setup);
                let worker = Arc::new(Mutex::new(child));
                let task_model = model.clone();
                let shutdown = Arc::new(AtomicBool::new(false));
                let operations = operations::Controller::default();
                let task_worker = worker.clone();
                let task_shutdown = shutdown.clone();
                let task_operations = operations.clone();
                let task = tokio::spawn(async move {
                    let operation = task_operations.begin(if setup { "setup" } else { "load" })?;
                    operation_worker(
                        &task_worker,
                        &task_shutdown,
                        &operation,
                        if setup {
                            WorkerAction::Setup(&task_model)
                        } else {
                            WorkerAction::Load(&task_model)
                        },
                    )
                    .await
                });
                // Child startup under parallel tests is not the cancellation deadline.
                timeout(Duration::from_secs(10), async {
                    while !marker.exists() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("controlled child must actually enter load/setup request");
                assert_eq!(
                    operations.status().unwrap().phase,
                    if setup { "setup" } else { "loading" }
                );
                if exit {
                    shutdown.store(true, Ordering::Release);
                } else {
                    operations.cancel();
                }
                assert_eq!(
                    timeout(Duration::from_secs(1), task)
                        .await
                        .unwrap()
                        .unwrap(),
                    Err(if exit {
                        operations::EXITING
                    } else {
                        operations::CANCELLED
                    }
                    .into())
                );
                {
                    let worker = worker.lock().await;
                    assert!(
                        worker.process_id().is_none(),
                        "cancelled operation must reap the actual child"
                    );
                }
                assert!(operations.status().is_none());
                if !exit {
                    let next = operations.begin("setup").unwrap();
                    timeout(
                        Duration::from_secs(1),
                        operation_worker(&worker, &shutdown, &next, WorkerAction::Setup(&model)),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    drop(next);
                    assert!(operations.status().is_none());
                    assert!(
                        worker.lock().await.process_id().is_some(),
                        "next request must restart a fresh child"
                    );
                    worker.lock().await.stop().await;
                    assert!(worker.lock().await.process_id().is_none());
                }
            }
        }
    }

    #[tokio::test]
    async fn command_owned_operation_does_not_wait_behind_worker_after_cancellation() {
        let worker = Mutex::new(super::super::Worker::launch(Path::new("/missing/worker")));
        let _held = worker.lock().await;
        let operations = operations::Controller::default();
        let operation = operations.begin("load").unwrap();
        operations.cancel();
        assert_eq!(
            timeout(
                Duration::from_millis(100),
                operation_worker(
                    &worker,
                    &AtomicBool::new(false),
                    &operation,
                    WorkerAction::Load(Path::new("/fake.gguf"))
                )
            )
            .await
            .unwrap(),
            Err(operations::CANCELLED.into())
        );
    }

    #[tokio::test]
    async fn cancelled_verification_joins_blocking_work_before_releasing_operation() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("model.gguf");
        let bytes = qwen3_header();
        std::fs::write(&file, bytes).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let operations = operations::Controller::default();
        let observer = operations.clone();
        let task = tokio::spawn(async move {
            let operation = operations.begin("load").unwrap();
            verify_operation_file(
                file,
                "0".repeat(64),
                64 * 1024 * 1024,
                &AtomicBool::new(false),
                &operation,
            )
            .await
        });
        timeout(Duration::from_secs(1), async {
            while observer.status().is_none() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        observer.cancel();
        assert_eq!(
            timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            Err(operations::CANCELLED.into())
        );
        assert!(observer.status().is_none());
    }

    #[tokio::test]
    async fn reset_does_not_wait_for_worker_lock_after_exit() {
        let worker = Mutex::new(super::super::Worker::launch(Path::new("/missing/worker")));
        let _held_by_exit = worker.lock().await;
        let shutdown = AtomicBool::new(true);
        assert_eq!(
            timeout(Duration::from_millis(200), reset_worker(&worker, &shutdown))
                .await
                .expect("an exiting command must not wait behind worker cleanup"),
            Err(EXIT_ERROR.into())
        );
    }

    #[tokio::test]
    async fn reset_reaps_the_running_server() {
        let (_directory, worker) = running_worker().await;
        let worker = Mutex::new(worker);
        timeout(
            Duration::from_secs(5),
            reset_worker(&worker, &AtomicBool::new(false)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(worker.lock().await.process_id().is_none());
    }

    #[tokio::test]
    async fn reset_cannot_relaunch_worker_after_exit_cleanup() {
        let (_directory, worker) = running_worker().await;
        let worker = Mutex::new(worker);
        worker.lock().await.stop().await;
        assert!(worker.lock().await.process_id().is_none());
        let shutdown = AtomicBool::new(true);
        assert_eq!(
            timeout(Duration::from_millis(200), reset_worker(&worker, &shutdown))
                .await
                .expect("a completed exit must not launch another process"),
            Err(EXIT_ERROR.into())
        );
        assert!(worker.lock().await.process_id().is_none());
    }
}
