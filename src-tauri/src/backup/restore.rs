//! Restore: staging validation, model inventory reconciliation, the durable
//! journal and next-launch recovery.
use super::*;

pub(super) fn staged_resource_names(
    root: &Path,
    directory: &Path,
    names: &mut HashSet<String>,
) -> Result<(), String> {
    for item in fs::read_dir(directory).map_err(|_| "복원 첨부 자료 목록을 읽지 못했습니다.")?
    {
        let item = item.map_err(|_| "복원 첨부 자료 목록을 읽지 못했습니다.")?;
        let kind = item
            .file_type()
            .map_err(|_| "복원 첨부 자료 종류를 확인하지 못했습니다.")?;
        let path = item.path();
        if unsafe_link(
            &fs::symlink_metadata(&path)
                .map_err(|_| "복원 첨부 자료 종류를 확인하지 못했습니다.")?,
        ) {
            return Err("백업 첨부 자료에 안전하지 않은 링크가 있습니다.".into());
        }
        if kind.is_dir() {
            staged_resource_names(root, &path, names)?;
        } else if kind.is_file() {
            let name = path
                .strip_prefix(root)
                .map_err(|_| "복원 첨부 경로를 확인하지 못했습니다.")?
                .to_str()
                .ok_or("복원 첨부 경로를 읽지 못했습니다.")?
                .replace('\\', "/");
            if !valid_path(&name) || !names.insert(name) {
                return Err("백업 첨부 자료 경로가 중복되었거나 안전하지 않습니다.".into());
            }
        } else {
            return Err("백업 첨부 자료 종류를 확인할 수 없습니다.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_staged_database(
    staging: &Path,
    expected_schema: i64,
    expected_documents: u64,
    expected_resources: u64,
) -> Result<(), String> {
    validate_staged_database_cancellable(
        staging,
        expected_schema,
        expected_documents,
        expected_resources,
        None,
    )
}
pub(super) fn validate_staged_database_cancellable(
    staging: &Path,
    expected_schema: i64,
    expected_documents: u64,
    expected_resources: u64,
    cancellation: Option<&BackupCancellation>,
) -> Result<(), String> {
    let database = staging.join("rhythm.sqlite3");
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "백업 데이터베이스를 열지 못했습니다.")?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| "백업 데이터베이스를 검사하지 못했습니다.")?;
    if integrity != "ok" {
        return Err("백업 데이터베이스의 내부 무결성이 손상되었습니다.".into());
    }
    let schema: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| "백업 데이터베이스 버전을 확인하지 못했습니다.")?;
    if schema != expected_schema || !(1..=crate::storage::SCHEMA_VERSION).contains(&schema) {
        return Err("이 백업의 데이터베이스 버전은 이 앱에서 복원할 수 없습니다.".into());
    }
    if schema < 2 {
        return Err("이 백업의 데이터베이스 버전은 지원하지 않습니다.".into());
    }
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .map_err(|_| "백업 공지 수를 확인하지 못했습니다.")?;
    if documents < 0 || documents as u64 != expected_documents {
        return Err("백업 공지 수가 매니페스트와 일치하지 않습니다.".into());
    }
    let mut check = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| "백업 데이터베이스 참조를 검사하지 못했습니다.")?;
    if check
        .exists([])
        .map_err(|_| "백업 데이터베이스 참조를 검사하지 못했습니다.")?
    {
        return Err("백업 데이터베이스에 올바르지 않은 참조가 있습니다.".into());
    }
    let mut statement = connection
        .prepare("SELECT local_path,bytes,sha256 FROM resources WHERE status='downloaded'")
        .map_err(|_| "백업 첨부 자료를 확인하지 못했습니다.")?;
    let mut rows = statement
        .query([])
        .map_err(|_| "백업 첨부 자료를 확인하지 못했습니다.")?;
    let mut referenced = HashSet::new();
    let mut actual_resources = HashSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "백업 첨부 자료를 확인하지 못했습니다.")?
    {
        check_cancellation(cancellation)?;
        let name: String = row
            .get(0)
            .map_err(|_| "백업 첨부 경로가 올바르지 않습니다.")?;
        let size: i64 = row
            .get(1)
            .map_err(|_| "백업 첨부 크기가 올바르지 않습니다.")?;
        let expected_hash: String = row
            .get(2)
            .map_err(|_| "백업 첨부 해시가 올바르지 않습니다.")?;
        if !valid_path(&name) || name == "rhythm.sqlite3" || size < 0 {
            return Err("백업 첨부 경로가 안전하지 않습니다.".into());
        }
        referenced.insert(name.clone());
        let path = staging.join(&name);
        let (actual_size, actual_hash) = hash_file(&path, cancellation)
            .map_err(|_| "백업에 데이터베이스가 참조하는 첨부 자료가 없습니다.".to_owned())?;
        if actual_size != size as u64 || actual_hash != expected_hash {
            return Err("백업 첨부 자료가 데이터베이스와 일치하지 않습니다.".into());
        }
    }
    let resources_root = staging.join("resources");
    match fs::symlink_metadata(&resources_root) {
        Ok(metadata) if metadata.is_dir() && !unsafe_link(&metadata) => {
            staged_resource_names(staging, &resources_root, &mut actual_resources)?;
        }
        Ok(_) => return Err("백업 첨부 자료 폴더가 안전하지 않습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("백업 첨부 자료 목록을 읽지 못했습니다.".into()),
    }
    if referenced != actual_resources || actual_resources.len() as u64 != expected_resources {
        return Err("백업 첨부 자료 목록이 데이터베이스와 일치하지 않습니다.".into());
    }
    Ok(())
}

