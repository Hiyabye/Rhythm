//! Persistent AI inventory, attempts and validated result cache. No filesystem or inference work.
use super::Store;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub catalog_id: Option<String>,
    pub name: String,
    pub ownership: String,
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
const MODEL_COLUMNS: &str = "id,catalog_id,name,ownership,path,sha256,expected_sha256,expected_bytes,bytes,source_revision,source_filename,status,downloaded_bytes,resume_etag,error,created_at,updated_at,storage_directory";
const JOB_COLUMNS: &str = "id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,priority,status,error,duration_ms,created_at,updated_at";
const RESULT_COLUMNS: &str = "id,job_id,document_id,revision_id,task,model_sha256,prompt_version,schema_version,configuration_hash,model_identity_json,result_json,partial,processed_chars,total_chars,duration_ms,created_at";
const KEY_WHERE: &str = "document_id=?1 AND revision_id=?2 AND task=?3 AND model_sha256=?4 AND prompt_version=?5 AND schema_version=?6 AND configuration_hash=?7";
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
fn valid_hash(value: &str) -> bool {
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
            || model.path.len() > 8192
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
            || model.error.as_ref().is_some_and(|e| e.len() > 8192)
        {
            return Err("모델 등록 정보를 확인해 주세요.".into());
        }
        self.connection.execute("INSERT INTO ai_models(id,catalog_id,name,ownership,path,sha256,expected_sha256,expected_bytes,bytes,source_revision,source_filename,status,downloaded_bytes,resume_etag,error,storage_directory) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(id) DO UPDATE SET catalog_id=excluded.catalog_id,name=excluded.name,ownership=excluded.ownership,path=excluded.path,sha256=excluded.sha256,expected_sha256=excluded.expected_sha256,expected_bytes=excluded.expected_bytes,bytes=excluded.bytes,source_revision=excluded.source_revision,source_filename=excluded.source_filename,status=excluded.status,downloaded_bytes=excluded.downloaded_bytes,resume_etag=excluded.resume_etag,error=excluded.error,storage_directory=excluded.storage_directory,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')", params![model.id,model.catalog_id,model.name,model.ownership,model.path,model.sha256,model.expected_sha256,model.expected_bytes.map(|v|v as i64),model.bytes as i64,model.source_revision,model.source_filename,model.status,model.downloaded_bytes as i64,model.resume_etag,model.error,model.storage_directory]).map_err(|e|e.to_string())?;
        Ok(())
    }
    /// Removes inventory only. Never deletes files or historical results.
    pub fn ai_remove_model(&self, id: &str) -> Result<(), String> {
        changed(
            self.connection
                .execute("DELETE FROM ai_models WHERE id=?1", [id])
                .map_err(|e| e.to_string())?,
        )
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
            || projector.path.len() > 8192
            || projector.expected_bytes == 0
            || projector.expected_bytes > i64::MAX as u64
            || projector.bytes > i64::MAX as u64
            || projector.downloaded_bytes > i64::MAX as u64
            || !valid_hash(&projector.expected_sha256)
            || projector.sha256.as_deref().is_some_and(|h| !valid_hash(h))
            || projector.error.as_ref().is_some_and(|e| e.len() > 8192)
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
    pub fn ai_preferences(&self) -> Result<AiPreferences, String> {
        self.connection.query_row("SELECT active_model_id,setup_completed,processing_mode,automatic_limit,max_tokens,execution_mode,model_directory FROM ai_preferences WHERE id=1",[],|r|Ok(AiPreferences { active_model_id:r.get(0)?,setup_completed:r.get(1)?,processing_mode:r.get(2)?,automatic_limit:r.get(3)?,max_tokens:r.get(4)?,execution_mode:r.get(5)?,model_directory:r.get(6)? })).map_err(|e|e.to_string())
    }
    pub fn ai_save_preferences(&self, preferences: &AiPreferences) -> Result<(), String> {
        validate_model_directory(preferences.model_directory.as_deref())?;
        if let Some(id) = &preferences.active_model_id {
            if self.ai_model(id)?.is_none_or(|m| m.status != "installed") {
                return Err("설치가 완료된 모델을 선택해 주세요.".into());
            }
        }
        if !["explicit", "recent", "backlog"].contains(&preferences.processing_mode.as_str())
            || !["auto", "cpu", "gpu"].contains(&preferences.execution_mode.as_str())
            || preferences.automatic_limit > 20
            || !(128..=4096).contains(&preferences.max_tokens)
        {
            return Err("AI 처리 설정을 확인해 주세요.".into());
        }
        self.connection.execute("UPDATE ai_preferences SET active_model_id=?1,setup_completed=?2,processing_mode=?3,automatic_limit=?4,max_tokens=?5,execution_mode=?6,model_directory=?7 WHERE id=1", params![preferences.active_model_id,preferences.setup_completed,preferences.processing_mode,preferences.automatic_limit,preferences.max_tokens,preferences.execution_mode,preferences.model_directory]).map_err(|e|e.to_string())?;
        Ok(())
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
            .query_map([limit.min(100)], job_row)
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
            .query_map(params![document_id, revision_id, limit.min(100)], job_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Latest revisions received after this session began, not latest publications.
    /// Initial historical backfill is excluded, but a meaningful later revision is
    /// eligible even if the document was originally historical. Any prior attempt
    /// suppresses automatic reanalysis/retry; explicit requests remain independent.
    pub fn ai_recent_candidates(
        &self,
        boundary: &str,
        limit: u32,
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
                 ORDER BY r.collected_at DESC,r.id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?
            .query_map(params![boundary, limit.min(100)], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    /// Latest revisions of notices that still have no attempt for that revision,
    /// newest publication first. Unlike `ai_recent_candidates` this includes the
    /// initial historical backfill; it is the opt-in idle backlog pass. Notices
    /// without archived text are skipped because the pipeline cannot ground them.
    pub fn ai_backlog_candidates(&self, limit: u32) -> Result<Vec<(i64, i64)>, String> {
        self.connection
            .prepare(
                "SELECT d.id,r.id FROM documents d
                 JOIN document_revisions r ON r.id=(
                     SELECT MAX(latest.id) FROM document_revisions latest WHERE latest.document_id=d.id
                 )
                 WHERE length(trim(d.body_text))>0
                   AND NOT EXISTS(SELECT 1 FROM ai_jobs j WHERE j.document_id=d.id AND j.revision_id=r.id)
                 ORDER BY d.published_at DESC,d.id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?
            .query_map([limit.min(100)], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
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
            || error.is_some_and(|e| e.len() > 8192)
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
        self.connection.prepare(&format!("SELECT {RESULT_COLUMNS} FROM ai_results WHERE document_id=?1 AND (?2 IS NULL OR revision_id=?2) ORDER BY id DESC LIMIT ?3")).map_err(|e|e.to_string())?.query_map(params![document_id,revision_id,limit.min(100)],result_row).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
    }
    pub fn ai_recent_results(&self, limit: u32) -> Result<Vec<AiResult>, String> {
        // One latest result per current revision, not a history/regeneration flood.
        self.connection.prepare(&format!("SELECT {RESULT_COLUMNS} FROM ai_results a WHERE a.id=(SELECT MAX(b.id) FROM ai_results b WHERE b.document_id=a.document_id AND b.revision_id=(SELECT MAX(r.id) FROM document_revisions r WHERE r.document_id=a.document_id)) ORDER BY a.id DESC LIMIT ?1")).map_err(|e|e.to_string())?.query_map([limit.min(100)],result_row).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())
    }
}
