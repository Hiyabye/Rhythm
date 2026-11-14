//! Curated weight installation. This transport is deliberately independent of notice collection.
use crate::storage::{intelligence::AiModel, Store};
use reqwest::header;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, Read, Seek},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use url::Url;

mod catalog;
mod download;
mod gguf;
mod hardware;
mod transport;
use catalog::*;
pub use catalog::{catalog, CatalogEntry};
use download::*;
pub use gguf::verify_file;
pub(crate) use hardware::available_space;
use hardware::*;
pub use hardware::{hardware, refresh_disk_space, Hardware};
use transport::*;

fn with_store<T>(
    store: &Arc<Mutex<Store>>,
    f: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, String> {
    let guard = store
        .lock()
        .map_err(|_| "AI 저장소에 접근하지 못했습니다.")?;
    f(&guard)
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("모델 설치를 취소했습니다. 다시 선택하면 안전하게 이어받습니다.".into())
    } else {
        Ok(())
    }
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
#[derive(Clone, Default)]
pub struct Controller {
    active: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}
impl Controller {
    pub fn busy(&self) -> bool {
        self.active.lock().map(|s| s.is_some()).unwrap_or(true)
    }
    pub fn cancel(&self) {
        if let Ok(state) = self.active.lock() {
            if let Some(cancel) = state.as_ref() {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    pub async fn wait(&self) {
        while self.busy() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    #[cfg(test)]
    pub fn start(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        catalog_id: String,
    ) -> Result<(), String> {
        let entry = installable_entry(&catalog_id)?;
        self.start_entry_with_lease(store, root, entry, None)
    }
    pub fn start_admitted(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        catalog_id: String,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<(), String> {
        let entry = installable_entry(&catalog_id)?;
        self.start_entry_with_lease(store, root, entry, Some(lease))
    }
    #[cfg(test)]
    fn start_entry(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        entry: CatalogEntry,
    ) -> Result<(), String> {
        self.start_entry_with_lease(store, root, entry, None)
    }
    fn start_entry_with_lease(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        entry: CatalogEntry,
        lease: Option<crate::operation_gate::AdmissionLease>,
    ) -> Result<(), String> {
        self.start_entry_with_transport(store, root, entry, lease, HttpsTransport)
    }
    fn start_entry_with_transport<T: Transport + Send + 'static>(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        entry: CatalogEntry,
        lease: Option<crate::operation_gate::AdmissionLease>,
        transport: T,
    ) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "모델 설치 상태에 접근하지 못했습니다.")?;
        if active.is_some() {
            return Err("이미 모델을 설치하고 있습니다.".into());
        }
        validate_manifest(&entry)?;
        let old = with_store(&store, |s| s.ai_model(&entry.id))?;
        let mut model = inventory(&entry);
        model.storage_directory = with_store(&store, |s| Ok(s.ai_preferences()?.model_directory))?;
        if let Some(old) = old {
            // Never overwrite an installed identity during a duplicate request.
            if old.status == "installed" {
                return Err(
                    "이미 설치된 모델입니다. 파일이 손상되었다면 삭제 후 다시 설치해 주세요."
                        .into(),
                );
            }
            if old.ownership == "managed"
                && old.path == model.path
                && old.expected_sha256 == model.expected_sha256
                && old.source_revision == model.source_revision
            {
                // Destination is part of an interrupted download's identity.
                // A new default must never abandon or relocate its partial file.
                model.storage_directory = old.storage_directory;
                model.resume_etag = old.resume_etag;
                model.downloaded_bytes = old.downloaded_bytes;
            }
        }
        with_store(&store, |s| s.ai_upsert_model(&model))?;
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(cancel.clone());
        let this = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let outcome = install(&store, &root, &entry, &mut model, &cancel, &transport).await;
            if let Err(error) = outcome {
                model.status = if cancel.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                model.error = Some(error);
                if let Err(error) = with_store(&store, |s| s.ai_upsert_model(&model)) {
                    eprintln!("AI model status: {error}");
                }
            }
            if let Ok(mut active) = this.active.lock() {
                *active = None;
            }
        });
        Ok(())
    }
}
#[cfg(test)]
fn managed_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    super::model_storage::managed_path(root, relative, None, true)
}

