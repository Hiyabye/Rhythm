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
    io::{Read, Seek, SeekFrom},
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub parameters: String,
    pub quantization: String,
    pub bytes: u64,
    /// Conservative estimate, not a measured peak or a promise of available memory.
    pub memory_bytes: u64,
    pub description: String,
    pub use_case: String,
    pub repository: String,
    pub revision: String,
    pub filename: String,
    pub sha256: String,
    pub license: String,
    pub compatibility: String,
    pub reason: Option<String>,
    /// `generation` (default) or `embedding`. Embedding entries are never
    /// selectable for summary analysis and need their own explicit install.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Pooling mode passed to `llama-server --pooling`; embedding entries only.
    #[serde(default)]
    pub pooling: Option<String>,
    /// Declared vector size; `None` for generation entries.
    #[serde(default)]
    pub embedding_dim: Option<u32>,
    /// Optional vision projector (mmproj). Installing it is a separate action.
    #[serde(default)]
    pub projector: Option<ProjectorEntry>,
}
/// A pinned multimodal projector artifact from the same repository/revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectorEntry {
    pub filename: String,
    pub bytes: u64,
    /// Conservative estimate, not a measured peak.
    pub memory_bytes: u64,
    pub sha256: String,
}
fn default_kind() -> String {
    "generation".into()
}
pub fn catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(include_str!("catalog.json")).expect("committed model manifest")
}
#[derive(Clone, Serialize)]
pub struct Hardware {
    pub os: &'static str,
    pub arch: &'static str,
    pub ram_bytes: u64,
    pub disk_available_bytes: u64,
    pub backend: &'static str,
}
pub fn hardware(root: &Path) -> Hardware {
    Hardware {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        ram_bytes: ram_bytes(),
        disk_available_bytes: available_space(root).unwrap_or(0),
        backend: BACKEND_LABEL,
    }
}
/// Refresh free disk without repeating the physical-RAM platform probe.
/// Call from a blocking task, outside SQLite/worker locks.
pub fn refresh_disk_space(mut hardware: Hardware, root: &Path) -> Hardware {
    hardware.disk_available_bytes = available_space(root).unwrap_or(0);
    hardware
}
// Main-process features cannot prove which backend the bundled worker uses;
// on Windows the runtime status separately confirms the device actually used.
#[cfg(windows)]
const BACKEND_LABEL: &str = "CPU · NVIDIA GPU (Vulkan) · 실제 실행 장치는 엔진 상태에서 확인";
#[cfg(not(windows))]
const BACKEND_LABEL: &str = "작업 프로세스 빌드에 따름 · 가속 사용 여부 미확인";
fn ram_bytes() -> u64 {
    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut info = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: info has the required size and remains valid for the call.
        return unsafe { GlobalMemoryStatusEx(&mut info) }
            .map(|()| info.ullTotalPhys)
            .unwrap_or(0);
    }
    #[allow(unreachable_code)]
    0
}
pub(crate) fn available_space(root: &Path) -> Result<u64, String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // SAFETY: wide is NUL-terminated and available is a valid output pointer.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) }
            .map_err(|_| "사용 가능한 저장 공간을 확인하지 못했습니다.")?;
        Ok(available)
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("df")
            .arg("-Pk")
            .arg(root)
            .output()
            .map_err(|_| "사용 가능한 저장 공간을 확인하지 못했습니다.")?;
        if !output.status.success() {
            return Err("저장 공간을 확인하지 못했습니다.".into());
        }
        let text =
            String::from_utf8(output.stdout).map_err(|_| "저장 공간 응답이 올바르지 않습니다.")?;
        let bytes = text
            .lines()
            .last()
            .and_then(|s| s.split_whitespace().nth(3))
            .and_then(|s| s.parse::<u64>().ok())
            .and_then(|v| v.checked_mul(1024));
        bytes.ok_or_else(|| "사용 가능한 저장 공간을 읽지 못했습니다.".into())
    }
}
fn require_space(free: u64, remaining: u64) -> Result<(), String> {
    // Leave room for SQLite/WAL and the OS; never consume the last free bytes.
    if free < remaining.saturating_add(1024 * 1024 * 1024) {
        return Err(
            "모델 설치 공간이 부족합니다. 모델 크기 외에 1 GiB 이상의 여유 공간이 필요합니다."
                .into(),
        );
    }
    Ok(())
}
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
    pub fn stop(&self) {
        self.cancel();
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
        let entry = catalog()
            .into_iter()
            .find(|e| e.id == catalog_id)
            .ok_or("모델 목록에서 항목을 선택해 주세요.")?;
        check_installable(&entry)?;
        self.start_entry_with_lease(store, root, entry, None)
    }
    pub fn start_admitted(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        catalog_id: String,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<(), String> {
        let entry = catalog()
            .into_iter()
            .find(|e| e.id == catalog_id)
            .ok_or("모델 목록에서 항목을 선택해 주세요.")?;
        check_installable(&entry)?;
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
fn inventory(entry: &CatalogEntry) -> AiModel {
    AiModel {
        id: entry.id.clone(),
        catalog_id: Some(entry.id.clone()),
        name: entry.name.clone(),
        ownership: "managed".into(),
        kind: entry.kind.clone(),
        path: format!("models/{}.gguf", entry.sha256),
        storage_directory: None,
        sha256: None,
        expected_sha256: Some(entry.sha256.clone()),
        expected_bytes: Some(entry.bytes),
        bytes: 0,
        source_revision: Some(entry.revision.clone()),
        source_filename: Some(entry.filename.clone()),
        status: "pending".into(),
        downloaded_bytes: 0,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
fn check_installable(entry: &CatalogEntry) -> Result<(), String> {
    if matches!(entry.compatibility.as_str(), "verified" | "available") {
        Ok(())
    } else {
        Err("이 모델은 실제 추론 검증 전이므로 원클릭 설치를 제공하지 않습니다.".into())
    }
}
fn validate_manifest(e: &CatalogEntry) -> Result<(), String> {
    let hex = |s: &str, n| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    };
    if !hex(&e.sha256, 64)
        || !hex(&e.revision, 40)
        || e.bytes == 0
        || e.bytes > 12 * 1024 * 1024 * 1024
        || ![
            "unsloth/Qwen3.5-2B-GGUF",
            "unsloth/Qwen3.5-4B-GGUF",
            "unsloth/gemma-4-E2B-it-GGUF",
            "unsloth/gemma-4-E4B-it-GGUF",
            "Qwen/Qwen3-Embedding-0.6B-GGUF",
        ]
        .contains(&e.repository.as_str())
        || !["generation", "embedding"].contains(&e.kind.as_str())
        || (e.kind == "embedding"
            && (e
                .pooling
                .as_deref()
                .is_none_or(|pooling| !["none", "mean", "cls", "last", "rank"].contains(&pooling))
                || e.embedding_dim.is_none_or(|dim| dim == 0 || dim > 8192)))
        || (e.kind == "generation" && (e.pooling.is_some() || e.embedding_dim.is_some()))
        || !e.filename.ends_with(".gguf")
        || e.filename.contains(['/', '\\', '\0'])
        || !e.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || e.projector.as_ref().is_some_and(|p| {
            !hex(&p.sha256, 64)
                || p.bytes == 0
                || p.bytes > 4 * 1024 * 1024 * 1024
                || p.memory_bytes == 0
                || !p.filename.ends_with(".gguf")
                || p.filename.contains(['/', '\\', '\0'])
        })
    {
        return Err("모델 배포 목록의 고정 식별자나 검증 정보가 올바르지 않습니다.".into());
    }
    Ok(())
}
fn download_url(e: &CatalogEntry) -> Result<Url, String> {
    Url::parse(&format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        e.repository, e.revision, e.filename
    ))
    .map_err(|_| "모델 다운로드 주소가 올바르지 않습니다.".into())
}
fn validate_destination(url: &Url, origin: &Url) -> Result<(), String> {
    let approved = match url.host_str() {
        Some("huggingface.co") => {
            url.path() == origin.path() && url.query().is_none_or(|q| q == "download=true")
        }
        Some(
            "cdn-lfs.huggingface.co"
            | "cdn-lfs-us-1.huggingface.co"
            | "cdn-lfs-eu-1.huggingface.co"
            | "cas-bridge.xethub.hf.co"
            | "us.aws.cdn.hf.co",
        ) => true,
        _ => false,
    };
    if !approved
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 16384
    {
        return Err("검증되지 않은 모델 다운로드 이동 주소를 차단했습니다.".into());
    }
    Ok(())
}
fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && [0, 2].contains(&c))
                || (a == 198 && [18, 19].contains(&b))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}
