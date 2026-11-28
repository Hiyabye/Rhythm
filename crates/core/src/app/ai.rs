//! The `ai` extension: the embedding model's inventory and folder, the
//! classification switch, and the stored classifications and similar notices
//! the lists and the reader show. Nothing here sends notice text anywhere; the
//! only network work is an explicit model install.

use super::{storage_error, App};
use crate::{
    extensions::AI,
    intelligence::{classification, model_storage, models},
    storage::{
        intelligence::{AiClassification, AiModel, AiPreferences, SimilarNotice},
        Store,
    },
};
use serde::Serialize;
use std::{path::PathBuf, sync::atomic::Ordering};

const EXIT_ERROR: &str = "앱을 종료하고 있습니다.";
const OFF: &str = "AI 분류 확장 기능이 꺼져 있습니다.";
/// The most notices one list page asks about.
const PAGE: usize = 200;

/// What the AI settings pane shows.
#[derive(Serialize)]
pub struct AiSnapshot {
    pub catalog: Vec<models::CatalogEntry>,
    pub models: Vec<AiModel>,
    pub preferences: AiPreferences,
    pub hardware: models::Hardware,
    pub model_storage: model_storage::Status,
    pub busy: bool,
    pub classification: classification::Status,
}

/// The likely cross-posts of one listed notice.
#[derive(Serialize)]
pub struct DuplicateNotices {
    pub document_id: i64,
    pub duplicates: Vec<SimilarNotice>,
}

pub struct Ai<'a>(pub(super) &'a App);

