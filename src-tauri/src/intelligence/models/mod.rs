//! Curated weight installation. This transport is deliberately independent of notice collection.
use crate::storage::{
    intelligence::{AiModel, AiProjector},
    Store,
};
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
use gguf::*;
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
    /// Installs the optional vision projector for an installed model.
    /// Shares the single download slot with model installation.
    pub fn start_projector_admitted(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        model_id: String,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<(), String> {
        self.start_projector_with_transport(
            store,
            root,
            model_id,
            None,
            Some(lease),
            HttpsTransport,
        )
    }
    #[cfg(test)]
    fn start_projector<T: Transport + Send + 'static>(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        model_id: String,
        entry: CatalogEntry,
        transport: T,
    ) -> Result<(), String> {
        self.start_projector_with_transport(store, root, model_id, Some(entry), None, transport)
    }
    fn start_projector_with_transport<T: Transport + Send + 'static>(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        model_id: String,
        entry: Option<CatalogEntry>,
        lease: Option<crate::operation_gate::AdmissionLease>,
        transport: T,
    ) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "모델 설치 상태에 접근하지 못했습니다.")?;
        if active.is_some() {
            return Err("이미 다른 모델이나 이미지 인식 모듈을 설치하고 있습니다.".into());
        }
        let model = with_store(&store, |s| s.ai_model(&model_id))?
            .filter(|model| model.status == "installed")
            .ok_or("이미지 인식을 추가하려면 설치가 완료된 모델을 선택해 주세요.")?;
        let entry = entry
            .or_else(|| {
                catalog()
                    .into_iter()
                    .find(|entry| Some(&entry.id) == model.catalog_id.as_ref())
            })
            .ok_or("이 모델은 배포 목록에 없어 이미지 인식 모듈을 자동으로 받을 수 없습니다.")?;
        validate_manifest(&entry)?;
        if with_store(&store, |s| s.ai_projector(&model_id))?
            .is_some_and(|projector| projector.status == "installed")
        {
            return Err("이미 이미지 인식 모듈이 설치되어 있습니다.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(cancel.clone());
        let this = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            if let Err(error) =
                install_projector(&store, &root, &model, &entry, &cancel, &transport).await
            {
                if !cancel.load(Ordering::Acquire) {
                    eprintln!("AI projector install: {error}");
                }
            }
            if let Ok(mut active) = this.active.lock() {
                *active = None;
            }
        });
        Ok(())
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

pub async fn register(
    store: Arc<Mutex<Store>>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<AiModel, String> {
    check_cancel(&cancel)?;
    if !path.is_absolute()
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Err("기존 GGUF 파일의 전체 경로를 입력해 주세요.".into());
    }
    let path = path
        .canonicalize()
        .map_err(|_| "기존 모델 파일을 찾지 못했습니다.")?;
    if !path.is_file() {
        return Err("일반 GGUF 파일을 선택해 주세요.".into());
    }
    let copy = path.clone();
    let token = cancel.clone();
    let (hash, bytes) = tokio::task::spawn_blocking(move || verify_file(&copy, None, &token))
        .await
        .map_err(|_| "기존 모델 파일 검증이 중단되었습니다.")??;
    check_cancel(&cancel)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("모델 파일 이름을 읽지 못했습니다.")?
        .to_owned();
    let path = path
        .to_str()
        .ok_or("모델 경로를 UTF-8로 읽지 못했습니다.")?
        .to_owned();
    let mut model = AiModel {
        id: format!("external-{:x}", Sha256::digest(path.as_bytes())),
        catalog_id: None,
        name,
        ownership: "external".into(),
        kind: "generation".into(),
        path,
        storage_directory: None,
        sha256: Some(hash),
        expected_sha256: None,
        expected_bytes: None,
        bytes,
        source_revision: None,
        source_filename: None,
        status: "installed".into(),
        downloaded_bytes: bytes,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    with_store(&store, |s| {
        // Restore deliberately keeps the original catalog ID while revoking
        // managed-file ownership. Re-register that row instead of inserting a
        // second external row with the same unique canonical path.
        if let Some(previous) = s
            .ai_models()?
            .into_iter()
            .find(|previous| previous.ownership == "external" && previous.path == model.path)
        {
            model.id = previous.id;
            if previous.expected_sha256 == model.sha256
                && previous.expected_bytes == Some(model.bytes)
            {
                // Only a freshly verified match may retain curated provenance.
                model.catalog_id = previous.catalog_id;
                model.expected_sha256 = previous.expected_sha256;
                model.expected_bytes = previous.expected_bytes;
                model.source_revision = previous.source_revision;
                model.source_filename = previous.source_filename;
                if model.catalog_id.is_some() {
                    model.name = previous.name;
                }
            }
        }
        s.ai_upsert_model(&model)
    })?;
    Ok(model)
}
/// Call after stopping inference and unloading/clearing the selected model. External bytes are never removed.
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
        if s.ai_preferences()?.active_model_id.as_deref() == Some(&id) {
            return Err(
                "사용 중인 모델입니다. 먼저 모델을 언로드하고 선택을 해제해 주세요.".into(),
            );
        }
        let model = s.ai_model(&id)?.ok_or("등록된 모델을 찾지 못했습니다.")?;
        if let Some(hash) = &model.sha256 {
            if s.ai_has_model_jobs(hash)? {
                return Err("이 모델의 분석 작업을 먼저 취소해 주세요.".into());
            }
        }
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
