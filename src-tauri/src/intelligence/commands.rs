use super::{analysis, models, operations, WorkerStatus};
use crate::{
    storage::intelligence::{AiJob, AiModel, AiPreferences, AiResult},
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
const EXIT_ERROR: &str = "앱을 종료하고 있습니다.";
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

#[derive(Serialize)]
pub struct Snapshot {
    catalog: Vec<models::CatalogEntry>,
    models: Vec<AiModel>,
    preferences: AiPreferences,
    jobs: Vec<AiJob>,
    runtime: WorkerStatus,
    hardware: models::Hardware,
    busy: bool,
    operation: Option<operations::Status>,
    progress: Option<analysis::Diagnostics>,
    analysis_contract: AnalysisContract,
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
fn path(state: &AppState, model: &AiModel) -> PathBuf {
    if model.ownership == "managed" {
        state.data_root.join(&model.path)
    } else {
        PathBuf::from(&model.path)
    }
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
        path(state, model),
        model.sha256.clone().ok_or("모델 무결성 정보가 없습니다.")?,
        model.bytes,
        &state.maintenance_shutdown,
        operation,
    )
    .await
}

enum WorkerAction<'a> {
    Reset,
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
                WorkerAction::Reset => {
                    operation.phase("restarting");
                    worker.unload().await.map(|_| ())
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
    let (models, preferences, jobs) = with_store(&state, |s| {
        let mut jobs = s.ai_active_jobs()?;
        jobs.extend(s.ai_jobs(100)?);
        jobs.sort_by_key(|j| std::cmp::Reverse(j.id));
        jobs.dedup_by_key(|j| j.id);
        Ok((s.ai_models()?, s.ai_preferences()?, jobs))
    })?;
    let configuration_hash = preferences
        .active_model_id
        .as_ref()
        .and_then(|id| models.iter().find(|model| &model.id == id))
        .and_then(|model| analysis::key_for(0, 0, model, preferences.max_tokens).ok())
        .map(|key| key.configuration_hash);
    let initial_hardware = state.hardware.clone();
    let root = state.data_root.clone();
    let hardware =
        tokio::task::spawn_blocking(move || models::refresh_disk_space(initial_hardware, &root))
            .await
            .map_err(|_| "저장 공간 정보를 확인하지 못했습니다.")?;
    let operation = state.ai_operation.status();
    Ok(Snapshot {
        catalog: models::catalog(),
        models,
        preferences,
        jobs,
        runtime,
        hardware,
        busy: state.models.busy() || state.analysis.running() || operation.is_some(),
        operation,
        progress: state.analysis.progress(),
        analysis_contract: AnalysisContract {
            prompt_version: analysis::PROMPT_VERSION,
            schema_version: analysis::SCHEMA_VERSION,
            configuration_hash,
        },
    })
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
        WorkerAction::Reset,
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
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
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
#[tauri::command]
pub async fn ai_unload(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    if state.analysis.running() {
        return Err("먼저 진행 중인 분석을 취소해 주세요.".into());
    }
    reset_worker(&state.worker, &state.maintenance_shutdown).await
}
#[tauri::command]
pub async fn ai_restart(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
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
    let _lease = state.operation_gate.shared().await?;
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
    preferences: AiPreferences,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
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
        return with_store(&state, |s| s.ai_save_preferences(&preferences));
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
    let lease = state.operation_gate.shared().await?;
    let _gate = state.ai_gate.lock().await;
    check_open(&state)?;
    state.analysis.start_admitted(
        state.store.clone(),
        state.worker.clone(),
        state.data_root.clone(),
        document_id,
        revision_id,
        regenerate,
        lease,
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

    fn delayed_model_worker(setup: bool) -> (super::super::Worker, PathBuf) {
        static FIXTURE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let directory = FIXTURE.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("worker.rs");
            std::fs::write(&source, r#"
use std::io::{self, BufRead, Write};
fn main() {
    let executable = std::env::current_exe().unwrap();
    let setup = executable.file_stem().unwrap().to_string_lossy().contains("setup");
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let id = line.split("\"id\":").nth(1).unwrap().split(|c: char| !c.is_ascii_digit()).next().unwrap();
        if !executable.with_extension("entered").exists()
            && ((setup && line.contains("\"infer\"")) || (!setup && line.contains("\"load\""))) {
            std::fs::write(executable.with_extension("entered"), b"entered").unwrap();
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        println!("{{\"version\":1,\"id\":{},\"ok\":true,\"result\":{{\"engine\":\"mistral.rs\",\"runtime_available\":true,\"model\":\"/fake.gguf\",\"text\":\"로컬 점검을 완료했습니다.\"}}}}", id);
        io::stdout().flush().unwrap();
    }
}
"#).unwrap();
            let executable = directory.path().join(format!("delayed-load{}", std::env::consts::EXE_SUFFIX));
            let output = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-name", "delayed_model_worker"])
                .arg(source).arg("-o").arg(&executable).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            std::fs::copy(executable, directory.path().join(format!("delayed-setup{}", std::env::consts::EXE_SUFFIX))).unwrap();
            directory
        });
        let executable = directory.path().join(format!(
            "delayed-{}{}",
            if setup { "setup" } else { "load" },
            std::env::consts::EXE_SUFFIX
        ));
        let marker = executable.with_extension("entered");
        let _ = std::fs::remove_file(&marker);
        (super::super::Worker::launch(&executable), marker)
    }

    #[tokio::test]
    async fn command_owned_load_and_setup_cancel_reap_and_clear_status() {
        for setup in [false, true] {
            for exit in [false, true] {
                let (child, marker) = delayed_model_worker(setup);
                let worker = Arc::new(Mutex::new(child));
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
                            WorkerAction::Setup(Path::new("/fake.gguf"))
                        } else {
                            WorkerAction::Load(Path::new("/fake.gguf"))
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
                        worker.child.is_none(),
                        "cancelled operation must reap the actual child"
                    );
                    assert!(worker.input.is_none() && worker.output.is_none());
                }
                assert!(operations.status().is_none());
                if !exit {
                    let next = operations.begin("setup").unwrap();
                    timeout(
                        Duration::from_secs(1),
                        operation_worker(
                            &worker,
                            &shutdown,
                            &next,
                            WorkerAction::Setup(Path::new("/fake.gguf")),
                        ),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    drop(next);
                    assert!(operations.status().is_none());
                    assert!(
                        worker.lock().await.child.is_some(),
                        "next request must restart a fresh child"
                    );
                    worker.lock().await.stop().await;
                    assert!(worker.lock().await.child.is_none());
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
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(20u64.to_le_bytes());
        bytes.extend(b"general.architecture");
        bytes.extend(8u32.to_le_bytes());
        bytes.extend(5u64.to_le_bytes());
        bytes.extend(b"qwen3");
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
