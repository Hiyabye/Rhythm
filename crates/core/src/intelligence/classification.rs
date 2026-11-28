//! Deterministic, local notice classification from embedding similarity.
//!
//! The pinned embedding model produces one unit-length vector per notice
//! revision; the inputs, prototypes and scoring are the shared contract in
//! `rhythm_embed`. A score below [`rhythm_embed::MIN_SCORE`] becomes `기타` with a
//! low-confidence marker kept in SQLite.

use crate::{
    intelligence::{
        model_storage::resolve_model_path,
        models::{self, catalog, verify_file},
        Worker,
    },
    storage::{
        intelligence::{AiModel, AiPreferences},
        Store,
    },
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
/// Resets a "classifying" state left behind by an early return of `advance`
/// (contended lock, pause, shutdown). A state a failure or the end of the pass
/// already set is kept.
struct ClassifyingGuard<'a>(&'a Mutex<&'static str>);

impl Drop for ClassifyingGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if *state == "classifying" {
            *state = "idle";
        }
    }
}

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

/// Pauses after consecutive failed embedding requests of one model identity;
/// the last one repeats.
const EMBED_RETRY_PAUSES: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];

/// A failed embedding request (not an integrity failure): the model is tried
/// again at `until`, with a longer pause after each further failure. Memory
/// only; a success or `invalidate` clears it.
struct EmbedBackoff {
    identity: String,
    failures: usize,
    until: Instant,
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
    backoff: Mutex<Option<EmbedBackoff>>,
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

/// `select_model` on the stored effective preferences.
fn selected_model(store: &Store) -> Result<Option<(AiModel, String)>, String> {
    let preferences = store.ai_effective_preferences()?;
    let model = preferences
        .embedding_model_id
        .as_ref()
        .and_then(|id| store.ai_model(id).ok().flatten());
    Ok(select_model(&preferences, model))
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
            backoff: Mutex::new(None),
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
        self.clear_backoff();
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
        if !self.release_worker(force).await {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            *state = if force { "disabled" } else { "idle" };
        }
        if let Ok(mut detail) = self.detail.lock() {
            *detail = None;
        }
    }

    /// Shuts the engine down once idle (or at once when `force`) without
    /// touching the reported state. False when it stayed up.
    async fn release_worker(&self, force: bool) -> bool {
        let idle = self
            .last_work
            .lock()
            .map(|last| last.is_none_or(|last| last.elapsed() >= IDLE_UNLOAD))
            .unwrap_or(true);
        if !force && !idle {
            return false;
        }
        let Ok(mut worker) = self.worker.try_lock() else {
            return false;
        };
        if worker.status().model.is_some() {
            worker.shutdown().await;
            if let Ok(mut verified) = self.verified.lock() {
                *verified = None;
            }
        }
        true
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
        self.set_failure(identity, "분류 모델 파일을 확인하지 못했습니다.");
    }

    /// A failed embedding request keeps classification and semantic search
    /// off only for a growing pause; a later pass tries the engine again.
    fn embed_failed(&self, identity: &str, detail: &str, now: Instant) {
        if let Ok(mut backoff) = self.backoff.lock() {
            let failures = backoff
                .as_ref()
                .filter(|current| current.identity == identity)
                .map_or(1, |current| current.failures + 1);
            let pause = EMBED_RETRY_PAUSES[(failures - 1).min(EMBED_RETRY_PAUSES.len() - 1)];
            *backoff = Some(EmbedBackoff {
                identity: identity.to_owned(),
                failures,
                until: now + pause,
            });
        }
        if let Ok(mut stored) = self.detail.lock() {
            *stored = Some(detail.to_owned());
        }
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = "failed";
    }

    fn clear_backoff(&self) {
        if let Ok(mut backoff) = self.backoff.lock() {
            *backoff = None;
        }
    }

