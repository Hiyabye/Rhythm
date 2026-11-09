//! Persistent AI inventory, attempts and validated result cache. No filesystem or inference work.
use super::Store;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub catalog_id: Option<String>,
    pub name: String,
    pub ownership: String,
    /// `generation` or `embedding`; persisted so a retired catalog entry keeps
    /// its role and an embedding model can never be selected for analysis.
    #[serde(default = "default_model_kind")]
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub storage_directory: Option<String>,
    pub sha256: Option<String>,
    pub expected_sha256: Option<String>,
    pub expected_bytes: Option<u64>,
    pub bytes: u64,
    pub source_revision: Option<String>,
    pub source_filename: Option<String>,
    pub status: String,
    pub downloaded_bytes: u64,
    pub resume_etag: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// Optional vision projector (mmproj) belonging to one model. It is a separate
/// artifact with its own integrity identity, so a text-only install or a failed
/// projector download never invalidates the model weights.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiProjector {
    pub model_id: String,
    pub path: String,
    pub sha256: Option<String>,
    pub expected_sha256: String,
    pub expected_bytes: u64,
    pub bytes: u64,
    pub status: String,
    pub downloaded_bytes: u64,
    pub resume_etag: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiPreferences {
    pub active_model_id: Option<String>,
    pub setup_completed: bool,
    pub processing_mode: String,
    pub automatic_limit: u32,
    pub max_tokens: u32,
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    #[serde(default)]
    pub model_directory: Option<String>,
    /// Selected embedding model; independent from the generation model.
    #[serde(default)]
    pub embedding_model_id: Option<String>,
    /// Automatic classification runs only when an embedding model is installed.
    #[serde(default = "default_enabled")]
    pub classification_enabled: bool,
    /// `local` (default) or `openai-compatible`. The local engine stays fully supported.
    #[serde(default = "default_engine")]
    pub engine: String,
    /// Normalized base URL of the user-chosen OpenAI-compatible endpoint.
    #[serde(default)]
    pub remote_base_url: Option<String>,
    #[serde(default)]
    pub remote_model: Option<String>,
    /// Separate embedding model; classification stays local unless remote embeddings are on.
    #[serde(default)]
    pub remote_embedding_model: Option<String>,
    #[serde(default = "default_remote_timeout")]
    pub remote_timeout_seconds: u32,
    /// Explicit consent for sending notice text to the configured origin.
    #[serde(default)]
    pub remote_consent: bool,
    /// Automatic (recent/backlog) remote analysis is a separate, cost-bearing opt-in.
    #[serde(default)]
    pub remote_automatic: bool,
    #[serde(default = "default_remote_daily_limit")]
    pub remote_daily_limit: u32,
    #[serde(default)]
    pub remote_embeddings: bool,
}

fn default_model_kind() -> String {
    "generation".into()
}

fn default_enabled() -> bool {
    true
}

fn default_engine() -> String {
    "local".into()
}

fn default_remote_timeout() -> u32 {
    120
}

fn default_remote_daily_limit() -> u32 {
    20
}

/// One revision-scoped embedding classification. The raw score and margin stay
/// local; the UI receives only the category and the low-confidence marker.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AiClassification {
    pub document_id: i64,
    pub revision_id: i64,
    pub category: String,
    pub low_confidence: bool,
    pub created_at: String,
}

/// A notice whose latest-revision embedding is close to another notice's. The
/// cosine score stays local; `duplicate` marks a likely cross-post.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SimilarNotice {
    pub document_id: i64,
    pub source_id: String,
    pub source_name: String,
    pub title: String,
    pub published_at: String,
    pub duplicate: bool,
}

/// One validated classification write. A struct keeps the persistence
/// boundary readable without loosening argument validation.
pub struct ClassificationWrite<'a> {
    pub document_id: i64,
    pub revision_id: i64,
    pub model_sha256: &'a str,
    pub config_hash: &'a str,
    pub category: &'a str,
    pub score: f32,
    pub margin: f32,
    pub low_confidence: bool,
}

#[derive(Clone, Debug)]
pub struct StoredEmbedding {
    pub dim: u32,
    pub vector: Vec<f32>,
}

