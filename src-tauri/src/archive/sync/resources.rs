//! Downloading, verifying and recording notice attachments and images.
use super::*;

impl Board<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn pending_resources<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        policy: &Robots,
        id: i64,
        total: &AtomicU64,
    ) -> Result<(), String> {
        for resource in with_store(store, |s| s.pending_resources(id))? {
            // A persisted resource failure is nonfatal to article/page completion.
            // Cancellation and database failures still propagate.
            download_resource(
                store,
                root,
                transport,
                self.cancel,
                policy,
                &self.controller.image_robots,
                &resource,
                total,
            )
            .await?;
        }
        Ok(())
    }
}

pub(super) const BUDGET_REACHED: &str =
    "이번 실행의 첨부 자료 한도(256 MB)에 도달했습니다. 남은 자료는 다음에 이어서 내려받습니다.";
/// The robots.txt policy of another image host, loaded once per job. A failed
/// load is cached too, so one unreachable host costs one request per job.
pub(super) async fn image_policy<T: Transport>(
    transport: &T,
    cache: &RobotsCache,
    url: &Url,
    cancel: &AtomicBool,
) -> Result<Arc<Robots>, String> {
    let origin = url.origin().ascii_serialization();
    if let Some(cached) = cache.lock().ok().and_then(|c| c.get(&origin).cloned()) {
        return cached;
    }
    let loaded = match Robots::load(transport, url, cancel, network::Access::EmbeddedImage).await {
        Ok(policy) => Ok(Arc::new(policy)),
        // Cancellation must not be remembered as the host's policy.
        Err(error) if cancel.load(Ordering::Acquire) => return Err(error),
        Err(error) => Err(error),
    };
    if let Ok(mut cache) = cache.lock() {
        cache.insert(origin, loaded.clone());
    }
    loaded
}
#[derive(Debug)]
pub enum DownloadOutcome {
    Downloaded,
    Failed(String),
}
impl DownloadOutcome {
    pub fn into_result(self) -> Result<(), String> {
        match self {
            Self::Downloaded => Ok(()),
            Self::Failed(error) => Err(error),
        }
    }
}
pub(super) fn raster(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
#[allow(clippy::too_many_arguments)] // One download shares the job's policies and budget.
pub(super) async fn download_resource<T: Transport>(
    store: &SharedStore,
    root: &Path,
    transport: &T,
    cancel: &AtomicBool,
    policy: &Robots,
    image_robots: &RobotsCache,
    resource: &ResourceRecord,
    total: &AtomicU64,
) -> Result<DownloadOutcome, String> {
    active(cancel)?;
    // Set when the remaining job budget, not the per-file cap, bounds this
    // download. A size failure then says nothing about the file itself.
    let mut budget_bound = false;
    let school = Url::parse(&resource.url)
        .is_ok_and(|url| url.origin().ascii_serialization() == policy.origin);
    let outcome: Result<(), String> = async {
        let url = Url::parse(&resource.url).map_err(|_| "첨부 자료 주소를 해석하지 못했습니다.")?;
        let access = if resource.kind == "image" {
            network::Access::EmbeddedImage
        } else {
            network::Access::Source
        };
        network::validate(&url, access)?;
        // The name alone decides this, so the file is never requested.
        if resource.kind != "image" && !supported_attachment(&resource.filename) {
            return Err(UNSUPPORTED_ATTACHMENT.into());
        }
        let limit = resource_limit(resource, total.load(Ordering::Acquire), &mut budget_bound)?;
        let other;
        let policy = if url.origin().ascii_serialization() == policy.origin {
            policy
        } else {
            other = image_policy(transport, image_robots, &url, cancel).await?;
            &*other
        };
        match fetch(transport, &url, None, limit, cancel, policy, access).await? {
            network::Fetch::Body { bytes, mime, .. } => {
                install_resource(store, root, cancel, resource, bytes, mime, limit, total).await
            }
            _ => Err("첨부 파일에 잘못된 변경 없음 응답이 도착했습니다.".into()),
        }
    }
    .await;
    match outcome {
        Ok(()) => Ok(DownloadOutcome::Downloaded),
        // The board's own server refuses further requests too, so the sync
        // stops and the file stays as it was for a later run.
        Err(error) if school && error == network::RATE_LIMITED => Err(error),
        Err(error) => record_failure(store, root, cancel, resource, error, budget_bound).await,
    }
}
const UNSUPPORTED_ATTACHMENT: &str = "자동 다운로드를 지원하지 않는 첨부 파일 형식입니다.";
fn supported_attachment(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    [
        ".pdf", ".hwp", ".hwpx", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip", ".txt",
        ".png", ".jpg", ".jpeg", ".gif", ".webp",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}
/// The byte limit of one download: the per-file cap or what is left of the
/// job budget, whichever is smaller.
pub(in crate::archive) fn resource_limit(
    resource: &ResourceRecord,
    total: u64,
    budget_bound: &mut bool,
) -> Result<usize, String> {
    let cap = if resource.kind == "image" {
        10 * 1024 * 1024
    } else {
        FILE_LIMIT
    };
    let remaining = IMPORT_RESOURCES.saturating_sub(total) as usize;
    *budget_bound = remaining < cap;
    if remaining == 0 {
        return Err(BUDGET_REACHED.into());
    }
    Ok(cap.min(remaining))
}
/// Validates received bytes and installs them at their generated hash path
/// before the reference is committed. Every transport shares this step.
#[allow(clippy::too_many_arguments)]
pub(in crate::archive) async fn install_resource(
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    resource: &ResourceRecord,
    bytes: Vec<u8>,
    mime: Option<String>,
    limit: usize,
    total: &AtomicU64,
) -> Result<(), String> {
    // Charge received bytes even when format validation or disk writes fail.
    // The transport independently caps the next response to remaining budget.
    // Boards on other hosts charge the same total concurrently, so only bytes
    // that still fit after this charge are installed.
    let charged = total.fetch_add(bytes.len() as u64, Ordering::AcqRel) + bytes.len() as u64;
    if bytes.len() > limit || charged > IMPORT_RESOURCES {
        return Err(BUDGET_REACHED.into());
    }
    let media = if resource.kind == "image" {
        Some(
            raster(&bytes)
                .ok_or("안전한 이미지 형식(PNG, JPEG, GIF, WebP)이 아니어서 보관하지 않았습니다.")?
                .to_string(),
        )
    } else {
        let mime = mime.filter(|m| {
            !matches!(
                m.as_str(),
                "text/html"
                    | "application/xhtml+xml"
                    | "application/javascript"
                    | "text/javascript"
                    | "image/svg+xml"
            )
        });
        if mime.is_none() || bytes.is_empty() {
            return Err("첨부 파일 대신 오류 페이지 또는 지원하지 않는 자료를 받았습니다.".into());
        }
        if !supported_attachment(&resource.filename) {
            return Err(UNSUPPORTED_ATTACHMENT.into());
        }
        if resource.filename.to_ascii_lowercase().ends_with(".pdf") && !bytes.starts_with(b"%PDF-")
        {
            return Err("PDF 파일 형식 검증에 실패했습니다.".into());
        }
        mime
    };
    active(cancel)?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let extension = if resource.kind == "image" {
        match media.as_deref() {
            Some("image/png") => "png",
            Some("image/jpeg") => "jpg",
            Some("image/gif") => "gif",
            _ => "webp",
        }
    } else {
        resource.filename.rsplit('.').next().unwrap_or("bin")
    };
    let extension = extension.to_ascii_lowercase();
    let relative = format!("resources/{hash}.{extension}");
    tokio::fs::create_dir_all(root.join("resources"))
        .await
        .map_err(|_| "첨부 저장 폴더를 만들지 못했습니다. 저장 공간과 권한을 확인해 주세요.")?;
    let temporary = root.join(format!("resources/{hash}.part"));
    tokio::fs::write(&temporary, &bytes)
        .await
        .map_err(|_| "첨부 파일을 저장하지 못했습니다. 남은 저장 공간과 권한을 확인해 주세요.")?;
    let destination = root.join(&relative);
    // Generated hash paths never contain a remote filename. A completed file
    // is installed before its reference is committed to SQLite.
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(|_| "첨부 파일 경로를 확인하지 못했습니다.")?
    {
        // A retry must repair corrupt bytes, not discard its verified download.
        let size = tokio::fs::metadata(&destination)
            .await
            .map_err(|_| "기존 첨부 파일을 확인하지 못했습니다.")?
            .len();
        let matches = if size <= FILE_LIMIT as u64 {
            let existing = tokio::fs::read(&destination)
                .await
                .map_err(|_| "기존 첨부 파일을 확인하지 못했습니다.")?;
            Sha256::digest(&existing) == Sha256::digest(&bytes)
        } else {
            false
        };
        if matches {
            tokio::fs::remove_file(&temporary)
                .await
                .map_err(|_| "임시 첨부 파일을 정리하지 못했습니다.")?;
        } else {
            // Windows does not replace an existing destination on rename.
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|_| "손상된 첨부 파일을 교체하지 못했습니다.")?;
            tokio::fs::rename(&temporary, &destination)
                .await
                .map_err(|_| "첨부 파일 복구를 완료하지 못했습니다.")?;
        }
    } else {
        tokio::fs::rename(&temporary, &destination)
            .await
            .map_err(|_| "첨부 파일 저장을 완료하지 못했습니다.")?;
    }
    with_store(store, |s| {
        ensure_active(cancel)?;
        s.resource_result(
            resource.id,
            "downloaded",
            media.as_deref(),
            Some(bytes.len() as u64),
            Some(&relative),
            Some(&hash),
            None,
        )
    })?;
    Ok(())
}
/// Records a failed download without discarding a verified earlier copy.
pub(in crate::archive) async fn record_failure(
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    resource: &ResourceRecord,
    error: String,
    budget_bound: bool,
) -> Result<DownloadOutcome, String> {
    active(cancel)?;
    if budget_bound && (error == BUDGET_REACHED || error.contains("크기")) {
        // Leave the row as it was (normally pending) for a later run.
        return Ok(DownloadOutcome::Failed(BUDGET_REACHED.into()));
    }
    if resource.status == "downloaded" && local_resource(store, root, resource.id).await.is_ok() {
        return Ok(DownloadOutcome::Failed(format!(
            "다시 내려받지 못했습니다. 기존 로컬 자료는 안전하게 유지됩니다. {error}"
        )));
    }
    let status = if error.contains("크기") || error.contains("한도") {
        "too_large"
    } else if error.contains("지원") || error.contains("검증된") || error.contains("안전한 이미지")
    {
        "unsupported"
    } else {
        "failed"
    };
    with_store(store, |s| {
        s.resource_result(resource.id, status, None, None, None, None, Some(&error))
    })?;
    Ok(DownloadOutcome::Failed(error))
}
pub async fn local_resource(
    store: &SharedStore,
    root: &Path,
    id: i64,
) -> Result<(ResourceRecord, Vec<u8>), String> {
    let resource = with_store(store, |s| s.resource(id))?;
    if resource.status != "downloaded" {
        return Err("이 자료는 아직 로컬에 저장되지 않았습니다.".into());
    }
    let path = root.join(
        resource
            .local_path
            .as_ref()
            .ok_or("로컬 자료 경로가 없습니다.")?,
    );
    let canonical_root = tokio::fs::canonicalize(root.join("resources"))
        .await
        .map_err(|_| "첨부 저장 폴더를 찾지 못했습니다.")?;
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| "로컬 첨부 파일을 찾지 못했습니다. 다시 내려받아 주세요.")?;
    if !canonical.starts_with(canonical_root) {
        return Err("안전하지 않은 로컬 자료 경로입니다.".into());
    }
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| "로컬 첨부 파일을 확인하지 못했습니다.")?;
    if metadata.len() > FILE_LIMIT as u64 {
        return Err("로컬 첨부 파일의 크기가 허용 범위를 초과했습니다.".into());
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|_| "로컬 첨부 파일을 읽지 못했습니다.")?;
    if Some(format!("{:x}", Sha256::digest(&bytes))) != resource.sha256 {
        return Err("로컬 자료의 무결성 검증에 실패했습니다. 다시 내려받아 주세요.".into());
    }
    Ok((resource, bytes))
}
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    raster(bytes)
}