// The fake transport changes only the network boundary; production disk/hash/resume logic is shared.
trait Transport: Sync {
    fn response(
        &self,
        url: &Url,
        total: u64,
        offset: u64,
        etag: Option<&str>,
        cancel: &AtomicBool,
    ) -> impl std::future::Future<Output = Result<DownloadResponse, String>> + Send;
}
struct DownloadResponse {
    status: u16,
    length: Option<u64>,
    range: Option<String>,
    etag: Option<String>,
    body: Body,
}
enum Body {
    Https(reqwest::Response),
    #[cfg(test)]
    Test(std::collections::VecDeque<Result<Vec<u8>, String>>),
}
impl Body {
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Https(response) => response
                .chunk()
                .await
                .map(|v| v.map(|b| b.to_vec()))
                .map_err(|_| {
                    "모델 연결이 끊어졌습니다. 다시 선택하면 이어받을 수 있습니다.".into()
                }),
            #[cfg(test)]
            Self::Test(chunks) => chunks.pop_front().transpose(),
        }
    }
}
struct HttpsTransport;
impl Transport for HttpsTransport {
    async fn response(
        &self,
        url: &Url,
        _total: u64,
        offset: u64,
        etag: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<DownloadResponse, String> {
        let origin = url.clone();
        let mut url = url.clone();
        for hop in 0..=5 {
            check_cancel(cancel)?;
            validate_destination(&url, &origin)?;
            let host = url.host_str().ok_or("모델 서버 주소가 없습니다.")?;
            let addresses: Vec<SocketAddr> = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::net::lookup_host((host, 443)),
            )
            .await
            .map_err(|_| "모델 서버 주소 조회 시간이 초과되었습니다.")?
            .map_err(|_| "모델 서버 주소를 찾지 못했습니다.")?
            .collect();
            if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
                return Err("공개 인터넷이 아닌 모델 서버 연결을 차단했습니다.".into());
            }
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &addresses)
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(45))
                .pool_max_idle_per_host(0)
                .user_agent(concat!("Rhythm/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| "모델 HTTPS 연결을 준비하지 못했습니다.")?;
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if offset > 0 {
                request = request.header(header::RANGE, format!("bytes={offset}-"));
                if let Some(etag) = etag {
                    request = request.header(header::IF_RANGE, etag);
                }
            }
            let response = tokio::select! {r=request.send()=>r.map_err(|_|"모델 서버에 안전하게 연결하지 못했습니다.")?,_=cancelled(cancel)=>return check_cancel(cancel).and(Err("설치를 취소했습니다.".into()))};
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err("모델 다운로드 이동 횟수가 너무 많습니다.".into());
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or("모델 이동 주소가 없습니다.")?;
                url = url
                    .join(location)
                    .map_err(|_| "모델 이동 주소가 올바르지 않습니다.")?;
                continue;
            }
            if response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|v| v != "identity")
            {
                return Err("압축된 모델 다운로드 응답을 거부했습니다.".into());
            }
            let text_header = |name| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .filter(|s| s.len() <= 4096)
                    .map(str::to_owned)
            };
            return Ok(DownloadResponse {
                status: response.status().as_u16(),
                length: response.content_length(),
                range: text_header(header::CONTENT_RANGE),
                etag: text_header(header::ETAG).filter(|v| strong_etag(v)),
                body: Body::Https(response),
            });
        }
        unreachable!()
    }
}
fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 1024
        && value.starts_with('"')
        && value.ends_with('"')
        && !value.contains(['\r', '\n'])
}
fn validate_response(
    response: &DownloadResponse,
    offset: u64,
    total: u64,
    expected_etag: Option<&str>,
) -> Result<bool, String> {
    match response.status {
        200 => {
            if response.length.is_some_and(|n| n != total) {
                return Err("모델 다운로드 크기가 고정 목록과 다릅니다.".into());
            }
            Ok(false)
        }
        206 if offset > 0 => {
            let expected = format!("bytes {offset}-{}/{total}", total - 1);
            if response.range.as_deref() != Some(expected.as_str())
                || response.length.is_some_and(|n| n != total - offset)
                || expected_etag.is_none()
                || response.etag.as_deref() != expected_etag
            {
                return Err(
                    "이어받기 응답의 범위나 파일 검증 값이 달라졌습니다. 다시 설치해 주세요."
                        .into(),
                );
            }
            Ok(true)
        }
        _ => Err(format!(
            "모델 다운로드에 실패했습니다. (HTTP {})",
            response.status
        )),
    }
}
#[cfg(test)]
fn managed_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    super::model_storage::managed_path(root, relative, None, true)
}
async fn install(
    store: &Arc<Mutex<Store>>,
    root: &Path,
    entry: &CatalogEntry,
    model: &mut AiModel,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
) -> Result<(), String> {
    check_cancel(cancel)?;
    let full = super::model_storage::managed_path(
        root,
        &model.path,
        model.storage_directory.as_deref(),
        true,
    )?;
    if full.exists() {
        // A chosen folder may be shared with another profile or hold a file the
        // user registered as external. Adopting it as managed would let model
        // removal delete bytes Rhythm never wrote, so only adopt there when this
        // inventory already received every byte (a crash after the final rename).
        if model.storage_directory.is_some() && model.downloaded_bytes != entry.bytes {
            return Err("선택한 폴더에 같은 모델 파일이 이미 있습니다. 삭제 권한을 넘겨받지 않도록 설치하지 않았습니다. 기존 파일은 '모델 파일 등록'으로 추가해 주세요.".into());
        }
        let file = full.clone();
        let expected = entry.clone();
        let cancelled = cancel.clone();
        tokio::task::spawn_blocking(move || {
            verify_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
        })
        .await
        .map_err(|_| "모델 검증 작업이 중단되었습니다.")??;
        check_cancel(cancel)?;
        model.status = "installed".into();
        model.sha256 = Some(entry.sha256.clone());
        model.bytes = entry.bytes;
        model.downloaded_bytes = entry.bytes;
        return with_store(store, |s| s.ai_upsert_model(model));
    }
    let part = full.with_extension("part");
    let actual = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if actual == entry.bytes {
        return finalize(store, entry, model, cancel, &part, &full).await;
    }
    let offset = if actual < entry.bytes && model.resume_etag.as_deref().is_some_and(strong_etag) {
        actual
    } else {
        0
    };
    require_space(
        available_space(full.parent().ok_or("모델 저장 폴더가 없습니다.")?)?,
        entry.bytes - offset,
    )?;
    model.status = "downloading".into();
    model.downloaded_bytes = offset;
    model.error = None;
    with_store(store, |s| s.ai_upsert_model(model))?;
    let url = download_url(entry)?;
    let mut response = tokio::select! {r=transport.response(&url,entry.bytes,offset,model.resume_etag.as_deref(),cancel)=>r?,_=cancelled(cancel)=>return check_cancel(cancel)};
    let append =
        match validate_response(&response, offset, entry.bytes, model.resume_etag.as_deref()) {
            Ok(append) => append,
            Err(error) => {
                if response.status == 206 || response.status == 416 {
                    // A changed validator/range cannot be retried as the same prefix.
                    tokio::fs::remove_file(&part)
                        .await
                        .map_err(|_| "잘못된 이어받기 파일을 정리하지 못했습니다.")?;
                    model.downloaded_bytes = 0;
                    model.resume_etag = None;
                }
                return Err(error);
            }
        };
    let mut received = if append { offset } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part)
        .await
        .map_err(|_| "임시 모델 파일을 열지 못했습니다.")?;
    model.resume_etag = response.etag.take();
    model.downloaded_bytes = received;
    with_store(store, |s| s.ai_upsert_model(model))?;
    let start = Instant::now();
    let mut progress = Instant::now();
    loop {
        let next = tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(45), response.body.chunk()) => r.unwrap_or_else(|_| Err("모델 다운로드 응답 시간이 초과되었습니다.".into())),
            _ = cancelled(cancel) => Err("모델 설치를 취소했습니다. 다시 선택하면 이어받습니다.".into()),
        };
        let chunk = match next {
            Ok(chunk) if !cancel.load(Ordering::Acquire) => chunk,
            outcome => {
                // Tokio writes may still be buffered when a network error arrives.
                file.flush()
                    .await
                    .map_err(|_| "임시 모델 저장에 실패했습니다.")?;
                return Err(outcome
                    .err()
                    .unwrap_or_else(|| "모델 설치를 취소했습니다.".into()));
            }
        };
        let Some(chunk) = chunk else { break };
        if start.elapsed() > Duration::from_secs(2 * 60 * 60)
            || chunk.len() as u64 > entry.bytes.saturating_sub(received)
        {
            return Err("모델 다운로드 크기 또는 시간 제한을 초과했습니다.".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| "모델 파일을 저장하지 못했습니다. 저장 공간과 권한을 확인해 주세요.")?;
        received += chunk.len() as u64;
        model.downloaded_bytes = received;
        if progress.elapsed() >= Duration::from_millis(350) {
            with_store(store, |s| s.ai_upsert_model(model))?;
            progress = Instant::now();
        }
    }
    file.flush()
        .await
        .map_err(|_| "모델 파일 저장을 마무리하지 못했습니다.")?;
    file.sync_all()
        .await
        .map_err(|_| "모델 파일을 디스크에 저장하지 못했습니다.")?;
    drop(file);
    if received != entry.bytes {
        return Err("모델 파일을 끝까지 받지 못했습니다. 다시 선택하면 이어받습니다.".into());
    }
    finalize(store, entry, model, cancel, &part, &full).await
}
async fn finalize(
    store: &Arc<Mutex<Store>>,
    entry: &CatalogEntry,
    model: &mut AiModel,
    cancel: &Arc<AtomicBool>,
    part: &Path,
    full: &Path,
) -> Result<(), String> {
    model.status = "verifying".into();
    with_store(store, |s| s.ai_upsert_model(model))?;
    let file = part.to_path_buf();
    let expected = entry.clone();
    let cancelled = cancel.clone();
    let checked = tokio::task::spawn_blocking(move || {
        verify_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
    })
    .await
    .map_err(|_| "모델 무결성 검증이 중단되었습니다.")?;
    if let Err(error) = checked {
        if !cancel.load(Ordering::Acquire) {
            // A failed full-file digest must not be reused as a supposedly safe prefix.
            tokio::fs::remove_file(part)
                .await
                .map_err(|_| "손상된 임시 모델을 정리하지 못했습니다.")?;
            model.downloaded_bytes = 0;
            model.resume_etag = None;
        }
        return Err(error);
    }
    check_cancel(cancel)?;
    tokio::fs::rename(part, full)
        .await
        .map_err(|_| "검증한 모델 설치를 마무리하지 못했습니다.")?;
    model.status = "installed".into();
    model.sha256 = Some(entry.sha256.clone());
    model.bytes = entry.bytes;
    model.downloaded_bytes = entry.bytes;
    model.error = None;
    with_store(store, |s| s.ai_upsert_model(model))?;
    if entry.kind == "embedding" {
        // An explicit embedding install is the opt-in: select it for
        // classification, but never touch the generation model.
        if let Err(error) = with_store(store, |s| {
            let mut preferences = s.ai_preferences()?;
            preferences.embedding_model_id = Some(model.id.clone());
            s.ai_save_preferences(&preferences)
        }) {
            eprintln!("Embedding model selection: {error}");
        }
    }
    Ok(())
}

/// The projector artifact of a catalog entry, resolved from the same pinned
/// repository revision as the text weights.
fn projector_url(entry: &CatalogEntry) -> Result<(Url, &ProjectorEntry), String> {
    let projector = entry
        .projector
        .as_ref()
        .ok_or("이 모델은 이미지 인식을 지원하는 배포 항목이 없습니다.")?;
    let url = Url::parse(&format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        entry.repository, entry.revision, projector.filename
    ))
    .map_err(|_| "이미지 인식 모듈 다운로드 주소가 올바르지 않습니다.")?;
    Ok((url, projector))
}

fn projector_record(model: &AiModel, entry: &CatalogEntry) -> Result<AiProjector, String> {
    let projector = entry
        .projector
        .clone()
        .ok_or("이 모델은 이미지 인식을 지원하는 배포 항목이 없습니다.")?;
    Ok(AiProjector {
        model_id: model.id.clone(),
        path: format!("models/{}.gguf", projector.sha256),
        sha256: None,
        expected_sha256: projector.sha256,
        expected_bytes: projector.bytes,
        bytes: 0,
        status: "pending".into(),
        downloaded_bytes: 0,
        resume_etag: None,
        error: None,
    })
}

