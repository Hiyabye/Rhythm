//! Explicit real-weight evaluation. Fixtures are synthetic Korean notices, not university content.
//! Register as `#[cfg(test)] mod evaluation;` in the intelligence module.
use crate::{
    archive::model::{ArticleInput, SourceDefinition},
    intelligence::{analysis, models, Worker},
    storage::{intelligence::AiModel, Store},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Deserialize)]
struct Case {
    id: String,
    title: String,
    text: String,
    category: String,
    deadline: Option<String>,
    required_summary_terms: Vec<String>,
    note: String,
    #[serde(default)]
    additional_deadlines: Vec<String>,
    #[serde(default)]
    repeat_text: String,
    #[serde(default)]
    repeat_count: usize,
    #[serde(default)]
    suffix: String,
}
impl Case {
    fn body(&self) -> String {
        format!(
            "{}{}{}",
            self.text,
            self.repeat_text.repeat(self.repeat_count),
            self.suffix
        )
    }
    fn article(&self) -> ArticleInput {
        let body = self.body();
        ArticleInput {
            external_id: self.id.clone(),
            canonical_url: format!("https://www.sogang.ac.kr/evaluation/{}", self.id),
            title: self.title.clone(),
            author: Some("합성 평가 자료".into()),
            category: None,
            published_at: "2026-01-01".into(),
            modified_at: None,
            original: body.clone(),
            unsupported_body: false,
            body_html: "<p>합성 평가 자료</p>".into(),
            body_text: body,
            resources: vec![],
            etag: None,
            last_modified: None,
        }
    }
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("evaluation.json")).expect("valid evaluation fixtures")
}
fn seed(store: &Store) {
    store
        .catalog(&[SourceDefinition {
            id: "ai-evaluation".into(),
            name: "합성 한국어 평가".into(),
            organization: "Rhythm".into(),
            description: "외부 수집이 아닌 결정적 합성 자료".into(),
            url: "https://www.sogang.ac.kr/".into(),
            family: "main".into(),
            supported: true,
        }])
        .unwrap();
}
pub(super) fn installed_model(root: &Path, id: &str) -> AiModel {
    let entry = models::catalog()
        .into_iter()
        .find(|entry| entry.id == id)
        .expect("RHYTHM_MODEL_EVAL_ID must identify a curated artifact");
    // Never open the downloader's DB: no recovery or shared-state mutation occurs here.
    let path = root.join("models").join(format!("{}.gguf", entry.sha256));
    assert!(
        path.is_file(),
        "install must finish first: {}",
        path.display()
    );
    assert_eq!(
        path.metadata().unwrap().len(),
        entry.bytes,
        "incomplete artifact"
    );
    AiModel {
        id: entry.id.clone(),
        catalog_id: Some(entry.id),
        name: entry.name,
        ownership: "external".into(),
        path: path.canonicalize().unwrap().to_string_lossy().into_owned(),
        storage_directory: None,
        sha256: Some(entry.sha256.clone()),
        expected_sha256: Some(entry.sha256),
        expected_bytes: Some(entry.bytes),
        bytes: entry.bytes,
        source_revision: Some(entry.revision),
        source_filename: Some(entry.filename),
        status: "installed".into(),
        downloaded_bytes: 0,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
fn configure(store: &Store, model: &AiModel) {
    store.ai_upsert_model(model).unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some(model.id.clone());
    preferences.processing_mode = "explicit".into();
    preferences.max_tokens = 1024;
    store.ai_save_preferences(&preferences).unwrap();
}
/// Test-only backend override. The bundled macOS build falls back to the same
/// binary with `-ngl 0 --device none` after a load or request failure; selecting
/// that CPU launch directly makes the fallback session evaluable end to end.
pub(super) fn evaluation_worker(executable: &Path, runtime: &Path) -> Worker {
    if std::env::var("RHYTHM_AI_EVAL_FORCE_CPU").as_deref() != Ok("1") {
        return Worker::launch_configured(executable, runtime, "auto");
    }
    let (_, cpu) = super::bundled(executable);
    Worker::with_launches(None, cpu, runtime, "auto")
}

pub(super) fn report_path(id: &str) -> PathBuf {
    std::env::var_os("RHYTHM_AI_EVAL_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("rhythm-ai-evaluation-{id}.json")))
}
pub(super) fn write_report(path: &Path, report: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    eprintln!("Actual inference evaluation report: {}", path.display());
}
#[cfg(unix)]
pub(super) fn worker_rss_kib(pid: Option<u32>) -> Option<u64> {
    let pid = pid?;
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}
#[cfg(windows)]
pub(super) fn worker_rss_kib(pid: Option<u32>) -> Option<u64> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
    };
    // SAFETY: The process handle is closed once and the output structure is valid.
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid?,
        )
        .ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let size = counters.cb;
        let result = GetProcessMemoryInfo(handle, &mut counters, size)
            .ok()
            .map(|()| counters.WorkingSetSize as u64 / 1024);
        let _ = CloseHandle(handle);
        result
    }
}
#[cfg(not(any(unix, windows)))]
pub(super) fn worker_rss_kib(_pid: Option<u32>) -> Option<u64> {
    None
}