    fn backing_off(&self, identity: &str, now: Instant) -> bool {
        self.backoff
            .lock()
            .map(|value| {
                value
                    .as_ref()
                    .is_some_and(|current| current.identity == identity && now < current.until)
            })
            .unwrap_or(false)
    }

    /// Permanently failed (until invalidated) or paused after a failed request.
    fn stopped(&self, identity: &str) -> bool {
        self.is_failed(identity) || self.backing_off(identity, Instant::now())
    }

    fn is_failed(&self, identity: &str) -> bool {
        self.failed
            .lock()
            .map(|value| value.as_deref() == Some(identity))
            .unwrap_or(false)
    }

    /// Reads current preferences and processes at most one bounded batch. The
    /// SQLite guard is never held across embedding.
    pub async fn advance(
        &self,
        store: &Mutex<Store>,
        models: &models::Controller,
        data_root: &Path,
        shutdown: &AtomicBool,
    ) {
        if self.paused.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return;
        }
        let selected = {
            let Ok(store) = store.try_lock() else {
                return;
            };
            let Ok(selected) = selected_model(&store) else {
                return;
            };
            selected
        };
        let Some((model, identity)) = selected else {
            self.unload_if_idle(true).await;
            return;
        };
        if self.stopped(&identity) {
            // A failed or paused model keeps its "failed" state and detail.
            self.release_worker(false).await;
            return;
        }
        if models.busy() || self.paused.load(Ordering::Acquire) {
            return;
        }
        let candidates = {
            let Ok(store) = store.try_lock() else {
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
        let _classifying = ClassifyingGuard(&self.state);
        let Ok(_batch) = self.batch.try_lock() else {
            return;
        };
        if self.paused.load(Ordering::Acquire) || shutdown.load(Ordering::Acquire) {
            return;
        }
        let Some((mut worker, path)) = self.local_engine(data_root, &model, &identity).await else {
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
        // Only an answered request shows the engine works again.
        let mut embedded = false;
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
                                self.embed_failed(&identity, &error, Instant::now());
                                return;
                            }
                        }
                    }
                    let loaded = owners
                        .into_iter()
                        .zip(vectors)
                        .collect::<Vec<(&'static str, Vec<f32>)>>();
                    *slot = Some(loaded.clone());
                    embedded = true;
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
                let Ok(store) = store.try_lock() else {
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
                        let Ok(store) = store.try_lock() else {
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
                    embedded = true;
                }
                Err(error) => {
                    eprintln!("Classification embedding: {error}");
                    self.embed_failed(&identity, &error, Instant::now());
                    return;
                }
            }
        }
        if embedded {
            self.clear_backoff();
        }
        drop(worker);
        let mut written = 0u64;
        {
            let Ok(store) = store.try_lock() else {
                return;
            };
            // The request can take minutes. Store nothing when the extension,
            // the switch or the model changed meanwhile; the next pass stops
            // the engine.
            let current = selected_model(&store).ok().flatten();
            if current.is_none_or(|(_, current)| current != identity) {
                return;
            }
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
                    .map_err(|_| "분류 모델 파일을 확인하지 못했습니다.".to_owned())
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
    pub async fn query_vector(
        &self,
        store: &Mutex<Store>,
        models: &models::Controller,
        data_root: &Path,
        query: &str,
    ) -> Option<(String, Vec<f32>)> {
        let (preferences, model) = {
            let store = store.lock().ok()?;
            let preferences = store.ai_effective_preferences().ok()?;
            let model = preferences
                .embedding_model_id
                .as_ref()
                .and_then(|id| store.ai_model(id).ok().flatten());
            (preferences, model)
        };
        let (model, identity) = select_model(&preferences, model)?;
        if models.busy() {
            return None;
        }
        let vector = self
            .embed_query(data_root, &model, &identity, query)
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
        if self.stopped(identity) || self.paused.load(Ordering::Acquire) {
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
                drop(worker);
                self.embed_failed(identity, &error, Instant::now());
                return None;
            }
        };
        drop(worker);
        self.clear_backoff();
        if let Ok(mut last) = self.last_work.lock() {
            *last = Some(Instant::now());
        }
        if let Ok(mut last_query) = self.last_query.lock() {
            *last_query = Some((identity.to_owned(), query, vector.clone()));
        }
        Some(vector)
    }
}