impl Ai<'_> {
    pub async fn snapshot(&self) -> Result<AiSnapshot, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        self.on()?;
        let (models, preferences) =
            self.with_store(|s| Ok((s.ai_models()?, s.ai_preferences()?)))?;
        let initial_hardware = app.hardware.clone();
        let root = app.data_root.clone();
        let directory = preferences.model_directory.clone();
        let (hardware, model_storage) = tokio::task::spawn_blocking(move || {
            (
                models::refresh_disk_space(initial_hardware, &root),
                model_storage::status(&root, directory.as_deref()),
            )
        })
        .await
        .map_err(|_| "저장 공간 정보를 확인하지 못했습니다.")?;
        Ok(AiSnapshot {
            catalog: models::catalog(),
            models,
            preferences,
            hardware,
            model_storage,
            busy: app.models.busy(),
            classification: app.classifier.status(),
        })
    }

    /// The folder the shell's folder picker should open on. Refused while a
    /// model install runs, so nobody picks a folder the change would then
    /// refuse. No lease outlives this call: backup, restore and updates must
    /// not wait for an open dialog.
    pub async fn model_directory(&self) -> Result<PathBuf, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        self.check_storage_change()?;
        Ok(self
            .with_store(|s| Ok(s.ai_preferences()?.model_directory))?
            .map(PathBuf::from)
            .unwrap_or_else(|| self.0.data_root.clone()))
    }

    /// Makes `path`, a folder the user picked, the place new model downloads
    /// go. Installed models and interrupted downloads keep their folders.
    pub async fn set_model_directory(&self, path: PathBuf) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let _gate = self.0.ai_gate.lock().await;
        self.check_storage_change()?;
        let directory =
            tokio::task::spawn_blocking(move || model_storage::prepare_selection(&path))
                .await
                .map_err(|_| "설치 폴더를 확인하지 못했습니다.")??;
        self.check_storage_change()?;
        self.with_store(|s| s.ai_save_model_directory(Some(&directory)))
    }

    pub async fn reset_model_directory(&self) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let _gate = self.0.ai_gate.lock().await;
        self.check_storage_change()?;
        self.with_store(|s| s.ai_save_model_directory(None))
    }

    /// Starts downloading a catalog model: the only network work of this area.
    pub async fn install(&self, catalog_id: String) -> Result<(), String> {
        let app = self.0;
        let lease = app.operation_gate.shared().await?;
        self.on()?;
        let _gate = app.ai_gate.lock().await;
        self.check_open()?;
        app.models
            .start_admitted(app.store.clone(), app.data_root.clone(), catalog_id, lease)
    }

    /// Cancels a model download. Stopping is never refused for the extension
    /// being off.
    pub async fn cancel_download(&self) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.0.models.cancel();
        Ok(())
    }

    /// Removes a model from the inventory with its vectors and
    /// classifications; `keep_files` leaves a managed model's bytes on disk.
    pub async fn remove(
        &self,
        id: String,
        confirmed: bool,
        keep_files: bool,
    ) -> Result<(), String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        self.on()?;
        if !confirmed {
            return Err("모델 삭제를 확인해 주세요.".into());
        }
        let _gate = app.ai_gate.lock().await;
        self.check_open()?;
        if app.models.busy() {
            return Err("진행 중인 모델 설치를 먼저 취소하거나 완료해 주세요.".into());
        }
        let kind = self.with_store(|s| Ok(s.ai_model(&id)?.map(|model| model.kind)))?;
        let _classifier_paused = if kind.as_deref() == Some("embedding") {
            // Windows cannot unlink a file mapped by the embedding process. Hold
            // classification paused until inventory and file removal finish.
            let guard = app.classifier.quiesce().await;
            app.classifier.stop_worker().await?;
            Some(guard)
        } else {
            None
        };
        models::remove_and_deselect(
            app.store.clone(),
            app.data_root.clone(),
            id,
            confirmed,
            keep_files,
        )
        .await
    }

    /// The only call that changes automatic classification. It quiesces the
    /// running batch so the saved preference and the worker cannot disagree.
    pub async fn set_classification(&self, enabled: bool) -> Result<(), String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        self.on()?;
        let _gate = app.ai_gate.lock().await;
        self.check_open()?;
        let _paused = app.classifier.quiesce().await;
        let result = self.with_store(|store| {
            let mut preferences = store.ai_preferences()?;
            preferences.classification_enabled = enabled;
            store.ai_save_preferences(&preferences)
        });
        app.classifier.invalidate();
        result
    }

    /// Latest-revision classifications for the visible page: the feed's, plus
    /// the local model's for other notices while one is installed.
    pub async fn classifications(
        &self,
        document_ids: &[i64],
    ) -> Result<Vec<AiClassification>, String> {
        if document_ids.len() > PAGE || document_ids.iter().any(|id| *id <= 0) {
            return Err("분류 조회 범위를 확인해 주세요.".into());
        }
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let sha = self.embedding_model_sha()?;
        self.with_store(|store| {
            store.ai_classifications_for_documents(
                document_ids,
                sha.as_deref()
                    .map(|sha| (sha, classification::config_hash())),
            )
        })
    }

    /// Revision-scoped classification for the reader. Older revisions without
    /// a stored row return `None`; they are classified only through the
    /// latest-revision background pass.
    pub async fn classification(
        &self,
        document_id: i64,
        revision_id: i64,
    ) -> Result<Option<AiClassification>, String> {
        if document_id <= 0 || revision_id <= 0 {
            return Err("분류 조회 식별자가 올바르지 않습니다.".into());
        }
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let sha = self.embedding_model_sha()?;
        self.with_store(|store| {
            store.ai_classification(
                document_id,
                revision_id,
                sha.as_deref()
                    .map(|sha| (sha, classification::config_hash())),
            )
        })
    }

    /// Similar notices for the reader, from stored vectors only (no model
    /// call): the feed's, plus the local model's while one is installed.
    pub async fn similar_notices(&self, document_id: i64) -> Result<Vec<SimilarNotice>, String> {
        if document_id <= 0 {
            return Err("비슷한 공지 조회 식별자가 올바르지 않습니다.".into());
        }
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let sha = self.embedding_model_sha()?;
        self.with_store(|store| {
            Ok(store
                .ai_similar_notices(&[document_id], sha.as_deref(), 5)?
                .pop()
                .map(|(_, similar)| similar)
                .unwrap_or_default())
        })
    }

    /// Likely cross-posts of each visible notice, for grouping list rows. Only
    /// documents with at least one duplicate are returned.
    pub async fn duplicate_notices(
        &self,
        document_ids: &[i64],
    ) -> Result<Vec<DuplicateNotices>, String> {
        if document_ids.len() > PAGE || document_ids.iter().any(|id| *id <= 0) {
            return Err("비슷한 공지 조회 범위를 확인해 주세요.".into());
        }
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        let sha = self.embedding_model_sha()?;
        self.with_store(|store| {
            Ok(store
                .ai_similar_notices(document_ids, sha.as_deref(), 1)?
                .into_iter()
                .filter_map(|(document_id, similar)| {
                    let duplicates = similar
                        .into_iter()
                        .filter(|notice| notice.duplicate)
                        .collect::<Vec<_>>();
                    (!duplicates.is_empty()).then_some(DuplicateNotices {
                        document_id,
                        duplicates,
                    })
                })
                .collect())
        })
    }

    /// Stored AI output and the model inventory stay while the extension is
    /// off; they are just not served.
    fn on(&self) -> Result<(), String> {
        if self.with_store(|store| store.enabled(AI))? {
            Ok(())
        } else {
            Err(OFF.into())
        }
    }

    fn check_open(&self) -> Result<(), String> {
        if self.0.maintenance_shutdown.load(Ordering::Acquire) {
            Err(EXIT_ERROR.into())
        } else {
            self.0.operation_gate.ensure_open()
        }
    }

    fn check_storage_change(&self) -> Result<(), String> {
        self.check_open()?;
        if self.0.models.busy() {
            return Err(
                "진행 중인 모델 설치를 완료하거나 취소한 뒤 설치 폴더를 변경해 주세요.".into(),
            );
        }
        Ok(())
    }

    /// SHA-256 of the selected, installed embedding model. Turning automatic
    /// classification off keeps existing rows visible, so the switch is not
    /// checked.
    fn embedding_model_sha(&self) -> Result<Option<String>, String> {
        self.with_store(|store| store.ai_embedding_model_sha())
    }

    fn with_store<T>(&self, f: impl FnOnce(&Store) -> Result<T, String>) -> Result<T, String> {
        let store = self
            .0
            .store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?;
        f(&store).map_err(storage_error)
    }
}
