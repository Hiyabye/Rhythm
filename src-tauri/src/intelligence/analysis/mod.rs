//! Local-only, revision-bound notice analysis. Collection never calls this module.
use super::{
    valid_execution_mode, InferenceMetrics, Worker, MAX_IMAGES, MAX_IMAGE_BYTES, MAX_PROMPT_BYTES,
};
use crate::storage::{
    intelligence::{AiJob, AiModel, AiResult, AnalysisKey},
    Store,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

mod dates;
mod integrity;
mod source;
mod validate;
use dates::*;
use integrity::*;
use source::*;
#[cfg(test)]
pub use validate::validate_output;
use validate::*;

pub const PROMPT_VERSION: u32 = 5;
pub const SCHEMA_VERSION: u32 = 3;
/// Cybercampus documents are the user's own course data (#86, #77).
pub const PRIVATE_REMOTE: &str =
    "사이버캠퍼스 자료는 원격 AI로 보내지 않습니다. 로컬 엔진으로 분석해 주세요.";
const CONSTRAINED_OUTPUT_DEFAULT: bool = true;
pub(super) fn decoding_mode() -> &'static str {
    #[cfg(test)]
    match std::env::var("RHYTHM_AI_EVAL_DECODING").as_deref() {
        Ok("analysis_json_v1") => return "analysis_json_v1",
        Ok("unconstrained") => return "unconstrained",
        Err(_) => (),
        Ok(_) => panic!("RHYTHM_AI_EVAL_DECODING must be unconstrained or analysis_json_v1"),
    }
    if CONSTRAINED_OUTPUT_DEFAULT {
        "analysis_json_v1"
    } else {
        "unconstrained"
    }
}
pub const CATEGORIES: [&str; 8] = [
    "학사",
    "장학",
    "수업·교육",
    "연구",
    "공모전·대회",
    "인턴·채용",
    "행사·활동",
    "기타",
];
/// Closed generation contract for constrained decoding. It must match the
/// compact generation ingress below; parent-side validation and human
/// semantic review are not replaced by it.
///
/// Kept as literal JSON because property order is part of the contract:
/// llama.cpp's grammar emits properties in schema order, and this order lets
/// the model summarize before it extracts facts, as the prompt describes.
/// A `serde_json::Value` would sort the keys.
pub(super) const GENERATION_SCHEMA: &str = r#"{"type":"object","properties":{"short_summary":{"type":"string","minLength":4,"maxLength":180},"summary":{"type":"string","minLength":4,"maxLength":1600},"uncertainty":{"type":"string","enum":["clear","ambiguous","unknown"]},"facts":{"type":"array","maxItems":6,"items":{"type":"object","properties":{"kind":{"type":"string","enum":["deadline","event_date","eligibility","documents","application_method","link","action"]},"label":{"type":"string","minLength":1,"maxLength":40},"value":{"type":"string","minLength":1,"maxLength":500},"evidence_id":{"type":"integer","minimum":0}},"required":["kind","label","value","evidence_id"],"additionalProperties":false}}},"required":["short_summary","summary","uncertainty","facts"],"additionalProperties":false}"#;
const INPUT_BYTES: usize = MAX_PROMPT_BYTES;
const MAX_IMAGE_FACTS: usize = 4;
/// Evidence label for an image fact the model did not attribute to one image.
const IMAGE_EVIDENCE: &str = "[이미지]";
const SOURCE_BYTES: usize = 18 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;
pub(crate) const CANCELLED: &str = "AI 분석을 취소했습니다.";
const VALIDATION_FAILED: &str =
    "AI 결과가 공지 원문과 맞지 않아 저장하지 않았습니다. 다시 요청해 주세요.";
type SharedStore = Arc<Mutex<Store>>;
type SharedWorker = Arc<AsyncMutex<Worker>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub short_summary: String,
    pub summary: String,
    /// A qualitative assessment, not a calibrated probability.
    pub uncertainty: String,
    pub facts: Vec<Fact>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub kind: String,
    pub label: String,
    pub value: String,
    pub evidence: String,
    /// `text` for an exact archived text passage, `image` for an archived raster.
    /// Image evidence is not a literal quotation: pixels cannot be substring-checked.
    #[serde(default = "text_origin")]
    pub origin: String,
    /// ISO calendar date only when the evidence explicitly contains year/month/day.
    pub date: Option<String>,
    pub uncertainty: String,
}
fn text_origin() -> String {
    "text".into()
}
#[derive(Clone, Serialize)]
pub struct StartResponse {
    pub cached: Option<AiResult>,
    pub job: Option<AiJob>,
    /// Admission wall time, including store access and cache lookup, not inference.
    pub admission_ms: u64,
    /// Exact cache-query wall time; absent when regeneration skips the lookup.
    pub cache_lookup_ms: Option<u64>,
}

