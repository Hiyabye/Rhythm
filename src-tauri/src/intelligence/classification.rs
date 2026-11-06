//! Deterministic, local notice classification from embedding similarity.
//!
//! The pinned embedding model produces one unit-length vector per notice
//! revision. Each category has authored Korean prototypes; the nearest
//! prototype wins. A score below [`MIN_SCORE`] becomes `기타` with a
//! low-confidence marker kept in SQLite. Prototype text, scoring and threshold
//! are part of the config hash, so changing them reclassifies stored vectors
//! without re-embedding and leaves old rows as history.

use crate::{
    intelligence::{
        analysis,
        model_storage::resolve_model_path,
        models::{catalog, verify_file},
        remote, Worker,
    },
    storage::intelligence::{AiModel, AiPreferences},
    AppState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::sync::{Mutex as AsyncMutex, Notify};

/// Bumped only for a contract change the literal hash below cannot express.
const CONFIG_VERSION: &str = "notice-classification-v1";
const BODY_CHARS: usize = 1000;
const MAX_INPUT_BYTES: usize = 7 * 1024;
/// Calibrated on the 18 committed synthetic fixtures: the lowest correct score
/// was 0.565. Below this the top match is only a weak hint.
pub const MIN_SCORE: f32 = 0.50;
const BATCH: u32 = 8;
/// Release the embedding process after this much idle time. Classification is
/// cheap, but the process still reserves model memory.
const IDLE_UNLOAD: Duration = Duration::from_secs(60);

const INSTRUCTION: &str = "Instruct: Classify the given Korean university notice into exactly one category: 학사, 장학, 수업·교육, 연구, 공모전·대회, 인턴·채용, 행사·활동, 기타.\nQuery: ";

/// Authored Korean prototypes. They encode the same category definitions the
/// generation prompt used and are deliberately separate from the evaluation
/// fixtures in `evaluation.json`.
pub(super) const CATEGORY_PROTOTYPES: [(&str, &[&str]); 7] = [
    ("학사", &[
        "학사 운영 안내: 휴학, 복학, 졸업, 학적 변경, 성적 정정, 수강신청, 등록금 납부, 학위 수여",
        "2026학년도 학사일정과 수강신청 기간을 안내합니다.",
        "졸업 요건 확인 및 학위 논문 제출 안내",
        "전과, 다전공, 부전공 신청 안내",
        "학생증 재발급과 재학증명서 발급 안내",
    ]),
    ("장학", &[
        "장학금 신청 안내: 성적우수 장학금, 국가장학금, 교내외 장학금, 등록금과 생활비 지원",
        "희망장학금 신청 자격과 제출 서류를 안내합니다.",
        "복수 조건 장학금 지원 자격 안내",
        "중복 지원 제한이 있는 장학금 안내",
        "생활비 지원 장학생 선발 공고",
    ]),
    ("수업·교육", &[
        "교육 과정과 강좌 수강생 모집 안내: 교과목, 비교과 프로그램, 학습 지원",
        "도서관 이용 교육 및 학술정보 검색 방법 안내",
        "교육 프로그램 신청 일정 정정 안내",
        "영어 강좌, 소프트웨어 교육, 자격증 과정 수강 안내",
        "교수학습센터 학습법 특강과 튜터링 프로그램 안내",
    ]),
    ("연구", &[
        "학부생 연구 참여자 모집: 연구실 인턴, 학부 연구 프로그램, 공동 연구 과제",
        "하계 연구 체험 프로그램 사전 안내",
        "연구비 신청과 논문 작성 지원 안내",
        "학술 연구 과제 공모와 연구실 소개",
        "연구 참여 학생 모집 및 연구 주제 안내",
    ]),
    ("공모전·대회", &[
        "공모전과 경진대회 참가자 모집 안내: 아이디어 공모, 데이터 분석 대회, 해커톤",
        "대학생 연구 아이디어 공모 일정표",
        "창업 경진대회와 캡스톤 디자인 대회 안내",
        "프로그래밍 대회와 알고리즘 경진대회 참가 신청",
        "작품 공모전 수상작 발표",
    ]),
    ("인턴·채용", &[
        "인턴 모집과 채용 공고 안내: 하계 인턴, 현장실습, 신입 채용, 커리어 프로그램",
        "하계 인턴 접수 마감 및 면접 시간 안내",
        "기업 채용 설명회와 직무 체험 안내",
        "현장실습생 모집과 채용 연계 프로그램",
        "취업 준비 특강과 채용 정보 안내",
    ]),
    ("행사·활동", &[
        "행사와 활동 안내: 특강, 세미나, 축제, 동아리, 진로 프로그램, 독서 모임",
        "오픈소스 기술 세미나 개최 안내",
        "진로 특강 사전 신청 및 행사 일정",
        "독서 모임 참가 신청 안내",
        "교내 축제와 학생 활동 프로그램 안내",
    ]),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Classified {
    pub category: &'static str,
    pub score: f32,
    pub margin: f32,
    pub low_confidence: bool,
}

/// Stable identity for the prototype set, scoring, threshold and input shape.
pub fn config_hash() -> &'static str {
    static HASH: LazyLock<String> = LazyLock::new(|| {
        let mut hasher = Sha256::new();
        hasher.update(CONFIG_VERSION);
        hasher.update(INSTRUCTION);
        hasher.update(BODY_CHARS.to_le_bytes());
        hasher.update(MIN_SCORE.to_le_bytes());
        for category in crate::intelligence::analysis::CATEGORIES {
            hasher.update(category);
        }
        for (category, prototypes) in CATEGORY_PROTOTYPES {
            hasher.update(category);
            for prototype in prototypes {
                hasher.update([0]);
                hasher.update(prototype);
            }
        }
        format!("{:x}", hasher.finalize())
    });
    &HASH
}

/// Bounded classifier input: instruction, title, then the first 1000 body
/// characters, capped so one long notice cannot exceed the embedding batch.
pub fn input(title: &str, body: &str) -> String {
    let body = body.chars().take(BODY_CHARS).collect::<String>();
    let text = format!("{INSTRUCTION}{title}\n{body}");
    if text.len() <= MAX_INPUT_BYTES {
        return text;
    }
    let mut end = MAX_INPUT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Classifies against already-embedded prototypes. Both sides are unit length,
/// so the dot product is cosine similarity.
pub fn classify_with(vector: &[f32], prototypes: &[(&'static str, Vec<f32>)]) -> Classified {
    let mut best = ("기타", f32::NEG_INFINITY);
    let mut second = f32::NEG_INFINITY;
    for (category, prototype) in prototypes {
        let score = cosine(vector, prototype);
        if score > best.1 {
            second = best.1;
            best = (category, score);
        } else if score > second {
            second = score;
        }
    }
    let low_confidence = best.1 < MIN_SCORE || !best.1.is_finite();
    Classified {
        category: if low_confidence { "기타" } else { best.0 },
        score: if best.1.is_finite() { best.1 } else { -1.0 },
        margin: if best.1.is_finite() && second.is_finite() {
            best.1 - second
        } else {
            0.0
        },
        low_confidence,
    }
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return f32::NEG_INFINITY;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

/// Runtime state exposed to Settings. Process-local, never persisted.
#[derive(Clone, Serialize)]
pub struct Status {
    pub state: &'static str,
    pub processed: u64,
    /// Why the current engine cannot classify; None when healthy.
    pub detail: Option<String>,
}

type Prototypes = Vec<(&'static str, Vec<f32>)>;

/// Holds classification quiesced. Drop resumes work after releasing the batch
/// guard, so a slow batch cannot be interrupted and a later one cannot start early.
pub struct PauseGuard {
    controller: Arc<Controller>,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for PauseGuard {
    fn drop(&mut self) {
        drop(self.guard.take());
        self.controller.resume();
    }
}

pub struct Controller {
    worker: AsyncMutex<Worker>,
    batch: Arc<AsyncMutex<()>>,
    paused: AtomicBool,
    processed: AtomicU64,
    state: Mutex<&'static str>,
    detail: Mutex<Option<String>>,
    last_work: Mutex<Option<Instant>>,
    verified: Mutex<Option<String>>,
    failed: Mutex<Option<String>>,
    prototypes: AsyncMutex<Option<Prototypes>>,
    prototypes_sha: Mutex<Option<String>>,
    /// Cancels the remote embedding request on quiesce/shutdown.
    active_embedding: Mutex<Option<Arc<analysis::Cancellation>>>,
    wake: Notify,
}

enum Source {
    Local(AiModel),
    Remote(remote::EmbeddingConfig),
}

/// Chooses the embedding path from preferences. No network or SQLite work here;
/// the caller supplies the selected local model. `None` means classification is
/// off or no usable local model is selected.
fn select_source(
    preferences: &AiPreferences,
    local_model: Option<AiModel>,
) -> Result<Option<(Source, String)>, String> {
    if !preferences.classification_enabled {
        return Ok(None);
    }
    if preferences.engine == "openai-compatible" && preferences.remote_embeddings {
        let config = remote::EmbeddingConfig::from_preferences(preferences)?;
        let identity = remote::identity_hash(config.endpoint.origin(), &config.model);
        return Ok(Some((Source::Remote(config), identity)));
    }
    Ok(local_model
        .filter(|model| {
            model.status == "installed" && model.kind == "embedding" && model.sha256.is_some()
        })
        .map(|model| {
            let identity = model.sha256.clone().unwrap_or_default();
            (Source::Local(model), identity)
        }))
}

impl Controller {
    pub fn new(llama: &Path, runtime: &Path) -> Self {
        let pooling = catalog()
            .into_iter()
            .find(|entry| entry.kind == "embedding")
            .and_then(|entry| entry.pooling)
            .unwrap_or_else(|| "last".into());
        Self {
            worker: AsyncMutex::new(Worker::embedding_configured(
                llama, runtime, "auto", &pooling,
            )),
            batch: Arc::new(AsyncMutex::new(())),
            paused: AtomicBool::new(false),
            processed: AtomicU64::new(0),
            state: Mutex::new("idle"),
            detail: Mutex::new(None),
            last_work: Mutex::new(None),
            verified: Mutex::new(None),
            failed: Mutex::new(None),
            prototypes: AsyncMutex::new(None),
            prototypes_sha: Mutex::new(None),
            active_embedding: Mutex::new(None),
            wake: Notify::new(),
        }
    }

    pub fn status(&self) -> Status {
        Status {
            state: self.state.lock().map(|value| *value).unwrap_or("idle"),
            processed: self.processed.load(Ordering::Relaxed),
            detail: self.detail.lock().ok().and_then(|value| value.clone()),
        }
    }

    pub fn wake(&self) {
        self.wake.notify_waiters();
    }

    /// Called after inventory or engine changes: the next batch re-verifies the
    /// file, reads the current key and reloads the process.
    pub fn invalidate(&self) {
        if let Ok(mut verified) = self.verified.lock() {
            *verified = None;
        }
        if let Ok(mut failed) = self.failed.lock() {
            *failed = None;
        }
        if let Ok(mut detail) = self.detail.lock() {
            *detail = None;
        }
        self.wake();
    }

    /// Blocks new batches and waits for one in flight. The guard keeps
    /// classification paused; dropping it resumes work.
    pub async fn quiesce(self: &Arc<Self>) -> PauseGuard {
        self.paused.store(true, Ordering::Release);
        if let Some(token) = self
            .active_embedding
            .lock()
            .ok()
            .and_then(|token| token.clone())
        {
            token.cancel();
        }
        PauseGuard {
            controller: self.clone(),
            guard: Some(self.batch.clone().lock_owned().await),
        }
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.wake();
    }

    /// Stops the process without acquiring the batch guard. The caller must
    /// hold a `PauseGuard` so no new batch starts before the file is removed.
    pub async fn stop_worker(&self) {
        self.worker.lock().await.shutdown().await;
        self.invalidate();
    }

    pub async fn shutdown(&self) {
        self.paused.store(true, Ordering::Release);
        if let Some(token) = self
            .active_embedding
            .lock()
            .ok()
            .and_then(|token| token.clone())
        {
            token.cancel();
        }
        let _guard = self.batch.clone().lock_owned().await;
        self.worker.lock().await.shutdown().await;
    }

    async fn unload_if_idle(&self, force: bool) {
        let idle = self
            .last_work
            .lock()
            .map(|last| last.is_none_or(|last| last.elapsed() >= IDLE_UNLOAD))
            .unwrap_or(true);
        if !force && !idle {
            return;
        }
        let Ok(mut worker) = self.worker.try_lock() else {
            return;
        };
        if worker.status().model.is_some() {
            worker.shutdown().await;
            if let Ok(mut verified) = self.verified.lock() {
                *verified = None;
            }
        }
        if let Ok(mut state) = self.state.lock() {
            *state = if force { "disabled" } else { "idle" };
        }
        if let Ok(mut detail) = self.detail.lock() {
            *detail = None;
        }
    }

    fn set_failure(&self, identity: &str, detail: &str) {
        if let Ok(mut failed) = self.failed.lock() {
            *failed = Some(identity.to_owned());
        }
        if let Ok(mut stored) = self.detail.lock() {
            *stored = Some(detail.to_owned());
        }
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = "failed";
    }

    /// Remembers the model that failed so it is not retried until invalidated.
    fn fail(&self, identity: &str) {
        self.set_failure(identity, "분류 모델의 무결성을 확인하지 못했습니다.");
    }

    fn is_failed(&self, identity: &str) -> bool {
        self.failed
            .lock()
            .map(|value| value.as_deref() == Some(identity))
            .unwrap_or(false)
    }

    /// Reads current preferences and processes at most one bounded batch. The
    /// SQLite guard is never held across embedding.
    pub async fn advance(&self, app: &AppState, shutdown: &AtomicBool) {
        if self.paused.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return;
        }
        let preferences = {
            let Ok(store) = app.store.try_lock() else {
                return;
            };
            let Ok(preferences) = store.ai_preferences() else {
                return;
            };
            preferences
        };
        if !preferences.classification_enabled {
            self.unload_if_idle(true).await;
            return;
        }
        let local_model =
            if preferences.engine == "openai-compatible" && preferences.remote_embeddings {
                None
            } else {
                let Ok(store) = app.store.try_lock() else {
                    return;
                };
                preferences
                    .embedding_model_id
                    .as_ref()
                    .and_then(|id| store.ai_model(id).ok().flatten())
            };
        let (source, identity) = match select_source(&preferences, local_model) {
            Ok(Some(source)) => source,
            Ok(None) => {
                self.unload_if_idle(true).await;
                return;
            }
            Err(detail) => {
                self.unload_if_idle(true).await;
                self.set_failure("remote-config", &detail);
                return;
            }
        };
        if self.is_failed(&identity) {
            self.unload_if_idle(false).await;
            return;
        }
        if app.analysis.running()
            || app.models.busy()
            || app.ai_operation.status().is_some()
            || self.paused.load(Ordering::Acquire)
        {
            return;
        }
        let candidates = {
            let Ok(store) = app.store.try_lock() else {
                return;
            };
            match store.ai_embedding_candidates(&identity, config_hash(), BATCH) {
                Ok(candidates) => candidates,
                Err(error) => {
                    eprintln!("Classification candidates: {error}");
                    return;
                }
            }
        };
        if candidates.is_empty() {
            self.unload_if_idle(false).await;
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            *state = "classifying";
        }
        if let Ok(mut detail) = self.detail.lock() {
            *detail = None;
        }
        let Ok(_batch) = self.batch.try_lock() else {
            return;
        };
        if self.paused.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return;
        }
        let token = Arc::new(analysis::Cancellation::default());
        if let Ok(mut active) = self.active_embedding.lock() {
            *active = Some(token.clone());
        }
        enum Engine<'a> {
            Local {
                worker: tokio::sync::MutexGuard<'a, Worker>,
                path: std::path::PathBuf,
            },
            Remote {
                remote: Arc<remote::Remote>,
                config: Box<remote::EmbeddingConfig>,
                key: String,
            },
        }
        impl Engine<'_> {
            async fn embed(
                &mut self,
                inputs: &[String],
                token: &analysis::Cancellation,
            ) -> Result<Vec<Vec<f32>>, String> {
                match self {
                    Self::Local { worker, path } => worker.embed(path, inputs).await,
                    Self::Remote {
                        remote,
                        config,
                        key,
                    } => remote.embed(config, key, inputs, token).await,
                }
            }
        }
        let mut engine = match source {
            Source::Local(model) => {
                let Ok(path) = resolve_model_path(&app.data_root, &model) else {
                    self.fail(&identity);
                    self.clear_active_embedding();
                    return;
                };
                let bytes = model.bytes;
                let needs_verify = self
                    .verified
                    .lock()
                    .map(|value| value.as_deref() != Some(&identity))
                    .unwrap_or(true);
                let mut worker = self.worker.lock().await;
                if needs_verify {
                    let copy = path.clone();
                    let expected = identity.clone();
                    let verified = tokio::task::spawn_blocking(move || {
                        verify_file(&copy, Some((&expected, bytes)), &AtomicBool::new(false))
                            .map_err(|_| "분류 모델의 무결성을 확인하지 못했습니다.".to_owned())
                    })
                    .await;
                    if !matches!(verified, Ok(Ok(_))) {
                        worker.shutdown().await;
                        self.fail(&identity);
                        self.clear_active_embedding();
                        return;
                    }
                    if let Ok(mut value) = self.verified.lock() {
                        *value = Some(identity.clone());
                    }
                }
                Engine::Local { worker, path }
            }
            Source::Remote(config) => {
                // A previously loaded local classifier must not keep memory.
                self.worker.lock().await.shutdown().await;
                let remote_owner = app.remote.clone();
                let origin = config.endpoint.origin().to_owned();
                let remote_for_read = remote_owner.clone();
                let key = tokio::task::spawn_blocking(move || remote_for_read.key(&origin)).await;
                let key = match key {
                    Ok(Ok(Some(key))) => key,
                    Ok(Ok(None)) => {
                        self.set_failure(&identity, "원격 분류 API 키를 저장해 주세요.");
                        self.clear_active_embedding();
                        return;
                    }
                    _ => {
                        self.set_failure(
                            &identity,
                            "운영체제 보안 저장소에서 API 키를 읽지 못했습니다.",
                        );
                        self.clear_active_embedding();
                        return;
                    }
                };
                Engine::Remote {
                    remote: remote_owner,
                    config: Box::new(config),
                    key,
                }
            }
        };
        // Prototype vectors depend on the embedding model, not on the scoring
        // config, but a config change reuses them while reclassifying. The key
        // only changes when the model changes.
        if self
            .prototypes_sha
            .lock()
            .map(|value| value.as_deref() != Some(&identity))
            .unwrap_or(true)
        {
            *self.prototypes.lock().await = None;
            if let Ok(mut value) = self.prototypes_sha.lock() {
                *value = Some(identity.clone());
            }
        }
        let prototypes = {
            let mut slot = self.prototypes.lock().await;
            match slot.as_ref() {
                Some(prototypes) => prototypes.clone(),
                None => {
                    let mut texts = Vec::new();
                    let mut owners = Vec::new();
                    for (category, entries) in CATEGORY_PROTOTYPES {
                        for entry in entries {
                            texts.push((*entry).to_owned());
                            owners.push(category);
                        }
                    }
                    let mut vectors = Vec::new();
                    for chunk in texts.chunks(crate::intelligence::MAX_EMBED_BATCH) {
                        match engine.embed(chunk, &token).await {
                            Ok(values) => vectors.extend(values),
                            Err(error) => {
                                self.clear_active_embedding();
                                if token.check().is_err() {
                                    // Quiesce/shutdown is not an engine failure.
                                    if let Ok(mut state) = self.state.lock() {
                                        *state = "waiting";
                                    }
                                    return;
                                }
                                eprintln!("Classification prototypes: {error}");
                                self.set_failure(&identity, &error);
                                return;
                            }
                        }
                    }
                    let loaded = owners
                        .into_iter()
                        .zip(vectors)
                        .collect::<Vec<(&'static str, Vec<f32>)>>();
                    *slot = Some(loaded.clone());
                    loaded
                }
            }
        };
        let dimension = prototypes
            .first()
            .map(|(_, vector)| vector.len())
            .unwrap_or(0);
        let mut results: Vec<(i64, i64, Vec<f32>)> = Vec::new();
        let mut inputs = Vec::new();
        let mut targets = Vec::new();
        for (document_id, revision_id) in candidates {
            let stored = {
                let Ok(store) = app.store.try_lock() else {
                    break;
                };
                store
                    .ai_embedding(document_id, revision_id, &identity)
                    .ok()
                    .flatten()
            };
            match stored {
                Some(stored)
                    if stored.dim as usize == dimension && stored.vector.len() == dimension =>
                {
                    results.push((document_id, revision_id, stored.vector));
                }
                _ => {
                    let text = {
                        let Ok(store) = app.store.try_lock() else {
                            break;
                        };
                        store
                            .classification_text(document_id, revision_id)
                            .ok()
                            .flatten()
                    };
                    if let Some((title, body)) = text {
                        inputs.push(input(&title, &body));
                        targets.push((document_id, revision_id));
                    }
                }
            }
        }
        if !inputs.is_empty() {
            match engine.embed(&inputs, &token).await {
                Ok(vectors) => {
                    results.extend(targets.into_iter().zip(vectors).map(
                        |((document_id, revision_id), vector)| (document_id, revision_id, vector),
                    ));
                }
                Err(error) => {
                    self.clear_active_embedding();
                    if token.check().is_err() {
                        if let Ok(mut state) = self.state.lock() {
                            *state = "waiting";
                        }
                        return;
                    }
                    eprintln!("Classification embedding: {error}");
                    self.set_failure(&identity, &error);
                    return;
                }
            }
        }
        drop(engine);
        self.clear_active_embedding();
        let mut written = 0u64;
        {
            let Ok(store) = app.store.try_lock() else {
                return;
            };
            for (document_id, revision_id, vector) in results {
                if vector.len() != dimension
                    || store
                        .ai_save_embedding(document_id, revision_id, &identity, &vector)
                        .is_err()
                {
                    continue;
                }
                let classified = classify_with(&vector, &prototypes);
                if let Err(error) = store.ai_save_classification(
                    &crate::storage::intelligence::ClassificationWrite {
                        document_id,
                        revision_id,
                        model_sha256: &identity,
                        config_hash: config_hash(),
                        category: classified.category,
                        score: classified.score,
                        margin: classified.margin,
                        low_confidence: classified.low_confidence,
                    },
                ) {
                    eprintln!("Classification save: {error}");
                    continue;
                }
                written += 1;
            }
        }
        self.processed.fetch_add(written, Ordering::Relaxed);
        if let Ok(mut last) = self.last_work.lock() {
            *last = Some(Instant::now());
        }
        if let Ok(mut state) = self.state.lock() {
            *state = if written > 0 { "idle" } else { "waiting" };
        }
        if let Ok(mut detail) = self.detail.lock() {
            *detail = None;
        }
    }

    fn clear_active_embedding(&self) {
        if let Ok(mut active) = self.active_embedding.lock() {
            *active = None;
        }
    }
}

pub async fn run(handle: tauri::AppHandle, shutdown: Arc<AtomicBool>) {
    loop {
        if let Some(state) = handle.try_state::<AppState>() {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                _ = state.classifier.wake.notified() => {}
            }
            if shutdown.load(Ordering::Acquire) {
                break;
            }
            state.classifier.advance(&state, &shutdown).await;
        } else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if shutdown.load(Ordering::Acquire) {
                break;
            }
        }
    }
    if let Some(state) = handle.try_state::<AppState>() {
        state.classifier.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preferences() -> AiPreferences {
        AiPreferences {
            active_model_id: None,
            setup_completed: false,
            processing_mode: "explicit".into(),
            automatic_limit: 3,
            max_tokens: 1024,
            execution_mode: "auto".into(),
            model_directory: None,
            embedding_model_id: Some("embed".into()),
            classification_enabled: true,
            engine: "local".into(),
            remote_base_url: None,
            remote_model: None,
            remote_embedding_model: None,
            remote_timeout_seconds: 120,
            remote_consent: false,
            remote_automatic: false,
            remote_daily_limit: 20,
            remote_embeddings: false,
        }
    }

    fn embedding_model() -> AiModel {
        AiModel {
            id: "embed".into(),
            catalog_id: None,
            name: "embedding fixture".into(),
            ownership: "external".into(),
            kind: "embedding".into(),
            path: "embed.gguf".into(),
            storage_directory: None,
            sha256: Some("c".repeat(64)),
            expected_sha256: None,
            expected_bytes: None,
            bytes: 1,
            source_revision: None,
            source_filename: None,
            status: "installed".into(),
            downloaded_bytes: 0,
            resume_etag: None,
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn classification_source_requires_opt_in_and_separates_remote_identity() {
        let (source, identity) = select_source(&preferences(), Some(embedding_model()))
            .unwrap()
            .unwrap();
        assert!(matches!(source, Source::Local(_)));
        assert_eq!(identity, "c".repeat(64));

        let mut disabled = preferences();
        disabled.classification_enabled = false;
        assert!(select_source(&disabled, Some(embedding_model()))
            .unwrap()
            .is_none());
        assert!(select_source(&preferences(), None).unwrap().is_none());
        let mut wrong_kind = embedding_model();
        wrong_kind.kind = "generation".into();
        assert!(select_source(&preferences(), Some(wrong_kind))
            .unwrap()
            .is_none());

        let mut remote = preferences();
        remote.engine = "openai-compatible".into();
        remote.remote_embeddings = true;
        remote.remote_consent = true;
        remote.remote_base_url = Some("https://api.example.com".into());
        remote.remote_embedding_model = Some("embed".into());
        let (source, remote_identity) = select_source(&remote, None).unwrap().unwrap();
        assert!(matches!(source, Source::Remote(_)));
        assert_eq!(remote_identity.len(), 64);
        assert_ne!(remote_identity, identity);

        remote.remote_consent = false;
        assert!(select_source(&remote, None).is_err());
        remote.remote_consent = true;
        remote.remote_embedding_model = None;
        assert!(select_source(&remote, None).is_err());
    }

    #[test]
    fn config_hash_is_stable_and_hex() {
        let hash = config_hash();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(hash, config_hash());
    }

    #[test]
    fn input_is_bounded_and_keeps_the_title() {
        let body = "가".repeat(50_000);
        let text = input("제목", &body);
        assert!(text.starts_with(INSTRUCTION));
        assert!(text.len() <= MAX_INPUT_BYTES);
        assert!(text.contains("제목"));
        assert!(text.chars().count() < 4_000);
    }

    #[test]
    fn threshold_falls_back_to_other_and_keeps_the_score() {
        let prototypes = vec![("장학", vec![0.0, 1.0]), ("학사", vec![1.0, 0.0])];
        let near = classify_with(&[1.0, 0.0], &prototypes);
        assert_eq!(near.category, "학사");
        assert!(!near.low_confidence);
        assert!(near.score > 0.9);
        // Orthogonal to every prototype: below the threshold.
        let far = classify_with(&[0.0, 0.0], &prototypes);
        assert_eq!(far.category, "기타");
        assert!(far.low_confidence);
    }

    #[test]
    fn margin_is_the_gap_to_the_second_category() {
        let prototypes = vec![("장학", vec![1.0, 0.0]), ("학사", vec![0.0, 1.0])];
        let result = classify_with(&[0.8, 0.2], &prototypes);
        assert_eq!(result.category, "장학");
        assert!((result.margin - 0.6).abs() < 1e-6, "{result:?}");
    }
}