fn default_execution_mode() -> String {
    "auto".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisKey {
    pub document_id: i64,
    pub revision_id: i64,
    pub task: String,
    pub model_sha256: String,
    pub prompt_version: u32,
    pub schema_version: u32,
    pub configuration_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiJob {
    pub id: i64,
    #[serde(flatten)]
    pub key: AnalysisKey,
    pub model_identity: Value,
    pub priority: i32,
    pub status: String,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiResult {
    pub id: i64,
    pub job_id: i64,
    #[serde(flatten)]
    pub key: AnalysisKey,
    pub model_identity: Value,
    pub result: Value,
    pub partial: bool,
    pub processed_chars: u64,
    pub total_chars: u64,
    pub duration_ms: u64,
    pub created_at: String,
}

const PROJECTOR_COLUMNS: &str = "model_id,path,sha256,expected_sha256,expected_bytes,bytes,status,downloaded_bytes,resume_etag,error";
const MODEL_COLUMNS: &str = "id,catalog_id,name,ownership,path,sha256,expected_sha256,expected_bytes,bytes,source_revision,source_filename,status,downloaded_bytes,resume_etag,error,created_at,updated_at,storage_directory,kind";
const JOB_COLUMNS: &str = "id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,priority,status,error,duration_ms,created_at,updated_at";
const RESULT_COLUMNS: &str = "id,job_id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,result_json,partial,processed_chars,total_chars,duration_ms,created_at";
const KEY_WHERE: &str = "document_id=?1 AND revision_id=?2 AND task=?3 AND model_sha256=?4 AND prompt_version=?5 AND schema_version=?6 AND configuration_hash=?7";
/// Upper bound on rows returned by the list/candidate queries below.
const MAX_LIST_LIMIT: u32 = 100;
/// Byte cap for stored paths and error text.
const MAX_FIELD_BYTES: usize = 8192;
/// Largest stored embedding dimension.
const MAX_EMBEDDING_DIM: usize = 8192;
/// Newest latest-revision vectors compared per similarity request, so cost
/// stays bounded as the archive grows. Older notices are not matched.
pub const SIMILARITY_WINDOW: usize = 2000;
/// Lowest cosine score shown as a similar notice.
const SIMILAR_MIN_SCORE: f32 = 0.75;
/// Duplicates need this score plus an identical normalized title and the same
/// digit runs in the body, so a notice with other dates, times or amounts is
/// never treated as a copy.
const DUPLICATE_MIN_SCORE: f32 = 0.97;
pub(super) fn decode_embedding(dim: i64, bytes: &[u8]) -> Result<Vec<f32>, String> {
    let dim =
        usize::try_from(dim).map_err(|_| "저장된 임베딩 크기가 올바르지 않습니다.".to_string())?;
    if dim == 0 || dim > MAX_EMBEDDING_DIM || bytes.len() != dim * 4 {
        return Err("저장된 임베딩이 완전하지 않습니다.".into());
    }
    let vector = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect::<Vec<_>>();
    if vector.iter().any(|value| !value.is_finite()) {
        return Err("저장된 임베딩 값이 올바르지 않습니다.".into());
    }
    Ok(vector)
}
/// Stored vectors are unit length, so the dot product is the cosine.
pub(super) fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    (a.len() == b.len()).then(|| a.iter().zip(b).map(|(x, y)| x * y).sum())
}
fn normalized_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
/// Splits leading board tags such as `[공지]` or `(학부)` from a title.
fn title_tags(title: &str) -> (Vec<String>, String) {
    const BRACKETS: [(char, char); 7] = [
        ('[', ']'),
        ('(', ')'),
        ('<', '>'),
        ('【', '】'),
        ('［', '］'),
        ('（', '）'),
        ('〈', '〉'),
    ];
    let mut tags = Vec::new();
    let mut rest = title.trim_start();
    while let Some(&(open, close)) = BRACKETS.iter().find(|(open, _)| rest.starts_with(*open)) {
        let Some(end) = rest.find(close) else {
            break;
        };
        tags.push(normalized_title(&rest[open.len_utf8()..end]));
        rest = rest[end + close.len_utf8()..].trim_start();
    }
    (tags, normalized_title(rest))
}
/// Board reposts add or drop tags; a conflicting tag can name another
/// audience (`[학부]` vs `[대학원]`), so one side's tags must contain the
/// other's. Digits anywhere in the title, tags included, must still agree.
fn same_title(a: &str, b: &str) -> bool {
    let ((a_tags, a_rest), (b_tags, b_rest)) = (title_tags(a), title_tags(b));
    let subset = |x: &[String], y: &[String]| x.iter().all(|tag| y.contains(tag));
    !a_rest.is_empty()
        && a_rest == b_rest
        && (subset(&a_tags, &b_tags) || subset(&b_tags, &a_tags))
        && digit_runs(a) == digit_runs(b)
}
/// Digit runs that are not contact numbers, so an appended contact line does
/// not split a cross-post. Full-width digits (U+FF10–U+FF19) read as ASCII,
/// so `３월` and `3월` match and two full-width dates still differ. Dates
/// written in words are not read.
fn digit_runs(text: &str) -> Vec<String> {
    let chars = text
        .chars()
        .map(|c| match c {
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            _ => c,
        })
        .collect::<Vec<_>>();
    let mut runs = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        if index > start {
            runs.push(start..index);
        } else {
            index += 1;
        }
    }
    // One separator, optionally followed by a space, joins two runs.
    let joined = |a: &std::ops::Range<usize>, b: &std::ops::Range<usize>, separators: &str| matches!(&chars[a.end..b.start], [s] | [s, ' '] if separators.contains(*s));
    let mut contact = vec![false; runs.len()];
    for k in 0..runs.len() {
        let run = &runs[k];
        // Area code: 02-705-8123, 02)705-8123, 010.1234.5678.
        if k + 2 < runs.len() {
            let (b, c) = (&runs[k + 1], &runs[k + 2]);
            if chars[run.start] == '0'
                && (2..=3).contains(&run.len())
                && joined(run, b, "-.)")
                && (3..=4).contains(&b.len())
                && joined(b, c, "-.")
                && c.len() == 4
            {
                contact[k..=k + 2].fill(true);
            }
        }
        // Local number: 705-8123, but not inside a longer chain. A four-digit
        // exchange is accepted only after a contact word, because `2025-2026`
        // is a year range.
        if k + 1 < runs.len() {
            let next = &runs[k + 1];
            let chained = (k > 0 && joined(&runs[k - 1], run, "-.)"))
                || (k + 2 < runs.len() && joined(next, &runs[k + 2], "-."));
            if !chained
                && next.len() == 4
                && joined(run, next, "-")
                && (run.len() == 3 || (run.len() == 4 && after_contact_word(&chars[..run.start])))
            {
                contact[k..=k + 1].fill(true);
            }
        }
        // Extension: 내선 8123, ☎ 8123, Tel. 8123.
        if (3..=4).contains(&run.len()) && after_contact_word(&chars[..run.start]) {
            contact[k] = true;
        }
    }
    runs.into_iter()
        .zip(contact)
        .filter(|(_, contact)| !contact)
        .map(|(run, _)| chars[run].iter().collect())
        .collect()
}
/// True when `prefix` ends with a contact word, ignoring spaces and `.:()`.
fn after_contact_word(prefix: &[char]) -> bool {
    let end = prefix
        .iter()
        .rposition(|c| !(c.is_whitespace() || ".:()".contains(*c)))
        .map_or(0, |i| i + 1);
    let tail = prefix[end.saturating_sub(3)..end]
        .iter()
        .collect::<String>()
        .to_lowercase();
    ["내선", "☎", "tel", "전화", "연락처"]
        .iter()
        .any(|word| tail.ends_with(word))
}
struct WindowNotice {
    document_id: i64,
    revision_id: i64,
    source_id: String,
    source_name: String,
    title: String,
    published_at: String,
    vector: Vec<f32>,
}
fn id_pair(row: &Row<'_>) -> rusqlite::Result<(i64, i64)> {
    Ok((row.get(0)?, row.get(1)?))
}
fn classification_row(row: &Row<'_>) -> rusqlite::Result<AiClassification> {
    Ok(AiClassification {
        document_id: row.get(0)?,
        revision_id: row.get(1)?,
        category: row.get(2)?,
        low_confidence: row.get(3)?,
        created_at: row.get(4)?,
    })
}
fn json_column(row: &Row<'_>, column: usize) -> rusqlite::Result<Value> {
    let value: String = row.get(column)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
fn key_row(row: &Row<'_>, offset: usize) -> rusqlite::Result<AnalysisKey> {
    Ok(AnalysisKey {
        document_id: row.get(offset)?,
        revision_id: row.get(offset + 1)?,
        task: row.get(offset + 2)?,
        model_sha256: row.get(offset + 3)?,
        prompt_version: row.get(offset + 4)?,
        schema_version: row.get(offset + 5)?,
        configuration_hash: row.get(offset + 6)?,
    })
}
fn model_row(row: &Row<'_>) -> rusqlite::Result<AiModel> {
    Ok(AiModel {
        id: row.get(0)?,
        catalog_id: row.get(1)?,
        name: row.get(2)?,
        ownership: row.get(3)?,
        path: row.get(4)?,
        sha256: row.get(5)?,
        expected_sha256: row.get(6)?,
        expected_bytes: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        bytes: row.get::<_, i64>(8)? as u64,
        source_revision: row.get(9)?,
        source_filename: row.get(10)?,
        status: row.get(11)?,
        downloaded_bytes: row.get::<_, i64>(12)? as u64,
        resume_etag: row.get(13)?,
        error: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        storage_directory: row.get(17)?,
        kind: row.get(18)?,
    })
}
fn job_row(row: &Row<'_>) -> rusqlite::Result<AiJob> {
    Ok(AiJob {
        id: row.get(0)?,
        key: key_row(row, 1)?,
        model_identity: json_column(row, 8)?,
        priority: row.get(9)?,
        status: row.get(10)?,
        error: row.get(11)?,
        duration_ms: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}
fn projector_row(row: &Row<'_>) -> rusqlite::Result<AiProjector> {
    Ok(AiProjector {
        model_id: row.get(0)?,
        path: row.get(1)?,
        sha256: row.get(2)?,
        expected_sha256: row.get(3)?,
        expected_bytes: row.get::<_, i64>(4)? as u64,
        bytes: row.get::<_, i64>(5)? as u64,
        status: row.get(6)?,
        downloaded_bytes: row.get::<_, i64>(7)? as u64,
        resume_etag: row.get(8)?,
        error: row.get(9)?,
    })
}
fn result_row(row: &Row<'_>) -> rusqlite::Result<AiResult> {
    Ok(AiResult {
        id: row.get(0)?,
        job_id: row.get(1)?,
        key: key_row(row, 2)?,
        model_identity: json_column(row, 9)?,
        result: json_column(row, 10)?,
        partial: row.get(11)?,
        processed_chars: row.get::<_, i64>(12)? as u64,
        total_chars: row.get::<_, i64>(13)? as u64,
        duration_ms: row.get::<_, i64>(14)? as u64,
        created_at: row.get(15)?,
    })
}
pub(super) fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
// Filesystem permissions, link checks and canonicalization belong to the caller.
// The persistence boundary still rejects malformed or relative directory input.
fn validate_model_directory(directory: Option<&str>) -> Result<(), String> {
    if directory.is_some_and(|directory| {
        directory.len() > 4096
            || directory.contains('\0')
            || !std::path::Path::new(directory).is_absolute()
    }) {
        return Err("모델 저장 폴더는 올바른 절대 경로로 지정해 주세요.".into());
    }
    Ok(())
}
fn validate_key(key: &AnalysisKey) -> Result<(), String> {
    if key.document_id <= 0
        || key.revision_id <= 0
        || key.task.is_empty()
        || key.task.len() > 64
        || !valid_hash(&key.model_sha256)
        || !valid_hash(&key.configuration_hash)
        || key.prompt_version == 0
        || key.schema_version == 0
    {
        return Err("AI 분석 식별자가 올바르지 않습니다.".into());
    }
    Ok(())
}
fn changed(count: usize) -> Result<(), String> {
    if count == 0 {
        Err("AI 항목을 찾을 수 없거나 상태가 변경되었습니다.".into())
    } else {
        Ok(())
    }
}

/// The source holding the user's personal data (the Cybercampus connector,
/// #86). Its documents never go to a remote engine, automatically or on request.
pub const PRIVATE_SOURCE: &str = crate::archive::cybercampus::SOURCE_ID;
impl Store {
    pub(super) fn reset_ai_interrupted(&self) -> Result<(), String> {
        self.connection.execute_batch("UPDATE ai_jobs SET status='failed',error='앱 종료로 분석이 중단되었습니다. 다시 시도해 주세요.',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status='running'; UPDATE ai_models SET status='cancelled',error='설치가 중단되었습니다. 이어받기를 선택해 주세요.',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status IN ('downloading','verifying'); UPDATE ai_projectors SET status='cancelled',error='이미지 인식 모듈 설치가 중단되었습니다. 이어받기를 선택해 주세요.',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status IN ('downloading','verifying');").map_err(|e|e.to_string())
    }
    pub fn ai_models(&self) -> Result<Vec<AiModel>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {MODEL_COLUMNS} FROM ai_models ORDER BY created_at,id"
            ))
            .map_err(|e| e.to_string())?
            .query_map([], model_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn ai_model(&self, id: &str) -> Result<Option<AiModel>, String> {
        self.connection
            .query_row(
                &format!("SELECT {MODEL_COLUMNS} FROM ai_models WHERE id=?1"),
                [id],
                model_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    /// Caller owns filesystem validation; `installed` requires an actual artifact identity.
    pub fn ai_upsert_model(&self, model: &AiModel) -> Result<(), String> {
        validate_model_directory(model.storage_directory.as_deref())?;
        if model.bytes > i64::MAX as u64
            || model.downloaded_bytes > i64::MAX as u64
            || model.expected_bytes.is_some_and(|v| v > i64::MAX as u64)
            || model.id.is_empty()
            || model.id.len() > 128
            || model.name.trim().is_empty()
            || model.name.len() > 512
            || model.path.is_empty()
            || model.path.len() > MAX_FIELD_BYTES
            || model.sha256.as_deref().is_some_and(|h| !valid_hash(h))
            || model
                .expected_sha256
                .as_deref()
                .is_some_and(|h| !valid_hash(h))
            || (model.status == "installed"
                && (model.sha256.is_none()
                    || model.bytes == 0
                    || model
                        .expected_sha256
                        .as_ref()
                        .is_some_and(|expected| model.sha256.as_ref() != Some(expected))
                    || model
                        .expected_bytes
                        .is_some_and(|expected| expected != model.bytes)))
            || !["generation", "embedding"].contains(&model.kind.as_str())
            || model
                .error
                .as_ref()
                .is_some_and(|e| e.len() > MAX_FIELD_BYTES)
        {
            return Err("모델 등록 정보를 확인해 주세요.".into());
        }
        self.connection.execute("INSERT INTO ai_models(id,catalog_id,name,ownership,path,sha256,expected_sha256,expected_bytes,bytes,source_revision,source_filename,status,downloaded_bytes,resume_etag,error,storage_directory,kind) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) ON CONFLICT(id) DO UPDATE SET catalog_id=excluded.catalog_id,name=excluded.name,ownership=excluded.ownership,path=excluded.path,sha256=excluded.sha256,expected_sha256=excluded.expected_sha256,expected_bytes=excluded.expected_bytes,bytes=excluded.bytes,source_revision=excluded.source_revision,source_filename=excluded.source_filename,status=excluded.status,downloaded_bytes=excluded.downloaded_bytes,resume_etag=excluded.resume_etag,error=excluded.error,storage_directory=excluded.storage_directory,kind=excluded.kind,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')", params![model.id,model.catalog_id,model.name,model.ownership,model.path,model.sha256,model.expected_sha256,model.expected_bytes.map(|v|v as i64),model.bytes as i64,model.source_revision,model.source_filename,model.status,model.downloaded_bytes as i64,model.resume_etag,model.error,model.storage_directory,model.kind]).map_err(|e|e.to_string())?;
        Ok(())
    }
    /// Removes inventory only. Never deletes files or historical analysis
    /// results. Derived embedding vectors and classifications are removed with
    /// the model identity they belong to.
    pub fn ai_remove_model(&self, id: &str) -> Result<(), String> {
        let sha: Option<String> = self
            .connection
            .query_row("SELECT sha256 FROM ai_models WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        changed(
            self.connection
                .execute("DELETE FROM ai_models WHERE id=?1", [id])
                .map_err(|e| e.to_string())?,
        )?;
        if let Some(sha) = sha {
            self.connection
                .execute(
                    "DELETE FROM notice_embeddings WHERE model_sha256=?1",
                    [&sha],
                )
                .map_err(|e| e.to_string())?;
            self.connection
                .execute(
                    "DELETE FROM notice_classifications WHERE model_sha256=?1",
                    [&sha],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn ai_projectors(&self) -> Result<Vec<AiProjector>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {PROJECTOR_COLUMNS} FROM ai_projectors ORDER BY model_id"
            ))
            .map_err(|e| e.to_string())?
            .query_map([], projector_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn ai_projector(&self, model_id: &str) -> Result<Option<AiProjector>, String> {
        self.connection
            .query_row(
                &format!("SELECT {PROJECTOR_COLUMNS} FROM ai_projectors WHERE model_id=?1"),
                [model_id],
                projector_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    /// Caller owns filesystem validation; `installed` requires the actual artifact identity.
    pub fn ai_upsert_projector(&self, projector: &AiProjector) -> Result<(), String> {
        if projector.path.is_empty()
            || projector.path.len() > MAX_FIELD_BYTES
            || projector.expected_bytes == 0
            || projector.expected_bytes > i64::MAX as u64
            || projector.bytes > i64::MAX as u64
            || projector.downloaded_bytes > i64::MAX as u64
            || !valid_hash(&projector.expected_sha256)
            || projector.sha256.as_deref().is_some_and(|h| !valid_hash(h))
            || projector
                .error
                .as_ref()
                .is_some_and(|e| e.len() > MAX_FIELD_BYTES)
            || (projector.status == "installed"
                && (projector.sha256.as_deref() != Some(&projector.expected_sha256)
                    || projector.bytes != projector.expected_bytes))
        {
            return Err("이미지 인식 모듈 정보를 확인해 주세요.".into());
        }
        let exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_models WHERE id=?1)",
                [&projector.model_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("이미지 인식 모듈을 연결할 모델을 찾을 수 없습니다.".into());
        }
        self.connection
            .execute("INSERT INTO ai_projectors(model_id,path,sha256,expected_sha256,expected_bytes,bytes,status,downloaded_bytes,resume_etag,error) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(model_id) DO UPDATE SET path=excluded.path,sha256=excluded.sha256,expected_sha256=excluded.expected_sha256,expected_bytes=excluded.expected_bytes,bytes=excluded.bytes,status=excluded.status,downloaded_bytes=excluded.downloaded_bytes,resume_etag=excluded.resume_etag,error=excluded.error,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')", params![projector.model_id, projector.path, projector.sha256, projector.expected_sha256, projector.expected_bytes as i64, projector.bytes as i64, projector.status, projector.downloaded_bytes as i64, projector.resume_etag, projector.error])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Removes inventory only. The caller deletes verified managed bytes first.
    pub fn ai_remove_projector(&self, model_id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM ai_projectors WHERE model_id=?1", [model_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// The preferences automatic AI work obeys. With the AI extension off the
    /// stored choices are kept, but no analysis, classification or semantic
    /// search runs on its own.
    pub fn ai_effective_preferences(&self) -> Result<AiPreferences, String> {
        let mut preferences = self.ai_preferences()?;
        if !self.enabled(crate::extensions::AI)? {
            preferences.processing_mode = "explicit".into();
            preferences.classification_enabled = false;
            preferences.remote_automatic = false;
        }
        Ok(preferences)
    }
    pub fn ai_preferences(&self) -> Result<AiPreferences, String> {
        self.connection.query_row("SELECT active_model_id,setup_completed,processing_mode,automatic_limit,max_tokens,execution_mode,model_directory,embedding_model_id,classification_enabled,engine,remote_base_url,remote_model,remote_embedding_model,remote_timeout_seconds,remote_consent,remote_automatic,remote_daily_limit,remote_embeddings FROM ai_preferences WHERE id=1",[],|r|Ok(AiPreferences { active_model_id:r.get(0)?,setup_completed:r.get(1)?,processing_mode:r.get(2)?,automatic_limit:r.get(3)?,max_tokens:r.get(4)?,execution_mode:r.get(5)?,model_directory:r.get(6)?,embedding_model_id:r.get(7)?,classification_enabled:r.get(8)?,engine:r.get(9)?,remote_base_url:r.get(10)?,remote_model:r.get(11)?,remote_embedding_model:r.get(12)?,remote_timeout_seconds:r.get(13)?,remote_consent:r.get(14)?,remote_automatic:r.get(15)?,remote_daily_limit:r.get(16)?,remote_embeddings:r.get(17)? })).map_err(|e|e.to_string())
    }
    pub fn ai_save_preferences(&self, preferences: &AiPreferences) -> Result<(), String> {
        validate_model_directory(preferences.model_directory.as_deref())?;
        if let Some(id) = &preferences.active_model_id {
            if self
                .ai_model(id)?
                .is_none_or(|m| m.status != "installed" || m.kind != "generation")
            {
                return Err("설치가 완료된 분석 모델을 선택해 주세요.".into());
            }
        }
        if let Some(id) = &preferences.embedding_model_id {
            if self
                .ai_model(id)?
                .is_none_or(|m| m.status != "installed" || m.kind != "embedding")
            {
                return Err("설치가 완료된 분류 모델을 선택해 주세요.".into());
            }
        }
        let bounded_field = |value: &Option<String>| {
            value.as_deref().is_some_and(|value| {
                value.len() > 2048 || value.contains('\0') || value.chars().any(char::is_control)
            })
        };
        if !["explicit", "recent", "backlog"].contains(&preferences.processing_mode.as_str())
            || !["auto", "cpu", "gpu"].contains(&preferences.execution_mode.as_str())
            || !["local", "openai-compatible"].contains(&preferences.engine.as_str())
            || preferences.automatic_limit > 20
            || !(128..=4096).contains(&preferences.max_tokens)
            || !(10..=600).contains(&preferences.remote_timeout_seconds)
            || preferences.remote_daily_limit > 200
            || bounded_field(&preferences.remote_base_url)
            || bounded_field(&preferences.remote_model)
            || bounded_field(&preferences.remote_embedding_model)
            || preferences
                .remote_model
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            || preferences
                .remote_embedding_model
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err("AI 처리 설정을 확인해 주세요.".into());
        }
        self.connection.execute("UPDATE ai_preferences SET active_model_id=?1,setup_completed=?2,processing_mode=?3,automatic_limit=?4,max_tokens=?5,execution_mode=?6,model_directory=?7,embedding_model_id=?8,classification_enabled=?9,engine=?10,remote_base_url=?11,remote_model=?12,remote_embedding_model=?13,remote_timeout_seconds=?14,remote_consent=?15,remote_automatic=?16,remote_daily_limit=?17,remote_embeddings=?18 WHERE id=1", params![preferences.active_model_id,preferences.setup_completed,preferences.processing_mode,preferences.automatic_limit,preferences.max_tokens,preferences.execution_mode,preferences.model_directory,preferences.embedding_model_id,preferences.classification_enabled,preferences.engine,preferences.remote_base_url,preferences.remote_model,preferences.remote_embedding_model,preferences.remote_timeout_seconds,preferences.remote_consent,preferences.remote_automatic,preferences.remote_daily_limit,preferences.remote_embeddings]).map_err(|e|e.to_string())?;
        Ok(())
    }
    /// A restored backup may contain remote engine preferences. They stay
    /// disabled until the user consents again; the key itself is never in the
    /// backup and remains in the OS credential store.
    pub fn reset_remote_after_restore(&self) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE ai_preferences SET engine='local',remote_consent=0,remote_automatic=0,remote_embeddings=0 WHERE id=1",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    /// Reserves one automatic remote attempt for the current local date.
    /// Direct user requests do not call this. `false` means the daily cap is reached.
    pub fn ai_reserve_remote_automatic(&self, limit: u32) -> Result<bool, String> {
        if limit == 0 {
            return Ok(false);
        }
        self.connection
            .execute(
                "DELETE FROM ai_remote_usage WHERE day<strftime('%Y-%m-%d','now','localtime','-7 days')",
                [],
            )
            .map_err(|error| error.to_string())?;
        let changed = self
            .connection
            .execute(
                "INSERT INTO ai_remote_usage(day,attempts) VALUES(strftime('%Y-%m-%d','now','localtime'),1) ON CONFLICT(day) DO UPDATE SET attempts=attempts+1 WHERE attempts<?1",
                [limit],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }
    /// Returns a reservation when no job was actually admitted.
    pub fn ai_release_remote_automatic(&self) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE ai_remote_usage SET attempts=attempts-1 WHERE day=strftime('%Y-%m-%d','now','localtime') AND attempts>0",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    pub fn ai_remote_usage_today(&self) -> Result<u32, String> {
        self.connection
            .query_row(
                "SELECT COALESCE((SELECT attempts FROM ai_remote_usage WHERE day=strftime('%Y-%m-%d','now','localtime')),0)",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }
    /// Changes only the destination for future downloads, never model inventory.
    pub fn ai_save_model_directory(&self, directory: Option<&str>) -> Result<(), String> {
        validate_model_directory(directory)?;
        self.connection
            .execute(
                "UPDATE ai_preferences SET model_directory=?1 WHERE id=1",
                [directory],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    /// Deduplicates pending/running attempts. Cache lookup is explicit via `ai_cached_result`.
    pub fn ai_enqueue(
        &self,
        key: &AnalysisKey,
        model_identity: &Value,
        priority: i32,
    ) -> Result<AiJob, String> {
        validate_key(key)?;
        let identity = serde_json::to_string(model_identity).map_err(|e| e.to_string())?;
        if identity.len() > 16384 {
            return Err("모델 정보가 너무 큽니다.".into());
        }
        self.connection.execute("INSERT INTO ai_jobs(document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,priority) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT DO UPDATE SET priority=MAX(ai_jobs.priority,excluded.priority)",params![key.document_id,key.revision_id,key.task,key.model_sha256,key.prompt_version,key.schema_version,key.configuration_hash,identity,priority]).map_err(|e|e.to_string())?;
        self.connection.query_row(&format!("SELECT {JOB_COLUMNS} FROM ai_jobs WHERE {KEY_WHERE} AND status IN ('pending','running')"),params![key.document_id,key.revision_id,key.task,key.model_sha256,key.prompt_version,key.schema_version,key.configuration_hash],job_row).map_err(|e|e.to_string())
    }
    pub fn ai_job(&self, id: i64) -> Result<Option<AiJob>, String> {
        self.connection
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM ai_jobs WHERE id=?1"),
                [id],
                job_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn ai_jobs(&self, limit: u32) -> Result<Vec<AiJob>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM ai_jobs ORDER BY id DESC LIMIT ?1"
            ))
            .map_err(|e| e.to_string())?
            .query_map([limit.min(MAX_LIST_LIMIT)], job_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Active work is authoritative and must not disappear behind a bounded history window.
    pub fn ai_active_jobs(&self) -> Result<Vec<AiJob>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM ai_jobs WHERE status IN ('pending','running') ORDER BY id DESC"
            ))
            .map_err(|e| e.to_string())?
            .query_map([], job_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Limit terminal history without hiding any active work for this revision.
    pub fn ai_document_jobs(
        &self,
        document_id: i64,
        revision_id: i64,
        limit: u32,
    ) -> Result<Vec<AiJob>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.connection
            .prepare(&format!(
                "SELECT {JOB_COLUMNS} FROM ai_jobs WHERE document_id=?1 AND revision_id=?2
                 AND (status IN ('pending','running') OR id IN (
                     SELECT id FROM ai_jobs WHERE document_id=?1 AND revision_id=?2
                     AND status NOT IN ('pending','running') ORDER BY id DESC LIMIT ?3
                 )) ORDER BY id DESC"
            ))
            .map_err(|e| e.to_string())?
            .query_map(
                params![document_id, revision_id, limit.min(MAX_LIST_LIMIT)],
                job_row,
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Latest revisions received after this session began, not latest publications.
    /// Initial historical backfill is excluded, but a meaningful later revision is
    /// eligible even if the document was originally historical. Any prior attempt
    /// suppresses automatic reanalysis/retry; explicit requests remain independent.
    /// `remote` excludes private sources, which never go to a remote engine.
    pub fn ai_recent_candidates(
        &self,
        boundary: &str,
        limit: u32,
        remote: bool,
    ) -> Result<Vec<(i64, i64)>, String> {
        self.connection
            .prepare(
                "SELECT d.id,r.id FROM documents d
                 JOIN document_revisions r ON r.id=(
                     SELECT MAX(latest.id) FROM document_revisions latest WHERE latest.document_id=d.id
                 )
                 WHERE r.collected_at>?1
                   AND (d.historical=0 OR EXISTS(
                       SELECT 1 FROM document_revisions earlier WHERE earlier.document_id=d.id AND earlier.id<r.id
                   ))
                   AND NOT EXISTS(SELECT 1 FROM ai_jobs j WHERE j.document_id=d.id AND j.revision_id=r.id)
                   AND (?3=0 OR d.source_id<>?4)
                   AND NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=d.id)
                 ORDER BY r.collected_at DESC,r.id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?
            .query_map(params![boundary, limit.min(MAX_LIST_LIMIT), remote, PRIVATE_SOURCE], id_pair)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Latest revisions of notices that still have no attempt for that revision,
    /// newest publication first. Unlike `ai_recent_candidates` this includes the
    /// initial historical backfill; it is the opt-in idle backlog pass. Notices
    /// without archived text are skipped because the pipeline cannot ground them.
    pub fn ai_backlog_candidates(
        &self,
        limit: u32,
        remote: bool,
    ) -> Result<Vec<(i64, i64)>, String> {
        self.connection
            .prepare(
                "SELECT d.id,r.id FROM documents d
                 JOIN document_revisions r ON r.id=(
                     SELECT MAX(latest.id) FROM document_revisions latest WHERE latest.document_id=d.id
                 )
                 WHERE length(trim(d.body_text))>0
                   AND NOT EXISTS(SELECT 1 FROM ai_jobs j WHERE j.document_id=d.id AND j.revision_id=r.id)
                   AND (?2=0 OR d.source_id<>?3)
                   AND NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=d.id)
                 ORDER BY d.published_at DESC,d.id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?
            .query_map(params![limit.min(MAX_LIST_LIMIT), remote, PRIVATE_SOURCE], id_pair)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Latest revisions whose current model/config classification is missing,
    /// newest publication first. A stored vector from another config is reused
    /// by the caller, so this query only tracks the classification row.
    pub fn ai_embedding_candidates(
        &self,
        model_sha256: &str,
        config_hash: &str,
        limit: u32,
        remote: bool,
    ) -> Result<Vec<(i64, i64)>, String> {
        if !valid_hash(model_sha256) || !valid_hash(config_hash) {
            return Err("분류 모델 정보가 올바르지 않습니다.".into());
        }
        self.connection
            .prepare(
                "SELECT d.id,r.id FROM documents d
                 JOIN document_revisions r ON r.id=(
                     SELECT MAX(latest.id) FROM document_revisions latest WHERE latest.document_id=d.id
                 )
                 WHERE length(trim(r.title))>0
                   AND NOT EXISTS(
                       SELECT 1 FROM notice_classifications c
                       WHERE c.document_id=d.id AND c.revision_id=r.id
                         AND c.model_sha256=?1 AND c.config_hash=?2
                   )
                   AND (?4=0 OR d.source_id<>?5)
                   AND NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=d.id)
                 ORDER BY r.collected_at DESC,r.id DESC LIMIT ?3",
            )
            .map_err(|e| e.to_string())?
            .query_map(params![model_sha256, config_hash, limit.min(MAX_LIST_LIMIT), remote, PRIVATE_SOURCE], id_pair)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Title and archived body for classification. Reads one revision only and
    /// never fetches the source or opens attachments.
    pub fn classification_text(
        &self,
        document_id: i64,
        revision_id: i64,
    ) -> Result<Option<(String, String)>, String> {
        self.connection
            .query_row(
                "SELECT title,body_text FROM document_revisions WHERE id=?1 AND document_id=?2",
                params![revision_id, document_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn ai_embedding(
        &self,
        document_id: i64,
        revision_id: i64,
        model_sha256: &str,
    ) -> Result<Option<StoredEmbedding>, String> {
        if !valid_hash(model_sha256) {
            return Err("분류 모델 정보가 올바르지 않습니다.".into());
        }
        let row: Option<(i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT dim,vector FROM notice_embeddings WHERE document_id=?1 AND revision_id=?2 AND model_sha256=?3",
                params![document_id, revision_id, model_sha256],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        row.map(|(dim, bytes)| {
            let vector = decode_embedding(dim, &bytes)?;
            Ok(StoredEmbedding {
                dim: vector.len() as u32,
                vector,
            })
        })
        .transpose()
    }
    pub fn ai_save_embedding(
        &self,
        document_id: i64,
        revision_id: i64,
        model_sha256: &str,
        vector: &[f32],
    ) -> Result<(), String> {
        if !valid_hash(model_sha256)
            || vector.is_empty()
            || vector.len() > MAX_EMBEDDING_DIM
            || vector.iter().any(|value| !value.is_finite())
        {
            return Err("임베딩 값을 저장할 수 없습니다.".into());
        }
        let mut bytes = Vec::with_capacity(vector.len() * 4);
        for value in vector {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.connection
            .execute(
                "INSERT INTO notice_embeddings(document_id,revision_id,model_sha256,dim,vector)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(document_id,revision_id,model_sha256) DO UPDATE SET
                   dim=excluded.dim,vector=excluded.vector,created_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                params![document_id, revision_id, model_sha256, vector.len() as i64, bytes],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn ai_save_classification(&self, value: &ClassificationWrite<'_>) -> Result<(), String> {
        if !valid_hash(value.model_sha256)
            || !valid_hash(value.config_hash)
            || value.category.is_empty()
            || value.category.len() > 32
            || !value.score.is_finite()
            || !value.margin.is_finite()
            || !(-1.0..=1.0).contains(&value.score)
            || !(-2.0..=2.0).contains(&value.margin)
        {
            return Err("분류 결과를 저장할 수 없습니다.".into());
        }
        self.connection
            .execute(
                "INSERT INTO notice_classifications(document_id,revision_id,model_sha256,config_hash,category,score,margin,low_confidence)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(document_id,revision_id,model_sha256,config_hash) DO UPDATE SET
                   category=excluded.category,score=excluded.score,margin=excluded.margin,
                   low_confidence=excluded.low_confidence,created_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                params![
                    value.document_id,
                    value.revision_id,
                    value.model_sha256,
                    value.config_hash,
                    value.category,
                    value.score as f64,
                    value.margin as f64,
                    value.low_confidence
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Latest-revision classifications for a visible page. Only the current
    /// model and config are returned; stale rows stay as history.
    pub fn ai_classifications_for_documents(
        &self,
        document_ids: &[i64],
        model_sha256: &str,
        config_hash: &str,
    ) -> Result<Vec<AiClassification>, String> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }
        if document_ids.len() > 200 || !valid_hash(model_sha256) || !valid_hash(config_hash) {
            return Err("분류 조회 범위를 확인해 주세요.".into());
        }
        let placeholders = (0..document_ids.len())
            .map(|index| format!("?{}", index + 3))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT c.document_id,c.revision_id,c.category,c.low_confidence,c.created_at
             FROM notice_classifications c
             WHERE c.model_sha256=?1 AND c.config_hash=?2
               AND c.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=c.document_id)
               AND c.document_id IN ({placeholders})
             ORDER BY c.document_id"
        );
        let mut values = vec![
            SqlValue::Text(model_sha256.to_owned()),
            SqlValue::Text(config_hash.to_owned()),
        ];
        values.extend(document_ids.iter().map(|id| SqlValue::Integer(*id)));
        self.connection
            .prepare(&sql)
            .map_err(|e| e.to_string())?
            .query_map(params_from_iter(values), classification_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn ai_classification(
        &self,
        document_id: i64,
        revision_id: i64,
        model_sha256: &str,
        config_hash: &str,
    ) -> Result<Option<AiClassification>, String> {
        if !valid_hash(model_sha256) || !valid_hash(config_hash) {
            return Err("분류 모델 정보가 올바르지 않습니다.".into());
        }
        self.connection
            .query_row(
                "SELECT document_id,revision_id,category,low_confidence,created_at
                 FROM notice_classifications
                 WHERE document_id=?1 AND revision_id=?2 AND model_sha256=?3 AND config_hash=?4",
                params![document_id, revision_id, model_sha256, config_hash],
                classification_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    /// Similar notices for each requested document, compared by the cosine of
    /// latest-revision vectors from one embedding model. Only the newest
    /// `SIMILARITY_WINDOW` embedded notices are candidates, and a document
    /// without a vector for this model gets an empty list. Each list holds every
    /// duplicate first, then up to `limit` matches in total.
    pub fn ai_similar_notices(
        &self,
        document_ids: &[i64],
        model_sha256: &str,
        limit: usize,
    ) -> Result<Vec<(i64, Vec<SimilarNotice>)>, String> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }
        if document_ids.len() > 200 || !valid_hash(model_sha256) || limit == 0 || limit > 50 {
            return Err("비슷한 공지 조회 범위를 확인해 주세요.".into());
        }
        let window = self
            .connection
            .prepare(
                "SELECT d.id,e.revision_id,d.source_id,s.name,d.title,d.published_at,e.dim,e.vector
                 FROM documents d
                 JOIN sources s ON s.id=d.source_id
                 JOIN notice_embeddings e ON e.document_id=d.id AND e.model_sha256=?1
                   AND e.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=d.id)
                 WHERE NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=d.id)
                 ORDER BY d.published_at DESC,d.id DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?
            .query_map(params![model_sha256, SIMILARITY_WINDOW as i64], |row| {
                Ok((
                    WindowNotice {
                        document_id: row.get(0)?,
                        revision_id: row.get(1)?,
                        source_id: row.get(2)?,
                        source_name: row.get(3)?,
                        title: row.get(4)?,
                        published_at: row.get(5)?,
                        vector: Vec::new(),
                    },
                    row.get::<_, i64>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .map(|row| {
                let (mut notice, dim, bytes) = row.map_err(|e| e.to_string())?;
                notice.vector = decode_embedding(dim, &bytes)?;
                Ok(notice)
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Candidate bodies are reduced to their digit runs once, so a cached
        // entry holds a title and a few short strings instead of a whole body.
        let mut bodies = std::collections::HashMap::<i64, Option<(String, Vec<String>)>>::new();
        let mut results = Vec::with_capacity(document_ids.len());
        for &document_id in document_ids {
            let target = match window.iter().find(|n| n.document_id == document_id) {
                Some(notice) => Some((notice.revision_id, notice.vector.clone())),
                None => {
                    let revision: Option<i64> = self
                        .connection
                        .query_row(
                            "SELECT MAX(id) FROM document_revisions WHERE document_id=?1",
                            [document_id],
                            |row| row.get(0),
                        )
                        .map_err(|e| e.to_string())?;
                    match revision {
                        Some(revision) => self
                            .ai_embedding(document_id, revision, model_sha256)?
                            .map(|stored| (revision, stored.vector)),
                        None => None,
                    }
                }
            };
            let Some((revision_id, vector)) = target else {
                results.push((document_id, Vec::new()));
                continue;
            };
            let target_text = self
                .classification_text(document_id, revision_id)?
                .map(|(title, body)| (title, digit_runs(&body)));
            let mut scored = Vec::new();
            for (notice, score) in window
                .iter()
                .filter(|n| n.document_id != document_id)
                .filter_map(|n| cosine(&vector, &n.vector).map(|score| (n, score)))
                .filter(|(_, score)| *score >= SIMILAR_MIN_SCORE)
            {
                let duplicate = score >= DUPLICATE_MIN_SCORE && {
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        bodies.entry(notice.document_id)
                    {
                        slot.insert(
                            self.classification_text(notice.document_id, notice.revision_id)?
                                .map(|(title, body)| (title, digit_runs(&body))),
                        );
                    }
                    match (&target_text, &bodies[&notice.document_id]) {
                        (Some((title, digits)), Some((other_title, other_digits))) => {
                            same_title(title, other_title) && digits == other_digits
                        }
                        _ => false,
                    }
                };
                scored.push((notice, score, duplicate));
            }
            scored.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then(b.1.total_cmp(&a.1))
                    .then(b.0.document_id.cmp(&a.0.document_id))
            });
            let duplicates = scored.iter().filter(|(_, _, duplicate)| *duplicate).count();
            let similar = scored
                .into_iter()
                .take(duplicates.max(limit))
                .map(|(notice, _, duplicate)| SimilarNotice {
                    document_id: notice.document_id,
                    source_id: notice.source_id.clone(),
                    source_name: notice.source_name.clone(),
                    title: notice.title.clone(),
                    published_at: notice.published_at.clone(),
                    duplicate,
                })
                .collect();
            results.push((document_id, similar));
        }
        Ok(results)
    }
    pub fn ai_has_model_jobs(&self, model_sha256: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_jobs WHERE model_sha256=?1 AND status IN ('pending','running'))",
                [model_sha256],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
    /// Cancels only a pending/running attempt. Returns false when it already reached
    /// a terminal state, so a concurrent completion is not treated as an error.
    pub fn ai_cancel_if_running(&self, id: i64, error: &str) -> Result<bool, String> {
        self.connection
            .execute(
                "UPDATE ai_jobs SET status='cancelled',error=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status IN ('pending','running')",
                params![id, error],
            )
            .map(|count| count > 0)
            .map_err(|e| e.to_string())
    }
    /// Atomically claims one pending job. Application-level concurrency remains one worker.
    pub fn ai_claim_next(&mut self) -> Result<Option<AiJob>, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let id = transaction
            .query_row(
                "SELECT id FROM ai_jobs WHERE status='pending' ORDER BY priority DESC,id LIMIT 1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(id) = id else {
            return Ok(None);
        };
        transaction.execute("UPDATE ai_jobs SET status='running',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",[id]).map_err(|e|e.to_string())?;
        let job = transaction
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM ai_jobs WHERE id=?1"),
                [id],
                job_row,
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(Some(job))
    }
    pub fn ai_finish_job(
        &self,
        id: i64,
        status: &str,
        error: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(), String> {
        if duration_ms.is_some_and(|v| v > i64::MAX as u64)
            || !["failed", "cancelled", "stale"].contains(&status)
            || error.is_some_and(|e| e.len() > MAX_FIELD_BYTES)
        {
            return Err("AI 작업 상태를 확인해 주세요.".into());
        }
        changed(self.connection.execute("UPDATE ai_jobs SET status=?2,error=?3,duration_ms=?4,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status IN ('pending','running')",params![id,status,error,duration_ms.map(|v|v as i64)]).map_err(|e|e.to_string())?)
    }
    /// The pipeline must schema-validate and ground `result` before invoking this method.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn ai_complete_job(
        &mut self,
        id: i64,
        result: &Value,
        partial: bool,
        processed_chars: u64,
        total_chars: u64,
        duration_ms: u64,
    ) -> Result<AiResult, String> {
        self.ai_complete_job_with_backend(
            id,
            result,
            partial,
            processed_chars,
            total_chars,
            duration_ms,
            None,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn ai_complete_job_with_backend(
        &mut self,
        id: i64,
        result: &Value,
        partial: bool,
        processed_chars: u64,
        total_chars: u64,
        duration_ms: u64,
        loaded_backend: Option<&str>,
        images: Option<(u64, u64)>,
    ) -> Result<AiResult, String> {
        let encoded = serde_json::to_string(result).map_err(|e| e.to_string())?;
        if total_chars > i64::MAX as u64
            || duration_ms > i64::MAX as u64
            || !result.is_object()
            || encoded.len() > 128 * 1024
            || processed_chars > total_chars
            || (!partial && processed_chars != total_chars)
            || loaded_backend
                .is_some_and(|backend| !["cpu", "vulkan", "cuda", "metal"].contains(&backend))
        {
            return Err("AI 분석 결과의 범위나 형식을 확인해 주세요.".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        if let Some(backend) = loaded_backend {
            // Record only an observed successful inference backend, atomically
            // with the result. An automatic fallback remains visible on reopen.
            changed(transaction.execute("UPDATE ai_jobs SET model_identity_json=json_set(model_identity_json,'$.loaded_backend',?2) WHERE id=?1 AND status='running'", params![id, backend]).map_err(|e| e.to_string())?)?;
        }
        if let Some((used, available)) = images {
            if used > available || available > 64 {
                return Err("이미지 분석 범위를 확인해 주세요.".into());
            }
            // Which archived images reached the model is part of the stored
            // result's provenance; it is not part of the semantic cache key.
            changed(transaction.execute("UPDATE ai_jobs SET model_identity_json=json_set(model_identity_json,'$.images_used',?2,'$.images_available',?3) WHERE id=?1 AND status='running'", params![id, used as i64, available as i64]).map_err(|e| e.to_string())?)?;
        }
        changed(transaction.execute("INSERT INTO ai_results(job_id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,result_json,partial,processed_chars,total_chars,duration_ms) SELECT id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,?2,?3,?4,?5,?6 FROM ai_jobs WHERE id=?1 AND status='running'", params![id,encoded,partial,processed_chars as i64,total_chars as i64,duration_ms as i64]).map_err(|e|e.to_string())?)?;
        let result_id = transaction.last_insert_rowid();
        transaction.execute("UPDATE ai_jobs SET status='completed',error=NULL,duration_ms=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",params![id,duration_ms as i64]).map_err(|e|e.to_string())?;
        let stored = transaction
            .query_row(
                &format!("SELECT {RESULT_COLUMNS} FROM ai_results WHERE id=?1"),
                [result_id],
                result_row,
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(stored)
    }
    pub fn ai_cached_result(&self, key: &AnalysisKey) -> Result<Option<AiResult>, String> {
        validate_key(key)?;
        self.connection.query_row(&format!("SELECT {RESULT_COLUMNS} FROM ai_results WHERE {KEY_WHERE} ORDER BY id DESC LIMIT 1"),params![key.document_id,key.revision_id,key.task,key.model_sha256,key.prompt_version,key.schema_version,key.configuration_hash],result_row).optional().map_err(|e|e.to_string())
    }
    pub fn ai_document_results(
        &self,
        document_id: i64,
        revision_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AiResult>, String> {
        self.connection.prepare(&format!("SELECT {RESULT_COLUMNS} FROM ai_results WHERE document_id=?1 AND (?2 IS NULL OR revision_id=?2) ORDER BY id DESC LIMIT ?3")).map_err(|e|e.to_string())?.query_map(params![document_id,revision_id,limit.min(MAX_LIST_LIMIT)],result_row).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
    }
    pub fn ai_recent_results(&self, limit: u32) -> Result<Vec<AiResult>, String> {
        // One latest result per current revision, not a history/regeneration flood.
        self.connection.prepare(&format!("SELECT {RESULT_COLUMNS} FROM ai_results a WHERE a.id=(SELECT MAX(b.id) FROM ai_results b WHERE b.document_id=a.document_id AND b.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=a.document_id)) AND NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=a.document_id) ORDER BY a.id DESC LIMIT ?1")).map_err(|e|e.to_string())?.query_map([limit.min(MAX_LIST_LIMIT)],result_row).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
    }
}