/// Ephemeral local measurements, not persisted output or source-derived text.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostics {
    pub job_id: i64,
    pub phase: String,
    pub elapsed_ms: u64,
    pub stages_ms: BTreeMap<String, u64>,
    /// One entry per attempted inference. Missing worker metrics remain unknown.
    pub attempts: Vec<InferenceMetrics>,
    /// Fixed validator messages (at most one per attempt); they quote no source text.
    pub validation_errors: Vec<String>,
}
struct ActiveDiagnostics {
    started: Instant,
    report: Diagnostics,
}
#[derive(Default)]
struct Profiles {
    active: Option<ActiveDiagnostics>,
    completed: VecDeque<Diagnostics>,
}
const DIAGNOSTICS_HISTORY: usize = 20;

fn millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

/// Record elapsed time even when a stage returns early with an error/cancellation.
struct Stage<'a> {
    controller: Option<&'a Controller>,
    job_id: i64,
    name: &'static str,
    started: Instant,
}
impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if let Some(controller) = self.controller {
            controller.record_stage(self.job_id, self.name, millis(self.started));
        }
    }
}
fn stage<'a>(
    controller: Option<&'a Controller>,
    job_id: i64,
    phase: &'static str,
    name: &'static str,
) -> Stage<'a> {
    if let Some(controller) = controller {
        controller.phase(job_id, phase);
    }
    Stage {
        controller,
        job_id,
        name,
        started: Instant::now(),
    }
}

#[derive(Default)]
pub(crate) struct Cancellation {
    flag: AtomicBool,
    notify: Notify,
}
impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.flag.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.flag.load(Ordering::SeqCst) {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
}
struct ActiveJob {
    id: i64,
    priority: i32,
    cancellation: Arc<Cancellation>,
}
#[derive(Default)]
struct State {
    running: bool,
    stopped: bool,
    active: Option<ActiveJob>,
}
#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    profiles: Mutex<Profiles>,
    idle: Notify,
}
#[derive(Clone, Default)]
pub struct Controller {
    inner: Arc<Inner>,
}

fn with_store<T>(
    store: &SharedStore,
    operation: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    operation(&mut *store.lock().map_err(|_| "저장소에 접근하지 못했습니다.")?)
}

/// Explicit/user-requested analysis. The queue runs this before any backlog work.
pub const DIRECT_PRIORITY: i32 = 100;
/// Opt-in idle backlog work. It must never run ahead of a direct request.
pub const BACKLOG_PRIORITY: i32 = 10;

