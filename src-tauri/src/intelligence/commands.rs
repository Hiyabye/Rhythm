use super::{analysis, models, WorkerStatus};
use crate::{
    storage::intelligence::{AiJob, AiModel, AiPreferences, AiResult},
    AppState,
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

async fn exiting(shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
const EXIT_ERROR: &str = "앱을 종료하고 있습니다.";
fn check_open(state: &AppState) -> Result<(), String> {
    if state.maintenance_shutdown.load(Ordering::Acquire) {
        Err(EXIT_ERROR.into())
    } else {
        Ok(())
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

#[derive(Serialize)]
pub struct Snapshot {
    catalog: Vec<models::CatalogEntry>,
    models: Vec<AiModel>,
    preferences: AiPreferences,
    jobs: Vec<AiJob>,
    runtime: WorkerStatus,
    hardware: models::Hardware,
    busy: bool,
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
fn path(state: &AppState, model: &AiModel) -> PathBuf {
    if model.ownership == "managed" {
        state.data_root.join(&model.path)
    } else {
        PathBuf::from(&model.path)
    }
}
async fn verified(state: &AppState, model: &AiModel) -> Result<PathBuf, String> {
    let file = path(state, model);
    let copy = file.clone();
    let hash = model.sha256.clone().ok_or("모델 무결성 정보가 없습니다.")?;
    let bytes = model.bytes;
    let shutdown = state.maintenance_shutdown.clone();
    tokio::task::spawn_blocking(move || {
        models::verify_file(&copy, Some((&hash, bytes)), &shutdown)
    })
    .await
    .map_err(|_| "모델 검증 작업을 완료하지 못했습니다.")??;
    Ok(file)
}
#[tauri::command]
pub async fn ai_snapshot(state: tauri::State<'_, AppState>) -> Result<Snapshot, String> {
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
    let (models, preferences, jobs) = with_store(&state, |s| {
        let mut jobs = s.ai_active_jobs()?;
        jobs.extend(s.ai_jobs(100)?);
        jobs.sort_by_key(|j| std::cmp::Reverse(j.id));
        jobs.dedup_by_key(|j| j.id);
        Ok((s.ai_models()?, s.ai_preferences()?, jobs))
    })?;
    Ok(Snapshot {
        catalog: models::catalog(),
        models,
        preferences,
        jobs,
        runtime,
        hardware: state.hardware.clone(),
        busy: state.models.busy() || state.analysis.running(),
    })
}
#[tauri::command]
pub async fn ai_install(
    state: tauri::State<'_, AppState>,
    catalog_id: String,
) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    state
        .models
        .start(state.store.clone(), state.data_root.clone(), catalog_id)
}
#[tauri::command]
pub fn ai_cancel_download(state: tauri::State<'_, AppState>) {
    state.models.cancel();
}
#[tauri::command]
pub async fn ai_register(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("분석이 끝난 뒤 모델을 등록해 주세요.".into());
    }
    // Re-registration can change bytes at the same canonical path. A path-only
    // loaded model must never be cached under the replacement's artifact hash.
    reset_worker(&state.worker, &state.maintenance_shutdown).await?;
    models::register(state.store.clone(), PathBuf::from(path))
        .await
        .map(|_| ())
}
#[tauri::command]
pub async fn ai_select(
    state: tauri::State<'_, AppState>,
    id: Option<String>,
) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
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
    reset_worker(&state.worker, &state.maintenance_shutdown).await?;
    prefs.active_model_id = id;
    with_store(&state, |s| s.ai_save_preferences(&prefs))
}
#[tauri::command]
pub async fn ai_load(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("분석이 완료된 뒤 다시 시도해 주세요.".into());
    }
    let model = selected(&state)?;
    let file = verified(&state, &model).await?;
    check_open(&state)?;
    let mut worker = state.worker.lock().await;
    let result = tokio::select! {
        biased;
        _ = exiting(&state.maintenance_shutdown) => None,
        result = worker.load(&file) => Some(result),
    };
    match result {
        Some(result) => result.map(|_| ()),
        None => {
            worker.stop().await;
            Err("종료 요청으로 모델 불러오기를 중단했습니다.".into())
        }
    }
}
#[tauri::command]
pub async fn ai_unload(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("먼저 진행 중인 분석을 취소해 주세요.".into());
    }
    reset_worker(&state.worker, &state.maintenance_shutdown).await
}
#[tauri::command]
pub async fn ai_restart(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("먼저 진행 중인 분석을 취소해 주세요.".into());
    }
    reset_worker(&state.worker, &state.maintenance_shutdown).await
}
#[tauri::command]
pub async fn ai_remove(
    state: tauri::State<'_, AppState>,
    id: String,
    confirmed: bool,
) -> Result<(), String> {
    if !confirmed {
        return Err("모델 삭제를 확인해 주세요.".into());
    }
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() || state.models.busy() {
        return Err("진행 중인 AI 작업을 먼저 취소하거나 완료해 주세요.".into());
    }
    let previous_preferences = with_store(&state, |s| s.ai_preferences())?;
    let mut prefs = previous_preferences.clone();
    // Unload even if an earlier selection was cleared, so no mapped model file is deleted.
    reset_worker(&state.worker, &state.maintenance_shutdown).await?;
    if prefs.active_model_id.as_deref() == Some(&id) {
        prefs.active_model_id = None;
        with_store(&state, |s| s.ai_save_preferences(&prefs))?;
    }
    let result = models::remove(state.store.clone(), state.data_root.clone(), id, confirmed).await;
    if result.is_err() && prefs.active_model_id != previous_preferences.active_model_id {
        // A pending persisted job or filesystem refusal must not silently change
        // the user's selection. Mutations remain serialized by ai_gate.
        with_store(&state, |s| s.ai_save_preferences(&previous_preferences))?;
    }
    result
}
#[tauri::command]
pub async fn ai_preferences(
    state: tauri::State<'_, AppState>,
    preferences: AiPreferences,
) -> Result<(), String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    let old = with_store(&state, |s| s.ai_preferences())?;
    if old.active_model_id != preferences.active_model_id {
        return Err("모델 선택 기능으로 활성 모델을 변경해 주세요.".into());
    }
    if preferences.setup_completed && !old.setup_completed {
        if state.analysis.running() {
            return Err("분석이 끝난 뒤 설정을 완료해 주세요.".into());
        }
        let model = selected(&state)?;
        let file = verified(&state, &model).await?;
        check_open(&state)?;
        let mut worker = state.worker.lock().await;
        let result = tokio::select! {
            biased;
            _ = exiting(&state.maintenance_shutdown) => None,
            result = worker.infer(&file, "다음은 로컬 AI 설치 점검용 데이터입니다. {\"title\":\"설치 점검\",\"body\":\"로컬 모델 설치가 완료되었습니다.\"} 내용을 한국어 한 문장으로 요약하세요.", 96) => Some(result),
        };
        let text = match result {
            Some(result) => result?,
            None => {
                worker.stop().await;
                return Err("종료 요청으로 설치 점검을 중단했습니다.".into());
            }
        };
        if text.trim().is_empty() || text.len() > 8192 {
            return Err("모델의 실제 추론 점검에 실패했습니다.".into());
        }
    }
    with_store(&state, |s| s.ai_save_preferences(&preferences))
}
#[tauri::command]
pub async fn ai_analyze(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
    regenerate: bool,
) -> Result<analysis::StartResponse, String> {
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    state.analysis.start(
        state.store.clone(),
        state.worker.clone(),
        state.data_root.clone(),
        document_id,
        revision_id,
        regenerate,
    )
}
#[tauri::command]
pub fn ai_cancel(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    state.analysis.cancel(&state.store, id)
}
#[tauri::command]
pub fn ai_results(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Vec<AiResult>, String> {
    with_store(&state, |s| {
        s.ai_document_results(document_id, Some(revision_id), 20)
    })
}
#[tauri::command]
pub fn ai_document_jobs(
    state: tauri::State<'_, AppState>,
    document_id: i64,
    revision_id: i64,
) -> Result<Vec<AiJob>, String> {
    with_store(&state, |s| s.ai_document_jobs(document_id, revision_id, 20))
}
#[tauri::command]
pub fn ai_recent(state: tauri::State<'_, AppState>) -> Result<Vec<AiResult>, String> {
    with_store(&state, |s| s.ai_recent_results(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, sync::Arc, time::Duration};
    use tokio::{sync::Mutex, time::timeout};

    fn unresponsive_worker() -> super::super::Worker {
        static FIXTURE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let directory = FIXTURE.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("worker.rs");
            std::fs::write(
                &source,
                "fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }",
            )
            .unwrap();
            let executable = directory.path().join(format!(
                "unresponsive-worker{}",
                std::env::consts::EXE_SUFFIX
            ));
            let output = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-name", "unresponsive_worker"])
                .arg(source)
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            directory
        });
        super::super::Worker::launch(&directory.path().join(format!(
            "unresponsive-worker{}",
            std::env::consts::EXE_SUFFIX
        )))
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
    async fn reset_cancels_and_reaps_a_restart_during_exit() {
        let worker = Mutex::new(unresponsive_worker());
        let shutdown = Arc::new(AtomicBool::new(false));
        let signal = shutdown.clone();
        let exit = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            signal.store(true, Ordering::Release);
        });
        assert_eq!(
            timeout(Duration::from_secs(1), reset_worker(&worker, &shutdown))
                .await
                .expect("exit must interrupt the pending restart status request"),
            Err(EXIT_ERROR.into())
        );
        exit.await.unwrap();
        let worker = worker.lock().await;
        assert!(worker.child.is_none(), "cancelled restart must be reaped");
        assert!(worker.input.is_none() && worker.output.is_none());
    }

    #[tokio::test]
    async fn reset_cannot_relaunch_worker_after_exit_cleanup() {
        let worker = Mutex::new(unresponsive_worker());
        worker.lock().await.stop().await;
        assert!(worker.lock().await.child.is_none());
        let shutdown = AtomicBool::new(true);
        assert_eq!(
            timeout(Duration::from_millis(200), reset_worker(&worker, &shutdown))
                .await
                .expect("a completed exit must not launch another process"),
            Err(EXIT_ERROR.into())
        );
        assert!(worker.lock().await.child.is_none());
    }
}