pub(super) fn reconcile_model_inventory(
    staging: &Path,
    model_root: &Path,
    cancellation: &BackupCancellation,
) -> Result<(), String> {
    let database = staging.join("rhythm.sqlite3");
    let connection = rusqlite::Connection::open(&database)
        .map_err(|_| "백업 데이터베이스를 열지 못했습니다.")?;
    let has_inventory: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ai_models')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "백업 모델 목록을 확인하지 못했습니다.")?;
    if !has_inventory {
        return Ok(());
    }
    let has_custom_directories: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ai_models') WHERE name='storage_directory')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "백업 모델 폴더 정보를 확인하지 못했습니다.")?;
    let has_embedding_model: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('ai_preferences') WHERE name='embedding_model_id')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "백업 분류 모델 정보를 확인하지 못했습니다.")?;
    let has_projectors: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ai_projectors')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "백업 이미지 인식 모듈 목록을 확인하지 못했습니다.")?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|_| "백업 모델 상태를 갱신하지 못했습니다.")?;
    if has_custom_directories {
        // A backup must never grant permission to delete or resume writing to
        // an arbitrary directory outside this installation's model folder.
        // Keep the old path and analysis history as registration hints only.
        transaction
            .execute_batch(
                "UPDATE ai_preferences SET model_directory=NULL;
                 UPDATE ai_preferences SET active_model_id=NULL WHERE active_model_id IN (
                     SELECT id FROM ai_models WHERE ownership='managed' AND storage_directory IS NOT NULL
                 );",
            )
            .map_err(|_| "백업의 외부 모델 폴더 권한을 초기화하지 못했습니다.")?;
        if has_embedding_model {
            transaction
                .execute(
                    "UPDATE ai_preferences SET embedding_model_id=NULL WHERE embedding_model_id IN (
                         SELECT id FROM ai_models WHERE ownership='managed' AND storage_directory IS NOT NULL
                     )",
                    [],
                )
                .map_err(|_| "백업의 분류 모델 선택을 초기화하지 못했습니다.")?;
        }
        let custom_models = transaction
            .prepare("SELECT id,path,storage_directory FROM ai_models WHERE ownership='managed' AND storage_directory IS NOT NULL")
            .map_err(|_| "백업의 외부 모델 목록을 읽지 못했습니다.")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(|_| "백업의 외부 모델 목록을 읽지 못했습니다.")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "백업의 외부 모델 목록을 읽지 못했습니다.")?;
        for (id, path, directory) in custom_models {
            check_cancellation(Some(cancellation))?;
            let absolute = restored_custom_model_path(&directory, &path);
            let already_external: bool = match absolute.as_deref() {
                Some(path) => transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM ai_models WHERE ownership='external' AND path=?1 AND id<>?2)",
                    rusqlite::params![path, id],
                    |row| row.get(0),
                ).map_err(|_| "백업의 중복 모델 등록을 확인하지 못했습니다.")?,
                None => false,
            };
            if already_external {
                // A managed file may also have been explicitly registered as
                // external. Keep that existing reference and remove only the
                // duplicate inventory row; analysis history has no model FK.
                transaction
                    .execute("DELETE FROM ai_models WHERE id=?1", [&id])
                    .map_err(|_| "백업의 중복 모델 등록을 정리하지 못했습니다.")?;
                continue;
            }
            let error = if absolute.is_some() {
                "다른 저장 폴더의 모델은 복원 후 다시 등록하거나 설치해 주세요. 원본 파일은 변경하지 않았습니다."
            } else {
                "백업의 모델 경로를 확인하지 못했습니다. 원본 파일을 다시 등록하거나 설치해 주세요. 원본 파일은 변경하지 않았습니다."
            };
            transaction
                .execute(
                    "UPDATE ai_models SET ownership='external',status='missing',path=?2,
                    downloaded_bytes=0,resume_etag=NULL,error=?3 WHERE id=?1",
                    rusqlite::params![id, absolute.unwrap_or(path), error],
                )
                .map_err(|_| "백업의 외부 모델 폴더 권한을 초기화하지 못했습니다.")?;
        }
        transaction
            .execute("UPDATE ai_models SET storage_directory=NULL", [])
            .map_err(|_| "백업의 외부 모델 폴더 권한을 초기화하지 못했습니다.")?;
    }
    let mut query = transaction
        .prepare("SELECT id,path,sha256,bytes FROM ai_models WHERE ownership='managed' AND status='installed'")
        .map_err(|_| "백업 모델 목록을 확인하지 못했습니다.")?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|_| "백업 모델 목록을 확인하지 못했습니다.")?;
    let mut missing = Vec::new();
    for row in rows {
        check_cancellation(Some(cancellation))?;
        let (id, relative, expected_hash, expected_bytes) =
            row.map_err(|_| "백업 모델 목록을 읽지 못했습니다.")?;
        if !managed_model_matches(
            model_root,
            &relative,
            expected_hash.as_deref(),
            expected_bytes,
            cancellation,
        )? {
            missing.push(id);
        }
    }
    drop(query);
    for id in missing {
        transaction.execute("UPDATE ai_models SET status='missing',error='해당 모델 파일이 없거나 무결성 확인에 실패했습니다. 다시 설치해 주세요.' WHERE id=?1", [&id])
            .map_err(|_| "백업 모델 상태를 갱신하지 못했습니다.")?;
        transaction
            .execute(
                "UPDATE ai_preferences SET active_model_id=NULL WHERE active_model_id=?1",
                [&id],
            )
            .map_err(|_| "백업에서 사용할 모델을 정리하지 못했습니다.")?;
        if has_embedding_model {
            transaction
                .execute(
                    "UPDATE ai_preferences SET embedding_model_id=NULL WHERE embedding_model_id=?1",
                    [&id],
                )
                .map_err(|_| "백업에서 사용할 분류 모델을 정리하지 못했습니다.")?;
        }
    }
    if has_projectors {
        // Projector bytes are excluded from backups like model weights. Only a
        // projector whose file is still present with the recorded identity, and
        // whose model is still installed here, may stay "installed".
        let mut orphaned = Vec::new();
        // Older backups have no custom-directory column; their managed models
        // are always in the default folder.
        let unreachable_models = if has_custom_directories {
            "SELECT id FROM ai_models WHERE status<>'installed' OR ownership<>'managed' OR storage_directory IS NOT NULL"
        } else {
            "SELECT id FROM ai_models WHERE status<>'installed' OR ownership<>'managed'"
        };
        transaction
            .execute(
                &format!(
                    "UPDATE ai_projectors SET status='missing',downloaded_bytes=0,resume_etag=NULL,
                     error='복원한 백업의 모델을 다시 설치해야 이미지 인식 모듈을 쓸 수 있습니다.'
                     WHERE status<>'missing' AND model_id IN ({unreachable_models})"
                ),
                [],
            )
            .map_err(|_| "백업 이미지 인식 모듈 상태를 갱신하지 못했습니다.")?;
        let mut query = transaction
            .prepare("SELECT model_id,path,sha256,expected_bytes FROM ai_projectors WHERE status='installed'")
            .map_err(|_| "백업 이미지 인식 모듈 목록을 읽지 못했습니다.")?;
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(|_| "백업 이미지 인식 모듈 목록을 읽지 못했습니다.")?;
        for row in rows {
            check_cancellation(Some(cancellation))?;
            let (id, relative, hash, bytes) =
                row.map_err(|_| "백업 이미지 인식 모듈 목록을 읽지 못했습니다.")?;
            if !managed_model_matches(model_root, &relative, hash.as_deref(), bytes, cancellation)?
            {
                orphaned.push(id);
            }
        }
        drop(query);
        for id in orphaned {
            transaction
                .execute(
                    "UPDATE ai_projectors SET status='missing',downloaded_bytes=0,resume_etag=NULL,
                     error='복원한 백업에는 이미지 인식 모듈 파일이 없습니다. 다시 설치해 주세요.' WHERE model_id=?1",
                    [&id],
                )
                .map_err(|_| "백업 이미지 인식 모듈 상태를 갱신하지 못했습니다.")?;
        }
    }
    transaction
        .commit()
        .map_err(|_| "백업 모델 상태를 저장하지 못했습니다.")?;
    Ok(())
}