impl Controller {
    /// Explicit admission only. A valid cache hit never starts the worker or verifies weights.
    /// `regenerate` creates a new attempt without deleting earlier successful results.
    #[cfg(test)]
    pub fn start(
        &self,
        store: SharedStore,
        worker: SharedWorker,
        data_dir: PathBuf,
        document_id: i64,
        revision_id: i64,
        regenerate: bool,
    ) -> Result<StartResponse, String> {
        self.start_with_lease(
            store,
            worker,
            data_dir,
            document_id,
            revision_id,
            regenerate,
            DIRECT_PRIORITY,
            None,
            None,
        )
    }
    /// `priority` separates direct requests from opt-in idle backlog work.
    #[allow(clippy::too_many_arguments)]
    pub fn start_admitted(
        &self,
        store: SharedStore,
        worker: SharedWorker,
        data_dir: PathBuf,
        document_id: i64,
        revision_id: i64,
        regenerate: bool,
        priority: i32,
        lease: crate::operation_gate::AdmissionLease,
        remote: Option<std::sync::Arc<super::remote::Remote>>,
    ) -> Result<StartResponse, String> {
        self.start_with_lease(
            store,
            worker,
            data_dir,
            document_id,
            revision_id,
            regenerate,
            priority,
            Some(lease),
            remote,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn start_with_lease(
        &self,
        store: SharedStore,
        worker: SharedWorker,
        data_dir: PathBuf,
        document_id: i64,
        revision_id: i64,
        regenerate: bool,
        priority: i32,
        lease: Option<crate::operation_gate::AdmissionLease>,
        remote: Option<std::sync::Arc<super::remote::Remote>>,
    ) -> Result<StartResponse, String> {
        let admission_started = Instant::now();
        // Admission and the queue's empty-to-idle transition use this same lock.
        // Lock order is always controller state, then Store; neither crosses an await.
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "AI 작업 상태에 접근하지 못했습니다.")?;
        if state.stopped {
            return Err("앱이 종료 중이어서 AI 분석을 시작할 수 없습니다.".into());
        }
        let mut response = with_store(&store, |store| {
            let document = store.document(document_id, Some(revision_id))?;
            let preferences = store.ai_preferences()?;
            // Both engines share the queue and the semantic cache. Their keys
            // differ, so switching engines never overwrites the other's results.
            let (key, identity) = if preferences.engine == "openai-compatible" {
                if document.item.source_id == crate::storage::intelligence::PRIVATE_SOURCE {
                    return Err(PRIVATE_REMOTE.into());
                }
                if !preferences.remote_consent {
                    return Err("원격 AI 사용에 먼저 동의해 주세요.".into());
                }
                let config = super::remote::GenerationConfig::from_preferences(&preferences)?;
                let key =
                    key_for_remote(document_id, revision_id, &config, preferences.max_tokens)?;
                let identity = json!({
                    "engine": "openai-compatible",
                    "name": config.model,
                    "origin": config.endpoint.origin(),
                    "host": config.endpoint.host(),
                    "base": config.endpoint.base(),
                    "max_tokens": preferences.max_tokens,
                    "timeout_seconds": config.timeout_seconds,
                    "runtime_policy": super::remote::ENGINE_ID,
                    "images_used": 0,
                    "images_available": 0
                });
                (key, identity)
            } else {
                let id = preferences
                    .active_model_id
                    .ok_or("AI 설정에서 사용할 모델을 선택해 주세요.")?;
                let model = store
                    .ai_model(&id)?
                    .filter(|model| model.status == "installed")
                    .ok_or("설치가 완료된 AI 모델이 없습니다.")?;
                let key = key_for_mode(
                    document_id,
                    revision_id,
                    &model,
                    preferences.max_tokens,
                    &preferences.execution_mode,
                )?;
                let identity = json!({"engine":"local","id":model.id,"name":model.name,"sha256":model.sha256,
                    "catalog_id":model.catalog_id,"source_revision":model.source_revision,
                    "source_filename":model.source_filename,"max_tokens":preferences.max_tokens,
                    "execution_mode":preferences.execution_mode,"runtime_policy":"llama-server-v1"});
                (key, identity)
            };
            let mut cache_lookup_ms = None;
            if !regenerate {
                let cache_started = Instant::now();
                let cached = store.ai_cached_result(&key)?;
                cache_lookup_ms = Some(millis(cache_started));
                if let Some(cached) = cached {
                    return Ok(StartResponse {
                        cached: Some(cached),
                        job: None,
                        admission_ms: 0,
                        cache_lookup_ms,
                    });
                }
            }
            let job = store.ai_enqueue(&key, &identity, priority)?;
            Ok(StartResponse {
                cached: None,
                job: Some(job),
                admission_ms: 0,
                cache_lookup_ms,
            })
        })?;
        if response.job.is_some() && !state.running {
            state.running = true;
            let controller = self.clone();
            tokio::spawn(async move {
                let _lease = lease;
                controller.run(store, worker, data_dir, remote).await;
            });
        }
        response.admission_ms = millis(admission_started);
        Ok(response)
    }

    /// Cancels the persisted attempt first, so late output cannot be committed.
    pub fn cancel(&self, store: &SharedStore, id: i64) -> Result<(), String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "AI 작업 상태에 접근하지 못했습니다.")?;
        with_store(store, |store| {
            let job = store.ai_job(id)?.ok_or("AI 작업을 찾을 수 없습니다.")?;
            if job.status == "cancelled" {
                return Ok(());
            }
            store.ai_finish_job(id, "cancelled", Some(CANCELLED), None)
        })?;
        if let Some(active) = state.active.as_ref().filter(|active| active.id == id) {
            active.cancellation.cancel();
        }
        Ok(())
    }

    /// Opt-in backlog work yields to explicit user actions. Cancels only a running
    /// backlog attempt and waits for it to finish, so the caller can reuse or stop
    /// the worker without abandoning a user-requested analysis.
    pub async fn cancel_automatic(&self, store: &SharedStore) -> Result<bool, String> {
        let target = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| "AI 작업 상태에 접근하지 못했습니다.")?;
            state
                .active
                .as_ref()
                .filter(|active| active.priority < DIRECT_PRIORITY)
                .map(|active| (active.id, active.cancellation.clone()))
        };
        let Some((id, cancellation)) = target else {
            return Ok(false);
        };
        let cancelled = with_store(store, |store| store.ai_cancel_if_running(id, CANCELLED))?;
        // A concurrent completion is harmless: the flag still stops any late work
        // and waiting settles the queue before the caller reuses the worker.
        cancellation.cancel();
        self.wait().await;
        Ok(cancelled)
    }

    /// Stops admission and cancels only the active operation. Pending jobs remain recoverable.
    pub fn stop(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.stopped = true;
            if let Some(active) = &state.active {
                active.cancellation.cancel();
            }
        }
    }
    pub fn resume(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.stopped = false;
        }
    }
    pub async fn wait(&self) {
        loop {
            let notified = self.inner.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self
                .inner
                .state
                .lock()
                .map(|state| state.running)
                .unwrap_or(false)
            {
                return;
            }
            notified.await;
        }
    }
    pub fn running(&self) -> bool {
        self.inner
            .state
            .lock()
            .map(|state| state.running)
            .unwrap_or(false)
    }

    /// Active report only. This does not wait for SQLite or the worker mutex.
    pub fn progress(&self) -> Option<Diagnostics> {
        let profiles = self.inner.profiles.lock().ok()?;
        let active = profiles.active.as_ref()?;
        let mut report = active.report.clone();
        report.elapsed_ms = millis(active.started);
        Some(report)
    }

    /// Bounded terminal diagnostics are observations, never active-job authority.
    pub fn diagnostics(&self, job_id: i64) -> Option<Diagnostics> {
        let profiles = self.inner.profiles.lock().ok()?;
        if let Some(active) = profiles
            .active
            .as_ref()
            .filter(|p| p.report.job_id == job_id)
        {
            let mut report = active.report.clone();
            report.elapsed_ms = millis(active.started);
            return Some(report);
        }
        profiles
            .completed
            .iter()
            .find(|p| p.job_id == job_id)
            .cloned()
    }

    fn begin_profile(&self, job_id: i64) {
        if let Ok(mut profiles) = self.inner.profiles.lock() {
            profiles.active = Some(ActiveDiagnostics {
                started: Instant::now(),
                report: Diagnostics {
                    job_id,
                    phase: "preparing".into(),
                    elapsed_ms: 0,
                    stages_ms: BTreeMap::new(),
                    attempts: Vec::new(),
                    validation_errors: Vec::new(),
                },
            });
        }
    }
    fn update_profile(&self, job_id: i64, update: impl FnOnce(&mut Diagnostics)) {
        if let Ok(mut profiles) = self.inner.profiles.lock() {
            if let Some(active) = profiles
                .active
                .as_mut()
                .filter(|p| p.report.job_id == job_id)
            {
                update(&mut active.report);
            }
        }
    }
    fn phase(&self, job_id: i64, phase: &str) {
        self.update_profile(job_id, |report| report.phase = phase.into());
    }
    fn record_stage(&self, job_id: i64, name: &str, elapsed_ms: u64) {
        self.update_profile(job_id, |report| {
            *report.stages_ms.entry(name.into()).or_default() += elapsed_ms;
        });
    }
    fn begin_attempt(&self, job_id: i64) {
        self.update_profile(job_id, |report| {
            report.attempts.push(InferenceMetrics::default())
        });
    }
    fn record_attempt(&self, job_id: i64, metrics: InferenceMetrics) {
        self.update_profile(job_id, |report| {
            if let Some(attempt) = report.attempts.last_mut() {
                *attempt = metrics;
            }
        });
    }
    fn record_validation_error(&self, job_id: i64, error: &str) {
        self.update_profile(job_id, |report| report.validation_errors.push(error.into()));
    }
    fn finish_profile(&self, job_id: i64, phase: &str) {
        if let Ok(mut profiles) = self.inner.profiles.lock() {
            if profiles
                .active
                .as_ref()
                .is_some_and(|p| p.report.job_id == job_id)
            {
                let mut active = profiles.active.take().expect("matching active profile");
                active.report.phase = phase.into();
                active.report.elapsed_ms = millis(active.started);
                profiles.completed.push_front(active.report);
                profiles.completed.truncate(DIAGNOSTICS_HISTORY);
            }
        }
    }

    async fn run(
        &self,
        store: SharedStore,
        worker: SharedWorker,
        data_dir: PathBuf,
        remote: Option<std::sync::Arc<super::remote::Remote>>,
    ) {
        loop {
            let next = {
                let mut state = match self.inner.state.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                let next = if state.stopped {
                    Ok(None)
                } else {
                    with_store(&store, Store::ai_claim_next)
                };
                match next {
                    Ok(Some(job)) => {
                        let cancellation = Arc::new(Cancellation::default());
                        state.active = Some(ActiveJob {
                            id: job.id,
                            priority: job.priority,
                            cancellation: cancellation.clone(),
                        });
                        self.begin_profile(job.id);
                        Some((job, cancellation))
                    }
                    idle => {
                        if let Err(error) = idle {
                            eprintln!("AI queue: {error}");
                        }
                        state.active = None;
                        state.running = false;
                        self.inner.idle.notify_waiters();
                        None
                    }
                }
            };
            let Some((job, cancellation)) = next else {
                return;
            };
            let started = Instant::now();
            let outcome = execute(
                &store,
                &worker,
                &data_dir,
                &job,
                &cancellation,
                Some(self),
                remote.clone(),
            )
            .await;
            let duration = millis(started);
            // Serialize final commit with cancellation/admission. SQLite also rejects
            // completion of an already-cancelled attempt within its own transaction.
            if let Ok(mut state) = self.inner.state.lock() {
                let persistence = stage(Some(self), job.id, "persisting", "persistence");
                let mut terminal_phase = "failed".to_owned();
                let saved = with_store(&store, |store| {
                    let Some(current) = store.ai_job(job.id)? else {
                        return Ok(());
                    };
                    if current.status != "running" {
                        terminal_phase = current.status;
                        return Ok(());
                    }
                    if cancellation.flag.load(Ordering::SeqCst) {
                        terminal_phase = "cancelled".into();
                        store.ai_finish_job(job.id, "cancelled", Some(CANCELLED), Some(duration))
                    } else {
                        match outcome {
                            Ok(output) => {
                                let value = serde_json::to_value(output.analysis)
                                    .map_err(|e| e.to_string())?;
                                store.ai_complete_job_with_backend(
                                    job.id,
                                    &value,
                                    output.partial,
                                    output.processed_chars,
                                    output.total_chars,
                                    duration,
                                    output.loaded_backend.as_deref(),
                                    Some((
                                        output.images_used as u64,
                                        output.images_available as u64,
                                    )),
                                )?;
                                terminal_phase = "completed".into();
                                Ok(())
                            }
                            Err(error) => {
                                store.ai_finish_job(job.id, "failed", Some(&error), Some(duration))
                            }
                        }
                    }
                });
                if let Err(error) = saved {
                    terminal_phase = "failed".into();
                    eprintln!("AI result storage: {error}");
                    // A failed result transaction must not strand an otherwise
                    // writable job in `running` until the next application launch.
                    let _ = with_store(&store, |store| {
                        store.ai_finish_job(
                            job.id,
                            "failed",
                            Some("분석 결과를 저장하지 못했습니다. 다시 시도해 주세요."),
                            Some(duration),
                        )
                    });
                }
                drop(persistence);
                self.finish_profile(job.id, &terminal_phase);
                state.active = None;
            }
        }
    }
}

