//! Deterministic, local notice classification from embedding similarity.
//!
//! The pinned embedding model produces one unit-length vector per notice
//! revision. Each category has authored Korean prototypes, embedded with the
//! same instruction prefix as the notices; the nearest prototype wins. A score
//! below [`MIN_SCORE`] becomes `기타` with a low-confidence marker kept in
//! SQLite. Prototype text, scoring and threshold
//! are part of the config hash, so changing them reclassifies stored vectors
//! without re-embedding and leaves old rows as history.

use crate::{
    intelligence::{
        model_storage::resolve_model_path,
        models::{catalog, verify_file},
        Worker,
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
const CONFIG_VERSION: &str = "notice-classification-v2";
const BODY_CHARS: usize = 1000;
const MAX_INPUT_BYTES: usize = 7 * 1024;
/// Calibrated on 1,073 hand-labeled archived notices: below this about half of
/// the top matches were wrong, so the match is only a weak hint
/// (docs/AI_EVALUATION.md).
pub const MIN_SCORE: f32 = 0.68;
const BATCH: u32 = 8;
/// Release the embedding process after this much idle time. Classification is
/// cheap, but the process still reserves model memory.
const IDLE_UNLOAD: Duration = Duration::from_secs(60);

/// Stored notice vectors carry this exact prefix and search compares against
/// them, so it keeps the original category list: changing it would require
/// re-embedding every notice, and the newer list measured no better.
pub(super) const INSTRUCTION: &str = "Instruct: Classify the given Korean university notice into exactly one category: 학사, 장학, 수업·교육, 연구, 공모전·대회, 인턴·채용, 행사·활동, 기타.\nQuery: ";
/// Search queries are compared with the stored classifier vectors above. Of
/// the instructions tried on the synthetic search fixtures, Qwen3's own web
/// search instruction ranked relevant notices highest (docs/AI_EVALUATION.md).
pub(super) const SEARCH_INSTRUCTION: &str =
    "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery: ";

/// The closed category set. `기타` has no prototypes: it is the fallback for
/// a best score below [`MIN_SCORE`].
pub const CATEGORIES: [&str; 11] = [
    "학사",
    "수업",
    "장학",
    "연구",
    "학과 소식",
    "공모전·대회",
    "인턴·채용",
    "교육·프로그램",
    "행사·활동",
    "생활·시설",
    "기타",
];

/// Authored Korean prototypes, deliberately separate from the evaluation
/// fixtures in `evaluation.json`. The first entry of each category names its
/// scope; the rest are typical notice titles. 벨라르미노 학사 is the dormitory,
/// so its notices need their own 생활·시설 prototypes to stay out of 학사.
pub(super) const CATEGORY_PROTOTYPES: [(&str, &[&str]); 10] = [
    ("학사", &[
        "학사 행정 안내: 휴학, 복학, 재입학, 졸업, 학적, 전공 신청, 등록금 납부, 학위 수여",
        "2026학년도 2학기 휴학 및 복학 신청 안내",
        "졸업예정 처리와 조기졸업 신청 안내",
        "전공 신청 및 변경, 다전공과 마이크로전공 신청 안내",
        "학부 등록금 납부 기간과 분할 납부 안내",
        "학칙 개정 공고와 시행세칙 변경 안내",
        "신입생 학번 안내와 학생증 발급 안내",
        "성적증명서와 졸업증명서 발급 일정 안내",
        "대학원 신입생 모집 요강과 입학 설명회 안내",
        "학과 요람 변경과 졸업 요건 안내",
    ]),
    ("수업", &[
        "수업과 수강신청 안내: 개설 교과목, 수강정원, 폐강, 휴강, 보강, 시험, 성적, 강의평가",
        "2026학년도 1학기 수강신청 일정과 개설 교과목 안내",
        "중간고사 일정과 시험 장소 안내",
        "이번 주 수업 휴강 및 보강 안내",
        "과제 제출 마감일과 실습 채점 결과 안내",
        "출석과 좌석표, 강의실 변경 안내",
        "하계 계절수업 개설 과목과 타 대학 학점교류 수강 안내",
        "강의평가 실시 안내",
        "강의계획서와 수업 운영 방식 안내",
        "교과목 증원과 강의 언어 변경 안내",
    ]),
    ("장학", &[
        "장학금 안내: 교내외 장학금, 국가장학금, 장학생 선발, 학자금 대출, 생활비 지원",
        "2026년 장학생 모집 공고와 지원 자격",
        "기업 장학생 선발 안내",
        "성적우수 장학금 선발 결과 안내",
        "학자금 대출 신청 안내",
        "재단 장학생 모집과 제출 서류 안내",
    ]),
    ("연구", &[
        "연구 참여 안내: 학부 연구생 모집, 연구실 인턴, 연구 과제, 연구비 지원 사업",
        "학부생 연구 참여 프로그램 모집",
        "연구실 안전교육 이수 안내",
        "연구 지원 사업 공모와 연구비 신청 안내",
        "박사학위 논문심사 공개 발표 안내",
        "학술대회 논문 모집과 투고 안내",
    ]),
    ("학과 소식", &[
        "학과 소식: 교수 연구팀 논문 채택, 수상 소식, 졸업생 임용, 언론 보도, 소식지",
        "교수 연구팀 최우수 국제학술대회 정규 논문 채택",
        "우리 학과 학생 논문상 수상을 축하합니다",
        "연구실 졸업생 대학 교수 임용 소식",
        "학과 언론 보도 기사",
        "대학 소식지 발간",
    ]),
    ("공모전·대회", &[
        "공모전과 경진대회 참가자 모집: 아이디어 공모, 영상 공모전, 데이터 분석 대회, 해커톤",
        "AI 영상 콘텐츠 공모전 참가 안내",
        "공공데이터 활용 아이디어 경진대회",
        "해커톤 참가팀 모집",
        "프로그래밍 대회와 해킹방어대회 참가 신청",
        "창업 경진대회 참가 모집",
    ]),
    ("인턴·채용", &[
        "채용과 인턴 모집 공고: 신입사원 공개채용, 채용연계형 인턴, 체험형 인턴, 현장실습",
        "2026년 하반기 신입사원 공개채용 안내",
        "채용연계형 인턴십 모집",
        "기업 채용설명회와 잡페어 안내",
        "학과사무실 근로학생과 조교 모집",
        "교과목 실습 멘토 모집",
        "전문연구요원 채용 공고",
        "교수 초빙 공고",
    ]),
    ("교육·프로그램", &[
        "외부 교육 프로그램 교육생 모집: 부트캠프, 아카데미, 국비지원 교육과정, 인재 양성 과정",
        "K-디지털 트레이닝 국비지원 부트캠프 교육생 모집",
        "AI 개발자 양성과정 수강생 모집, 교육비 전액 무료",
        "기업 아카데미 기수 교육생 선발",
        "자격증 취득 과정과 자격 시험 접수 안내",
        "해외 대학 AI 연수 프로그램 참가자 모집",
        "필수 법정 교육 이수 안내",
        "청년취업사관학교 캠퍼스 교육생 모집",
        "보안 아카데미 직무 트랙 교육생 모집",
    ]),
    ("행사·활동", &[
        "행사와 활동 안내: 세미나, 콜로퀴엄, 특강, 컨퍼런스, 포럼, 설명회, 동아리, 대외활동",
        "학과 콜로퀴엄 초청 강연 안내",
        "기술 세미나와 컨퍼런스 사전 등록 안내",
        "포럼 개최 안내",
        "동아리 신입 회원 모집",
        "대학생 대외활동과 서포터즈 모집",
        "축제와 입학식, 오리엔테이션 행사 안내",
        "기관 견학 투어 참가자 모집",
        "기술 엑스포와 서밋, 네트워킹 행사 참가 안내",
    ]),
    ("생활·시설", &[
        "생활과 시설 안내: 기숙사(벨라르미노 학사) 생활, 식당 식단, 시설 공사, 건물과 공간 이용, 정전, 서버 점검",
        "벨라르미노 학사 기숙사 사생 모집과 입사 안내",
        "기숙사 입사자 발표와 기숙사비 납부 안내",
        "주간 식단표",
        "아침 점심 저녁 메뉴: 밥, 국, 반찬, 김치",
        "건물 보수공사로 인한 시설 이용 제한 안내",
        "실습실 사용 불가와 새 공간 조성 안내",
        "기숙사 택배 수령과 생활 수칙, 운영 시간 안내",
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
        for category in CATEGORIES {
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

/// Each prototype with its category, embedded like a notice: without the
/// shared instruction prefix every notice scored in a narrow band and the
/// nearest prototype was close to arbitrary.
fn prototype_inputs() -> Vec<(&'static str, String)> {
    CATEGORY_PROTOTYPES
        .iter()
        .flat_map(|(category, entries)| {
            entries
                .iter()
                .map(move |entry| (*category, format!("{INSTRUCTION}{entry}")))
        })
        .collect()
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

impl PauseGuard {
    /// Releases the batch guard so `shutdown` can acquire it during exit, but
    /// keeps classification paused: no new batch starts before the process ends.
    pub fn release_for_exit(self) {
        let mut this = std::mem::ManuallyDrop::new(self);
        drop(this.guard.take());
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
    /// Last search query vector (model identity, query, vector), so paging
    /// through results does not embed the same query again.
    last_query: Mutex<Option<(String, String, Vec<f32>)>>,
    wake: Notify,
}

type LocalEngine<'a> = (tokio::sync::MutexGuard<'a, Worker>, std::path::PathBuf);

/// The selected, installed embedding model and its identity (the artifact
/// SHA-256). No network or SQLite work here. `None` means classification is
/// off or no usable model is selected.
fn select_model(preferences: &AiPreferences, model: Option<AiModel>) -> Option<(AiModel, String)> {
    if !preferences.classification_enabled {
        return None;
    }
    model
        .filter(|model| model.status == "installed" && model.kind == "embedding")
        .and_then(|model| {
            let identity = model.sha256.clone()?;
            Some((model, identity))
        })
}

impl Controller {
    /// `mode` is the saved execution mode; it applies only on Windows.
    pub fn new(llama: &Path, runtime: &Path, mode: &str) -> Self {
        let pooling = catalog()
            .into_iter()
            .find(|entry| entry.kind == "embedding")
            .and_then(|entry| entry.pooling)
            .unwrap_or_else(|| "last".into());
        Self::with_worker(Worker::configured(llama, runtime, mode, &pooling))
    }

    fn with_worker(worker: Worker) -> Self {
        Self {
            worker: AsyncMutex::new(worker),
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
            last_query: Mutex::new(None),
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

    /// Called after inventory or preference changes: the next batch
    /// re-verifies the file and reloads the process.
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
        if let Ok(mut last_query) = self.last_query.lock() {
            *last_query = None;
        }
        self.wake();
    }

    /// Blocks new batches and waits for one in flight. The guard keeps
    /// classification paused; dropping it resumes work.
    pub async fn quiesce(self: &Arc<Self>) -> PauseGuard {
        self.paused.store(true, Ordering::Release);
        PauseGuard {
            controller: self.clone(),
            guard: Some(self.batch.clone().lock_owned().await),
        }
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.wake();
    }

    /// Stops the process and waits until it is reaped, without acquiring the
    /// batch guard. The caller must hold a `PauseGuard` so no new batch starts
    /// before a model file is removed or an installer replaces the executable.
    pub async fn stop_worker(&self) -> Result<(), String> {
        let result = self.worker.lock().await.stop_confirmed().await;
        self.invalidate();
        result
    }

    /// Applies a saved execution mode. The caller must hold a `PauseGuard`;
    /// the running process is reaped so the next batch starts on the new build.
    pub async fn set_execution_mode(&self, mode: &str) -> Result<(), String> {
        let mut worker = self.worker.lock().await;
        if worker.execution_mode() == mode {
            return Ok(());
        }
        worker.stop_confirmed().await?;
        worker.set_execution_mode(mode)?;
        drop(worker);
        self.invalidate();
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.paused.store(true, Ordering::Release);
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
            let Ok(preferences) = store.ai_effective_preferences() else {
                return;
            };
            preferences
        };
        let model = {
            let Ok(store) = app.store.try_lock() else {
                return;
            };
            preferences
                .embedding_model_id
                .as_ref()
                .and_then(|id| store.ai_model(id).ok().flatten())
        };
        let Some((model, identity)) = select_model(&preferences, model) else {
            self.unload_if_idle(true).await;
            return;
        };
        if self.is_failed(&identity) {
            self.unload_if_idle(false).await;
            return;
        }
        if app.models.busy() || self.paused.load(Ordering::Acquire) {
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
        let Some((mut worker, path)) = self.local_engine(&app.data_root, &model, &identity).await
        else {
            return;
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
                    let (owners, texts): (Vec<_>, Vec<_>) = prototype_inputs().into_iter().unzip();
                    let mut vectors = Vec::new();
                    for chunk in texts.chunks(crate::intelligence::MAX_EMBED_BATCH) {
                        match worker.embed(&path, chunk).await {
                            Ok(values) => vectors.extend(values),
                            Err(error) => {
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
            match worker.embed(&path, &inputs).await {
                Ok(vectors) => {
                    results.extend(targets.into_iter().zip(vectors).map(
                        |((document_id, revision_id), vector)| (document_id, revision_id, vector),
                    ));
                }
                Err(error) => {
                    eprintln!("Classification embedding: {error}");
                    self.set_failure(&identity, &error);
                    return;
                }
            }
        }
        drop(worker);
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

    /// The local worker and model path, verifying the file once per identity.
    /// A failed check marks the identity failed until invalidated.
    async fn local_engine(
        &self,
        data_root: &Path,
        model: &AiModel,
        identity: &str,
    ) -> Option<LocalEngine<'_>> {
        let Ok(path) = resolve_model_path(data_root, model) else {
            self.fail(identity);
            return None;
        };
        let bytes = model.bytes;
        let needs_verify = self
            .verified
            .lock()
            .map(|value| value.as_deref() != Some(identity))
            .unwrap_or(true);
        let mut worker = self.worker.lock().await;
        if needs_verify {
            let copy = path.clone();
            let expected = identity.to_owned();
            let verified = tokio::task::spawn_blocking(move || {
                verify_file(&copy, Some((&expected, bytes)), &AtomicBool::new(false))
                    .map_err(|_| "분류 모델의 무결성을 확인하지 못했습니다.".to_owned())
            })
            .await;
            if !matches!(verified, Ok(Ok(_))) {
                worker.shutdown().await;
                self.fail(identity);
                return None;
            }
            if let Ok(mut value) = self.verified.lock() {
                *value = Some(identity.to_owned());
            }
        }
        Some((worker, path))
    }

    /// Embeds a search query with the selected local embedding model, for
    /// comparison with stored notice vectors of the same model identity.
    /// `None` means search stays text-only: classification is off, no model
    /// is installed, or the engine is busy with an inventory operation.
    pub async fn query_vector(&self, app: &AppState, query: &str) -> Option<(String, Vec<f32>)> {
        let (preferences, model) = {
            let store = app.store.lock().ok()?;
            let preferences = store.ai_effective_preferences().ok()?;
            let model = preferences
                .embedding_model_id
                .as_ref()
                .and_then(|id| store.ai_model(id).ok().flatten());
            (preferences, model)
        };
        let (model, identity) = select_model(&preferences, model)?;
        if app.models.busy() {
            return None;
        }
        let vector = self
            .embed_query(&app.data_root, &model, &identity, query)
            .await?;
        Some((identity, vector))
    }

    /// The query vector, from the last search or one embedding request. The
    /// batch guard keeps it from overlapping a classification batch, and a
    /// paused or failed engine is not started.
    async fn embed_query(
        &self,
        data_root: &Path,
        model: &AiModel,
        identity: &str,
        query: &str,
    ) -> Option<Vec<f32>> {
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
        if query.is_empty() {
            return None;
        }
        if let Some((_, _, vector)) = self.last_query.lock().ok().and_then(|last| {
            last.clone()
                .filter(|(cached, text, _)| cached == identity && *text == query)
        }) {
            return Some(vector);
        }
        if self.is_failed(identity) || self.paused.load(Ordering::Acquire) {
            return None;
        }
        let _batch = self.batch.lock().await;
        if self.paused.load(Ordering::Acquire) {
            return None;
        }
        let (mut worker, path) = self.local_engine(data_root, model, identity).await?;
        let vector = match worker
            .embed(&path, &[format!("{SEARCH_INSTRUCTION}{query}")])
            .await
        {
            Ok(mut vectors) if vectors.len() == 1 => vectors.pop()?,
            Ok(_) => return None,
            Err(error) => {
                eprintln!("Search embedding: {error}");
                return None;
            }
        };
        drop(worker);
        if let Ok(mut last) = self.last_work.lock() {
            *last = Some(Instant::now());
        }
        if let Ok(mut last_query) = self.last_query.lock() {
            *last_query = Some((identity.to_owned(), query, vector.clone()));
        }
        Some(vector)
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
            execution_mode: "auto".into(),
            model_directory: None,
            embedding_model_id: Some("embed".into()),
            classification_enabled: true,
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
    fn classification_and_search_need_the_switch_and_an_installed_embedding_model() {
        let (model, identity) = select_model(&preferences(), Some(embedding_model())).unwrap();
        assert_eq!(model.id, "embed");
        assert_eq!(identity, "c".repeat(64));
        let mut disabled = preferences();
        disabled.classification_enabled = false;
        assert!(select_model(&disabled, Some(embedding_model())).is_none());
        assert!(select_model(&preferences(), None).is_none());
        let mut wrong_kind = embedding_model();
        wrong_kind.kind = "generation".into();
        assert!(select_model(&preferences(), Some(wrong_kind)).is_none());
        let mut downloading = embedding_model();
        downloading.status = "downloading".into();
        assert!(select_model(&preferences(), Some(downloading)).is_none());
    }

    fn fake_controller() -> (tempfile::TempDir, Controller, AiModel) {
        let directory = tempfile::tempdir().unwrap();
        let (executable, path) = super::super::test_worker::fake_engine(directory.path(), "ok");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        let controller = Controller::with_worker(Worker::embedding(&executable));
        let mut model = embedding_model();
        model.path = path.to_string_lossy().into_owned();
        (directory, controller, model)
    }

    #[tokio::test]
    async fn query_vector_uses_the_search_instruction_and_reuses_the_last_query() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        // File integrity has its own tests; the fake GGUF cannot pass it.
        *controller.verified.lock().unwrap() = Some(identity.clone());
        let vector = controller
            .embed_query(directory.path(), &model, &identity, "  생활비   지원 ")
            .await
            .unwrap();
        assert!((vector[0] - 0.6).abs() < 1e-6 && (vector[1] - 0.8).abs() < 1e-6);
        let bodies = std::fs::read_to_string(directory.path().join("embedding-bodies")).unwrap();
        assert!(bodies.contains("Given a web search query"), "{bodies}");
        assert!(bodies.contains("Query: 생활비 지원"), "{bodies}");
        // Paging repeats the query: no second request.
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "생활비 지원")
            .await
            .is_some());
        let requests = || {
            std::fs::read_to_string(directory.path().join("requests"))
                .unwrap()
                .lines()
                .filter(|line| *line == "embedding")
                .count()
        };
        assert_eq!(requests(), 1);
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_some());
        assert_eq!(requests(), 2);
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "   ")
            .await
            .is_none());
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn query_vector_never_starts_a_paused_or_failed_engine() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        let controller = Arc::new(controller);
        {
            let _pause = controller.quiesce().await;
            assert!(controller
                .embed_query(directory.path(), &model, &identity, "장학")
                .await
                .is_none());
        }
        // The fake GGUF fails verification: no request, and the model is marked failed.
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_none());
        assert!(controller.is_failed(&identity));
        assert!(!directory.path().join("requests").exists());
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn pause_released_for_exit_lets_shutdown_finish_and_stays_paused() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        let controller = Arc::new(controller);
        controller.quiesce().await.release_for_exit();
        tokio::time::timeout(Duration::from_secs(5), controller.shutdown())
            .await
            .expect("shutdown must not wait for a pause held until exit");
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_none());
        assert!(!controller.is_failed(&identity));
    }

    #[test]
    fn every_category_but_other_has_prototypes_embedded_like_notices() {
        let inputs = prototype_inputs();
        for category in CATEGORIES {
            let count = inputs
                .iter()
                .filter(|(owner, _)| *owner == category)
                .count();
            assert_eq!(count == 0, category == "기타", "{category}: {count}");
        }
        assert!(inputs.iter().all(|(owner, _)| CATEGORIES.contains(owner)));
        assert!(inputs
            .iter()
            .all(|(_, text)| text.starts_with(INSTRUCTION) && text.len() > INSTRUCTION.len()));
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