/// The background loop: one bounded batch at a time, woken by new work or
/// every five seconds, until `shutdown`; then the engine is stopped.
pub async fn run(
    classifier: Arc<Controller>,
    store: Arc<Mutex<Store>>,
    models: models::Controller,
    data_root: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = classifier.wake.notified() => {}
        }
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        classifier
            .advance(&store, &models, &data_root, &shutdown)
            .await;
    }
    classifier.shutdown().await;
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

    /// A store with one Cybercampus notice and `model` selected and installed.
    fn store_with_notice(model: &AiModel) -> (tempfile::TempDir, Mutex<Store>) {
        let (data, store) = crate::storage::temp_store();
        store.catalog(&crate::archive::definitions()).unwrap();
        store.set_enabled(crate::extensions::AI, true).unwrap();
        store
            .upsert_article(
                "cybercampus",
                &crate::archive::model::ArticleInput {
                    external_id: "1".into(),
                    canonical_url: "https://eclass.sogang.ac.kr/1".into(),
                    title: "공지".into(),
                    author: None,
                    category: None,
                    published_at: "2026-03-01".into(),
                    modified_at: None,
                    original: "본문".into(),
                    unsupported_body: false,
                    body_html: "<p>본문</p>".into(),
                    body_text: "본문".into(),
                    resources: vec![],
                    etag: None,
                    last_modified: None,
                },
                true,
            )
            .unwrap();
        let mut installed = model.clone();
        installed.bytes = 1;
        installed.downloaded_bytes = 1;
        store.ai_upsert_model(&installed).unwrap();
        let mut preferences = store.ai_preferences().unwrap();
        preferences.embedding_model_id = Some(installed.id.clone());
        preferences.classification_enabled = true;
        store.ai_save_preferences(&preferences).unwrap();
        (data, Mutex::new(store))
    }

    #[tokio::test]
    async fn an_early_return_does_not_leave_the_state_classifying() {
        let (directory, controller, model) = fake_controller();
        let (_data, store) = store_with_notice(&model);
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        // A pass already in flight holds the batch lock.
        let _busy = controller.batch.try_lock().unwrap();
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert_eq!(controller.status().state, "idle");
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

    /// Authored prototypes would need a multi-vector reply the fake cannot
    /// give; seed two so a pass reaches the batch request.
    async fn seed_prototypes(controller: &Controller, identity: &str) {
        *controller.prototypes.lock().await =
            Some(vec![("학사", vec![0.6, 0.8]), ("장학", vec![0.8, 0.6])]);
        *controller.prototypes_sha.lock().unwrap() = Some(identity.to_owned());
    }

    fn set_mode(directory: &Path, mode: &str) {
        std::fs::write(directory.join("mode"), mode).unwrap();
    }

    fn embedding_requests(directory: &Path) -> usize {
        std::fs::read_to_string(directory.join("requests"))
            .map(|text| text.lines().filter(|line| *line == "embedding").count())
            .unwrap_or(0)
    }

    fn pause_of(controller: &Controller) -> Duration {
        let backoff = controller.backoff.lock().unwrap();
        backoff.as_ref().unwrap().until - Instant::now()
    }

    /// Ends the pause as if it had elapsed, without sleeping.
    fn end_pause(controller: &Controller) {
        controller.backoff.lock().unwrap().as_mut().unwrap().until = Instant::now();
    }

    #[test]
    fn consecutive_embedding_failures_pause_one_five_then_fifteen_minutes() {
        let (_directory, controller, _model) = fake_controller();
        let now = Instant::now();
        let mut pauses = Vec::new();
        for _ in 0..5 {
            controller.embed_failed("m", "err", now);
            let until = controller.backoff.lock().unwrap().as_ref().unwrap().until;
            pauses.push(until - now);
        }
        let minutes = |value: u64| Duration::from_secs(value * 60);
        assert_eq!(
            pauses,
            [
                minutes(1),
                minutes(5),
                minutes(15),
                minutes(15),
                minutes(15)
            ]
        );
        assert!(controller.backing_off("m", now + minutes(14)));
        assert!(!controller.backing_off("m", now + minutes(15)));
        // Another model identity starts over, and a success resets the count.
        controller.embed_failed("other", "err", now);
        assert_eq!(
            controller
                .backoff
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .failures,
            1
        );
        controller.clear_backoff();
        controller.embed_failed("other", "err", now);
        assert_eq!(
            controller
                .backoff
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .failures,
            1
        );
    }

    #[tokio::test]
    async fn a_failed_embedding_request_is_retried_after_the_pause_and_search_follows() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        // The fake GGUF cannot pass file integrity, which has its own tests.
        *controller.verified.lock().unwrap() = Some(identity.clone());
        let (_data, store) = store_with_notice(&model);
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        seed_prototypes(&controller, &identity).await;
        set_mode(directory.path(), "crash-on-embed");

        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert_eq!(controller.status().state, "failed");
        assert!(!controller.is_failed(&identity), "no permanent latch");
        let first = pause_of(&controller);
        assert!(first > Duration::from_secs(50) && first <= Duration::from_secs(60));
        let requests = embedding_requests(directory.path());
        assert!(requests > 0);

        // During the pause neither the loop nor search touches the engine.
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_none());
        assert_eq!(embedding_requests(directory.path()), requests);

        // The engine recovers; after the pause classification resumes by itself.
        set_mode(directory.path(), "ok");
        end_pause(&controller);
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert!(controller.backoff.lock().unwrap().is_none());
        assert!(controller.status().processed > 0);
        assert_ne!(controller.status().state, "failed");
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_some());
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_batch_answered_after_the_extension_was_turned_off_is_not_stored() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        *controller.verified.lock().unwrap() = Some(identity.clone());
        let (_data, store) = store_with_notice(&model);
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        seed_prototypes(&controller, &identity).await;
        // The engine holds its answer until the extension is off.
        std::fs::write(directory.path().join("hold"), b"").unwrap();
        let turn_off = async {
            while embedding_requests(directory.path()) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            store
                .lock()
                .unwrap()
                .set_enabled(crate::extensions::AI, false)
                .unwrap();
            std::fs::remove_file(directory.path().join("hold")).unwrap();
        };
        tokio::join!(
            controller.advance(&store, &models, directory.path(), &shutdown),
            turn_off
        );
        assert_eq!(controller.status().processed, 0);
        {
            let store = store.lock().unwrap();
            store.set_enabled(crate::extensions::AI, true).unwrap();
            let waiting = store
                .ai_embedding_candidates(&identity, config_hash(), BATCH)
                .unwrap();
            assert_eq!(waiting.len(), 1, "nothing was stored for the notice");
        }
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn an_answered_search_ends_the_backoff_and_an_idle_pass_does_not() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        *controller.verified.lock().unwrap() = Some(identity.clone());
        let (_data, store) = store_with_notice(&model);
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        seed_prototypes(&controller, &identity).await;
        set_mode(directory.path(), "crash-on-embed");
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert!(controller.backoff.lock().unwrap().is_some());

        // A pass that finds every vector stored asks the engine nothing, so
        // it says nothing about the engine either.
        set_mode(directory.path(), "ok");
        end_pause(&controller);
        let (document_id, revision_id) = store
            .lock()
            .unwrap()
            .ai_embedding_candidates(&identity, config_hash(), BATCH)
            .unwrap()[0];
        let dimension = controller.prototypes.lock().await.as_ref().unwrap()[0]
            .1
            .len();
        let mut vector = vec![0.0; dimension];
        vector[0] = 1.0;
        store
            .lock()
            .unwrap()
            .ai_save_embedding(document_id, revision_id, &identity, &vector)
            .unwrap();
        let requests = embedding_requests(directory.path());
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert_eq!(embedding_requests(directory.path()), requests);
        assert!(controller.backoff.lock().unwrap().is_some());

        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_some());
        assert!(controller.backoff.lock().unwrap().is_none());
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_repeated_failure_lengthens_the_pause_and_invalidate_clears_it() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        *controller.verified.lock().unwrap() = Some(identity.clone());
        let (_data, store) = store_with_notice(&model);
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        seed_prototypes(&controller, &identity).await;
        set_mode(directory.path(), "crash-on-embed");
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        end_pause(&controller);
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        let second = pause_of(&controller);
        assert!(second > Duration::from_secs(290) && second <= Duration::from_secs(300));
        assert!(controller.backing_off(&identity, Instant::now()));
        controller.invalidate();
        assert!(!controller.backing_off(&identity, Instant::now()));
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn an_integrity_failure_still_stops_until_invalidated() {
        let (directory, controller, model) = fake_controller();
        let (_data, store) = store_with_notice(&model);
        let identity = model.sha256.clone().unwrap();
        let models = models::Controller::new("Rhythm/test");
        let shutdown = AtomicBool::new(false);
        // No `verified` mark: the fake GGUF fails the SHA-256 check.
        controller
            .advance(&store, &models, directory.path(), &shutdown)
            .await;
        assert!(controller.is_failed(&identity));
        assert!(controller.backoff.lock().unwrap().is_none());
        controller.invalidate();
        assert!(!controller.is_failed(&identity));
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_stopped_model_keeps_its_failed_state_and_detail_when_the_worker_is_released() {
        for crash in [true, false] {
            let (directory, controller, model) = fake_controller();
            let identity = model.sha256.clone().unwrap();
            let (_data, store) = store_with_notice(&model);
            let models = models::Controller::new("Rhythm/test");
            let shutdown = AtomicBool::new(false);
            if crash {
                // A failed embedding request: paused, not latched.
                *controller.verified.lock().unwrap() = Some(identity.clone());
                seed_prototypes(&controller, &identity).await;
                set_mode(directory.path(), "crash-on-embed");
            }
            // Otherwise the fake GGUF fails integrity: stopped until invalidated.
            controller
                .advance(&store, &models, directory.path(), &shutdown)
                .await;
            let failed = controller.status();
            assert_eq!(failed.state, "failed");
            assert!(failed.detail.is_some());
            // The idle condition is met; the next pass must not wipe the failure.
            *controller.last_work.lock().unwrap() = None;
            controller
                .advance(&store, &models, directory.path(), &shutdown)
                .await;
            let after = controller.status();
            assert_eq!(after.state, "failed", "crash: {crash}");
            assert_eq!(after.detail, failed.detail, "crash: {crash}");
            assert!(controller.worker.lock().await.status().model.is_none());
            controller.shutdown().await;
        }
    }

    #[tokio::test]
    async fn a_failed_search_embedding_starts_the_pause_so_the_next_search_skips_the_engine() {
        let (directory, controller, model) = fake_controller();
        let identity = model.sha256.clone().unwrap();
        *controller.verified.lock().unwrap() = Some(identity.clone());
        set_mode(directory.path(), "crash-on-embed");
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "장학")
            .await
            .is_none());
        let requests = embedding_requests(directory.path());
        assert!(requests > 0);
        assert!(controller.backing_off(&identity, Instant::now()));
        assert!(controller
            .embed_query(directory.path(), &model, &identity, "생활")
            .await
            .is_none());
        assert_eq!(embedding_requests(directory.path()), requests);
        controller.shutdown().await;
    }
}