#[cfg(test)]
pub fn key_for(
    document_id: i64,
    revision_id: i64,
    model: &AiModel,
    max_tokens: u32,
) -> Result<AnalysisKey, String> {
    key_for_mode(document_id, revision_id, model, max_tokens, "auto")
}

pub fn key_for_mode(
    document_id: i64,
    revision_id: i64,
    model: &AiModel,
    max_tokens: u32,
    execution_mode: &str,
) -> Result<AnalysisKey, String> {
    if !valid_execution_mode(execution_mode) {
        return Err("AI 실행 장치를 확인해 주세요.".into());
    }
    let hash = model
        .sha256
        .clone()
        .ok_or("모델 무결성 검증이 필요합니다.")?;
    Ok(AnalysisKey {
        document_id,
        revision_id,
        task: "notice_analysis".into(),
        model_sha256: hash,
        prompt_version: PROMPT_VERSION,
        schema_version: SCHEMA_VERSION,
        configuration_hash: configuration_hash(max_tokens, decoding_mode(), execution_mode),
    })
}
/// Remote generation has no artifact hash. Its identity is derived from the
/// normalized origin and model name, and the configuration hash records the
/// engine, base URL and timeout so local and remote results never collide.
pub fn key_for_remote(
    document_id: i64,
    revision_id: i64,
    config: &super::remote::GenerationConfig,
    max_tokens: u32,
) -> Result<AnalysisKey, String> {
    Ok(AnalysisKey {
        document_id,
        revision_id,
        task: "notice_analysis".into(),
        model_sha256: super::remote::identity_hash(config.endpoint.origin(), &config.model),
        prompt_version: PROMPT_VERSION,
        schema_version: SCHEMA_VERSION,
        configuration_hash: remote_configuration_hash(
            config.endpoint.origin(),
            &config.model,
            config.timeout_seconds,
            max_tokens,
        ),
    })
}

