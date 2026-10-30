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
pub(super) fn report_path(id: &str) -> PathBuf {
    std::env::var_os("RHYTHM_AI_EVAL_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("rhythm-ai-evaluation-{id}.json")))
}
pub(super) fn write_report(path: &Path, report: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    eprintln!("Actual inference evaluation report: {}", path.display());
}
fn worker_pid(path: &Path) -> Option<u32> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .ok()?;
    let expected = path.to_str()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (pid, executable) = line.split_once(char::is_whitespace)?;
            (executable.trim() == expected)
                .then(|| pid.parse().ok())
                .flatten()
        })
}
pub(super) fn worker_rss_kib(path: &Path) -> Option<u64> {
    let pid = worker_pid(path)?;
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
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
async fn exercise_recovery(
    controller: &analysis::Controller,
    store: &Arc<Mutex<Store>>,
    worker: &Arc<AsyncMutex<Worker>>,
    root: &Path,
    executable: &Path,
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
        return json!({"recovered":false,"cancelled_status":cancelled_status,"error":"crash sample never reached inference IPC phase"});
    }
    let Some(pid) = worker_pid(executable) else {
        controller.cancel(store, crashed.id).unwrap();
        settle(controller).await;
        return json!({"recovered":false,"error":"could not identify this test-specific child PID"});
    };
    // Only the child copied into this test's unique directory is deliberately killed.
    let killed = std::process::Command::new("/bin/kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .unwrap()
        .success();
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
    json!({"interruption_boundary":"parent inference IPC phase; not proof of an active model kernel",
        "cancelled_status":cancelled_status,"prior_result_preserved":before == after_cancel,
        "child_killed":killed,"crashed_status":crashed_status,"recovery_status":recovered_status,
        "recovered":cancelled_status == "cancelled" && before == after_cancel && killed && crashed_status == "failed" && recovered_status == "completed"})
}

/// Same artifact/document/configuration, cold model then forced warm regeneration and cache.
/// Run both this parent test binary and the worker with --release for release comparisons.
#[tokio::test]
#[ignore = "real installed model and worker required; local bounded latency benchmark"]
async fn actual_analysis_latency_benchmark() {
    let executable = PathBuf::from(std::env::var_os("RHYTHM_WORKER_TEST_BIN").expect("worker"));
    let root = PathBuf::from(std::env::var_os("RHYTHM_MODEL_EVAL_ROOT").expect("model root"));
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-2b".into());
    let model = installed_model(&root, &id);
    let directory = tempfile::tempdir().unwrap();
    let local_worker = directory.path().join("rhythm-benchmark-worker");
    std::fs::copy(&executable, &local_worker).unwrap();
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
    let worker = Arc::new(AsyncMutex::new(Worker::launch(&local_worker)));
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
            "job":job,"diagnostics":controller.diagnostics(job_id),"worker_rss_kib":worker_rss_kib(&local_worker)});
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
    let executable = PathBuf::from(
        std::env::var_os("RHYTHM_WORKER_TEST_BIN").expect("set RHYTHM_WORKER_TEST_BIN"),
    );
    let root = PathBuf::from(
        std::env::var_os("RHYTHM_MODEL_EVAL_ROOT")
            .expect("set RHYTHM_MODEL_EVAL_ROOT to completed model install"),
    );
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-4b".into());
    let model = installed_model(&root, &id);
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("evaluation.sqlite3");
    let local_worker = directory.path().join(format!(
        "rhythm-evaluation-worker{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(&executable, &local_worker).unwrap();
    let store = Store::open(&db).unwrap();
    seed(&store);
    configure(&store, &model);
    let store = Arc::new(Mutex::new(store));
    let worker = Arc::new(AsyncMutex::new(Worker::launch(&local_worker)));
    let controller = analysis::Controller::default();
    let mut report = json!({
        "model":model,"worker":executable,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "backend_note":std::env::var("RHYTHM_AI_EVAL_BACKEND").unwrap_or_else(|_| "unreported; inspect worker build".into()),
        "fixture_provenance":"synthetic Korean notices, not collected university notices",
        "prompt_version":analysis::PROMPT_VERSION,"schema_version":analysis::SCHEMA_VERSION,
        "decoding_mode":analysis::decoding_mode(),
        "max_tokens":1024,"samples":[],"native_ui_verified":false,
        "network_note":"test constructs no HTTP client; this alone is not OS-enforced network denial"
    });
    let report_file = report_path(&id);
    let load_started = Instant::now();
    let loaded = worker.lock().await.load(Path::new(&model.path)).await;
    report["load_ms"] = json!(load_started.elapsed().as_millis());
    report["post_load_worker_rss_kib"] = json!(worker_rss_kib(&local_worker));
    if let Err(error) = loaded {
        report["load_error"] = json!(error);
        write_report(&report_file, &report);
        worker.lock().await.shutdown().await;
        panic!("actual Qwen3.5 load failed; see {}", report_file.display());
    }
    let limit = std::env::var("RHYTHM_AI_EVAL_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 18);
    let filter = std::env::var("RHYTHM_AI_EVAL_CASES").ok();
    let mut failures = Vec::new();
    let mut completed = Vec::new();
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
            "worker_rss_kib":worker_rss_kib(&local_worker)});
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
                failures.push(format!("{}: quality assertion failed", case.id));
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
    if failures.is_empty() && std::env::var("RHYTHM_AI_EVAL_RECOVERY").as_deref() == Ok("1") {
        if let Some((document_id, revision_id, _)) = completed.first() {
            let recovery = exercise_recovery(
                &controller,
                &store,
                &worker,
                directory.path(),
                &local_worker,
                *document_id,
                *revision_id,
            )
            .await;
            if recovery["recovered"] != true {
                failures.push("real-worker recovery failed".into());
            }
            report["recovery"] = recovery;
        }
    }
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
        assert!(cached.job.is_none());
        assert_eq!(cached.cached.unwrap().id, *result_id);
        assert_eq!(worker.lock().await.status().state, "unavailable");
    }
    report["reopened_cache_reads_with_worker_stopped"] = json!(completed.len());
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
