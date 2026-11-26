//! Deterministic, local notice classification from embedding similarity.
//!
//! The pinned embedding model produces one unit-length vector per notice
//! revision; the inputs, prototypes and scoring are the shared contract in
//! `rhythm_embed`. A score below [`rhythm_embed::MIN_SCORE`] becomes `기타` with a
//! low-confidence marker kept in SQLite.

use crate::{
    intelligence::{
        model_storage::resolve_model_path,
        models::{catalog, verify_file},
        Worker,
    },
    storage::intelligence::{AiModel, AiPreferences},
    AppState,
};
pub use rhythm_embed::config_hash;
use rhythm_embed::{classify_with, input, prototype_inputs, SEARCH_INSTRUCTION};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::sync::{Mutex as AsyncMutex, Notify};

const BATCH: u32 = 8;
/// Release the embedding process after this much idle time. Classification is
/// cheap, but the process still reserves model memory.
const IDLE_UNLOAD: Duration = Duration::from_secs(60);

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
    pub fn new(llama: &Path, runtime: &Path) -> Self {
        let pooling = catalog()
            .into_iter()
            .find(|entry| entry.kind == "embedding")
            .and_then(|entry| entry.pooling)
            .unwrap_or_else(|| "last".into());
        Self::with_worker(Worker::configured(llama, runtime, &pooling))
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
}