/// Installs (or resumes) the vision projector of an installed model. It is a
/// separate artifact: failure never changes the model weights or selection.
async fn install_projector(
    store: &Arc<Mutex<Store>>,
    root: &Path,
    model: &AiModel,
    entry: &CatalogEntry,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
) -> Result<(), String> {
    check_cancel(cancel)?;
    let projector = entry
        .projector
        .clone()
        .ok_or("이 모델은 이미지 인식을 지원하는 배포 항목이 없습니다.")?;
    let full = super::model_storage::managed_path(
        root,
        &format!("models/{}.gguf", projector.sha256),
        model.storage_directory.as_deref(),
        true,
    )?;
    let mut record = projector_record(model, entry)?;
    if let Some(old) = with_store(store, |s| s.ai_projector(&model.id))? {
        if old.status == "installed" {
            return Err("이미 이미지 인식 모듈이 설치되어 있습니다.".into());
        }
        if old.path == record.path && old.expected_sha256 == record.expected_sha256 {
            record.resume_etag = old.resume_etag;
            record.downloaded_bytes = old.downloaded_bytes;
        }
    }
    with_store(store, |s| s.ai_upsert_projector(&record))?;
    let outcome = install_projector_bytes(
        store,
        entry,
        &projector,
        &mut record,
        cancel,
        transport,
        &full,
    )
    .await;
    if let Err(error) = outcome {
        record.status = if cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "failed"
        }
        .into();
        record.error = Some(error.clone());
        if let Err(error) = with_store(store, |s| s.ai_upsert_projector(&record)) {
            eprintln!("AI projector status: {error}");
        }
        return Err(error);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn install_projector_bytes(
    store: &Arc<Mutex<Store>>,
    entry: &CatalogEntry,
    projector: &ProjectorEntry,
    record: &mut AiProjector,
    cancel: &Arc<AtomicBool>,
    transport: &impl Transport,
    full: &Path,
) -> Result<(), String> {
    if full.exists() {
        // A crash after the final rename: adopt only bytes this inventory wrote.
        // A chosen shared folder could already hold a same-named file, so it is
        // adopted only when the recorded download had received every byte.
        if record.downloaded_bytes != projector.bytes {
            return Err("선택한 폴더에 같은 이미지 인식 모듈 파일이 이미 있습니다. 삭제 권한을 넘겨받지 않도록 설치하지 않았습니다.".into());
        }
        let file = full.to_path_buf();
        let expected = projector.clone();
        let cancelled = cancel.clone();
        tokio::task::spawn_blocking(move || {
            verify_projector_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
        })
        .await
        .map_err(|_| "이미지 인식 모듈 검증 작업이 중단되었습니다.")??;
        check_cancel(cancel)?;
        record.status = "installed".into();
        record.sha256 = Some(projector.sha256.clone());
        record.bytes = projector.bytes;
        record.downloaded_bytes = projector.bytes;
        record.error = None;
        return with_store(store, |s| s.ai_upsert_projector(record));
    }
    let part = full.with_extension("part");
    let actual = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if actual == projector.bytes {
        return finalize_projector(store, projector, record, cancel, &part, full).await;
    }
    let offset =
        if actual < projector.bytes && record.resume_etag.as_deref().is_some_and(strong_etag) {
            actual
        } else {
            0
        };
    require_space(
        available_space(full.parent().ok_or("모델 저장 폴더가 없습니다.")?)?,
        projector.bytes - offset,
    )?;
    record.status = "downloading".into();
    record.downloaded_bytes = offset;
    record.error = None;
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let (url, _) = projector_url(entry)?;
    let mut response = tokio::select! {r=transport.response(&url,projector.bytes,offset,record.resume_etag.as_deref(),cancel)=>r?,_=cancelled(cancel)=>return check_cancel(cancel)};
    let append = match validate_response(
        &response,
        offset,
        projector.bytes,
        record.resume_etag.as_deref(),
    ) {
        Ok(append) => append,
        Err(error) => {
            if response.status == 206 || response.status == 416 {
                tokio::fs::remove_file(&part)
                    .await
                    .map_err(|_| "잘못된 이어받기 파일을 정리하지 못했습니다.")?;
                record.downloaded_bytes = 0;
                record.resume_etag = None;
            }
            return Err(error);
        }
    };
    let mut received = if append { offset } else { 0 };
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part)
        .await
        .map_err(|_| "임시 이미지 인식 모듈 파일을 열지 못했습니다.")?;
    record.resume_etag = response.etag.take();
    record.downloaded_bytes = received;
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let start = Instant::now();
    let mut progress = Instant::now();
    loop {
        let next = tokio::select! {
            r = tokio::time::timeout(Duration::from_secs(45), response.body.chunk()) => r.unwrap_or_else(|_| Err("이미지 인식 모듈 다운로드 응답 시간이 초과되었습니다.".into())),
            _ = cancelled(cancel) => Err("이미지 인식 모듈 설치를 취소했습니다. 다시 선택하면 이어받습니다.".into()),
        };
        let chunk = match next {
            Ok(chunk) if !cancel.load(Ordering::Acquire) => chunk,
            outcome => {
                file.flush()
                    .await
                    .map_err(|_| "임시 이미지 인식 모듈 저장에 실패했습니다.")?;
                return Err(outcome
                    .err()
                    .unwrap_or_else(|| "이미지 인식 모듈 설치를 취소했습니다.".into()));
            }
        };
        let Some(chunk) = chunk else { break };
        if start.elapsed() > Duration::from_secs(2 * 60 * 60)
            || chunk.len() as u64 > projector.bytes.saturating_sub(received)
        {
            return Err("이미지 인식 모듈 다운로드 크기 또는 시간 제한을 초과했습니다.".into());
        }
        file.write_all(&chunk).await.map_err(|_| {
            "이미지 인식 모듈 파일을 저장하지 못했습니다. 저장 공간과 권한을 확인해 주세요."
        })?;
        received += chunk.len() as u64;
        record.downloaded_bytes = received;
        if progress.elapsed() >= Duration::from_millis(350) {
            with_store(store, |s| s.ai_upsert_projector(record))?;
            progress = Instant::now();
        }
    }
    file.flush()
        .await
        .map_err(|_| "이미지 인식 모듈 파일 저장을 마무리하지 못했습니다.")?;
    file.sync_all()
        .await
        .map_err(|_| "이미지 인식 모듈 파일을 디스크에 저장하지 못했습니다.")?;
    drop(file);
    if received != projector.bytes {
        return Err(
            "이미지 인식 모듈 파일을 끝까지 받지 못했습니다. 다시 선택하면 이어받습니다.".into(),
        );
    }
    finalize_projector(store, projector, record, cancel, &part, full).await
}