fn remote_configuration_hash(
    origin: &str,
    model: &str,
    timeout_seconds: u32,
    max_tokens: u32,
) -> String {
    let configuration = format!("analysis-v5;selector=head-tail-keywords-v4;evidence_encoding=explicit-id-plus-image-v1;clock_rules=meridiem-v2;decoding=analysis_json_v1;max_tokens={max_tokens};source_bytes={SOURCE_BYTES};thinking=false;sampler=deterministic;repair_attempts=1;engine=openai-compatible-v1;vision=none;image_source=none;base={origin};model={model};timeout_seconds={timeout_seconds}");
    format!("{:x}", Sha256::digest(configuration.as_bytes()))
}

/// `engine=llama-server-v1` separates results of the supervised llama-server
/// from earlier mistral.rs/worker results, which stay readable as outdated.
/// Windows also keys each execution mode: its Vulkan and CPU builds use
/// different context sizes. `auto` admits a result from either build, and each
/// result records and displays the device that produced it. Elsewhere one
/// build serves every mode, so the mode is not part of the identity.
fn configuration_hash(max_tokens: u32, decoding: &str, execution_mode: &str) -> String {
    let base = format!("analysis-v5;selector=head-tail-keywords-v4;evidence_encoding=explicit-id-plus-image-v1;clock_rules=meridiem-v2;decoding={decoding};max_tokens={max_tokens};source_bytes={SOURCE_BYTES};thinking=false;sampler=deterministic;repair_attempts=1;engine=llama-server-v1;vision=mmproj-v1;image_source=archived-local-v1");
    let configuration = if cfg!(windows) {
        format!("{base};execution_mode={execution_mode}")
    } else {
        let _ = execution_mode;
        base
    };
    format!("{:x}", Sha256::digest(configuration.as_bytes()))
}