/// Reconstructs a registration hint without opening any path from the backup.
/// Missing drives and files are deliberately allowed; deletion authority is not.
pub(super) fn restored_custom_model_path(directory: &str, relative: &str) -> Option<String> {
    use std::path::Component;
    let directory_path = Path::new(directory);
    if directory.len() > 4096
        || directory.contains('\0')
        || !directory_path.is_absolute()
        || directory_path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    let relative = Path::new(relative);
    if relative.components().count() != 2
        || relative.parent() != Some(Path::new("models"))
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let name = relative.file_name()?.to_str()?;
    let hash = name.strip_suffix(".gguf")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    Some(directory_path.join(name).to_string_lossy().into_owned())
}

pub(super) fn managed_model_matches(
    model_root: &Path,
    relative: &str,
    expected_hash: Option<&str>,
    expected_bytes: Option<i64>,
    cancellation: &BackupCancellation,
) -> Result<bool, String> {
    let Some(name) = relative.strip_prefix("models/") else {
        return Ok(false);
    };
    if !valid_model_filename(name)
        || !expected_hash.is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        || !expected_bytes.is_some_and(|bytes| bytes >= 0)
    {
        return Ok(false);
    }
    let root_metadata = match fs::symlink_metadata(model_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("로컬 모델 파일 목록을 확인하지 못했습니다.".into()),
    };
    if !root_metadata.is_dir() || unsafe_link(&root_metadata) {
        return Ok(false);
    }
    let path = model_root.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("로컬 모델 파일을 확인하지 못했습니다.".into()),
    };
    if !metadata.is_file()
        || unsafe_link(&metadata)
        || metadata.len() != expected_bytes.unwrap_or_default() as u64
    {
        return Ok(false);
    }
    let (bytes, hash) = hash_file(&path, Some(cancellation))
        .map_err(|_| "로컬 모델 파일의 무결성을 확인하지 못했습니다.".to_owned())?;
    Ok(bytes == expected_bytes.unwrap_or_default() as u64
        && expected_hash.is_some_and(|expected| hash.eq_ignore_ascii_case(expected)))
}