#[cfg(unix)]
struct WorkerTerminator(u32);
#[cfg(unix)]
impl WorkerTerminator {
    fn open(pid: u32) -> Option<Self> {
        Some(Self(pid))
    }
    fn terminate(&self) -> bool {
        std::process::Command::new("/bin/kill")
            .args(["-KILL", &self.0.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    }
}
#[cfg(windows)]
struct WorkerTerminator(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WorkerTerminator {
    fn open(pid: u32) -> Option<Self> {
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE};
        // SAFETY: pid belongs to this test's child; the owned handle is closed on drop.
        unsafe { OpenProcess(PROCESS_TERMINATE, false, pid).ok().map(Self) }
    }
    fn terminate(&self) -> bool {
        // SAFETY: the handle identifies the specific child opened before inference,
        // even if the child exits and Windows later reuses its numeric PID.
        unsafe { windows::Win32::System::Threading::TerminateProcess(self.0, 1).is_ok() }
    }
}
#[cfg(windows)]
impl Drop for WorkerTerminator {
    fn drop(&mut self) {
        // SAFETY: this object owns the handle, which is closed exactly once.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}
#[cfg(not(any(unix, windows)))]
struct WorkerTerminator;
#[cfg(not(any(unix, windows)))]
impl WorkerTerminator {
    fn open(_pid: u32) -> Option<Self> {
        None
    }
    fn terminate(&self) -> bool {
        false
    }
}
async fn settle(controller: &analysis::Controller) {
    tokio::time::timeout(Duration::from_secs(1500), controller.wait())
        .await
        .expect("controller exceeded bounded hash/load/inference deadline");
}
#[test]
fn stored_result_screening_rechecks_shape_and_literal_source() {
    let source = "재학생은 2026년 4월 30일까지 장학금을 신청하세요.";
    let valid = json!({"short_summary":"재학생 장학금 신청 안내입니다.","summary":"재학생은 2026년 4월 30일까지 신청해야 합니다.","category":"장학","uncertainty":"clear","facts":[{"kind":"deadline","label":"신청 마감","value":"2026년 4월 30일","evidence":source,"date":"2026-04-30","uncertainty":"clear"}]});
    assert_eq!(stored_result_checks(&valid, source), (true, true));
    assert_eq!(
        stored_result_checks(&valid, "다른 공지입니다."),
        (true, false)
    );
    let mut bad_date = valid.clone();
    bad_date["facts"][0]["date"] = json!("2026-05-01");
    assert_eq!(stored_result_checks(&bad_date, source), (true, false));
    let mut bad_shape = valid;
    bad_shape["facts"] = json!("not an array");
    assert_eq!(stored_result_checks(&bad_shape, source), (false, false));
}
pub(super) fn stored_result_checks(result: &Value, source: &str) -> (bool, bool) {
    // Recheck the stored shape and exact source grounding independently of job
    // completion. Neither check proves the model preserved the source's meaning.
    let shape = serde_json::from_value::<analysis::Analysis>(result.clone()).is_ok();
    let grounded = analysis::validate_output(&result.to_string(), source).is_ok();
    (shape, grounded)
}
fn assess(case: &Case, result: &Value, partial: bool) -> Value {
    let (schema_valid, literal_grounding_valid) =
        stored_result_checks(result, &format!("{}\n\n{}", case.title, case.body()));
    let category_correct = result["category"].as_str() == Some(case.category.as_str());
    let summary = format!(
        "{} {}",
        result["short_summary"].as_str().unwrap_or(""),
        result["summary"].as_str().unwrap_or("")
    );
    let missing_summary_terms = case
        .required_summary_terms
        .iter()
        .filter(|term| !summary.contains(term.as_str()))
        .collect::<Vec<_>>();
    let dates = result["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|fact| fact["kind"] == "deadline")
        .filter_map(|fact| fact["date"].as_str())
        .collect::<Vec<_>>();
    let expected_dates = case
        .deadline
        .iter()
        .chain(case.additional_deadlines.iter())
        .map(String::as_str)
        .collect::<Vec<_>>();
    let extra_deadline_dates = dates
        .iter()
        .filter(|date| !expected_dates.contains(date))
        .collect::<Vec<_>>();
    let missing_deadline_dates = expected_dates
        .iter()
        .filter(|date| !dates.contains(date))
        .collect::<Vec<_>>();
    let deadline_correct = extra_deadline_dates.is_empty() && missing_deadline_dates.is_empty();
    json!({"category_correct":category_correct,"deadline_correct":deadline_correct,
        "missing_summary_terms":missing_summary_terms,
        "extra_deadline_dates":extra_deadline_dates,"missing_deadline_dates":missing_deadline_dates,
        "schema_valid":schema_valid,"literal_grounding_valid":literal_grounding_valid,
        "partial_disclosure_correct":!matches!(case.id.as_str(), "long" | "long_tail_correction") || partial,
        "human_review_required":true,"review_note":case.note})
}

fn recovery_eligible(job_status: &str, assessment: &Value) -> bool {
    job_status == "completed"
        && assessment["schema_valid"] == true
        && assessment["literal_grounding_valid"] == true
}

#[test]
fn recovery_eligibility_is_independent_of_semantic_quality() {
    let assessment = json!({"schema_valid":true,"literal_grounding_valid":true,
        "category_correct":false,"deadline_correct":true});
    assert!(recovery_eligible("completed", &assessment));
    assert!(!recovery_eligible("failed", &assessment));
    let mut invalid = assessment.clone();
    invalid["schema_valid"] = json!(false);
    assert!(!recovery_eligible("completed", &invalid));
    invalid = assessment;
    invalid["literal_grounding_valid"] = json!(false);
    assert!(!recovery_eligible("completed", &invalid));
}

async fn inference_request_entered(controller: &analysis::Controller) -> bool {
    tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            // This observes dispatched parent-side inference IPC, not an active
            // model kernel. Mutex contention alone does not establish this phase.
            if controller
                .progress()
                .is_some_and(|progress| progress.phase == "inference")
            {
                return true;
            }
            if !controller.running() {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or(false)
}

// This is used only by the explicitly ignored, isolated real-model evaluation.
// A separate OS thread bounds cleanup even if an OS file/SQLite call blocks the
// current-thread async runtime. Ending the test process closes its worker Job
// Object, and must be treated as a failed test, not successful recovery.
async fn cancel_and_join_or_exit(
    controller: &analysis::Controller,
    store: &Arc<Mutex<Store>>,
    job_id: i64,
) {
    let (finished, watch) = std::sync::mpsc::channel::<()>();
    let watchdog = std::thread::Builder::new()
        .name("rhythm-eval-cleanup-watchdog".into())
        .spawn(move || {
            if watch.recv_timeout(Duration::from_secs(45)).is_err() {
                eprintln!("evaluation cleanup did not complete in 45 seconds; terminating isolated test process");
                std::process::exit(1);
            }
        })
        .unwrap_or_else(|error| {
            eprintln!("could not start evaluation cleanup watchdog: {error}");
            std::process::exit(1);
        });
    if let Err(error) = controller.cancel(store, job_id) {
        // The attempt may already be terminal; still wait for its owner to exit.
        eprintln!("evaluation cancellation could not be recorded: {error}");
    }
    if tokio::time::timeout(Duration::from_secs(30), controller.wait())
        .await
        .is_err()
    {
        eprintln!(
            "evaluation cleanup did not join in 30 seconds; terminating isolated test process"
        );
        std::process::exit(1);
    }
    let _ = finished.send(());
    let _ = watchdog.join();
}
async fn exercise_recovery(
    controller: &analysis::Controller,
    store: &Arc<Mutex<Store>>,
    worker: &Arc<AsyncMutex<Worker>>,
    root: &Path,
    document_id: i64,
    revision_id: i64,
) -> Value {
    let before = store
        .lock()
        .unwrap()
        .ai_document_results(document_id, Some(revision_id), 100)
        .unwrap()
        .len();
    let cancelled = controller
        .start(
            store.clone(),
            worker.clone(),
            root.into(),
            document_id,
            revision_id,
            true,
        )
        .unwrap()
        .job
        .unwrap();
    if !inference_request_entered(controller).await {
        cancel_and_join_or_exit(controller, store, cancelled.id).await;
        return json!({"recovered":false,"error":"cancel sample never reached inference IPC phase"});
    }
    controller.cancel(store, cancelled.id).unwrap();
    settle(controller).await;
    let cancelled_status = store
        .lock()
        .unwrap()
        .ai_job(cancelled.id)
        .unwrap()
        .unwrap()
        .status;
    let after_cancel = store
        .lock()
        .unwrap()
        .ai_document_results(document_id, Some(revision_id), 100)
        .unwrap()
        .len();
    // Cancellation reaps the prior child. Start and record the PID of this
    // isolated test-owned child before its mutex is held by the next inference.
    let terminator = {
        let mut engine = worker.lock().await;
        if engine.restart().await.is_err() {
            return json!({"recovered":false,"error":"could not restart child"});
        }
        engine.process_id().and_then(WorkerTerminator::open)
    };
    let Some(terminator) = terminator else {
        return json!({"recovered":false,"error":"could not open this test-specific child"});
    };
    let crashed = controller
        .start(
            store.clone(),
            worker.clone(),
            root.into(),
            document_id,
            revision_id,
            true,
        )
        .unwrap()
        .job
        .unwrap();
    if !inference_request_entered(controller).await {
        cancel_and_join_or_exit(controller, store, crashed.id).await;
        return json!({"recovered":false,"cancelled_status":cancelled_status,"error":"crash sample never reached inference IPC phase"});
    }
    // Only the child copied into this test's unique directory is deliberately killed.
    let killed = terminator.terminate();
    settle(controller).await;
    let crashed_status = store
        .lock()
        .unwrap()
        .ai_job(crashed.id)
        .unwrap()
        .unwrap()
        .status;
    let recovered = controller
        .start(
            store.clone(),
            worker.clone(),
            root.into(),
            document_id,
            revision_id,
            true,
        )
        .unwrap()
        .job
        .unwrap();
    settle(controller).await;
    let recovered_status = store
        .lock()
        .unwrap()
        .ai_job(recovered.id)
        .unwrap()
        .unwrap()
        .status;
    // Forced reanalysis creates a new result for the same cache key. Identify
    // it by the actual recovery job rather than assuming the original ID.
    let recovered_result_id = store
        .lock()
        .unwrap()
        .ai_document_results(document_id, Some(revision_id), 100)
        .unwrap()
        .into_iter()
        .find(|result| result.job_id == recovered.id)
        .map(|result| result.id);
    json!({"interruption_boundary":"parent inference IPC phase; not proof of an active model kernel",
        "cancelled_status":cancelled_status,"prior_result_preserved":before == after_cancel,
        "child_killed":killed,"crashed_status":crashed_status,"recovery_status":recovered_status,
        "recovered_result_id":recovered_result_id,
        "recovered":cancelled_status == "cancelled" && before == after_cancel && killed && crashed_status == "failed" && recovered_status == "completed" && recovered_result_id.is_some()})
}

/// Same artifact/document/configuration, cold model then forced warm regeneration and cache.
/// Run both this parent test binary and the worker with --release for release comparisons.
#[tokio::test]
#[ignore = "real installed model and worker required; local bounded latency benchmark"]
async fn actual_analysis_latency_benchmark() {
    let executable = crate::intelligence::staged_llama()
        .expect("stage llama.cpp with `node scripts/stage-llama.mjs` or set RHYTHM_LLAMA_TEST_DIR");
    let root = PathBuf::from(std::env::var_os("RHYTHM_MODEL_EVAL_ROOT").expect("model root"));
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-2b".into());
    let model = installed_model(&root, &id);
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("benchmark.sqlite3")).unwrap();
    seed(&store);
    configure(&store, &model);
    let case = cases().into_iter().find(|c| c.id == "scholarship").unwrap();
    let (document_id, _) = store
        .upsert_article("ai-evaluation", &case.article(), false)
        .unwrap();
    let revision_id = store.document(document_id, None).unwrap().revision_id;
    let store = Arc::new(Mutex::new(store));
    let startup = Instant::now();
    let worker = Arc::new(AsyncMutex::new(evaluation_worker(
        &executable,
        &directory.path().join("run"),
    )));
    let health = worker.lock().await.refresh().await;
    let mut report = json!({"label":std::env::var("RHYTHM_AI_BENCH_LABEL").ok(),
        "parent_debug_assertions":cfg!(debug_assertions),"worker":executable,"model":model,
        "case":"scholarship","prompt_version":analysis::PROMPT_VERSION,
        "schema_version":analysis::SCHEMA_VERSION,"max_tokens":1024,
        "decoding_mode":analysis::decoding_mode(),
        "startup_ms":startup.elapsed().as_millis(),"initial_runtime":health,"samples":[]});
    let controller = analysis::Controller::default();
    let mut failures = Vec::new();
    for mode in ["cold", "warm"] {
        let started = Instant::now();
        let admitted = controller
            .start(
                store.clone(),
                worker.clone(),
                directory.path().into(),
                document_id,
                revision_id,
                true,
            )
            .unwrap();
        let job_id = admitted.job.unwrap().id;
        tokio::time::timeout(Duration::from_secs(1500), controller.wait())
            .await
            .expect("bounded benchmark deadline");
        let wall_ms = started.elapsed().as_millis();
        let job = store.lock().unwrap().ai_job(job_id).unwrap().unwrap();
        let results = store
            .lock()
            .unwrap()
            .ai_document_results(document_id, Some(revision_id), 10)
            .unwrap();
        let mut sample = json!({"mode":mode,"wall_ms":wall_ms,"admission_ms":admitted.admission_ms,
            "job":job,"diagnostics":controller.diagnostics(job_id),"worker_rss_kib":worker_rss_kib(worker.lock().await.process_id())});
        if job.status == "completed" {
            let result = results.first().unwrap();
            sample["assessment"] = assess(&case, &result.result, result.partial);
            sample["result"] = json!(result);
            let cached_started = Instant::now();
            let cached = controller
                .start(
                    store.clone(),
                    worker.clone(),
                    directory.path().into(),
                    document_id,
                    revision_id,
                    false,
                )
                .unwrap();
            sample["cached_wall_us"] = json!(cached_started.elapsed().as_micros());
            sample["cache_lookup_ms"] = json!(cached.cache_lookup_ms);
            assert_eq!(cached.cached.unwrap().id, result.id);
            assert!(cached.job.is_none());
        } else {
            failures.push(format!("{mode}: {:?}", job.error));
        }
        report["samples"].as_array_mut().unwrap().push(sample);
        write_report(&report_path(&id), &report);
    }
    let shutdown = Instant::now();
    controller.stop();
    controller.wait().await;
    worker.lock().await.shutdown().await;
    report["shutdown_ms"] = json!(shutdown.elapsed().as_millis());
    report["failures"] = json!(failures);
    write_report(&report_path(&id), &report);
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn reproducible_evaluation_fixtures_cover_required_shapes() {
    let cases = cases();
    assert_eq!(cases.len(), 18);
    for original in [
        "academic",
        "scholarship",
        "internship",
        "competition",
        "research",
        "multiple_dates",
        "no_deadline",
        "long",
        "complex_eligibility",
        "ambiguous",
    ] {
        assert!(cases.iter().any(|case| case.id == original));
    }
    let identities = cases
        .iter()
        .map(|case| &case.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(identities.len(), 18);
    for case in &cases {
        assert!(analysis::CATEGORIES.contains(&case.category.as_str()));
        assert!(!case.text.is_empty() && !case.required_summary_terms.is_empty());
    }
    assert!(
        cases
            .iter()
            .find(|case| case.id == "long")
            .unwrap()
            .body()
            .len()
            > 100_000
    );
    assert!(cases
        .iter()
        .find(|case| case.id == "ambiguous")
        .unwrap()
        .deadline
        .is_none());
    assert!(cases
        .iter()
        .find(|case| case.id == "no_deadline")
        .unwrap()
        .deadline
        .is_none());
}

/// Full production Controller/Store/Worker path; no fake generation and no network client.
/// Defaults to two cases. Set RHYTHM_AI_EVAL_LIMIT=10 for the full opt-in set.
#[tokio::test]
#[ignore = "real multi-gigabyte Qwen3.5 model and local worker required; CPU can take minutes per sample"]
async fn actual_qwen35_analysis_evaluation() {
    let executable = crate::intelligence::staged_llama()
        .expect("stage llama.cpp with `node scripts/stage-llama.mjs` or set RHYTHM_LLAMA_TEST_DIR");
    let root = PathBuf::from(
        std::env::var_os("RHYTHM_MODEL_EVAL_ROOT")
            .expect("set RHYTHM_MODEL_EVAL_ROOT to completed model install"),
    );
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-4b".into());
    let model = installed_model(&root, &id);
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("evaluation.sqlite3");
    let store = Store::open(&db).unwrap();
    seed(&store);
    configure(&store, &model);
    let store = Arc::new(Mutex::new(store));
    let worker = Arc::new(AsyncMutex::new(evaluation_worker(
        &executable,
        &directory.path().join("run"),
    )));
    let controller = analysis::Controller::default();
    let mut report = json!({
        "model":model,"worker":executable,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "backend_note":std::env::var("RHYTHM_AI_EVAL_BACKEND").unwrap_or_else(|_| "unreported; inspect worker build".into()),
        "fixture_provenance":"synthetic Korean notices, not collected university notices",
        "prompt_version":analysis::PROMPT_VERSION,"schema_version":analysis::SCHEMA_VERSION,
        "decoding_mode":analysis::decoding_mode(),
        "max_tokens":1024,"samples":[],"native_ui_verified":false,
        "final_cache_check":{"status":"pending"},
        "network_note":"test constructs no HTTP client; this alone is not OS-enforced network denial"
    });
    let report_file = report_path(&id);
    let load_started = Instant::now();
    let loaded = worker.lock().await.load(Path::new(&model.path)).await;
    report["load_ms"] = json!(load_started.elapsed().as_millis());
    report["forced_cpu"] = json!(std::env::var("RHYTHM_AI_EVAL_FORCE_CPU").as_deref() == Ok("1"));
    report["post_load_worker_rss_kib"] = json!(worker_rss_kib(worker.lock().await.process_id()));
    match loaded {
        // The status detail reports the backend the server actually loaded, so a
        // forced CPU run is evidence, not just the requested launch.
        Ok(status) => report["loaded_worker_status"] = json!(status),
        Err(error) => {
            report["load_error"] = json!(error);
            write_report(&report_file, &report);
            worker.lock().await.shutdown().await;
            panic!("actual Qwen3.5 load failed; see {}", report_file.display());
        }
    }
    let limit = std::env::var("RHYTHM_AI_EVAL_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 18);
    let filter = std::env::var("RHYTHM_AI_EVAL_CASES").ok();
    let mut failures = Vec::new();
    let mut quality_failures = Vec::new();
    let mut completed = Vec::new();
    let mut recovery_candidate = None;
    for case in cases()
        .into_iter()
        .filter(|case| {
            filter
                .as_ref()
                .is_none_or(|filter| filter.split(',').any(|id| id == case.id))
        })
        .take(limit)
    {
        let (document_id, revision_id) = {
            let store = store.lock().unwrap();
            let (document_id, _) = store
                .upsert_article("ai-evaluation", &case.article(), false)
                .unwrap();
            (
                document_id,
                store.document(document_id, None).unwrap().revision_id,
            )
        };
        let started = Instant::now();
        let response = controller
            .start(
                store.clone(),
                worker.clone(),
                directory.path().into(),
                document_id,
                revision_id,
                false,
            )
            .unwrap();
        let job_id = response.job.unwrap().id;
        settle(&controller).await;
        let (job, results) = {
            let store = store.lock().unwrap();
            (
                store.ai_job(job_id).unwrap().unwrap(),
                store
                    .ai_document_results(document_id, Some(revision_id), 10)
                    .unwrap(),
            )
        };
        let mut sample = json!({"case":case.id,"job":job,"wall_ms":started.elapsed().as_millis(),
            "diagnostics":controller.diagnostics(job_id),
            "worker_rss_kib":worker_rss_kib(worker.lock().await.process_id())});
        if let Some(result) = results.first() {
            sample["assessment"] = assess(&case, &result.result, result.partial);
            sample["result"] = json!(result);
            // Cache works without launching inference; preserve the same validated result ID.
            let cached_started = Instant::now();
            let cached = controller
                .start(
                    store.clone(),
                    worker.clone(),
                    directory.path().into(),
                    document_id,
                    revision_id,
                    false,
                )
                .unwrap();
            sample["cached_wall_us"] = json!(cached_started.elapsed().as_micros());
            assert!(cached.job.is_none());
            assert_eq!(cached.cached.unwrap().id, result.id);
            sample["cache_reused"] = json!(true);
            completed.push((document_id, revision_id, result.id));
            if recovery_eligible(&job.status, &sample["assessment"]) {
                recovery_candidate.get_or_insert((document_id, revision_id));
            }
            if sample["assessment"]["schema_valid"] != true
                || sample["assessment"]["literal_grounding_valid"] != true
                || sample["assessment"]["category_correct"] != true
                || sample["assessment"]["deadline_correct"] != true
                || sample["assessment"]["partial_disclosure_correct"] != true
                || !sample["assessment"]["missing_summary_terms"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            {
                let failure = format!("{}: quality assertion failed", case.id);
                quality_failures.push(failure.clone());
                failures.push(failure);
            }
            // A real new archived revision must not accidentally reuse the old revision cache.
            let mut revised = case.article();
            revised
                .body_text
                .push_str("\n정정: 세부 안내는 원문을 확인해 주세요.");
            store
                .lock()
                .unwrap()
                .upsert_article("ai-evaluation", &revised, false)
                .unwrap();
            let new_revision = store
                .lock()
                .unwrap()
                .document(document_id, None)
                .unwrap()
                .revision_id;
            assert_ne!(new_revision, revision_id);
            let key = analysis::key_for(document_id, new_revision, &model, 1024).unwrap();
            assert!(store
                .lock()
                .unwrap()
                .ai_cached_result(&key)
                .unwrap()
                .is_none());
            assert_eq!(
                store
                    .lock()
                    .unwrap()
                    .ai_document_results(document_id, Some(revision_id), 10)
                    .unwrap()[0]
                    .id,
                result.id
            );
            sample["revision_cache_isolated"] = json!(true);
        } else {
            failures.push(format!(
                "{}: {}",
                case.id,
                job.error.as_deref().unwrap_or("no validated result")
            ));
        }
        report["samples"].as_array_mut().unwrap().push(sample);
        write_report(&report_file, &report);
    }
    let samples_count = report["samples"].as_array().unwrap().len();
    let quality_status = if !quality_failures.is_empty() {
        "failed"
    } else if samples_count == 0 || completed.len() != samples_count {
        "incomplete"
    } else {
        "passed"
    };
    if quality_status == "incomplete" {
        failures.push("quality evaluation incomplete".into());
    }
    report["quality"] = json!({"status":quality_status,"failures":quality_failures});
    report["failures"] = json!(failures);
    write_report(&report_file, &report);
    if std::env::var("RHYTHM_AI_EVAL_RECOVERY").as_deref() == Ok("1") {
        if let Some((document_id, revision_id)) = recovery_candidate {
            report["recovery"] = json!({"status":"running"});
            write_report(&report_file, &report);
            let mut recovery = exercise_recovery(
                &controller,
                &store,
                &worker,
                directory.path(),
                document_id,
                revision_id,
            )
            .await;
            if let Some(recovered_result_id) = recovery["recovered_result_id"].as_i64() {
                if let Some((_, _, expected_id)) = completed
                    .iter_mut()
                    .find(|(doc, revision, _)| *doc == document_id && *revision == revision_id)
                {
                    *expected_id = recovered_result_id;
                }
            }
            if recovery["recovered"] != true {
                failures.push("real-worker recovery failed".into());
                recovery["status"] = json!("failed");
            } else {
                recovery["status"] = json!("passed");
            }
            report["recovery"] = recovery;
        } else {
            report["recovery"] = json!({"recovered":false,"status":"skipped",
                "error":"no completed result passed schema and literal grounding checks"});
            failures.push("real-worker recovery skipped: no eligible completed result".into());
        }
    } else {
        report["recovery"] = json!({"status":"not_requested"});
    }
    report["failures"] = json!(failures);
    write_report(&report_file, &report);
    worker.lock().await.shutdown().await;
    controller.stop();
    controller.wait().await;
    drop(controller);
    drop(store);
    // Only this test's isolated Store is reopened. Downloader/production DBs stay untouched.
    let reopened = Arc::new(Mutex::new(Store::open(&db).unwrap()));
    let cache_controller = analysis::Controller::default();
    // An explicitly stopped worker must remain stopped across persisted cache reads.
    worker.lock().await.stop().await;
    let mut cache_checks = Vec::new();
    for (document_id, revision_id, result_id) in &completed {
        let cached = cache_controller
            .start(
                reopened.clone(),
                worker.clone(),
                directory.path().into(),
                *document_id,
                *revision_id,
                false,
            )
            .unwrap();
        let cached_id = cached.cached.map(|result| result.id);
        let worker_stopped = worker.lock().await.status().state == "unavailable";
        let passed = cached.job.is_none() && cached_id == Some(*result_id) && worker_stopped;
        if !passed {
            failures.push(format!(
                "persisted cache check failed for document {document_id}"
            ));
        }
        cache_checks.push(json!({"document_id":document_id,"revision_id":revision_id,
            "expected_result_id":result_id,"cached_result_id":cached_id,
            "worker_stopped":worker_stopped,"passed":passed}));
    }
    let passed_cache_checks = cache_checks
        .iter()
        .filter(|check| check["passed"] == true)
        .count();
    let cache_status = if cache_checks.is_empty() {
        failures.push("persisted cache check incomplete: no completed result".into());
        "incomplete"
    } else if passed_cache_checks == cache_checks.len() {
        "passed"
    } else {
        "failed"
    };
    report["final_cache_check"] = json!({"status":cache_status,"checks":cache_checks});
    report["reopened_cache_reads_with_worker_stopped"] = json!(passed_cache_checks);
    report["current_revision_result_count"] = json!(reopened
        .lock()
        .unwrap()
        .ai_recent_results(100)
        .unwrap()
        .len());
    report["stored_history_count"] = json!(reopened.lock().unwrap().ai_jobs(100).unwrap().len());
    report["failures"] = json!(failures);
    write_report(&report_file, &report);
    assert!(
        failures.is_empty(),
        "actual model evaluation failed: {failures:?}; see {}",
        report_file.display()
    );
}

/// Real vision path for one curated model + its pinned projector: the archived
/// image must reach the server as an image part, the result must pass the same
/// grounding validator as text, and image-derived facts must be marked
/// unverified. Quality is reported, not asserted, because a model may answer
/// "unknown"; structural failures do fail the test.
#[tokio::test]
#[ignore = "real multi-gigabyte model + mmproj projector required; set RHYTHM_VISION_EVAL_ROOT and RHYTHM_MODEL_EVAL_ID"]
async fn actual_vision_image_analysis_evaluation() {
    use sha2::{Digest, Sha256};
    let executable = crate::intelligence::staged_llama()
        .expect("stage llama.cpp with `node scripts/stage-llama.mjs` or set RHYTHM_LLAMA_TEST_DIR");
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-2b".into());
    let entry = models::catalog()
        .into_iter()
        .find(|entry| entry.id == id)
        .expect("RHYTHM_MODEL_EVAL_ID must identify a curated artifact");
    let projector = entry
        .projector
        .clone()
        .expect("curated entry must pin a projector");
    let artifacts = PathBuf::from(
        std::env::var_os("RHYTHM_VISION_EVAL_ROOT")
            .expect("set RHYTHM_VISION_EVAL_ROOT with <id>.gguf and <id>-mmproj.gguf"),
    );
    let model_source = artifacts.join(format!("{id}.gguf"));
    let projector_source = artifacts.join(format!("{id}-mmproj.gguf"));
    assert_eq!(
        std::fs::metadata(&model_source).unwrap().len(),
        entry.bytes,
        "model artifact size mismatch: {}",
        model_source.display()
    );
    assert_eq!(
        std::fs::metadata(&projector_source).unwrap().len(),
        projector.bytes,
        "projector artifact size mismatch: {}",
        projector_source.display()
    );

    // The production layout is a managed `models/` folder; hard links avoid
    // copying multi-gigabyte artifacts while staying regular files.
    let run = tempfile::tempdir().unwrap();
    let root = run.path();
    std::fs::create_dir(root.join("models")).unwrap();
    let model_relative = format!("models/{}.gguf", entry.sha256);
    let projector_relative = format!("models/{}.gguf", projector.sha256);
    std::fs::hard_link(&model_source, root.join(&model_relative)).unwrap();
    std::fs::hard_link(&projector_source, root.join(&projector_relative)).unwrap();

    let store = Store::open(&root.join("vision.sqlite3")).unwrap();
    seed(&store);
    let model = AiModel {
        id: entry.id.clone(),
        catalog_id: Some(entry.id.clone()),
        name: entry.name.clone(),
        ownership: "managed".into(),
        path: model_relative,
        storage_directory: None,
        sha256: Some(entry.sha256.clone()),
        expected_sha256: Some(entry.sha256.clone()),
        expected_bytes: Some(entry.bytes),
        bytes: entry.bytes,
        source_revision: Some(entry.revision.clone()),
        source_filename: Some(entry.filename.clone()),
        status: "installed".into(),
        downloaded_bytes: entry.bytes,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    store.ai_upsert_model(&model).unwrap();
    store
        .ai_upsert_projector(&crate::storage::intelligence::AiProjector {
            model_id: model.id.clone(),
            path: projector_relative,
            sha256: Some(projector.sha256.clone()),
            expected_sha256: projector.sha256.clone(),
            expected_bytes: projector.bytes,
            bytes: projector.bytes,
            status: "installed".into(),
            downloaded_bytes: projector.bytes,
            resume_etag: None,
            error: None,
        })
        .unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some(model.id.clone());
    preferences.processing_mode = "explicit".into();
    preferences.max_tokens = 1024;
    store.ai_save_preferences(&preferences).unwrap();

    // The archived body is nearly empty: the deadline exists only in the image.
    let poster = include_bytes!("../archive/fixtures/vision/poster.png");
    std::fs::create_dir(root.join("resources")).unwrap();
    std::fs::write(root.join("resources/poster.png"), poster).unwrap();
    let (document_id, revision_id) = {
        let (document_id, _) = store
            .upsert_article(
                "ai-evaluation",
                &ArticleInput {
                    external_id: "vision-poster".into(),
                    canonical_url: "https://www.sogang.ac.kr/evaluation/vision-poster".into(),
                    title: "2026학년도 2학기 국가장학금 신청 안내".into(),
                    author: Some("합성 평가 자료".into()),
                    category: None,
                    published_at: "2026-01-01".into(),
                    modified_at: None,
                    original: "붙임 포스터를 확인하세요.".into(),
                    unsupported_body: false,
                    body_html: "<p>붙임 포스터를 확인하세요.</p>".into(),
                    body_text: "붙임 포스터를 확인하세요.".into(),
                    resources: Vec::new(),
                    etag: None,
                    last_modified: None,
                },
                false,
            )
            .unwrap();
        let revision_id = store.document(document_id, None).unwrap().revision_id;
        let resource = store
            .add_resource_for_test(
                document_id,
                revision_id,
                "https://www.sogang.ac.kr/evaluation/poster.png",
                "poster.png",
                "image",
            )
            .unwrap();
        store
            .resource_result(
                resource,
                "downloaded",
                Some("image/png"),
                Some(poster.len() as u64),
                Some("resources/poster.png"),
                Some(&format!("{:x}", Sha256::digest(poster))),
                None,
            )
            .unwrap();
        (document_id, revision_id)
    };

    let store = Arc::new(Mutex::new(store));
    let worker = Arc::new(AsyncMutex::new(evaluation_worker(
        &executable,
        &root.join("run"),
    )));
    let controller = analysis::Controller::default();
    let started = Instant::now();
    let response = controller
        .start(
            store.clone(),
            worker.clone(),
            root.into(),
            document_id,
            revision_id,
            false,
        )
        .unwrap();
    let job_id = response.job.unwrap().id;
    settle(&controller).await;
    let (job, results) = {
        let store = store.lock().unwrap();
        (
            store.ai_job(job_id).unwrap().unwrap(),
            store
                .ai_document_results(document_id, Some(revision_id), 10)
                .unwrap(),
        )
    };
    let result = results.first();
    let mut report = json!({
        "model": model,
        "projector": {"filename": projector.filename, "bytes": projector.bytes, "sha256": projector.sha256},
        "worker": executable,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "prompt_version": analysis::PROMPT_VERSION,
        "schema_version": analysis::SCHEMA_VERSION,
        "fixture": "synthetic poster; deadline exists only in the image",
        "wall_ms": started.elapsed().as_millis(),
        "job": job,
        "diagnostics": controller.diagnostics(job_id),
        "peak_worker_rss_kib": worker_rss_kib(worker.lock().await.process_id()),
        "result": result,
        "native_ui_verified": false,
    });
    let mut structural = Vec::new();
    match (job.status.as_str(), result) {
        ("completed", Some(result)) => {
            let images_used = result.model_identity["images_used"].as_u64().unwrap_or(0);
            if images_used != 1 {
                structural.push(format!("images_used={images_used}, expected 1"));
            }
            let facts = result.result["facts"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let image_facts = facts
                .iter()
                .filter(|fact| fact["origin"] == "image")
                .count();
            let deadline = facts.iter().any(|fact| {
                let value = fact["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>();
                fact["kind"] == "deadline"
                    && ["9.12", "9월12", "09-12", "2026-09-12"]
                        .iter()
                        .any(|marker| value.contains(marker))
            });
            report["image_facts"] = json!(image_facts);
            report["deadline_read_from_poster"] = json!(deadline);
            report["assessment"] = json!({"structural_ok": structural.is_empty(), "deadline": deadline, "image_facts": image_facts});
        }
        (status, _) => structural.push(format!("job status {status}: {:?}", job.error)),
    }
    report["structural_failures"] = json!(structural);
    let report_file = std::env::var_os("RHYTHM_AI_EVAL_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("vision-eval-{id}.json"))
        });
    write_report(&report_file, &report);
    worker.lock().await.shutdown().await;
    assert!(
        structural.is_empty(),
        "vision evaluation structural failure: {structural:?}; see {}",
        report_file.display()
    );
}