struct Output {
    analysis: Analysis,
    partial: bool,
    processed_chars: u64,
    total_chars: u64,
    loaded_backend: Option<String>,
    images_used: u32,
    images_available: u32,
}
async fn execute(
    store: &SharedStore,
    worker: &SharedWorker,
    data_dir: &Path,
    job: &AiJob,
    cancellation: &Arc<Cancellation>,
    controller: Option<&Controller>,
    remote: Option<Arc<super::remote::Remote>>,
) -> Result<Output, String> {
    cancellation.check()?;
    let remote_job = job.model_identity["engine"].as_str() == Some("openai-compatible");
    let max_tokens = job.model_identity["max_tokens"]
        .as_u64()
        .filter(|value| (128..=4096).contains(value))
        .ok_or("작업의 출력 설정이 올바르지 않습니다.")? as u32;
    let document_stage = stage(controller, job.id, "preparing", "document_model_read");
    let (document, model, preferences) = with_store(store, |store| {
        let document = store.document(job.key.document_id, Some(job.key.revision_id))?;
        let preferences = store.ai_preferences()?;
        let model = if remote_job {
            None
        } else {
            let id = job.model_identity["id"]
                .as_str()
                .ok_or("작업의 모델 정보가 없습니다.")?;
            let model = store
                .ai_model(id)?
                .filter(|model| model.status == "installed")
                .ok_or("분석에 필요한 모델이 설치되어 있지 않습니다.")?;
            if model.sha256.as_deref() != Some(job.key.model_sha256.as_str()) {
                return Err("모델 파일이 변경되었습니다. 다시 분석해 주세요.".into());
            }
            Some(model)
        };
        Ok((document, model, preferences))
    })?;
    drop(document_stage);
    // Pending attempts can outlive an application upgrade. Never run the current
    // prompt/configuration while recording an older semantic cache identity.
    // Checked again at execution: a queued job must not reach the remote engine
    // for a private source even if the admission check was bypassed.
    if remote_job && document.item.source_id == crate::storage::intelligence::PRIVATE_SOURCE {
        return Err(PRIVATE_REMOTE.into());
    }
    let remote_config = if remote_job {
        let config =
            super::remote::GenerationConfig::from_preferences(&preferences).map_err(|_| {
                "이전 버전의 분석 설정입니다. 현재 설정으로 다시 요청해 주세요.".to_string()
            })?;
        if !preferences.remote_consent {
            return Err("원격 AI 사용에 먼저 동의해 주세요.".into());
        }
        if job.key
            != key_for_remote(
                job.key.document_id,
                job.key.revision_id,
                &config,
                max_tokens,
            )?
        {
            return Err("이전 버전의 분석 설정입니다. 현재 설정으로 다시 요청해 주세요.".into());
        }
        Some(config)
    } else {
        let execution_mode = job.model_identity["execution_mode"]
            .as_str()
            .unwrap_or("auto");
        let model = model
            .as_ref()
            .ok_or("분석에 필요한 모델 정보가 없습니다.")?;
        if execution_mode != preferences.execution_mode
            || job.key
                != key_for_mode(
                    job.key.document_id,
                    job.key.revision_id,
                    model,
                    max_tokens,
                    execution_mode,
                )?
        {
            return Err("이전 버전의 분석 설정입니다. 현재 설정으로 다시 요청해 주세요.".into());
        }
        None
    };
    let source = {
        let _stage = stage(controller, job.id, "preparing", "source_selection");
        bounded_source(&document.item.title, &document.body_text)
    };
    let processed_chars = source.processed_chars;
    let total_chars = source.total_chars;
    let passages = source.passages();
    let (path, projector_path, images, images_available) = if let Some(model) = &model {
        let root = data_dir.to_owned();
        let token = cancellation.clone();
        let integrity_model = model.clone();
        let path = {
            let _stage = stage(controller, job.id, "integrity", "integrity");
            tokio::task::spawn_blocking(move || verified_path(&root, &integrity_model, &token))
                .await
                .map_err(|_| "모델 무결성 검사 작업이 중단되었습니다.")??
        };
        cancellation.check()?;
        // Vision is opt-in per model: without an installed projector the archived
        // images stay unused and the result reports that count.
        let projector = with_store(store, |store| store.ai_projector(&model.id))?
            .filter(|projector| projector.status == "installed");
        let projector_path = match &projector {
            Some(projector) => {
                let root = data_dir.to_owned();
                let record = projector.clone();
                let token = cancellation.clone();
                let owner = model.clone();
                let path = tokio::task::spawn_blocking(move || {
                    verified_projector_path(&root, &owner, &record, &token)
                })
                .await
                .map_err(|_| "이미지 인식 모듈 무결성 검사 작업이 중단되었습니다.")??;
                Some(path)
            }
            None => None,
        };
        let (images, images_available) = {
            let _stage = stage(controller, job.id, "preparing", "image_selection");
            archived_images(
                store,
                data_dir,
                &document.resources,
                projector_path.is_some(),
                cancellation,
            )
            .await?
        };
        (path, projector_path, images, images_available)
    } else {
        // Remote analysis is text-only: the user-entered provider receives the
        // same text prompt, never archived image bytes.
        (PathBuf::new(), None, Vec::new(), 0)
    };
    let vision = images
        .iter()
        .map(|image| super::VisionImage {
            mime: image.mime,
            bytes: &image.bytes,
        })
        .collect::<Vec<_>>();
    let images_used = images.len().min(u32::MAX as usize) as u32;
    let prompt = {
        let _stage = stage(controller, job.id, "preparing", "prompt_construction");
        prompt(&passages, &images, processed_chars != total_chars)
    };
    if prompt.len() > INPUT_BYTES {
        return Err("분석 입력이 허용 범위를 벗어났습니다.".into());
    }
    let mut request = prompt;
    #[cfg(test)]
    trace_request(job.id, &request);
    let mut validated = None;
    let mut loaded_backend = None;
    if let Some(config) = &remote_config {
        let remote = remote
            .as_ref()
            .ok_or("원격 AI 설정을 불러오지 못했습니다.")?;
        let origin = config.endpoint.origin().to_owned();
        let key_owner = remote.clone();
        let key = tokio::task::spawn_blocking(move || key_owner.key(&origin))
            .await
            .map_err(|_| "API 키를 확인하지 못했습니다.".to_string())??
            .ok_or("원격 AI API 키를 저장해 주세요.".to_string())?;
        for attempt in 1..=2 {
            if let Some(controller) = controller {
                controller.begin_attempt(job.id);
            }
            let inference_stage = stage(controller, job.id, "inference", "inference");
            let generated = tokio::select! {
                response = async {
                    let started = Instant::now();
                    remote
                        .complete(config, &key, &request, max_tokens, cancellation)
                        .await
                        .map(|completion| super::InferenceResponse {
                            text: completion.text,
                            metrics: super::InferenceMetrics {
                                generation_ms: Some(started.elapsed().as_millis() as u64),
                                usage: completion.usage,
                                ..Default::default()
                            },
                        })
                } => Some(response),
                _ = cancellation.cancelled() => None,
            };
            drop(inference_stage);
            let Some(generated) = generated else {
                return Err(CANCELLED.into());
            };
            cancellation.check()?;
            let generated = generated?;
            if let Some(controller) = controller {
                controller.record_attempt(job.id, generated.metrics);
            }
            #[cfg(test)]
            trace_generation(job.id, attempt, &generated.text);
            match validate_attempt(
                controller,
                job.id,
                attempt,
                &generated.text,
                &source,
                &passages,
                &images,
            )? {
                Attempt::Validated(analysis) => {
                    validated = Some(analysis);
                    break;
                }
                Attempt::Repair(error) => {
                    request = repair_request(&request, &error)?;
                }
            }
        }
    } else {
        let worker_wait = stage(controller, job.id, "waiting", "worker_lock_wait");
        let mut engine = tokio::select! {
            guard = worker.lock() => guard,
            _ = cancellation.cancelled() => return Err(CANCELLED.into()),
        };
        drop(worker_wait);
        cancellation.check()?;
        for attempt in 1..=2 {
            if let Some(controller) = controller {
                controller.begin_attempt(job.id);
            }
            let inference_stage = stage(controller, job.id, "inference", "inference");
            let generated = tokio::select! {
                response = async {
                    if decoding_mode() == "analysis_json_v1" {
                        engine
                            .infer_analysis(
                                &path,
                                projector_path.as_deref(),
                                &request,
                                &vision,
                                max_tokens,
                            )
                            .await
                    } else {
                        engine.infer_detailed(&path, &request, max_tokens).await
                    }
                } => Some(response),
                _ = cancellation.cancelled() => None,
            };
            drop(inference_stage);
            let Some(generated) = generated else {
                // A dropped future leaves an outstanding response on serial stdout.
                // Reap before anybody can use the worker again.
                let _stage = stage(controller, job.id, "cancelling", "worker_stop");
                engine.stop().await;
                return Err(CANCELLED.into());
            };
            cancellation.check()?;
            let generated = generated?;
            loaded_backend =
                generated.metrics.loaded_backend.clone().filter(|backend| {
                    ["cpu", "vulkan", "cuda", "metal"].contains(&backend.as_str())
                });
            if let Some(controller) = controller {
                controller.record_attempt(job.id, generated.metrics);
            }
            #[cfg(test)]
            trace_generation(job.id, attempt, &generated.text);
            match validate_attempt(
                controller,
                job.id,
                attempt,
                &generated.text,
                &source,
                &passages,
                &images,
            )? {
                Attempt::Validated(analysis) => {
                    validated = Some(analysis);
                    break;
                }
                Attempt::Repair(error) => {
                    request = repair_request(&request, &error)?;
                }
            }
        }
        drop(engine);
    }
    let analysis = validated.ok_or("AI 결과를 검증하지 못했습니다.")?;
    Ok(Output {
        analysis,
        partial: processed_chars != total_chars,
        processed_chars,
        total_chars,
        loaded_backend,
        images_used,
        images_available,
    })
}