pub(super) fn valid_model_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.contains('\0')
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|c| !c.is_control() && !"<>\"|?*".contains(c))
}

#[derive(Serialize, Deserialize)]
pub(super) struct RestoreJournal {
    pub(super) version: u32,
    pub(super) previous: String,
    pub(super) had_resources: bool,
    pub(super) schema: i64,
    pub(super) documents: u64,
    pub(super) resources: u64,
    /// Hash of the prepared database after model/token reconciliation. Absent
    /// only in historical Unix journals.
    #[serde(default)]
    pub(super) database_sha256: Option<String>,
}
pub(super) const STAGING: &str = ".rhythm-restore-staging";
pub(super) const JOURNAL: &str = ".rhythm-restore-journal.json";
pub(super) const PREVIOUS_PREFIX: &str = ".rhythm-restore-previous-";
#[cfg(windows)]
pub(super) const COMPLETE_MARKER: &str = ".rhythm-restore-complete";

pub(super) fn entry_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("복원 파일 경로를 확인하지 못했습니다. 기존 파일을 보존했습니다.".into()),
    }
}

#[cfg(windows)]
pub(super) fn check_unjournaled_restore(root: &Path) -> Result<(), String> {
    let ambiguous = || {
        "완료되지 않은 복원 흔적이 있습니다. 복구 파일을 보존했으며 데이터 폴더를 확인해야 합니다."
            .to_string()
    };
    if entry_exists(&root.join(STAGING))?
        || entry_exists(&root.join(".rhythm-restore-journal.tmp"))?
    {
        return Err(ambiguous());
    }
    for item in fs::read_dir(root).map_err(|_| ambiguous())? {
        let item = item.map_err(|_| ambiguous())?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with(PREVIOUS_PREFIX) {
            let metadata = fs::symlink_metadata(item.path()).map_err(|_| ambiguous())?;
            if !metadata.is_dir() || unsafe_link(&metadata) {
                return Err(ambiguous());
            }
            let marker = item.path().join(COMPLETE_MARKER);
            let metadata = fs::symlink_metadata(&marker).map_err(|_| ambiguous())?;
            if !metadata.is_file()
                || unsafe_link(&metadata)
                || fs::read(&marker).map_err(|_| ambiguous())? != name.as_bytes()
            {
                return Err(ambiguous());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn mark_restore_complete(previous: &Path, previous_name: &str) -> Result<(), String> {
    let marker = previous.join(COMPLETE_MARKER);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(mut file) => {
            file.write_all(previous_name.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| "복원 완료 상태를 저장하지 못했습니다.")?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&marker)
                .map_err(|_| "복원 완료 상태를 확인하지 못했습니다.")?;
            if !metadata.is_file()
                || unsafe_link(&metadata)
                || fs::read(&marker).map_err(|_| "복원 완료 상태를 확인하지 못했습니다.")?
                    != previous_name.as_bytes()
            {
                return Err("복원 완료 상태가 올바르지 않습니다.".into());
            }
        }
        Err(_) => return Err("복원 완료 상태를 저장하지 못했습니다.".into()),
    }
    Ok(())
}

pub(super) fn write_journal(
    root: &Path,
    previous: &str,
    had_resources: bool,
    preview: &BackupPreview,
) -> Result<(), String> {
    write_journal_with_sync(root, previous, had_resources, preview, sync_directory)
}
pub(super) fn write_journal_with_sync(
    root: &Path,
    previous: &str,
    had_resources: bool,
    preview: &BackupPreview,
    sync: impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let temp = root.join(".rhythm-restore-journal.tmp");
    let _cleanup = RemoveOnDrop(temp.clone());
    let stage_db = root.join(STAGING).join("rhythm.sqlite3");
    let database_sha256 = if stage_db.is_file() {
        Some(hash_file(&stage_db, None)?.1)
    } else {
        None
    };
    let bytes = serde_json::to_vec(&RestoreJournal {
        version: 1,
        previous: previous.to_owned(),
        had_resources,
        schema: preview.database_schema,
        documents: preview.documents,
        resources: preview.resources,
        database_sha256,
    })
    .map_err(|_| "복원 상태를 기록하지 못했습니다.")?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "복원 상태 파일을 만들지 못했습니다.")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "복원 상태를 저장하지 못했습니다.")?;
    drop(file);
    restore_rename(&temp, &root.join(JOURNAL)).map_err(|_| "복원 상태를 확정하지 못했습니다.")?;
    sync(root)
}
pub(super) fn publish_without_replace(temporary: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows::rename(temporary, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "같은 이름의 파일이 이미 있습니다. 다른 저장 위치를 선택해 주세요.".into()
        } else {
            "백업 파일을 안전하게 저장 위치로 옮기지 못했습니다. 저장 위치와 파일 시스템 권한을 확인해 주세요.".into()
        }
    });
    #[cfg(not(windows))]
    fs::hard_link(temporary, destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            "같은 이름의 파일이 이미 있습니다. 다른 저장 위치를 선택해 주세요.".into()
        } else {
            "백업 파일을 안전하게 저장 위치로 옮기지 못했습니다. 저장 위치와 파일 시스템 권한을 확인해 주세요.".into()
        }
    })
}
#[cfg(unix)]
pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "복원 상태를 안정적으로 저장하지 못했습니다.".into())
}
#[cfg(not(unix))]
pub(super) fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}
pub(super) fn sync_tree(_root: &Path, directory: &Path) -> Result<(), String> {
    for item in fs::read_dir(directory).map_err(|_| "복원 폴더를 저장하지 못했습니다.")?
    {
        let item = item.map_err(|_| "복원 폴더를 저장하지 못했습니다.")?;
        let metadata =
            fs::symlink_metadata(item.path()).map_err(|_| "복원 폴더를 저장하지 못했습니다.")?;
        if unsafe_link(&metadata) {
            return Err("복원 폴더에 안전하지 않은 링크가 있습니다.".into());
        }
        if metadata.is_dir() {
            sync_tree(_root, &item.path())?;
        }
    }
    sync_directory(directory)
}