async fn finalize_projector(
    store: &Arc<Mutex<Store>>,
    projector: &ProjectorEntry,
    record: &mut AiProjector,
    cancel: &Arc<AtomicBool>,
    part: &Path,
    full: &Path,
) -> Result<(), String> {
    record.status = "verifying".into();
    with_store(store, |s| s.ai_upsert_projector(record))?;
    let file = part.to_path_buf();
    let expected = projector.clone();
    let cancelled = cancel.clone();
    let checked = tokio::task::spawn_blocking(move || {
        verify_projector_file(&file, Some((&expected.sha256, expected.bytes)), &cancelled)
    })
    .await
    .map_err(|_| "이미지 인식 모듈 무결성 검증이 중단되었습니다.")?;
    if let Err(error) = checked {
        if !cancel.load(Ordering::Acquire) {
            tokio::fs::remove_file(part)
                .await
                .map_err(|_| "손상된 이미지 인식 모듈 파일을 정리하지 못했습니다.")?;
            record.downloaded_bytes = 0;
            record.resume_etag = None;
        }
        return Err(error);
    }
    check_cancel(cancel)?;
    tokio::fs::rename(part, full)
        .await
        .map_err(|_| "검증한 이미지 인식 모듈 설치를 마무리하지 못했습니다.")?;
    record.status = "installed".into();
    record.sha256 = Some(projector.sha256.clone());
    record.bytes = projector.bytes;
    record.downloaded_bytes = projector.bytes;
    record.error = None;
    with_store(store, |s| s.ai_upsert_projector(record))
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

/// Validates the local GGUF container and streams its complete identity. No weights are loaded.
pub fn verify_file(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    verify_artifact(path, expected, cancel, true)
}

/// A projector is a GGUF multimodal encoder, not a text model: its
/// architecture is `clip`, so only the container and the pinned identity apply.
fn verify_projector_file(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    verify_artifact(path, expected, cancel, false)
}

fn verify_artifact(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
    require_architecture: bool,
) -> Result<(String, u64), String> {
    let mut file = File::open(path).map_err(|_| "모델 파일을 열지 못했습니다.")?;
    let size = file
        .metadata()
        .map_err(|_| "모델 파일 크기를 읽지 못했습니다.")?
        .len();
    if size == 0 || size > 32 * 1024 * 1024 * 1024 || expected.is_some_and(|(_, n)| n != size) {
        return Err("모델 파일 크기가 올바르지 않습니다.".into());
    }
    validate_gguf(&mut file, size, require_architecture)?;
    file.rewind().map_err(|_| "모델 파일을 읽지 못했습니다.")?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        check_cancel(cancel)?;
        let n = file
            .read(&mut buffer)
            .map_err(|_| "모델 파일을 검증하지 못했습니다.")?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > size {
            return Err("검증 중 모델 파일이 변경되었습니다.".into());
        }
        hash.update(&buffer[..n]);
    }
    let hash = format!("{:x}", hash.finalize());
    if bytes != size || expected.is_some_and(|(h, _)| h != hash) {
        return Err("모델 SHA-256 무결성 검증에 실패했습니다. 다시 설치해 주세요.".into());
    }
    Ok((hash, size))
}
fn read_u32(file: &mut File) -> Result<u32, String> {
    let mut b = [0; 4];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 메타데이터가 완전하지 않습니다.")?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64(file: &mut File) -> Result<u64, String> {
    let mut b = [0; 8];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 메타데이터가 완전하지 않습니다.")?;
    Ok(u64::from_le_bytes(b))
}
fn gguf_string(file: &mut File, max: u64) -> Result<String, String> {
    let n = read_u64(file)?;
    if n > max {
        return Err("GGUF 메타데이터 문자열이 너무 큽니다.".into());
    }
    let mut b = vec![0; n as usize];
    file.read_exact(&mut b)
        .map_err(|_| "GGUF 문자열을 읽지 못했습니다.")?;
    String::from_utf8(b).map_err(|_| "GGUF 문자열이 UTF-8이 아닙니다.".into())
}
fn skip(file: &mut File, bytes: u64, size: u64) -> Result<(), String> {
    let pos = file
        .stream_position()
        .map_err(|_| "GGUF 위치를 읽지 못했습니다.")?;
    let next = pos
        .checked_add(bytes)
        .filter(|n| *n <= size && *n <= 64 * 1024 * 1024)
        .ok_or("GGUF 메타데이터 범위를 초과했습니다.")?;
    file.seek(SeekFrom::Start(next))
        .map_err(|_| "GGUF 메타데이터를 읽지 못했습니다.")?;
    Ok(())
}
fn skip_value(file: &mut File, kind: u32, size: u64, in_array: bool) -> Result<(), String> {
    match kind {
        0 | 1 | 7 => skip(file, 1, size),
        2 | 3 => skip(file, 2, size),
        4..=6 => skip(file, 4, size),
        10..=12 => skip(file, 8, size),
        8 => {
            let n = read_u64(file)?;
            skip(file, n, size)
        }
        9 if !in_array => {
            let kind = read_u32(file)?;
            let count = read_u64(file)?;
            if count > 1_000_000 {
                return Err("GGUF 배열이 너무 큽니다.".into());
            }
            for _ in 0..count {
                skip_value(file, kind, size, true)?;
            }
            Ok(())
        }
        _ => Err("지원하지 않는 GGUF 메타데이터입니다.".into()),
    }
}
fn validate_gguf(file: &mut File, size: u64, require_architecture: bool) -> Result<(), String> {
    let mut magic = [0; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "GGUF 파일이 완전하지 않습니다.")?;
    if magic != *b"GGUF" || ![2, 3].contains(&read_u32(file)?) {
        return Err("지원하는 GGUF 2/3 형식이 아닙니다.".into());
    }
    let tensors = read_u64(file)?;
    let metadata = read_u64(file)?;
    if tensors == 0 || tensors > 100_000 || metadata == 0 || metadata > 100_000 {
        return Err("GGUF 모델 메타데이터가 올바르지 않습니다.".into());
    }
    for _ in 0..metadata {
        let key = gguf_string(file, 1024)?;
        let kind = read_u32(file)?;
        if key == "general.architecture" {
            if !require_architecture {
                return Ok(());
            }
            if kind != 8 {
                return Err("GGUF 모델 아키텍처 형식이 올바르지 않습니다.".into());
            }
            let architecture = gguf_string(file, 128)?;
            if !["qwen3", "qwen35", "gemma4"].contains(&architecture.as_str()) {
                return Err(format!(
                    "이 GGUF 아키텍처는 Rhythm에서 검증하지 않았습니다: {architecture}"
                ));
            }
            return Ok(());
        }
        skip_value(file, kind, size, false)?;
    }
    if require_architecture {
        Err("GGUF 모델 아키텍처를 찾지 못했습니다.".into())
    } else {
        Ok(())
    }
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
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_hardware_reads_ram_and_space_from_unicode_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("한글 모델 폴더");
        std::fs::create_dir(&path).unwrap();
        assert!(ram_bytes() > 0);
        assert!(available_space(&path).is_ok());
        assert!(available_space(&path.join("없는 폴더")).is_err());
    }
    #[test]
    fn catalog_offers_only_verified_models() {
        let ids: Vec<_> = catalog()
            .into_iter()
            .inspect(|entry| assert_eq!(entry.compatibility, "verified", "{}", entry.id))
            .map(|entry| entry.id)
            .collect();
        // 9B was never run and 0.8B failed the synthetic evaluation; both were removed.
        assert_eq!(
            ids,
            [
                "qwen35-2b",
                "qwen35-4b",
                "gemma4-e2b",
                "gemma4-e4b",
                "qwen3-embedding-0-6b"
            ]
        );
        let mut experimental = catalog().remove(0);
        experimental.compatibility = "experimental".into();
        assert!(check_installable(&experimental).is_err());
    }
    #[test]
    fn sha256_target_features_preserve_standard_known_answers() {
        assert_eq!(
            format!("{:x}", Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(vec![b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
    fn gguf(architecture: &str) -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(20u64.to_le_bytes());
        bytes.extend(b"general.architecture");
        bytes.extend(8u32.to_le_bytes());
        bytes.extend((architecture.len() as u64).to_le_bytes());
        bytes.extend(architecture.as_bytes());
        bytes.extend([0u8; 256]);
        bytes
    }
    fn fixture() -> (tempfile::TempDir, Arc<Mutex<Store>>, CatalogEntry, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Mutex::new(
            Store::open(&dir.path().join("rhythm.sqlite3")).unwrap(),
        ));
        let bytes = gguf("qwen35");
        let mut entry = catalog().remove(0);
        entry.bytes = bytes.len() as u64;
        entry.sha256 = format!("{:x}", Sha256::digest(&bytes));
        (dir, store, entry, bytes)
    }
    struct Fake {
        bytes: Vec<u8>,
        fail: bool,
        ignore_range: bool,
        requests: std::sync::atomic::AtomicUsize,
    }
    impl Fake {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                fail: false,
                ignore_range: false,
                requests: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }
    impl Transport for Fake {
        async fn response(
            &self,
            _: &Url,
            _: u64,
            offset: u64,
            etag: Option<&str>,
            _: &AtomicBool,
        ) -> Result<DownloadResponse, String> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let resume = offset > 0 && !self.ignore_range;
            if resume {
                assert_eq!(etag, Some("\"immutable\""));
            }
            let start = if resume { offset as usize } else { 0 };
            let mut chunks = std::collections::VecDeque::new();
            if self.fail {
                chunks.push_back(Ok(self.bytes[start..start + 32].to_vec()));
                chunks.push_back(Err("interrupted".into()));
            } else {
                chunks.push_back(Ok(self.bytes[start..].to_vec()));
            }
            Ok(DownloadResponse {
                status: if resume { 206 } else { 200 },
                length: Some((self.bytes.len() - start) as u64),
                range: resume.then(|| {
                    format!(
                        "bytes {offset}-{}/{}",
                        self.bytes.len() - 1,
                        self.bytes.len()
                    )
                }),
                etag: Some("\"immutable\"".into()),
                body: Body::Test(chunks),
            })
        }
    }
    struct ObservedFake {
        fake: Fake,
        offsets: Arc<Mutex<Vec<u64>>>,
    }
    impl Transport for ObservedFake {
        async fn response(
            &self,
            url: &Url,
            total: u64,
            offset: u64,
            etag: Option<&str>,
            cancel: &AtomicBool,
        ) -> Result<DownloadResponse, String> {
            self.offsets.lock().unwrap().push(offset);
            self.fake.response(url, total, offset, etag, cancel).await
        }
    }
    async fn wait_for_install(controller: &Controller) {
        tokio::time::timeout(Duration::from_secs(10), controller.wait())
            .await
            .expect("small fake download must finish");
    }
    fn choose_directory(store: &Arc<Mutex<Store>>, directory: &Path) -> String {
        let selected = super::super::model_storage::prepare_selection(directory).unwrap();
        with_store(store, |store| {
            let mut preferences = store.ai_preferences()?;
            preferences.model_directory = Some(selected.clone());
            store.ai_save_preferences(&preferences)
        })
        .unwrap();
        selected
    }
    #[tokio::test]
    async fn custom_korean_directory_download_is_verified_and_persisted() {
        let (root, store, entry, bytes) = fixture();
        let destination = tempfile::tempdir().unwrap();
        let directory = destination.path().join("한글 모델 저장 위치");
        std::fs::create_dir(&directory).unwrap();
        let selected = choose_directory(&store, &directory);
        let controller = Controller::default();
        controller
            .start_entry_with_transport(
                store.clone(),
                root.path().into(),
                entry.clone(),
                None,
                Fake::new(bytes.clone()),
            )
            .unwrap();
        wait_for_install(&controller).await;
        let model = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
        assert_eq!(model.status, "installed", "{:?}", model.error);
        assert_eq!(model.storage_directory.as_deref(), Some(selected.as_str()));
        let full = directory.join(format!("{}.gguf", entry.sha256));
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert_eq!(
            verify_file(
                &full,
                Some((&entry.sha256, entry.bytes)),
                &AtomicBool::new(false)
            )
            .unwrap(),
            (entry.sha256.clone(), entry.bytes)
        );
        assert!(!full.with_extension("part").exists());
        assert!(!root.path().join("models").exists());
        drop(controller);
        drop(store);
        let reopened = Store::open(&root.path().join("rhythm.sqlite3")).unwrap();
        assert_eq!(
            reopened
                .ai_model(&entry.id)
                .unwrap()
                .unwrap()
                .storage_directory,
            Some(selected)
        );
    }
    #[tokio::test]
    async fn existing_file_in_chosen_folder_is_never_adopted_for_deletion() {
        let (root, store, entry, bytes) = fixture();
        let destination = tempfile::tempdir().unwrap();
        let directory = destination.path().join("공유 모델 폴더");
        std::fs::create_dir(&directory).unwrap();
        let selected = choose_directory(&store, &directory);
        // Another profile, or an external registration, already owns this file.
        let foreign = directory.join(format!("{}.gguf", entry.sha256));
        std::fs::write(&foreign, &bytes).unwrap();
        let mut model = inventory(&entry);
        model.storage_directory = Some(selected.clone());
        let error = install(
            &store,
            root.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap_err();
        assert!(error.contains("모델 파일 등록"), "{error}");
        assert_ne!(model.status, "installed");
        assert_eq!(std::fs::read(&foreign).unwrap(), bytes);

        // Rhythm's own download that crashed after the final rename recovers.
        model.downloaded_bytes = entry.bytes;
        install(
            &store,
            root.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        assert_eq!(model.status, "installed");
        assert_eq!(model.storage_directory.as_deref(), Some(selected.as_str()));
    }
    #[tokio::test]
    async fn changing_default_keeps_custom_and_legacy_partial_downloads_in_place() {
        for custom_origin in [false, true] {
            let (root, store, entry, bytes) = fixture();
            let destinations = tempfile::tempdir().unwrap();
            let original = destinations.path().join("기존 모델 폴더");
            let replacement = destinations.path().join("새 다운로드 폴더");
            std::fs::create_dir(&original).unwrap();
            std::fs::create_dir(&replacement).unwrap();
            let previous = custom_origin.then(|| choose_directory(&store, &original));
            let controller = Controller::default();
            let offsets = Arc::new(Mutex::new(Vec::new()));
            let mut interrupted = Fake::new(bytes.clone());
            interrupted.fail = true;
            controller
                .start_entry_with_transport(
                    store.clone(),
                    root.path().into(),
                    entry.clone(),
                    None,
                    ObservedFake {
                        fake: interrupted,
                        offsets: offsets.clone(),
                    },
                )
                .unwrap();
            wait_for_install(&controller).await;
            let failed = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
            assert_eq!(failed.status, "failed");
            let original_full =
                super::super::model_storage::resolve_model_path(root.path(), &failed).unwrap();
            assert_eq!(
                std::fs::read(original_full.with_extension("part")).unwrap(),
                bytes[..32]
            );
            choose_directory(&store, &replacement);
            drop(controller);
            drop(store);
            let store = Arc::new(Mutex::new(
                Store::open(&root.path().join("rhythm.sqlite3")).unwrap(),
            ));
            let controller = Controller::default();
            controller
                .start_entry_with_transport(
                    store.clone(),
                    root.path().into(),
                    entry.clone(),
                    None,
                    ObservedFake {
                        fake: Fake::new(bytes.clone()),
                        offsets: offsets.clone(),
                    },
                )
                .unwrap();
            wait_for_install(&controller).await;
            let model = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
            assert_eq!(model.status, "installed", "{:?}", model.error);
            assert_eq!(model.storage_directory, previous);
            assert_eq!(*offsets.lock().unwrap(), vec![0, 32]);
            assert_eq!(std::fs::read(&original_full).unwrap(), bytes);
            assert!(!original_full.with_extension("part").exists());
            assert_eq!(std::fs::read_dir(&replacement).unwrap().count(), 0);
            if custom_origin {
                assert!(!root.path().join("models").exists());
            }
        }
    }
    #[tokio::test]
    async fn removing_custom_model_preserves_unrelated_files_and_default_model() {
        let (root, store, entry, bytes) = fixture();
        let destination = tempfile::tempdir().unwrap();
        let selected = choose_directory(&store, destination.path());
        let mut custom = inventory(&entry);
        custom.storage_directory = Some(selected);
        let cancel = Arc::new(AtomicBool::new(false));
        install(
            &store,
            root.path(),
            &entry,
            &mut custom,
            &cancel,
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let mut default_entry = entry.clone();
        default_entry.id = "default-fixture".into();
        let mut default_bytes = bytes.clone();
        default_bytes.push(0);
        default_entry.bytes = default_bytes.len() as u64;
        default_entry.sha256 = format!("{:x}", Sha256::digest(&default_bytes));
        let mut default_model = inventory(&default_entry);
        install(
            &store,
            root.path(),
            &default_entry,
            &mut default_model,
            &cancel,
            &Fake::new(default_bytes.clone()),
        )
        .await
        .unwrap();
        let full = destination.path().join(format!("{}.gguf", entry.sha256));
        std::fs::write(full.with_extension("part"), b"stale partial").unwrap();
        let unrelated = destination.path().join("사용자 자료.txt");
        std::fs::write(&unrelated, b"keep this file").unwrap();
        // Even an identically named artifact in the default folder is not this model's file.
        let default_shadow = root.path().join(&custom.path);
        std::fs::write(&default_shadow, b"other default artifact").unwrap();
        remove(store.clone(), root.path().into(), custom.id.clone(), true)
            .await
            .unwrap();
        assert!(!full.exists());
        assert!(!full.with_extension("part").exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep this file");
        assert_eq!(
            std::fs::read(&default_shadow).unwrap(),
            b"other default artifact"
        );
        assert_eq!(
            std::fs::read(root.path().join(&default_model.path)).unwrap(),
            default_bytes
        );
        let store = store.lock().unwrap();
        assert!(store.ai_model(&custom.id).unwrap().is_none());
        assert_eq!(
            store.ai_model(&default_model.id).unwrap().unwrap().status,
            "installed"
        );
        assert!(destination.path().is_dir());
    }
    #[tokio::test]
    async fn missing_custom_directory_never_redirects_install_or_remove_to_default() {
        let (root, store, entry, bytes) = fixture();
        let destinations = tempfile::tempdir().unwrap();
        let original = destinations.path().join("선택한 폴더");
        let retained = destinations.path().join("원본 보존 폴더");
        std::fs::create_dir(&original).unwrap();
        let mut model = inventory(&entry);
        model.storage_directory = Some(choose_directory(&store, &original));
        let cancel = Arc::new(AtomicBool::new(false));
        install(
            &store,
            root.path(),
            &entry,
            &mut model,
            &cancel,
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let filename = format!("{}.gguf", entry.sha256);
        std::fs::write(
            original.join(&filename).with_extension("part"),
            b"keep partial",
        )
        .unwrap();
        // Both directories are direct children of this test's temporary root.
        std::fs::rename(&original, &retained).unwrap();
        let default_file = managed_path(root.path(), &model.path).unwrap();
        std::fs::write(&default_file, b"keep default").unwrap();
        std::fs::write(default_file.with_extension("part"), b"keep default partial").unwrap();
        assert!(super::super::model_storage::resolve_model_path(root.path(), &model).is_err());
        let fake = Fake::new(bytes.clone());
        assert!(
            install(&store, root.path(), &entry, &mut model, &cancel, &fake)
                .await
                .is_err()
        );
        assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
        assert!(
            remove(store.clone(), root.path().into(), model.id.clone(), true)
                .await
                .is_err()
        );
        assert!(!original.exists());
        assert_eq!(std::fs::read(retained.join(&filename)).unwrap(), bytes);
        assert_eq!(
            std::fs::read(retained.join(&filename).with_extension("part")).unwrap(),
            b"keep partial"
        );
        assert_eq!(std::fs::read(&default_file).unwrap(), b"keep default");
        assert_eq!(
            std::fs::read(default_file.with_extension("part")).unwrap(),
            b"keep default partial"
        );
        let saved = store.lock().unwrap().ai_model(&model.id).unwrap().unwrap();
        assert_eq!(saved.status, "installed");
        assert_eq!(saved.storage_directory, model.storage_directory);
        assert!(remove_with_policy(
            store.clone(),
            root.path().into(),
            model.id.clone(),
            false,
            true
        )
        .await
        .is_err());
        remove_with_policy(
            store.clone(),
            root.path().into(),
            model.id.clone(),
            true,
            true,
        )
        .await
        .unwrap();
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
        assert_eq!(std::fs::read(retained.join(&filename)).unwrap(), bytes);
        assert_eq!(std::fs::read(&default_file).unwrap(), b"keep default");
        let replacement = destinations.path().join("새 설치 폴더");
        std::fs::create_dir(&replacement).unwrap();
        choose_directory(&store, &replacement);
        let controller = Controller::default();
        controller
            .start_entry_with_transport(
                store.clone(),
                root.path().into(),
                entry,
                None,
                Fake::new(bytes.clone()),
            )
            .unwrap();
        controller.wait().await;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
        assert_eq!(std::fs::read(replacement.join(&filename)).unwrap(), bytes);
        assert!(!original.exists());
    }
    #[test]
    fn catalog_provenance_and_destination_policy() {
        let entries = catalog();
        assert_eq!(entries.len(), 5);
        for e in &entries {
            validate_manifest(e).unwrap();
            let origin = download_url(e).unwrap();
            validate_destination(&origin, &origin).unwrap();
            if e.kind == "generation" {
                assert_eq!(e.quantization, "Q4_K_M");
            } else {
                assert_eq!(e.kind, "embedding");
                assert_eq!(e.quantization, "Q8_0");
                assert!(e.pooling.is_some() && e.embedding_dim == Some(1024));
            }
        }
        let e = &entries[0];
        for bad in [
            "http://huggingface.co/a",
            "https://huggingface.co/other",
            "https://cas-bridge.xethub.hf.co.evil.test/a",
            "https://user@cas-bridge.xethub.hf.co/a",
            "https://127.0.0.1/a",
            "https://cas-bridge.xethub.hf.co:8443/a",
        ] {
            assert!(
                validate_destination(&Url::parse(bad).unwrap(), &download_url(e).unwrap()).is_err(),
                "{bad}"
            );
        }
        for ip in [
            "127.0.0.1",
            "192.168.0.1",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
    }
    #[test]
    fn resume_headers_and_disk_limits_fail_closed() {
        let body = || Body::Test(Default::default());
        let mut r = DownloadResponse {
            status: 206,
            length: Some(50),
            range: Some("bytes 50-99/100".into()),
            etag: Some("\"a\"".into()),
            body: body(),
        };
        assert!(validate_response(&r, 50, 100, Some("\"a\"")).unwrap());
        assert!(validate_response(&r, 50, 100, None).is_err());
        assert!(validate_response(&r, 50, 100, Some("\"b\"")).is_err());
        r.range = Some("bytes 0-49/100".into());
        assert!(validate_response(&r, 50, 100, Some("\"a\"")).is_err());
        r.status = 200;
        r.length = Some(100);
        assert!(!validate_response(&r, 50, 100, Some("\"a\"")).unwrap());
        r.length = Some(99);
        assert!(validate_response(&r, 0, 100, None).is_err());
        assert!(require_space(1024, 100).is_err());
        assert!(require_space(2 * 1024 * 1024 * 1024, 100).is_ok());
        assert!(!strong_etag("W/\"a\""));
        assert!(strong_etag("\"a\""));
    }
    #[tokio::test]
    async fn interrupted_download_resumes_then_reopen_and_no_duplicate_network() {
        let (dir, store, e, bytes) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut m = inventory(&e);
        with_store(&store, |s| s.ai_upsert_model(&m)).unwrap();
        let mut fake = Fake::new(bytes);
        fake.fail = true;
        assert!(install(&store, dir.path(), &e, &mut m, &cancel, &fake)
            .await
            .is_err());
        assert_eq!(m.downloaded_bytes, 32);
        assert_eq!(
            std::fs::metadata(dir.path().join(&m.path).with_extension("part"))
                .unwrap()
                .len(),
            32
        );
        drop(store);
        let store = Arc::new(Mutex::new(
            Store::open(&dir.path().join("rhythm.sqlite3")).unwrap(),
        ));
        m = store.lock().unwrap().ai_model(&e.id).unwrap().unwrap();
        assert_eq!(m.status, "cancelled");
        // Disk size is authoritative even if the last throttled DB progress was zero.
        assert_eq!(m.downloaded_bytes, 0);
        fake.fail = false;
        install(&store, dir.path(), &e, &mut m, &cancel, &fake)
            .await
            .unwrap();
        assert_eq!(m.status, "installed");
        assert_eq!(m.bytes, e.bytes);
        let requests = fake.requests.load(Ordering::SeqCst);
        install(&store, dir.path(), &e, &mut m, &cancel, &fake)
            .await
            .unwrap();
        assert_eq!(fake.requests.load(Ordering::SeqCst), requests);
        drop(store);
        let reopened = Store::open(&dir.path().join("rhythm.sqlite3")).unwrap();
        assert_eq!(
            reopened.ai_model(&e.id).unwrap().unwrap().sha256,
            Some(e.sha256)
        );
    }
    #[tokio::test]
    async fn completed_partial_recovers_without_network() {
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        let full = managed_path(dir.path(), &model.path).unwrap();
        std::fs::write(full.with_extension("part"), &bytes).unwrap();
        let fake = Fake::new(bytes);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &fake,
        )
        .await
        .unwrap();
        assert_eq!(fake.requests.load(Ordering::SeqCst), 0);
        assert!(full.exists());
        assert!(!full.with_extension("part").exists());
    }
    #[tokio::test]
    async fn range_ignored_restarts_and_corrupt_hash_never_installs() {
        let (dir, store, e, bytes) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut m = inventory(&e);
        let full = managed_path(dir.path(), &m.path).unwrap();
        std::fs::write(full.with_extension("part"), &bytes[..16]).unwrap();
        m.resume_etag = Some("\"immutable\"".into());
        let mut fake = Fake::new(bytes);
        fake.ignore_range = true;
        install(&store, dir.path(), &e, &mut m, &cancel, &fake)
            .await
            .unwrap();
        assert_eq!(std::fs::metadata(&full).unwrap().len(), e.bytes);
        let (dir, store, e, mut bytes) = fixture();
        bytes[100] = 1;
        let mut m = inventory(&e);
        assert!(
            install(&store, dir.path(), &e, &mut m, &cancel, &Fake::new(bytes))
                .await
                .unwrap_err()
                .contains("SHA-256")
        );
        assert!(!dir.path().join(&m.path).exists());
        assert!(!dir.path().join(&m.path).with_extension("part").exists());
        assert_eq!(m.downloaded_bytes, 0);
    }
    #[tokio::test]
    async fn cancellation_unsupported_and_duplicate_admission() {
        let (dir, store, e, bytes) = fixture();
        let cancel = Arc::new(AtomicBool::new(true));
        let mut m = inventory(&e);
        assert!(
            install(&store, dir.path(), &e, &mut m, &cancel, &Fake::new(bytes))
                .await
                .is_err()
        );
        assert!(!dir.path().join("models").exists());
        let c = Controller::default();
        // Use an ID the catalog does not offer, never a downloadable entry:
        // ordinary tests must not start a real weight transfer.
        assert!(c
            .start(store.clone(), dir.path().into(), "qwen35-0-8b".into())
            .is_err());
        *c.active.lock().unwrap() = Some(Arc::new(AtomicBool::new(false)));
        assert!(c
            .start_entry(store, dir.path().into(), catalog().remove(0))
            .unwrap_err()
            .contains("이미"));
        c.cancel();
        assert!(c
            .active
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn restored_external_model_reregisters_the_same_row_and_preserves_original_bytes() {
        let (dir, store, entry, bytes) = fixture();
        let custom = tempfile::tempdir().unwrap();
        let path = custom.path().join(format!("{}.gguf", entry.sha256));
        std::fs::write(&path, &bytes).unwrap();
        let mut restored = inventory(&entry);
        restored.ownership = "external".into();
        restored.path = path.canonicalize().unwrap().to_str().unwrap().into();
        restored.status = "missing".into();
        restored.sha256 = Some(entry.sha256.clone());
        restored.bytes = entry.bytes;
        restored.error = Some("복원 후 다시 등록해 주세요.".into());
        with_store(&store, |store| store.ai_upsert_model(&restored)).unwrap();

        let registered = register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(registered.id, entry.id);
        assert_eq!(registered.ownership, "external");
        assert_eq!(registered.status, "installed");
        assert_eq!(registered.storage_directory, None);
        assert_eq!(registered.catalog_id, Some(entry.id.clone()));
        assert_eq!(registered.expected_sha256, Some(entry.sha256));
        assert_eq!(registered.expected_bytes, Some(entry.bytes));
        assert_eq!(registered.source_revision, Some(entry.revision));
        assert_eq!(registered.source_filename, Some(entry.filename));
        assert_eq!(registered.name, entry.name);
        assert_eq!(registered.error, None);
        assert_eq!(store.lock().unwrap().ai_models().unwrap().len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        remove(store.clone(), dir.path().into(), registered.id, true)
            .await
            .unwrap();
        assert!(store.lock().unwrap().ai_models().unwrap().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn changed_restored_external_file_loses_catalog_provenance_after_verification() {
        let (dir, store, entry, mut bytes) = fixture();
        let path = dir.path().join("replaced.gguf");
        std::fs::write(&path, &bytes).unwrap();
        let mut restored = inventory(&entry);
        restored.ownership = "external".into();
        restored.path = path.canonicalize().unwrap().to_str().unwrap().into();
        restored.status = "missing".into();
        restored.sha256 = Some(entry.sha256.clone());
        restored.bytes = entry.bytes;
        with_store(&store, |store| store.ai_upsert_model(&restored)).unwrap();

        // An unsupported replacement must leave the restored registration alone.
        std::fs::write(&path, gguf("unsupported")).unwrap();
        assert!(register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false))
        )
        .await
        .is_err());
        let unchanged = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
        assert_eq!(unchanged.status, "missing");
        assert_eq!(unchanged.sha256, Some(entry.sha256.clone()));

        // Keep valid GGUF metadata and length, but change the verified identity.
        bytes[100] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        let registered = register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(registered.id, entry.id);
        assert_eq!(
            registered.sha256,
            Some(format!("{:x}", Sha256::digest(&bytes)))
        );
        assert_ne!(registered.sha256, Some(entry.sha256));
        assert_eq!(registered.catalog_id, None);
        assert_eq!(registered.expected_sha256, None);
        assert_eq!(registered.expected_bytes, None);
        assert_eq!(registered.source_revision, None);
        assert_eq!(registered.source_filename, None);
        assert_eq!(registered.name, "replaced.gguf");
        assert_eq!(registered.ownership, "external");
        assert_eq!(registered.storage_directory, None);
        assert_eq!(store.lock().unwrap().ai_models().unwrap().len(), 1);
        remove(store.clone(), dir.path().into(), registered.id, true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn external_registration_is_reference_only_and_never_deletes_bytes() {
        let (dir, store, _, bytes) = fixture();
        let path = dir.path().join("existing.gguf");
        std::fs::write(&path, bytes).unwrap();
        let model = register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(model.ownership, "external");
        assert_eq!(model.path, path.canonicalize().unwrap().to_str().unwrap());
        assert!(!dir.path().join("models").exists());
        let again = register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(again.id, model.id);
        assert_eq!(store.lock().unwrap().ai_models().unwrap().len(), 1);
        assert!(
            remove(store.clone(), dir.path().into(), model.id.clone(), false)
                .await
                .is_err()
        );
        let connection = rusqlite::Connection::open(dir.path().join("rhythm.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER refuse_external_delete BEFORE DELETE ON ai_models BEGIN SELECT RAISE(ABORT, 'simulated inventory failure'); END;").unwrap();
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(
            error.contains("외부 모델 파일은 삭제하지 않았습니다"),
            "{error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), gguf("qwen35"));
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
        connection
            .execute_batch("DROP TRIGGER refuse_external_delete;")
            .unwrap();
        drop(connection);
        remove(store.clone(), dir.path().into(), model.id, true)
            .await
            .unwrap();
        assert!(path.exists());
        assert!(store.lock().unwrap().ai_models().unwrap().is_empty());
        std::fs::write(&path, gguf("unsupported")).unwrap();
        assert!(register(store, path, Arc::new(AtomicBool::new(false)))
            .await
            .is_err());
    }
    #[tokio::test]
    async fn external_registration_accepts_uppercase_gguf_extension() {
        let (dir, store, _, bytes) = fixture();
        let path = dir.path().join("기존 모델.GGUF");
        std::fs::write(&path, &bytes).unwrap();
        let model = register(
            store.clone(),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(model.ownership, "external");
        remove(store, dir.path().into(), model.id, true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[tokio::test]
    async fn managed_delete_clears_files_but_rejects_active_and_path_escape() {
        let (dir, store, e, bytes) = fixture();
        let mut m = inventory(&e);
        let cancel = Arc::new(AtomicBool::new(false));
        install(&store, dir.path(), &e, &mut m, &cancel, &Fake::new(bytes))
            .await
            .unwrap();
        {
            let s = store.lock().unwrap();
            let mut p = s.ai_preferences().unwrap();
            p.active_model_id = Some(m.id.clone());
            s.ai_save_preferences(&p).unwrap();
        }
        assert!(remove(store.clone(), dir.path().into(), m.id.clone(), true)
            .await
            .is_err());
        {
            let s = store.lock().unwrap();
            let mut p = s.ai_preferences().unwrap();
            p.active_model_id = None;
            s.ai_save_preferences(&p).unwrap();
        }
        remove(store.clone(), dir.path().into(), m.id, true)
            .await
            .unwrap();
        assert!(!dir.path().join(m.path).exists());
        assert!(managed_path(dir.path(), "models/../../document").is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(
                outside.path().join("target"),
                dir.path().join("models/evil.gguf"),
            )
            .unwrap();
            assert!(managed_path(dir.path(), "models/evil.gguf").is_err());
        }
    }
    #[tokio::test]
    async fn retired_catalog_model_reopens_and_can_be_removed() {
        // A v1.0.0 profile may hold qwen35-9b, which the catalog no longer offers.
        let (dir, store, mut entry, bytes) = fixture();
        entry.id = "qwen35-9b".into();
        entry.name = "Qwen3.5 9B".into();
        assert!(catalog().iter().all(|current| current.id != entry.id));
        let mut model = inventory(&entry);
        model.status = "installed".into();
        model.sha256 = Some(entry.sha256.clone());
        model.bytes = entry.bytes;
        model.downloaded_bytes = entry.bytes;
        let weights = dir.path().join(&model.path);
        std::fs::create_dir_all(weights.parent().unwrap()).unwrap();
        std::fs::write(&weights, &bytes).unwrap();
        store.lock().unwrap().ai_upsert_model(&model).unwrap();
        drop(store);

        let store = Arc::new(Mutex::new(
            Store::open(&dir.path().join("rhythm.sqlite3")).unwrap(),
        ));
        let reopened = store.lock().unwrap().ai_model(&model.id).unwrap().unwrap();
        assert_eq!(reopened.catalog_id.as_deref(), Some("qwen35-9b"));
        assert_eq!(reopened.status, "installed");
        assert_eq!(std::fs::read(&weights).unwrap(), bytes);

        remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap();
        assert!(!weights.exists());
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
    }
    #[tokio::test]
    async fn invalid_partial_path_preserves_installed_weights_and_inventory() {
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let full = dir.path().join(&model.path);
        std::fs::create_dir(full.with_extension("part")).unwrap();
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        // managed_path rejects directories/symlinks before any unlink occurs.
        assert!(error.contains("외부 경로"), "{error}");
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
    }
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn ancillary_unlink_failure_does_not_delete_installed_weights() {
        struct ClearImmutable(PathBuf);
        impl Drop for ClearImmutable {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/chflags")
                    .arg("nouchg")
                    .arg(&self.0)
                    .status();
            }
        }
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let full = dir.path().join(&model.path);
        let partial = full.with_extension("part");
        std::fs::write(&partial, b"stale partial").unwrap();
        let clear_partial = ClearImmutable(partial.clone());
        let output = std::process::Command::new("/usr/bin/chflags")
            .arg("uchg")
            .arg(&partial)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(error.contains("권한"), "{error}");
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
        // A failure on the final artifact itself restores the installed record.
        drop(clear_partial);
        let _clear_full = ClearImmutable(full.clone());
        assert!(std::process::Command::new("/usr/bin/chflags")
            .arg("uchg")
            .arg(&full)
            .status()
            .unwrap()
            .success());
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(error.contains("권한"), "{error}");
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn locked_ancillary_file_does_not_delete_installed_weights() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let full = dir.path().join(&model.path);
        let partial = full.with_extension("part");
        std::fs::write(&partial, b"stale partial").unwrap();
        // No sharing, including FILE_SHARE_DELETE: removal must fail until closed.
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&partial)
            .unwrap();
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(error.contains("권한"), "{error}");
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
        drop(locked);
        assert_eq!(std::fs::read(&partial).unwrap(), b"stale partial");
        remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap();
        assert!(!full.exists());
        assert!(!partial.exists());
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn locked_installed_file_restores_inventory_and_can_be_removed_after_unlock() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let full = dir.path().join(&model.path);
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&full)
            .unwrap();
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(error.contains("권한"), "{error}");
        let saved = store.lock().unwrap().ai_model(&model.id).unwrap().unwrap();
        assert_eq!(saved.status, "installed");
        assert_eq!(saved.sha256, model.sha256);
        drop(locked);
        assert_eq!(std::fs::read(&full).unwrap(), bytes);
        remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap();
        assert!(!full.exists());
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
    }
    #[tokio::test]
    async fn unavailable_inventory_cannot_be_saved_before_unlink_preserves_weights() {
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes.clone()),
        )
        .await
        .unwrap();
        let connection = rusqlite::Connection::open(dir.path().join("rhythm.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER refuse_model_update BEFORE UPDATE ON ai_models BEGIN SELECT RAISE(ABORT, 'simulated inventory write failure'); END;").unwrap();
        assert!(
            remove(store.clone(), dir.path().into(), model.id.clone(), true)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(dir.path().join(&model.path)).unwrap(), bytes);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .ai_model(&model.id)
                .unwrap()
                .unwrap()
                .status,
            "installed"
        );
    }
    #[tokio::test]
    async fn failed_inventory_delete_records_missing_weights_and_can_be_retried() {
        let (dir, store, entry, bytes) = fixture();
        let mut model = inventory(&entry);
        install(
            &store,
            dir.path(),
            &entry,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes),
        )
        .await
        .unwrap();
        let connection = rusqlite::Connection::open(dir.path().join("rhythm.sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER refuse_model_delete BEFORE DELETE ON ai_models BEGIN SELECT RAISE(ABORT, 'simulated inventory failure'); END;").unwrap();
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(!dir.path().join(&model.path).exists());
        let remaining = store.lock().unwrap().ai_model(&model.id).unwrap().unwrap();
        assert_eq!(remaining.status, "missing", "{error}");
        assert_eq!(remaining.downloaded_bytes, 0);
        assert!(remaining
            .error
            .as_deref()
            .is_some_and(|message| message.contains("삭제")));
        assert_eq!(remaining.sha256, model.sha256);
        assert_eq!(remaining.expected_sha256, model.expected_sha256);
        connection
            .execute_batch("DROP TRIGGER refuse_model_delete;")
            .unwrap();
        drop(connection);
        remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap();
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_none());
    }
    #[tokio::test]
    async fn model_removal_rejects_active_artifact_beyond_global_history_window() {
        use crate::{archive::model::ArticleInput, storage::intelligence::AnalysisKey};
        use serde_json::json;
        let (dir, store, e, bytes) = fixture();
        let mut model = inventory(&e);
        install(
            &store,
            dir.path(),
            &e,
            &mut model,
            &Arc::new(AtomicBool::new(false)),
            &Fake::new(bytes),
        )
        .await
        .unwrap();
        let active;
        {
            let mut s = store.lock().unwrap();
            s.catalog(&crate::archive::definitions()).unwrap();
            let article = ArticleInput {
                external_id: "model-test".into(),
                canonical_url: "https://www.sogang.ac.kr/notice".into(),
                title: "장학 안내".into(),
                author: None,
                category: None,
                published_at: "2026-01-01".into(),
                modified_at: None,
                original: "original".into(),
                unsupported_body: false,
                body_html: "<p>신청 안내</p>".into(),
                body_text: "신청 안내".into(),
                resources: Vec::new(),
                etag: None,
                last_modified: None,
            };
            let (document_id, _) = s
                .upsert_article("sogang-academic", &article, false)
                .unwrap();
            let revision_id = s.document(document_id, None).unwrap().revision_id;
            let key = AnalysisKey {
                document_id,
                revision_id,
                task: "notice_analysis".into(),
                model_sha256: model.sha256.clone().unwrap(),
                prompt_version: 1,
                schema_version: 1,
                configuration_hash: "b".repeat(64),
            };
            active = s.ai_enqueue(&key, &json!({"id":model.id}), 1).unwrap();
            assert_eq!(s.ai_claim_next().unwrap().unwrap().id, active.id);
            for version in 2..=105 {
                let newer = s
                    .ai_enqueue(
                        &AnalysisKey {
                            prompt_version: version,
                            model_sha256: "c".repeat(64),
                            ..key.clone()
                        },
                        &json!({}),
                        1,
                    )
                    .unwrap();
                s.ai_finish_job(newer.id, "failed", None, None).unwrap();
            }
            assert!(!s
                .ai_jobs(100)
                .unwrap()
                .iter()
                .any(|job| job.id == active.id));
        }
        let error = remove(store.clone(), dir.path().into(), model.id.clone(), true)
            .await
            .unwrap_err();
        assert!(error.contains("분석 작업"), "{error}");
        assert!(dir.path().join(&model.path).exists());
        assert!(store.lock().unwrap().ai_model(&model.id).unwrap().is_some());
        store
            .lock()
            .unwrap()
            .ai_finish_job(active.id, "cancelled", None, None)
            .unwrap();
        let saved = {
            let mut s = store.lock().unwrap();
            let key = s.ai_job(active.id).unwrap().unwrap().key;
            let attempt = s.ai_enqueue(&key, &json!({"id":model.id}), 1).unwrap();
            assert_eq!(s.ai_claim_next().unwrap().unwrap().id, attempt.id);
            s.ai_complete_job(attempt.id, &json!({"summary":"기존 분석"}), false, 5, 5, 10)
                .unwrap();
            s.ai_document_results(key.document_id, Some(key.revision_id), 10)
                .unwrap()
                .remove(0)
        };
        remove(store.clone(), dir.path().into(), model.id, true)
            .await
            .unwrap();
        assert!(!dir.path().join(model.path).exists());
        let results = store
            .lock()
            .unwrap()
            .ai_document_results(saved.key.document_id, Some(saved.key.revision_id), 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, saved.id);
        assert_eq!(results[0].result, saved.result);
        assert_eq!(results[0].key.model_sha256, saved.key.model_sha256);
    }
    #[test]
    fn verified_architectures_accept_curated_families() {
        let dir = tempfile::tempdir().unwrap();
        for architecture in ["qwen3", "qwen35", "gemma4"] {
            let path = dir.path().join(format!("{architecture}.gguf"));
            std::fs::write(&path, gguf(architecture)).unwrap();
            verify_file(&path, None, &AtomicBool::new(false)).unwrap();
        }
    }
    #[test]
    fn truncated_and_unsupported_gguf_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.gguf");
        for bytes in [
            b"not-gguf".to_vec(),
            gguf("other"),
            gguf("qwen35")[..40].to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(verify_file(&path, None, &AtomicBool::new(false)).is_err());
        }
    }

    /// Uses the production HTTPS transport and controller, observing only HTTP
    /// status/range validators; no body or secret redirect URL is logged.
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "downloads pinned 2B GGUF; set RHYTHM_MODEL_RESUME_DIR to a new isolated directory under .build-tools"]
    async fn real_curated_model_cancel_resume() {
        #[derive(Clone, Debug)]
        struct Observation {
            offset: u64,
            request_etag: Option<String>,
            status: u16,
            range: Option<String>,
            response_etag: Option<String>,
        }
        #[derive(Clone, Default)]
        struct ObservedHttps(Arc<Mutex<Vec<Observation>>>);
        impl Transport for ObservedHttps {
            async fn response(
                &self,
                url: &Url,
                total: u64,
                offset: u64,
                etag: Option<&str>,
                cancel: &AtomicBool,
            ) -> Result<DownloadResponse, String> {
                let response = HttpsTransport
                    .response(url, total, offset, etag, cancel)
                    .await?;
                self.0.lock().unwrap().push(Observation {
                    offset,
                    request_etag: etag.map(str::to_owned),
                    status: response.status,
                    range: response.range.clone(),
                    response_etag: response.etag.clone(),
                });
                // A full response would redownload the already received prefix.
                // Fail before reading its body; never call that a successful resume.
                if offset > 0 && response.status != 206 {
                    return Err(format!(
                        "resume server returned HTTP {}, not 206",
                        response.status
                    ));
                }
                Ok(response)
            }
        }

        let supplied = PathBuf::from(
            std::env::var_os("RHYTHM_MODEL_RESUME_DIR").expect("set RHYTHM_MODEL_RESUME_DIR"),
        );
        assert!(supplied.is_absolute(), "resume directory must be absolute");
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let safe_base = workspace
            .join(".build-tools")
            .canonicalize()
            .expect("isolated .build-tools directory");
        let parent = supplied
            .parent()
            .expect("resume directory parent")
            .canonicalize()
            .expect("create the resume directory's parent first");
        assert!(
            parent.starts_with(&safe_base),
            "resume directory must be inside the isolated .build-tools directory"
        );
        if !supplied.exists() {
            std::fs::create_dir(&supplied).expect("create isolated resume directory");
        }
        let root = supplied.canonicalize().expect("resolve resume directory");
        assert!(
            root.starts_with(&safe_base) && root != safe_base,
            "invalid isolated resume directory"
        );
        assert!(
            std::fs::read_dir(&root)
                .expect("read resume directory")
                .next()
                .is_none(),
            "resume directory must be new and empty; existing files are never removed"
        );

        let entry = catalog()
            .into_iter()
            .find(|entry| entry.id == "qwen35-2b")
            .unwrap();
        let store = Arc::new(Mutex::new(
            Store::open(&root.join("rhythm.sqlite3")).unwrap(),
        ));
        let manager = Controller::default();
        let observer = ObservedHttps::default();
        let partial = root.join("models").join(format!("{}.part", entry.sha256));
        manager
            .start_entry_with_transport(
                store.clone(),
                root.clone(),
                entry.clone(),
                None,
                observer.clone(),
            )
            .unwrap();
        let progress = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                let bytes = std::fs::metadata(&partial)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                if bytes >= 1024 * 1024 {
                    return Ok(bytes);
                }
                if !manager.busy() {
                    return Err("download ended before a partial file could be cancelled");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        // Cancel even if progress times out or the download fails, and wait for
        // buffered writes and status persistence before inspecting the artifact.
        manager.cancel();
        tokio::time::timeout(Duration::from_secs(60), manager.wait())
            .await
            .expect("cancelled download did not finish cleanup");
        let observed_before_cancel = progress
            .expect("timed out waiting for download progress")
            .expect("first download failed or completed before cancellation");
        let size = std::fs::metadata(&partial)
            .expect("cancelled partial file")
            .len();
        assert!(
            size >= observed_before_cancel && size > 0 && size < entry.bytes,
            "partial download is not resumable"
        );
        let cancelled_model = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
        assert_eq!(
            cancelled_model.status, "cancelled",
            "{:?}",
            cancelled_model.error
        );
        let etag = cancelled_model
            .resume_etag
            .as_deref()
            .filter(|etag| strong_etag(etag))
            .expect("server did not supply a strong ETag; refusing a full restart")
            .to_owned();
        eprintln!(
            "cancelled after {size} of {} bytes; strong ETag recorded",
            entry.bytes
        );

        manager
            .start_entry_with_transport(
                store.clone(),
                root.clone(),
                entry.clone(),
                None,
                observer.clone(),
            )
            .unwrap();
        if tokio::time::timeout(Duration::from_secs(3600), manager.wait())
            .await
            .is_err()
        {
            manager.cancel();
            tokio::time::timeout(Duration::from_secs(60), manager.wait())
                .await
                .expect("resumed download did not stop");
            panic!("resumed download exceeded one hour");
        }
        let completed = store.lock().unwrap().ai_model(&entry.id).unwrap().unwrap();
        assert_eq!(completed.status, "installed", "{:?}", completed.error);
        let observations = observer.0.lock().unwrap().clone();
        assert_eq!(
            observations.len(),
            2,
            "expected one initial and one resumed HTTP response: {observations:?}"
        );
        assert_eq!((observations[0].offset, observations[0].status), (0, 200));
        let resumed = &observations[1];
        assert_eq!(
            resumed.offset, size,
            "request must start at the verified partial size"
        );
        assert_eq!(resumed.status, 206);
        assert_eq!(resumed.request_etag.as_deref(), Some(etag.as_str()));
        assert_eq!(resumed.response_etag.as_deref(), Some(etag.as_str()));
        let expected_range = format!("bytes {size}-{}/{}", entry.bytes - 1, entry.bytes);
        assert_eq!(resumed.range.as_deref(), Some(expected_range.as_str()));
        let file = root.join(&completed.path);
        let (hash, bytes) = verify_file(
            &file,
            Some((&entry.sha256, entry.bytes)),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!((hash.as_str(), bytes), (entry.sha256.as_str(), entry.bytes));
        assert!(
            !partial.exists(),
            "finalization must remove the partial file"
        );
        eprintln!(
            "verified HTTP 206 resume from {size} bytes; final {} bytes SHA-256 {hash}",
            entry.bytes
        );
    }
    /// Downloads only a pinned catalog artifact, into isolated storage, with explicit operator opt-in.
    #[tokio::test]
    #[ignore = "multi-gigabyte HTTPS download; set RHYTHM_MODEL_INSTALL_ID and optionally RHYTHM_MODEL_INSTALL_DIR"]
    async fn real_curated_model_install() {
        let id = std::env::var("RHYTHM_MODEL_INSTALL_ID").expect("set RHYTHM_MODEL_INSTALL_ID");
        let root = std::env::var_os("RHYTHM_MODEL_INSTALL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("rhythm-model-validation"));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(Mutex::new(
            Store::open(&root.join("rhythm.sqlite3")).unwrap(),
        ));
        let entry = catalog()
            .into_iter()
            .find(|e| e.id == id)
            .expect("curated model ID");
        let manager = Controller::default();
        manager
            .start_entry(store.clone(), root.clone(), entry.clone())
            .unwrap();
        let start = Instant::now();
        while manager.busy() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let m = store.lock().unwrap().ai_model(&id).unwrap().unwrap();
            eprintln!(
                "{} {} / {:?} bytes [{}]",
                m.name, m.downloaded_bytes, m.expected_bytes, m.status
            );
        }
        let m = store.lock().unwrap().ai_model(&id).unwrap().unwrap();
        assert_eq!(m.status, "installed", "{:?}", m.error);
        if entry.kind == "embedding" {
            assert_eq!(
                store
                    .lock()
                    .unwrap()
                    .ai_preferences()
                    .unwrap()
                    .embedding_model_id,
                Some(id.clone()),
                "an embedding install selects itself without touching the generation model"
            );
        }
        eprintln!(
            "VERIFIED {} {} bytes {} sha256 {:?}, elapsed {:?}",
            m.name,
            m.bytes,
            root.join(m.path).display(),
            m.sha256,
            start.elapsed()
        );
    }

    fn installed_fixture() -> (tempfile::TempDir, Arc<Mutex<Store>>, CatalogEntry, AiModel) {
        let (dir, store, mut entry, bytes) = fixture();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        let mut model = inventory(&entry);
        std::fs::write(dir.path().join(&model.path), &bytes).unwrap();
        model.status = "installed".into();
        model.sha256 = Some(entry.sha256.clone());
        model.bytes = entry.bytes;
        model.downloaded_bytes = entry.bytes;
        with_store(&store, |s| s.ai_upsert_model(&model)).unwrap();
        // The fake transport serves whatever bytes it is given; keep the catalog
        // entry and the served artifact identical.
        entry.projector = Some(ProjectorEntry {
            filename: "mmproj-F16.gguf".into(),
            bytes: 0,
            memory_bytes: 1024 * 1024 * 1024,
            sha256: "0".repeat(64),
        });
        (dir, store, entry, model)
    }
    fn with_projector(mut entry: CatalogEntry, bytes: &[u8]) -> (CatalogEntry, Vec<u8>) {
        entry.projector = Some(ProjectorEntry {
            filename: "mmproj-F16.gguf".into(),
            bytes: bytes.len() as u64,
            memory_bytes: 1024 * 1024 * 1024,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
        (entry, bytes.to_vec())
    }
    #[tokio::test]
    async fn projector_installs_verifies_and_removes_without_touching_the_model() {
        let (dir, store, entry, model) = installed_fixture();
        let projector_bytes = gguf("clip");
        let (entry, projector_bytes) = with_projector(entry, &projector_bytes);
        validate_manifest(&entry).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        install_projector(
            &store,
            dir.path(),
            &model,
            &entry,
            &cancel,
            &Fake::new(projector_bytes.clone()),
        )
        .await
        .unwrap();
        let record = with_store(&store, |s| s.ai_projector(&model.id))
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "installed");
        assert_eq!(
            record.sha256.as_deref(),
            Some(entry.projector.as_ref().unwrap().sha256.as_str())
        );
        let projector_path = dir.path().join(&record.path);
        assert_eq!(std::fs::read(&projector_path).unwrap(), projector_bytes);
        assert!(dir.path().join(&model.path).exists());
        // A successful install is not repeated over the network.
        assert!(install_projector(
            &store,
            dir.path(),
            &model,
            &entry,
            &cancel,
            &Fake::new(projector_bytes.clone()),
        )
        .await
        .is_err());
        remove_projector(&store, dir.path(), &model.id, false)
            .await
            .unwrap();
        assert!(with_store(&store, |s| s.ai_projector(&model.id))
            .unwrap()
            .is_none());
        assert!(!projector_path.exists());
        let kept = with_store(&store, |s| s.ai_model(&model.id))
            .unwrap()
            .unwrap();
        assert_eq!(kept.status, "installed");
        assert!(dir.path().join(&kept.path).exists());
    }
    #[tokio::test]
    async fn projector_with_a_wrong_digest_is_rejected_and_reported() {
        let (dir, store, entry, model) = installed_fixture();
        let projector_bytes = gguf("clip");
        let (mut entry, _) = with_projector(entry, &projector_bytes);
        entry.projector.as_mut().unwrap().sha256 = "1".repeat(64);
        let cancel = Arc::new(AtomicBool::new(false));
        let error = install_projector(
            &store,
            dir.path(),
            &model,
            &entry,
            &cancel,
            &Fake::new(projector_bytes),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("SHA-256") || error.contains("무결성"),
            "{error}"
        );
        let record = with_store(&store, |s| s.ai_projector(&model.id))
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "failed");
        assert!(!dir
            .path()
            .join(&record.path)
            .with_extension("part")
            .exists());
        assert!(!dir.path().join(&record.path).exists());
    }
    #[tokio::test]
    async fn projector_install_controller_shares_the_download_slot_and_reports_progress() {
        let (dir, store, entry, model) = installed_fixture();
        let projector_bytes = gguf("clip");
        let (entry, projector_bytes) = with_projector(entry, &projector_bytes);
        let controller = Controller::default();
        controller
            .start_projector(
                store.clone(),
                dir.path().into(),
                model.id.clone(),
                entry.clone(),
                Fake::new(projector_bytes),
            )
            .unwrap();
        wait_for_install(&controller).await;
        let record = with_store(&store, |s| s.ai_projector(&model.id))
            .unwrap()
            .unwrap();
        assert_eq!(record.status, "installed");
        // Removal of the model removes its projector row and bytes.
        let projector_path = dir.path().join(&record.path);
        remove_with_policy(
            store.clone(),
            dir.path().into(),
            model.id.clone(),
            true,
            false,
        )
        .await
        .unwrap();
        assert!(!projector_path.exists());
        assert!(with_store(&store, |s| s.ai_projector(&model.id))
            .unwrap()
            .is_none());
        assert!(with_store(&store, |s| s.ai_model(&model.id))
            .unwrap()
            .is_none());
        assert!(dir.path().join(&entry.filename).exists() == entry.filename.is_empty());
    }
}
