//! Curated weight installation. This transport is deliberately independent of notice collection.
use crate::storage::{intelligence::AiModel, Store};
use reqwest::header;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    net::{IpAddr, SocketAddr},
    path::{Component, Path, PathBuf},
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
        // Main-process features cannot prove which backend the bundled worker uses.
        backend: "작업 프로세스 빌드에 따름 · 가속 사용 여부 미확인",
    }
}
/// Refresh free disk without repeating the physical-RAM platform probe.
/// Call from a blocking task, outside SQLite/worker locks.
pub fn refresh_disk_space(mut hardware: Hardware, root: &Path) -> Hardware {
    hardware.disk_available_bytes = available_space(root).unwrap_or(0);
    hardware
}
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
    #[cfg(target_os = "linux")]
    {
        return std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .map(str::to_owned)
            })
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|v| v.saturating_mul(1024))
            .unwrap_or(0);
    }
    #[cfg(windows)]
    {
        return std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
    }
    #[allow(unreachable_code)]
    0
}
fn available_space(root: &Path) -> Result<u64, String> {
    #[cfg(unix)]
    let result = std::process::Command::new("df")
        .arg("-Pk")
        .arg(root)
        .output();
    #[cfg(windows)]
    let result = std::process::Command::new("powershell.exe")
        .env("RHYTHM_DISK_PATH", root)
        .args(["-NoProfile", "-NonInteractive", "-Command", "([System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot($env:RHYTHM_DISK_PATH))).AvailableFreeSpace"])
        .output();
    let output = result.map_err(|_| "사용 가능한 저장 공간을 확인하지 못했습니다.")?;
    if !output.status.success() {
        return Err("저장 공간을 확인하지 못했습니다.".into());
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| "저장 공간 응답이 올바르지 않습니다.")?;
    #[cfg(unix)]
    let bytes = text
        .lines()
        .last()
        .and_then(|s| s.split_whitespace().nth(3))
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(|v| v.checked_mul(1024));
    #[cfg(windows)]
    let bytes = text.trim().parse::<u64>().ok();
    bytes.ok_or_else(|| "사용 가능한 저장 공간을 읽지 못했습니다.".into())
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
        if entry.compatibility != "verified" {
            return Err(
                "이 모델은 실제 추론 검증 전이므로 원클릭 설치를 제공하지 않습니다.".into(),
            );
        }
        self.start_entry(store, root, entry)
    }
    fn start_entry(
        &self,
        store: Arc<Mutex<Store>>,
        root: PathBuf,
        entry: CatalogEntry,
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
        if let Some(old) = old {
            // Never overwrite an installed identity during a duplicate request.
            if old.status == "installed" {
                return Err(
                    "이미 설치된 모델입니다. 파일이 손상되었다면 삭제 후 다시 설치해 주세요."
                        .into(),
                );
            }
            if old.expected_sha256 == model.expected_sha256
                && old.source_revision == model.source_revision
            {
                model.resume_etag = old.resume_etag;
                model.downloaded_bytes = old.downloaded_bytes;
            }
        }
        with_store(&store, |s| s.ai_upsert_model(&model))?;
        let cancel = Arc::new(AtomicBool::new(false));
        *active = Some(cancel.clone());
        let this = self.clone();
        tokio::spawn(async move {
            let outcome =
                install(&store, &root, &entry, &mut model, &cancel, &HttpsTransport).await;
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
        path: format!("models/{}.gguf", entry.sha256),
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
            "unsloth/Qwen3.5-9B-GGUF",
        ]
        .contains(&e.repository.as_str())
        || !e.filename.ends_with(".gguf")
        || e.filename.contains(['/', '\\', '\0'])
        || !e.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
fn validate_destination(url: &Url, entry: &CatalogEntry) -> Result<(), String> {
    let approved = match url.host_str() {
        Some("huggingface.co") => {
            url.path() == download_url(entry)?.path()
                && url.query().is_none_or(|q| q == "download=true")
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
        entry: &CatalogEntry,
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
        entry: &CatalogEntry,
        offset: u64,
        etag: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<DownloadResponse, String> {
        let mut url = download_url(entry)?;
        for hop in 0..=5 {
            check_cancel(cancel)?;
            validate_destination(&url, entry)?;
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
fn managed_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let p = Path::new(relative);
    if p.components().any(|c| !matches!(c, Component::Normal(_)))
        || p.components().count() != 2
        || p.parent() != Some(Path::new("models"))
    {
        return Err("Rhythm 모델 저장 경로가 올바르지 않습니다.".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "모델 데이터 경로를 찾지 못했습니다.")?;
    let dir = root.join("models");
    std::fs::create_dir_all(&dir).map_err(|_| "모델 저장 폴더를 만들지 못했습니다.")?;
    let canonical_dir = dir
        .canonicalize()
        .map_err(|_| "모델 저장 폴더를 확인하지 못했습니다.")?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err("모델 저장 폴더의 외부 경로 연결을 차단했습니다.".into());
    }
    let full = dir.join(p.file_name().ok_or("모델 파일 이름이 없습니다.")?);
    for file in [&full, &full.with_extension("part")] {
        if std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file())
        {
            return Err("모델 파일의 외부 경로 연결을 차단했습니다.".into());
        }
    }
    Ok(full)
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
    let full = managed_path(root, &model.path)?;
    if full.exists() {
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
    require_space(available_space(root)?, entry.bytes - offset)?;
    model.status = "downloading".into();
    model.downloaded_bytes = offset;
    model.error = None;
    with_store(store, |s| s.ai_upsert_model(model))?;
    let mut response = tokio::select! {r=transport.response(entry,offset,model.resume_etag.as_deref(),cancel)=>r?,_=cancelled(cancel)=>return check_cancel(cancel)};
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
    with_store(store, |s| s.ai_upsert_model(model))
}

/// Validates the local GGUF container and streams its complete identity. No weights are loaded.
pub fn verify_file(
    path: &Path,
    expected: Option<(&str, u64)>,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    let mut file = File::open(path).map_err(|_| "모델 파일을 열지 못했습니다.")?;
    let size = file
        .metadata()
        .map_err(|_| "모델 파일 크기를 읽지 못했습니다.")?
        .len();
    if size == 0 || size > 32 * 1024 * 1024 * 1024 || expected.is_some_and(|(_, n)| n != size) {
        return Err("모델 파일 크기가 올바르지 않습니다.".into());
    }
    validate_gguf(&mut file, size)?;
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
fn validate_gguf(file: &mut File, size: u64) -> Result<(), String> {
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
            if kind != 8 {
                return Err("GGUF 모델 아키텍처 형식이 올바르지 않습니다.".into());
            }
            let architecture = gguf_string(file, 128)?;
            if !["qwen3", "qwen35"].contains(&architecture.as_str()) {
                return Err(format!(
                    "이 GGUF 아키텍처는 Rhythm에서 검증하지 않았습니다: {architecture}"
                ));
            }
            return Ok(());
        }
        skip_value(file, kind, size, false)?;
    }
    Err("GGUF 모델 아키텍처를 찾지 못했습니다.".into())
}
pub async fn register(
    store: Arc<Mutex<Store>>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<AiModel, String> {
    check_cancel(&cancel)?;
    if !path.is_absolute() || path.extension().is_none_or(|e| e != "gguf") {
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
    let model = AiModel {
        id: format!("external-{:x}", Sha256::digest(path.as_bytes())),
        catalog_id: None,
        name,
        ownership: "external".into(),
        path,
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
    with_store(&store, |s| s.ai_upsert_model(&model))?;
    Ok(model)
}
/// Call after stopping inference and unloading/clearing the selected model. External bytes are never removed.
pub async fn remove(
    store: Arc<Mutex<Store>>,
    root: PathBuf,
    id: String,
    confirmed: bool,
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
    if model.ownership == "managed" {
        let full = managed_path(&root, &model.path)?;
        // Ancillary cleanup can fail independently (for example immutable flags).
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
            _: &CatalogEntry,
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
    #[test]
    fn catalog_provenance_and_destination_policy() {
        let entries = catalog();
        assert_eq!(entries.len(), 3);
        for e in &entries {
            validate_manifest(e).unwrap();
            validate_destination(&download_url(e).unwrap(), e).unwrap();
            assert_eq!(e.quantization, "Q4_K_M");
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
                validate_destination(&Url::parse(bad).unwrap(), e).is_err(),
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
        // Use the unverified candidate, never a promoted downloadable entry:
        // ordinary tests must not start a real weight transfer.
        let unverified = catalog()
            .into_iter()
            .find(|entry| entry.compatibility != "verified")
            .expect("update admission test if all catalog entries become verified");
        assert!(c
            .start(store.clone(), dir.path().into(), unverified.id)
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
            .start_entry(store.clone(), root.clone(), entry)
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
        eprintln!(
            "VERIFIED {} {} bytes {} sha256 {:?}, elapsed {:?}",
            m.name,
            m.bytes,
            root.join(m.path).display(),
            m.sha256,
            start.elapsed()
        );
    }
}