enum Attempt {
    Validated(Analysis),
    /// The validator message is a repair instruction for the next request.
    Repair(String),
}

fn validate_attempt(
    controller: Option<&Controller>,
    job_id: i64,
    attempt: usize,
    text: &str,
    source: &SelectedSource,
    passages: &[&str],
    images: &[SourceImage],
) -> Result<Attempt, String> {
    let validation = {
        let _stage = stage(controller, job_id, "validating", "validation");
        validate_generation(text, &source.text, passages, images)
    };
    match validation {
        Ok(analysis) => Ok(Attempt::Validated(analysis)),
        Err(error) => {
            // Validator messages are instructions to the model; they stay in
            // diagnostics and never become the job error.
            if let Some(controller) = controller {
                controller.record_validation_error(job_id, &error);
            }
            if attempt == 2 {
                Err(VALIDATION_FAILED.into())
            } else {
                Ok(Attempt::Repair(error))
            }
        }
    }
}

/// One repair only; unchanged model/data bounds, no tool capabilities.
fn repair_request(request: &str, error: &str) -> Result<String, String> {
    let request = format!("{request}\n이전 출력의 검증 오류: {error}\nJSON을 처음부터 다시 작성하세요. evidence_id에는 해당 사실을 뒷받침하는 원문 객체에 적힌 id를 그대로 쓰세요. 다른 객체의 날짜·시간을 가져오지 마세요. evidence 필드는 쓰지 마세요. 뒷받침되는 사실만 포함하고 불확실한 사실은 빼세요.");
    if request.len() > INPUT_BYTES {
        return Err(VALIDATION_FAILED.into());
    }
    Ok(request)
}