pub(super) fn sync_file(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let file = fs::OpenOptions::new().read(true).write(true).open(path);
    #[cfg(not(windows))]
    let file = File::open(path);
    file.and_then(|file| file.sync_all())
        .map_err(|_| "복원 데이터베이스를 저장하지 못했습니다.".into())
}

#[cfg(all(test, windows))]
pub(super) fn restore_fault_point(name: &str) {
    if std::env::var_os("RHYTHM_RESTORE_TEST_FAULT").as_deref() == Some(std::ffi::OsStr::new(name))
    {
        std::process::abort();
    }
}

#[cfg(not(all(test, windows)))]
pub(super) fn restore_fault_point(_name: &str) {}

/// Tokens refer to an earlier app installation's identities; never resurrect
/// them from a backup. Older schemas get empty capability tables on upgrade.
pub(super) fn clear_restored_activation_tokens(stage: &Path) -> Result<(), String> {
    let database = stage.join("rhythm.sqlite3");
    let connection = rusqlite::Connection::open(&database)
        .map_err(|_| "복원 데이터베이스를 열지 못했습니다.")?;
    for table in ["notification_activation_tokens", "studium_alert_tokens"] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(|_| "복원 알림 상태를 확인하지 못했습니다.")?;
        if exists {
            connection
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(|_| "복원 알림 상태를 초기화하지 못했습니다.")?;
        }
    }
    drop(connection);
    sync_file(&database)
}

