use super::*;

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
pub(super) fn default_kind() -> String {
    "generation".into()
}
pub fn catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(include_str!("../catalog.json")).expect("committed model manifest")
}
pub(super) fn inventory(entry: &CatalogEntry) -> AiModel {
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
pub(super) fn installable_entry(catalog_id: &str) -> Result<CatalogEntry, String> {
    let entry = catalog()
        .into_iter()
        .find(|e| e.id == catalog_id)
        .ok_or("모델 목록에서 항목을 선택해 주세요.")?;
    check_installable(&entry)?;
    Ok(entry)
}
pub(super) fn check_installable(entry: &CatalogEntry) -> Result<(), String> {
    if matches!(entry.compatibility.as_str(), "verified" | "available") {
        Ok(())
    } else {
        Err("이 모델은 실제 추론 검증 전이므로 원클릭 설치를 제공하지 않습니다.".into())
    }
}
pub(super) fn validate_manifest(e: &CatalogEntry) -> Result<(), String> {
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
pub(super) fn download_url(e: &CatalogEntry) -> Result<Url, String> {
    Url::parse(&format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        e.repository, e.revision, e.filename
    ))
    .map_err(|_| "모델 다운로드 주소가 올바르지 않습니다.".into())
}
/// The projector artifact of a catalog entry, resolved from the same pinned
/// repository revision as the text weights.
pub(super) fn projector_url(entry: &CatalogEntry) -> Result<(Url, &ProjectorEntry), String> {
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

pub(super) fn projector_record(
    model: &AiModel,
    entry: &CatalogEntry,
) -> Result<AiProjector, String> {
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