/// Removes the projector inventory entry and, unless `keep_files`, its managed bytes.
pub(super) async fn remove_projector(
    store: &Arc<Mutex<Store>>,
    root: &Path,
    model_id: &str,
    keep_files: bool,
) -> Result<(), String> {
    let (projector, model) = with_store(store, |s| {
        Ok((
            s.ai_projector(model_id)?,
            s.ai_model(model_id)?
                .ok_or("등록된 모델을 찾지 못했습니다.")?,
        ))
    })?;
    let Some(projector) = projector else {
        return Ok(());
    };
    if !keep_files {
        if let Ok(full) = super::model_storage::managed_path(
            root,
            &projector.path,
            model.storage_directory.as_deref(),
            false,
        ) {
            for path in [full.clone(), full.with_extension("part")] {
                if let Err(error) = tokio::fs::remove_file(&path).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(
                            "이미지 인식 모듈 파일을 삭제하지 못했습니다. 권한을 확인해 주세요."
                                .into(),
                        );
                    }
                }
            }
        }
    }
    with_store(store, |s| s.ai_remove_projector(model_id))
}

/// Call after stopping the embedding process that may map the file. External bytes are never removed.
pub async fn remove(
    store: Arc<Mutex<Store>>,
    root: PathBuf,
    id: String,
    confirmed: bool,
) -> Result<(), String> {
    remove_with_policy(store, root, id, confirmed, false).await
}

pub async fn remove_with_policy(
    store: Arc<Mutex<Store>>,
    root: PathBuf,
    id: String,
    confirmed: bool,
    keep_files: bool,
) -> Result<(), String> {
    if !confirmed {
        return Err("모델 삭제를 먼저 확인해 주세요.".into());
    }
    let model = with_store(&store, |s| {
        let model = s.ai_model(&id)?.ok_or("등록된 모델을 찾지 못했습니다.")?;
        if ["pending", "downloading", "verifying"].contains(&model.status.as_str()) {
            return Err("모델 설치를 먼저 취소하고 완료될 때까지 기다려 주세요.".into());
        }
        Ok(model)
    })?;
    // The projector belongs to this model; remove its managed bytes before the
    // weights so a failure never leaves the inventory pretending it is gone.
    remove_projector(&store, &root, &id, keep_files).await?;
    if keep_files {
        // Explicit inventory-only recovery also works when a drive is gone.
        // Do not resolve or access any model/partial path in this branch.
        return with_store(&store, |s| s.ai_remove_model(&id));
    }
    if model.ownership == "managed" {
        let full = super::model_storage::resolve_model_path(&root, &model)?;
        // Ancillary cleanup can fail independently (for example an exclusive file lock).
        // Never remove good installed weights before that failure is known.
        if let Err(error) = tokio::fs::remove_file(full.with_extension("part")).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err("모델 파일을 삭제하지 못했습니다. 권한을 확인해 주세요.".into());
            }
        }
        // Filesystem and SQLite cannot share a transaction. Make the inventory
        // unavailable before unlinking, so even a later database failure/crash
        // cannot leave missing weights marked installed. Preserve provenance.
        let mut missing = model.clone();
        missing.status = "missing".into();
        missing.downloaded_bytes = 0;
        missing.resume_etag = None;
        missing.error = Some(
            "모델 삭제가 완료되지 않았습니다. 다시 삭제하거나 설치 상태를 확인해 주세요.".into(),
        );
        with_store(&store, |s| s.ai_upsert_model(&missing)).map_err(|error| {
            eprintln!("Model removal state: {error}");
            "모델 목록을 갱신하지 못해 삭제를 중단했습니다. 저장 공간과 파일 권한을 확인해 주세요."
                .to_owned()
        })?;
        if let Err(error) = tokio::fs::remove_file(full).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                // No weights were removed. Restore availability when the store
                // permits it; otherwise leave the conservative missing state.
                if let Err(error) = with_store(&store, |s| s.ai_upsert_model(&model)) {
                    eprintln!("Model removal reconciliation: {error}");
                    return Err("모델 파일을 삭제하지 못했고 목록 상태도 복원하지 못했습니다. 파일 권한과 설치 상태를 확인해 주세요.".into());
                }
                return Err("모델 파일을 삭제하지 못했습니다. 권한을 확인해 주세요.".into());
            }
        }
    }
    with_store(&store, |s| s.ai_remove_model(&id)).map_err(|error| {
        eprintln!("Model inventory removal: {error}");
        if model.ownership == "managed" {
            "모델 파일은 삭제했지만 목록에서 제거하지 못했습니다. 다시 삭제하거나 설치해 주세요."
                .into()
        } else {
            "모델 등록 정보를 제거하지 못했습니다. 외부 모델 파일은 삭제하지 않았습니다. 저장 공간과 파일 권한을 확인해 주세요.".into()
        }
    })
}

#[cfg(test)]
mod tests;