/// What a completed next-launch restore installed, for the reopened window.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RestoreOutcome {
    pub(super) documents: u64,
    /// Distinct verified resource files restored.
    pub(super) resources: u64,
    /// Resource rows the backup itself recorded as not downloaded.
    pub(super) unavailable_resources: u64,
    /// Models that require installation or registration after restoration.
    pub(super) models_needing_reinstall: u64,
}

pub(super) fn restore_outcome(
    root: &Path,
    journal: &RestoreJournal,
) -> Result<RestoreOutcome, String> {
    let connection = rusqlite::Connection::open_with_flags(
        root.join("rhythm.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "복원된 데이터베이스를 열지 못했습니다.")?;
    let count = |sql: &str| -> Result<u64, String> {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .map(|value| value.max(0) as u64)
            .map_err(|_| "복원 결과를 확인하지 못했습니다.".to_owned())
    };
    let has_models = count(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ai_models')",
    )? == 1;
    Ok(RestoreOutcome {
        documents: journal.documents,
        resources: journal.resources,
        unavailable_resources: count("SELECT COUNT(*) FROM resources WHERE status<>'downloaded'")?,
        models_needing_reinstall: if has_models {
            count("SELECT COUNT(*) FROM ai_models WHERE status='missing'")?
        } else {
            0
        },
    })
}

/// Stale online-backup snapshots are private to an interrupted `backup_create`.
pub(crate) fn remove_stale_backup_snapshots(root: &Path) {
    let Ok(items) = fs::read_dir(root) else {
        return;
    };
    for item in items.flatten() {
        let name = item.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".rhythm-backup-")
            && name.ends_with(".sqlite3")
            && item.file_type().is_ok_and(|kind| kind.is_file())
        {
            let _ = fs::remove_file(item.path());
        }
    }
}

/// Called after the process lock is acquired and before Store::open. Each rename
/// is recoverable from the journal and previous files remain available.
/// Returns what was installed when this launch finished a pending restore.
pub(crate) fn recover_pending_restore(root: &Path) -> Result<Option<RestoreOutcome>, String> {
    let journal = root.join(JOURNAL);
    if !entry_exists(&journal)? {
        #[cfg(windows)]
        {
            check_unjournaled_restore(root)?;
            return Ok(None);
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(root.join(".rhythm-restore-journal.tmp"));
            let _ = fs::remove_dir_all(root.join(STAGING));
            return Ok(None);
        }
    }
    restore_support(root)?;
    let journal_metadata =
        fs::symlink_metadata(&journal).map_err(|_| "복원 상태를 확인하지 못했습니다.")?;
    if !journal_metadata.is_file() || unsafe_link(&journal_metadata) {
        return Err("복원 상태 경로가 안전하지 않습니다.".into());
    }
    let data: RestoreJournal =
        serde_json::from_slice(&fs::read(&journal).map_err(|_| "복원 상태를 읽지 못했습니다.")?)
            .map_err(|_| "복원 상태가 손상되었습니다. 원본 데이터를 변경하지 않았습니다.")?;
    if data.version != 1
        || !data.previous.starts_with(PREVIOUS_PREFIX)
        || Path::new(&data.previous).components().count() != 1
        || !matches!(
            Path::new(&data.previous).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("복원 상태 버전 또는 복구 경로가 올바르지 않습니다.".into());
    }
    let stage = root.join(STAGING);
    let previous = root.join(&data.previous);
    #[cfg(windows)]
    for path in [
        &stage,
        &previous,
        &root.join("rhythm.sqlite3"),
        &root.join("resources"),
    ] {
        windows::check_path(path)?;
    }
    if fs::symlink_metadata(&previous).is_ok_and(|meta| unsafe_link(&meta)) {
        return Err("기존 데이터 복구 폴더가 안전하지 않습니다.".into());
    }
    fs::create_dir_all(&previous).map_err(|_| "기존 데이터 복구 폴더를 만들지 못했습니다.")?;
    sync_directory(root)?;
    let stage_metadata = fs::symlink_metadata(&stage);
    match stage_metadata {
        Ok(metadata) if metadata.is_dir() && !unsafe_link(&metadata) => {}
        Ok(_) => return Err("복원 임시 폴더 경로가 안전하지 않습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("복원 임시 폴더를 확인하지 못했습니다.".into()),
    }
    let live_db = root.join("rhythm.sqlite3");
    let old_db = previous.join("rhythm.sqlite3");
    let stage_db = stage.join("rhythm.sqlite3");
    #[cfg(windows)]
    if data.database_sha256.is_none() {
        return Err(
            "복원 상태에 데이터베이스 검증 정보가 없습니다. 복구 파일을 보존했습니다.".into(),
        );
    }
    if let Some(expected) = &data.database_sha256 {
        let selected = if stage_db.exists() {
            &stage_db
        } else {
            &live_db
        };
        if hash_file(selected, None)?.1 != *expected {
            return Err(
                "복원 데이터베이스가 준비된 파일과 다릅니다. 복구 파일을 보존했습니다.".into(),
            );
        }
    }
    for path in [
        &live_db,
        &old_db,
        &stage_db,
        &root.join("resources"),
        &previous.join("resources"),
        &stage.join("resources"),
    ] {
        if fs::symlink_metadata(path).is_ok_and(|metadata| unsafe_link(&metadata)) {
            return Err("복원 경로에 안전하지 않은 링크가 있습니다.".into());
        }
    }
    if stage_db.exists() {
        for suffix in ["-wal", "-shm", "-journal"] {
            let live = root.join(format!("rhythm.sqlite3{suffix}"));
            let old = previous.join(format!("rhythm.sqlite3{suffix}"));
            #[cfg(windows)]
            {
                windows::check_path(&live)?;
                windows::check_path(&old)?;
            }
            if live.exists() && !old.exists() {
                restore_rename(&live, &old)
                    .map_err(|_| "기존 데이터베이스 로그를 복구 폴더로 옮기지 못했습니다.")?;
                sync_directory(root)?;
                sync_directory(&previous)?;
            }
        }
    }
    if stage_db.exists() {
        if live_db.exists() && !old_db.exists() {
            restore_rename(&live_db, &old_db)
                .map_err(|_| "기존 데이터베이스를 복구 폴더로 옮기지 못했습니다.")?;
            restore_fault_point("old-db");
            sync_directory(root)?;
            sync_directory(&previous)?;
        }
        restore_rename(&stage_db, &live_db)
            .map_err(|_| "복원 데이터베이스를 설치하지 못했습니다.")?;
        restore_fault_point("new-db");
        sync_directory(root)?;
        sync_directory(&stage)?;
    } else if !live_db.exists() || !old_db.exists() {
        return Err("복원 데이터베이스 또는 기존 데이터 복구본을 찾지 못했습니다.".into());
    }
    let live_resources = root.join("resources");
    let old_resources = previous.join("resources");
    let stage_resources = stage.join("resources");
    for path in [
        &live_db,
        &old_db,
        &stage_db,
        &live_resources,
        &old_resources,
        &stage_resources,
    ] {
        if fs::symlink_metadata(path).is_ok_and(|metadata| unsafe_link(&metadata)) {
            return Err("복원 경로에 안전하지 않은 링크가 있습니다.".into());
        }
    }
    if stage_resources.exists() {
        if live_resources.exists() {
            if !data.had_resources || old_resources.exists() {
                return Err("복원 첨부 자료 상태가 일치하지 않습니다.".into());
            }
            restore_rename(&live_resources, &old_resources)
                .map_err(|_| "기존 첨부 자료를 복구 폴더로 옮기지 못했습니다.")?;
            restore_fault_point("old-resources");
            sync_directory(root)?;
            sync_directory(&previous)?;
        }
        restore_rename(&stage_resources, &live_resources)
            .map_err(|_| "복원 첨부 자료를 설치하지 못했습니다.")?;
        restore_fault_point("new-resources");
        sync_directory(root)?;
        sync_directory(&stage)?;
    } else if data.had_resources {
        if !live_resources.exists() || !old_resources.exists() {
            return Err("복원 첨부 자료 또는 기존 복구본을 찾지 못했습니다.".into());
        }
    } else if !live_resources.exists() {
        fs::create_dir(&live_resources).map_err(|_| "첨부 자료 폴더를 복구하지 못했습니다.")?;
        sync_directory(root)?;
    }
    validate_staged_database_cancellable(root, data.schema, data.documents, data.resources, None)?;
    sync_file(&live_db)?;
    let outcome = restore_outcome(root, &data)?;
    restore_fault_point("verified");
    #[cfg(windows)]
    {
        mark_restore_complete(&previous, &data.previous)?;
        restore_fault_point("marked");
        if stage.exists() {
            fs::remove_dir(&stage)
                .map_err(|_| "복원 준비 폴더를 정리하지 못했습니다. 복구 파일을 보존했습니다.")?;
        }
        restore_fault_point("stage-removed");
    }
    #[cfg(not(windows))]
    let _ = fs::remove_dir_all(&stage);
    fs::remove_file(journal).map_err(|_| "복원 상태를 정리하지 못했습니다.")?;
    sync_directory(root)?;
    Ok(Some(outcome))
}

/// The restore this launch finalized, if any; it stays available for the
/// whole session so a remounted window still reports it.
#[tauri::command]
pub(crate) fn backup_restore_outcome(
    state: tauri::State<'_, crate::AppState>,
) -> Option<RestoreOutcome> {
    state.restore_outcome.clone()
}

/// Verifies the archive into staging and writes the durable restore journal.
/// The next launch finalizes it through `recover_pending_restore`.
pub(super) fn stage_restore(
    root: &Path,
    source: &Path,
    cancellation: &BackupCancellation,
) -> Result<(), String> {
    restore_support(root)?;
    #[cfg(windows)]
    check_unjournaled_restore(root)?;
    if root.join(STAGING).exists() || root.join(JOURNAL).exists() {
        return Err(
            "이전 복원 작업의 복구 파일이 남아 있습니다. 앱을 다시 시작한 뒤 확인해 주세요.".into(),
        );
    }
    let previous_name = format!("{PREVIOUS_PREFIX}{}-{}", timestamp(), std::process::id());
    if root.join(&previous_name).exists() {
        return Err("기존 데이터 복구 폴더가 이미 있습니다.".into());
    }
    let database_metadata = fs::symlink_metadata(root.join("rhythm.sqlite3"))
        .map_err(|_| "현재 데이터베이스를 찾지 못했습니다.")?;
    if !database_metadata.is_file() || unsafe_link(&database_metadata) {
        return Err("현재 데이터베이스 경로가 안전하지 않습니다.".into());
    }
    let resources_metadata = fs::symlink_metadata(root.join("resources"));
    let had_resources = match resources_metadata {
        Ok(metadata) if metadata.is_dir() && !unsafe_link(&metadata) => true,
        Ok(_) => return Err("현재 첨부 자료 폴더 경로가 안전하지 않습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("현재 첨부 자료 폴더를 확인하지 못했습니다.".into()),
    };
    let stage = root.join(STAGING);
    let result = (|| {
        let preview = extract_verified_cancellable(source, &stage, cancellation)?;
        fs::create_dir_all(stage.join("resources"))
            .map_err(|_| "첨부 자료 폴더를 준비하지 못했습니다.")?;
        // Establish the archive's integrity before opening the staged database
        // for either model reconciliation or activation-token deletion.
        validate_staged_database_cancellable(
            &stage,
            preview.database_schema,
            preview.documents,
            preview.resources,
            Some(cancellation),
        )?;
        reconcile_model_inventory(&stage, &root.join("models"), cancellation)?;
        clear_restored_activation_tokens(&stage)?;
        validate_staged_database_cancellable(
            &stage,
            preview.database_schema,
            preview.documents,
            preview.resources,
            Some(cancellation),
        )?;
        sync_directory(&stage.join("resources"))?;
        sync_directory(&stage)?;
        check_cancellation(Some(cancellation))?;
        cancellation.begin_commit()?;
        write_journal(root, &previous_name, had_resources, &preview)?;
        Ok(())
    })();
    if result.is_err() && !root.join(JOURNAL).exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[tauri::command]
pub(crate) async fn backup_restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    source: String,
    confirmed: bool,
    operation_id: String,
) -> Result<(), String> {
    if !confirmed {
        return Err("복원을 시작하려면 확인이 필요합니다.".into());
    }
    let registration = register_cancellation(operation_id)?;
    let cancellation = registration.cancellation.clone();
    restore_support(&state.data_root)?;
    let maintenance = state.exclusive_maintenance().await?;
    let root = state.data_root.clone();
    let app_for_exit = app.clone();
    tokio::task::spawn_blocking(move || {
        let _registration = registration;
        check_cancellation(Some(&cancellation))?;
        let maintenance = maintenance;
        let output = stage_restore(&root, Path::new(&source), &cancellation);
        if output.is_ok() || root.join(JOURNAL).exists() {
            // Keep maintenance closed until exit. The restore is finalized by the
            // next launch; Rhythm does not relaunch itself because a relaunched
            // instance can race this process for the single-instance socket and
            // data-directory lock and then exit silently.
            maintenance.hold_until_exit();
            app_for_exit.exit(0);
        }
        output
    })
    .await
    .map_err(|_| "복원 준비를 마치지 못했습니다.".to_owned())??;
    Ok(())
}