#[cfg(test)]
#[test]
fn configuration_hash_pins_llama_server_identity_and_retires_worker_results() {
    // Literal inputs. A change here invalidates every cached result.
    let released = format!("analysis-v3;selector=head-tail-keywords-v4;prompt_rules=primary-purpose-positive-v1;evidence_encoding=explicit-id-v1;clock_rules=meridiem-v2;decoding=analysis_json_v1;max_tokens=1024;source_bytes={SOURCE_BYTES};thinking=false;sampler=deterministic;repair_attempts=1;engine=llama-server-v1");
    // Vision added a new identity; removing generation category adds another.
    let vision = format!("analysis-v4;selector=head-tail-keywords-v4;prompt_rules=primary-purpose-positive-v1;evidence_encoding=explicit-id-plus-image-v1;clock_rules=meridiem-v2;decoding=analysis_json_v1;max_tokens=1024;source_bytes={SOURCE_BYTES};thinking=false;sampler=deterministic;repair_attempts=1;engine=llama-server-v1;vision=mmproj-v1;image_source=archived-local-v1");
    let current = format!("analysis-v5;selector=head-tail-keywords-v4;evidence_encoding=explicit-id-plus-image-v1;clock_rules=meridiem-v2;decoding=analysis_json_v1;max_tokens=1024;source_bytes={SOURCE_BYTES};thinking=false;sampler=deterministic;repair_attempts=1;engine=llama-server-v1;vision=mmproj-v1;image_source=archived-local-v1");
    let hash = |input: &str| format!("{:x}", Sha256::digest(input.as_bytes()));
    for mode in ["auto", "cpu", "gpu"] {
        let actual = configuration_hash(1024, "analysis_json_v1", mode);
        let expected = if cfg!(windows) {
            hash(&format!("{current};execution_mode={mode}"))
        } else {
            hash(&current)
        };
        assert_eq!(actual, expected);
        // v1.0 (macOS), v1.1 (Windows) and v1.2 vision results read as outdated.
        assert_ne!(actual, hash(&released));
        assert_ne!(actual, hash(&vision));
        assert_ne!(
            actual,
            hash(&format!(
                "{released};runtime_policy=llama-vulkan-v1;execution_mode={mode}"
            ))
        );
    }
}

#[cfg(test)]
#[test]
fn generation_schema_is_closed_bounded_and_ordered_like_the_prompt() {
    let schema: serde_json::Value = serde_json::from_str(GENERATION_SCHEMA).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["required"].as_array().unwrap().len(), 4);
    assert!(schema["properties"].get("category").is_none());
    let facts = &schema["properties"]["facts"];
    assert_eq!(facts["maxItems"], 6);
    assert!(facts.get("uniqueItems").is_none());
    assert_eq!(facts["items"]["additionalProperties"], false);
    assert_eq!(
        facts["items"]["required"],
        json!(["kind", "label", "value", "evidence_id"])
    );
    assert_eq!(facts["items"]["properties"]["evidence_id"]["minimum"], 0);
    assert_eq!(facts["items"]["properties"]["label"]["maxLength"], 40);
    assert_eq!(facts["items"]["properties"]["value"]["maxLength"], 500);
    assert_eq!(facts["items"]["properties"].as_object().unwrap().len(), 4);
    // Properties appear in the order the grammar must emit them.
    let order = |keys: &[&str]| {
        let positions = keys
            .iter()
            .map(|key| GENERATION_SCHEMA.find(&format!("\"{key}\":{{")).unwrap())
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{keys:?}"
        );
    };
    order(&["short_summary", "summary", "uncertainty", "facts"]);
    order(&["kind", "label", "value", "evidence_id"]);
}

/// Explicit opt-in debugging only; never compiled into release builds, and
/// never retains source or raw generation in production storage.
#[cfg(test)]
fn trace(entry: std::fmt::Arguments) {
    if let Some(path) = std::env::var_os("RHYTHM_AI_EVAL_TRACE") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{entry}\n");
        }
    }
}

#[cfg(test)]
fn trace_request(job_id: i64, request: &str) {
    trace(format_args!("job={job_id} request\n{request}"));
}

#[cfg(test)]
fn trace_generation(job_id: i64, attempt: usize, generated: &str) {
    trace(format_args!("job={job_id} attempt={attempt}\n{generated}"));
}

#[cfg(test)]
mod tests;
