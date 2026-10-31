//! Versioned ZIP backup for Rhythm's database and downloaded archive resources.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_ENTRY: u64 = 32 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024 * 1024;
const COPY_BUFFER: usize = 128 * 1024;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, LazyLock, Mutex,
};

const CANCELLABLE: u8 = 0;
const CANCEL_REQUESTED: u8 = 1;
const COMMITTING: u8 = 2;
static ACTIVE_CANCELLATIONS: LazyLock<Mutex<HashMap<String, Arc<BackupCancellation>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct BackupCancellation(AtomicU8);
impl BackupCancellation {
    fn check(&self) -> Result<(), String> {
        if self.0.load(Ordering::Acquire) == CANCEL_REQUESTED {
            Err("백업 또는 복원을 취소했습니다.".into())
        } else {
            Ok(())
        }
    }
    fn request(&self) -> bool {
        self.0
            .compare_exchange(
                CANCELLABLE,
                CANCEL_REQUESTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
            || self.0.load(Ordering::Acquire) == CANCEL_REQUESTED
    }
    fn begin_commit(&self) -> Result<(), String> {
        self.0
            .compare_exchange(CANCELLABLE, COMMITTING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "백업 또는 복원을 취소했습니다.".into())
    }
}
struct CancellationRegistration {
    id: String,
    cancellation: Arc<BackupCancellation>,
}
impl Drop for CancellationRegistration {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_CANCELLATIONS.lock() {
            if active
                .get(&self.id)
                .is_some_and(|item| Arc::ptr_eq(item, &self.cancellation))
            {
                active.remove(&self.id);
            }
        }
    }
}
fn register_cancellation(id: String) -> Result<CancellationRegistration, String> {
    if id.is_empty() || id.len() > 128 {
        return Err("백업 작업 식별자가 올바르지 않습니다.".into());
    }
    let cancellation = Arc::new(BackupCancellation(AtomicU8::new(CANCELLABLE)));
    let mut active = ACTIVE_CANCELLATIONS
        .lock()
        .map_err(|_| "백업 작업 상태를 확인하지 못했습니다.")?;
    if active.contains_key(&id) {
        return Err("같은 식별자의 백업 작업이 이미 실행 중입니다.".into());
    }
    active.insert(id.clone(), cancellation.clone());
    Ok(CancellationRegistration { id, cancellation })
}
#[tauri::command]
pub(crate) fn backup_cancel(operation_id: String) -> Result<bool, String> {
    let active = ACTIVE_CANCELLATIONS
        .lock()
        .map_err(|_| "백업 작업 상태를 확인하지 못했습니다.")?;
    Ok(active
        .get(&operation_id)
        .is_some_and(|cancellation| cancellation.request()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    created_at: String,
    database_schema: i64,
    documents: u64,
    resources: u64,
    resource_bytes: u64,
    entries: Vec<Entry>,
}
struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct RemoveDirectoryOnDrop(PathBuf);
impl Drop for RemoveDirectoryOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct BackupPreview {
    format_version: u32,
    created_at: String,
    database_schema: i64,
    documents: u64,
    resources: u64,
    resource_bytes: u64,
    includes_model_weights: bool,
}
impl From<&Manifest> for BackupPreview {
    fn from(m: &Manifest) -> Self {
        Self {
            format_version: m.format_version,
            created_at: m.created_at.clone(),
            database_schema: m.database_schema,
            documents: m.documents,
            resources: m.resources,
            resource_bytes: m.resource_bytes,
            includes_model_weights: false,
        }
    }
}

fn valid_path(name: &str) -> bool {
    if name == "rhythm.sqlite3" {
        return true;
    }
    let Some(rest) = name.strip_prefix("resources/") else {
        return false;
    };
    if rest.is_empty() || name.contains('\\') || name.contains(':') || name.contains('\0') {
        return false;
    }
    rest.split('/').all(|part| {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
        {
            return false;
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(['.', ' '])
            .to_ascii_uppercase();
        let numbered_device = ["COM", "LPT"].into_iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        // Windows treats these superscripts as COM/LPT digits.
                        | "¹"
                        | "²"
                        | "³"
                )
            })
        });
        !numbered_device
            && !matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
            )
    }) && Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}
fn check_cancellation(cancellation: Option<&BackupCancellation>) -> Result<(), String> {
    if let Some(cancellation) = cancellation {
        cancellation.check()?;
    }
    Ok(())
}
fn hash_file(
    path: &Path,
    cancellation: Option<&BackupCancellation>,
) -> Result<(u64, String), String> {
    let mut file = File::open(path)
        .map_err(|_| "백업할 파일을 열지 못했습니다. 파일 권한을 확인해 주세요.")?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; COPY_BUFFER];
    let mut size = 0u64;
    loop {
        check_cancellation(cancellation)?;
        let n = file
            .read(&mut buf)
            .map_err(|_| "백업 파일을 읽지 못했습니다.")?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or("백업 파일 크기가 올바르지 않습니다.")?;
        if size > MAX_ENTRY {
            return Err("백업 파일 하나가 허용 크기를 초과했습니다.".into());
        }
        hash.update(&buf[..n]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}
fn referenced_resources(
    database: &Path,
    resources_root: &Path,
    cancellation: Option<&BackupCancellation>,
) -> Result<Vec<(String, PathBuf)>, String> {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "백업 데이터베이스를 확인하지 못했습니다.")?;
    let mut query = connection.prepare("SELECT local_path,bytes,sha256 FROM resources WHERE status='downloaded' ORDER BY local_path")
        .map_err(|_| "백업 첨부 자료 목록을 확인하지 못했습니다.")?;
    let mut rows = query
        .query([])
        .map_err(|_| "백업 첨부 자료 목록을 확인하지 못했습니다.")?;
    let mut files = Vec::new();
    let mut seen: HashMap<String, (i64, String)> = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|_| "백업 첨부 자료 목록을 확인하지 못했습니다.")?
    {
        check_cancellation(cancellation)?;
        let name: String = row
            .get(0)
            .map_err(|_| "첨부 자료 경로가 올바르지 않습니다.")?;
        let expected_bytes: i64 = row
            .get(1)
            .map_err(|_| "첨부 자료 크기가 올바르지 않습니다.")?;
        let expected_hash: String = row
            .get(2)
            .map_err(|_| "첨부 자료 해시가 올바르지 않습니다.")?;
        if !valid_path(&name) || name == "rhythm.sqlite3" || expected_bytes < 0 {
            return Err("첨부 자료 경로가 안전하지 않습니다.".into());
        }
        if let Some((size, hash)) = seen.get(&name) {
            if *size != expected_bytes || hash != &expected_hash {
                return Err("같은 첨부 경로를 가리키는 데이터베이스 정보가 서로 다릅니다.".into());
            }
            continue;
        }
        seen.insert(name.clone(), (expected_bytes, expected_hash.clone()));
        let path = resources_root
            .parent()
            .unwrap_or(resources_root)
            .join(&name);
        let mut current = resources_root
            .parent()
            .unwrap_or(resources_root)
            .to_path_buf();
        for part in Path::new(&name).components() {
            if let Component::Normal(component) = part {
                current.push(component);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|_| "데이터베이스가 참조하는 첨부 자료가 없습니다.")?;
                if metadata.file_type().is_symlink() {
                    return Err("첨부 자료 경로에 안전하지 않은 링크가 있습니다.".into());
                }
            }
        }
        let (actual_bytes, actual_hash) = hash_file(&path, cancellation)?;
        if actual_bytes != expected_bytes as u64 || actual_hash != expected_hash {
            return Err("보관된 첨부 자료가 데이터베이스와 일치하지 않습니다.".into());
        }
        files.push((name, path));
    }
    Ok(files)
}

/// Writes a standard ZIP archive. The caller supplies a SQLite online-backup
/// snapshot (never the live WAL database file) and the resources directory.
fn validate_snapshot_summary(
    path: &Path,
    expected_schema: i64,
    expected_documents: u64,
) -> Result<(), String> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "데이터베이스 스냅샷을 열지 못했습니다.")?;
    let schema: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| "데이터베이스 스키마를 읽지 못했습니다.")?;
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .map_err(|_| "백업 공지 수를 확인하지 못했습니다.")?;
    if schema != expected_schema || documents < 0 || documents as u64 != expected_documents {
        return Err("데이터베이스 스냅샷의 요약 정보가 일치하지 않습니다.".into());
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn create(
    destination: &Path,
    snapshot: &Path,
    resources_root: &Path,
    created_at: String,
    database_schema: i64,
    documents: u64,
) -> Result<BackupPreview, String> {
    let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
    create_cancellable(
        destination,
        snapshot,
        resources_root,
        created_at,
        database_schema,
        documents,
        &cancellation,
    )
}
fn create_cancellable(
    destination: &Path,
    snapshot: &Path,
    resources_root: &Path,
    created_at: String,
    database_schema: i64,
    documents: u64,
    cancellation: &BackupCancellation,
) -> Result<BackupPreview, String> {
    check_cancellation(Some(cancellation))?;
    validate_snapshot_summary(snapshot, database_schema, documents)?;
    let mut sources = vec![("rhythm.sqlite3".to_owned(), snapshot.to_owned())];
    sources.extend(referenced_resources(
        snapshot,
        resources_root,
        Some(cancellation),
    )?);
    if sources.len() > MAX_ENTRIES {
        return Err("백업에 포함된 파일이 너무 많습니다.".into());
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(sources.len());
    let mut total = 0u64;
    let mut resource_bytes = 0u64;
    for (name, path) in &sources {
        if !valid_path(name) || !seen.insert(name.as_str()) {
            return Err("백업 파일 경로가 중복되었거나 올바르지 않습니다.".into());
        }
        check_cancellation(Some(cancellation))?;
        let (bytes, sha256) = hash_file(path, Some(cancellation))?;
        total = total
            .checked_add(bytes)
            .ok_or("백업 전체 크기가 올바르지 않습니다.")?;
        if total > MAX_TOTAL {
            return Err("백업 전체 크기가 허용 범위를 초과했습니다.".into());
        }
        if name.starts_with("resources/") {
            resource_bytes += bytes;
        }
        entries.push(Entry {
            path: name.clone(),
            bytes,
            sha256,
        });
    }
    if !seen.contains("rhythm.sqlite3") {
        return Err("데이터베이스 스냅샷이 없습니다.".into());
    }
    let resources = entries
        .iter()
        .filter(|e| e.path.starts_with("resources/"))
        .count() as u64;
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        created_at,
        database_schema,
        documents,
        resources,
        resource_bytes,
        entries,
    };
    if destination.exists() {
        return Err("같은 이름의 파일이 이미 있습니다. 다른 저장 위치를 선택해 주세요.".into());
    }
    static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(
        ".rhythm-backup-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _guard = RemoveOnDrop(temporary.clone());
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "백업 임시 파일을 만들지 못했습니다. 저장 위치와 권한을 확인해 주세요.")?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    zip.start_file(MANIFEST, options)
        .map_err(|_| "백업 압축 파일을 만들지 못했습니다.")?;
    serde_json::to_writer(&mut zip, &manifest).map_err(|_| "백업 정보를 기록하지 못했습니다.")?;
    let mut buffer = [0u8; COPY_BUFFER];
    for ((name, source), entry) in sources.iter().zip(&manifest.entries) {
        if name != &entry.path {
            return Err("백업 파일 목록이 변경되었습니다.".into());
        }
        zip.start_file(name.as_str(), options)
            .map_err(|_| "백업 압축 파일을 만들지 못했습니다.")?;
        let mut input = File::open(source).map_err(|_| "백업할 파일을 다시 열지 못했습니다.")?;
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        loop {
            check_cancellation(Some(cancellation))?;
            let n = input
                .read(&mut buffer)
                .map_err(|_| "백업 파일을 읽지 못했습니다.")?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            hash.update(&buffer[..n]);
            zip.write_all(&buffer[..n])
                .map_err(|_| "백업 파일을 기록하지 못했습니다. 저장 공간을 확인해 주세요.")?;
        }
        if bytes != entry.bytes || format!("{:x}", hash.finalize()) != entry.sha256 {
            return Err("백업 자료가 생성 중 변경되어 무결성 확인에 실패했습니다.".into());
        }
    }
    let file = zip
        .finish()
        .map_err(|_| "백업 파일을 마무리하지 못했습니다. 저장 공간을 확인해 주세요.")?;
    file.sync_all()
        .map_err(|_| "백업 파일을 저장하지 못했습니다. 저장 공간을 확인해 주세요.")?;
    if destination.exists() {
        return Err("같은 이름의 파일이 이미 있습니다. 다른 저장 위치를 선택해 주세요.".into());
    }
    cancellation.begin_commit()?;
    publish_without_replace(&temporary, destination)?;
    Ok(BackupPreview::from(&manifest))
}

fn read_manifest<R: Read + io::Seek>(zip: &mut ZipArchive<R>) -> Result<Manifest, String> {
    if zip.len() < 2 || zip.len() > MAX_ENTRIES + 1 {
        return Err("백업 파일 목록이 올바르지 않습니다.".into());
    }
    let mut file = zip
        .by_name(MANIFEST)
        .map_err(|_| "백업 정보가 없는 파일입니다.")?;
    if file.size() > MAX_MANIFEST {
        return Err("백업 정보가 너무 큽니다.".into());
    }
    let mut raw = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut raw)
        .map_err(|_| "백업 정보를 읽지 못했습니다.")?;
    drop(file);
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|_| "백업 정보가 손상되었습니다.")?;
    if manifest.format_version != FORMAT_VERSION {
        return Err("이 Rhythm 백업 형식은 지원하지 않습니다.".into());
    }
    if manifest.entries.len() + 1 != zip.len() || manifest.entries.len() > MAX_ENTRIES {
        return Err("백업 파일 목록과 내용이 일치하지 않습니다.".into());
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut resource_bytes = 0u64;
    let mut resources = 0u64;
    for entry in &manifest.entries {
        if !valid_path(&entry.path) || !seen.insert(entry.path.as_str()) || entry.bytes > MAX_ENTRY
        {
            return Err("백업 파일 경로나 크기가 올바르지 않습니다.".into());
        }
        total = total
            .checked_add(entry.bytes)
            .ok_or("백업 전체 크기가 올바르지 않습니다.")?;
        if total > MAX_TOTAL {
            return Err("백업 전체 크기가 허용 범위를 초과했습니다.".into());
        }
        if entry.path.starts_with("resources/") {
            resources += 1;
            resource_bytes += entry.bytes;
        }
    }
    if !seen.contains("rhythm.sqlite3")
        || resources != manifest.resources
        || resource_bytes != manifest.resource_bytes
    {
        return Err("백업 요약과 파일 목록이 일치하지 않습니다.".into());
    }
    Ok(manifest)
}

fn verify_zip<R: Read + io::Seek>(
    zip: &mut ZipArchive<R>,
    manifest: &Manifest,
    extract: Option<&Path>,
    cancellation: Option<&BackupCancellation>,
) -> Result<(), String> {
    let mut sum = 0u64;
    let mut buffer = [0u8; COPY_BUFFER];
    for entry in &manifest.entries {
        check_cancellation(cancellation)?;
        let mut file = zip
            .by_name(&entry.path)
            .map_err(|_| "백업 파일이 누락되었습니다.")?;
        if file.is_dir()
            || file.size() != entry.bytes
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("백업에 안전하지 않은 파일이 있습니다.".into());
        }
        let path = extract.map(|root| root.join(&entry.path));
        let mut output = if let Some(path) = &path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| "복원 임시 폴더를 만들지 못했습니다.")?;
            }
            Some(File::create(path).map_err(|_| "복원 임시 파일을 만들지 못했습니다.")?)
        } else {
            None
        };
        let mut hash = Sha256::new();
        let mut copied = 0u64;
        loop {
            check_cancellation(cancellation)?;
            let n = file
                .read(&mut buffer)
                .map_err(|_| "백업 파일의 압축을 풀거나 읽지 못했습니다.")?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > entry.bytes {
                return Err("백업 파일 크기가 선언과 일치하지 않습니다.".into());
            }
            hash.update(&buffer[..n]);
            if let Some(out) = output.as_mut() {
                out.write_all(&buffer[..n])
                    .map_err(|_| "복원 임시 파일을 기록하지 못했습니다.")?;
            }
        }
        if copied != entry.bytes || format!("{:x}", hash.finalize()) != entry.sha256 {
            return Err(format!("백업 파일 무결성 확인 실패: {}", entry.path));
        }
        if let Some(out) = output {
            out.sync_all()
                .map_err(|_| "복원 임시 파일을 저장하지 못했습니다.")?;
        }
        sum = sum
            .checked_add(copied)
            .ok_or("백업 전체 크기가 올바르지 않습니다.")?;
    }
    if sum > MAX_TOTAL {
        return Err("백업 전체 크기가 허용 범위를 초과했습니다.".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn inspect(path: &Path) -> Result<BackupPreview, String> {
    let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
    inspect_cancellable(path, &cancellation)
}
fn inspect_cancellable(
    path: &Path,
    cancellation: &BackupCancellation,
) -> Result<BackupPreview, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let staging = std::env::temp_dir().join(format!(
        "rhythm-backup-inspect-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    let cleanup = RemoveDirectoryOnDrop(staging.clone());
    let preview = extract_verified_cancellable(path, &staging, cancellation)?;
    validate_staged_database_cancellable(
        &staging,
        preview.database_schema,
        preview.documents,
        preview.resources,
        Some(cancellation),
    )?;
    check_cancellation(Some(cancellation))?;
    drop(cleanup);
    Ok(preview)
}

fn require_restore_space(available: u64, expanded: u64) -> Result<(), String> {
    let required = expanded
        .checked_add(1024 * 1024 * 1024)
        .ok_or("복원에 필요한 저장 공간을 확인하지 못했습니다.")?;
    if available < required {
        return Err("복원에 필요한 저장 공간이 부족합니다. 백업 크기 외에 1 GiB 이상의 여유 공간을 확보해 주세요.".into());
    }
    Ok(())
}

#[cfg(test)]
/// Extracts only manifest-listed regular files and verifies each hash.
pub(crate) fn extract_verified(path: &Path, staging: &Path) -> Result<BackupPreview, String> {
    let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
    extract_verified_cancellable(path, staging, &cancellation)
}
fn extract_verified_cancellable(
    path: &Path,
    staging: &Path,
    cancellation: &BackupCancellation,
) -> Result<BackupPreview, String> {
    fs::create_dir(staging).map_err(|_| "복원 준비 폴더를 만들지 못했습니다.")?;
    let result = (|| {
        let file = File::open(path).map_err(|_| "백업 파일을 열지 못했습니다.")?;
        let mut zip =
            ZipArchive::new(file).map_err(|_| "ZIP 백업 파일이 손상되었거나 올바르지 않습니다.")?;
        let manifest = read_manifest(&mut zip)?;
        let expanded = manifest
            .entries
            .iter()
            .try_fold(0u64, |sum, entry| sum.checked_add(entry.bytes))
            .ok_or("백업 전체 크기가 올바르지 않습니다.")?;
        let available = crate::intelligence::models::available_space(staging)?;
        require_restore_space(available, expanded)?;
        verify_zip(&mut zip, &manifest, Some(staging), Some(cancellation))?;
        check_cancellation(Some(cancellation))?;
        sync_tree(staging, staging)?;
        if let Some(parent) = staging.parent() {
            sync_directory(parent)?;
        }
        Ok(BackupPreview::from(&manifest))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(path: &Path, document_count: u64, resource: Option<(&str, &[u8])>) {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("CREATE TABLE documents(id INTEGER PRIMARY KEY); CREATE TABLE resources(local_path TEXT,bytes INTEGER,sha256 TEXT,status TEXT); PRAGMA user_version=6;").unwrap();
        for id in 1..=document_count {
            connection
                .execute("INSERT INTO documents(id) VALUES(?1)", [id as i64])
                .unwrap();
        }
        if let Some((name, bytes)) = resource {
            connection
                .execute(
                    "INSERT INTO resources VALUES(?1,?2,?3,'downloaded')",
                    rusqlite::params![
                        name,
                        bytes.len() as i64,
                        format!("{:x}", Sha256::digest(bytes))
                    ],
                )
                .unwrap();
        }
    }
    fn journal_preview(resources: u64) -> BackupPreview {
        BackupPreview {
            format_version: 1,
            created_at: "test".into(),
            database_schema: 6,
            documents: 0,
            resources,
            resource_bytes: 0,
            includes_model_weights: false,
        }
    }
    #[test]
    fn cancellation_and_restore_commit_are_mutually_exclusive() {
        let cancelled = BackupCancellation(AtomicU8::new(CANCELLABLE));
        assert!(cancelled.request());
        assert!(cancelled.check().is_err());
        assert!(cancelled.begin_commit().is_err());
        let committing = BackupCancellation(AtomicU8::new(CANCELLABLE));
        committing.begin_commit().unwrap();
        assert!(!committing.request());
        assert!(committing.check().is_ok());
    }
    #[test]
    fn backup_publish_never_replaces_a_concurrently_created_destination() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = directory.path().join("complete.tmp");
        let destination = directory.path().join("user-file.rhythm-backup");
        fs::write(&temporary, b"completed backup").unwrap();
        fs::write(&destination, b"preexisting user data").unwrap();
        assert!(publish_without_replace(&temporary, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"preexisting user data");
        assert_eq!(fs::read(&temporary).unwrap(), b"completed backup");
    }
    #[test]
    fn journal_directory_sync_failure_keeps_recovery_journal_and_stage() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(STAGING);
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("marker"), b"verified staging").unwrap();
        let result = write_journal_with_sync(
            root.path(),
            &format!("{PREVIOUS_PREFIX}test"),
            false,
            &journal_preview(0),
            |_| Err("simulated directory sync failure".into()),
        );
        assert!(result.is_err());
        assert!(root.path().join(JOURNAL).is_file());
        assert_eq!(fs::read(stage.join("marker")).unwrap(), b"verified staging");
        assert!(!root.path().join(".rhythm-restore-journal.tmp").exists());
    }

    #[test]
    fn cancellation_before_create_or_restore_commit_leaves_no_published_files() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("snapshot.db");
        let resources = directory.path().join("resources");
        fs::create_dir(&resources).unwrap();
        database(&db, 0, None);
        let archive = directory.path().join("cancelled.rhythm-backup");
        let cancelled = BackupCancellation(AtomicU8::new(CANCEL_REQUESTED));
        assert!(
            create_cancellable(&archive, &db, &resources, "test".into(), 6, 0, &cancelled).is_err()
        );
        assert!(!archive.exists());

        let valid_archive = directory.path().join("valid.rhythm-backup");
        create(&valid_archive, &db, &resources, "test".into(), 6, 0).unwrap();
        let staging = directory.path().join("cancelled-stage");
        assert!(extract_verified_cancellable(&valid_archive, &staging, &cancelled).is_err());
        assert!(!staging.exists());
        assert!(!directory.path().join(JOURNAL).exists());
    }
    #[test]
    fn archive_round_trip_verifies_manifest_and_hashes() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("snapshot.db");
        let resources = directory.path().join("resources");
        fs::create_dir(&resources).unwrap();
        database(&db, 1, Some(("resources/image.png", b"resource bytes")));
        fs::write(resources.join("image.png"), b"resource bytes").unwrap();
        let archive = directory.path().join("one.rhythm-backup");
        create(
            &archive,
            &db,
            &resources,
            "2026-10-31T00:00:00Z".into(),
            6,
            1,
        )
        .unwrap();
        let preview = inspect(&archive).unwrap();
        assert_eq!(preview.format_version, FORMAT_VERSION);
        assert_eq!(preview.database_schema, 6);
        assert_eq!(preview.documents, 1);
        assert_eq!(preview.resources, 1);
        assert_eq!(preview.resource_bytes, b"resource bytes".len() as u64);
        assert!(!preview.includes_model_weights);
        let output = directory.path().join("restored");
        extract_verified(&archive, &output).unwrap();
        validate_staged_database(&output, 6, 1, 1).unwrap();
        assert_eq!(
            fs::read(output.join("resources/image.png")).unwrap(),
            b"resource bytes"
        );
    }

    #[test]
    fn rejects_corrupt_or_incomplete_archive_and_unsafe_manifest_paths() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("snapshot.db");
        let resources = directory.path().join("resources");
        fs::create_dir(&resources).unwrap();
        database(&db, 0, None);
        let archive = directory.path().join("backup.zip");
        create(&archive, &db, &resources, "now".into(), 6, 0).unwrap();
        let mut bytes = fs::read(&archive).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&archive, bytes).unwrap();
        assert!(inspect(&archive).is_err());
        assert!(!valid_path("../models/weight.gguf"));
        assert!(!valid_path("resources/../../outside"));
        assert!(valid_path("resources/a/b.png"));
    }
    #[test]
    fn portable_path_check_rejects_windows_aliases_and_reserved_names() {
        for name in [
            "resources/../outside",
            "resources/.. /outside",
            "resources/item./x",
            "resources/item /x",
            "resources/CON",
            "resources/nul.txt",
            "resources/COM1.bin",
            "resources/LPT9",
            "resources/CON .txt",
            "resources/COM¹",
            "resources/COM².txt",
            "resources/COM³.bin",
            "resources/LPT¹",
            "resources/LPT².txt",
            "resources/com¹.log",
            "resources/LPT³.bin",
        ] {
            assert!(!valid_path(name), "accepted unsafe archive name: {name}");
        }
    }
    #[test]
    fn restore_space_check_rejects_insufficient_or_overflowing_capacity() {
        const RESERVE: u64 = 1024 * 1024 * 1024;
        assert!(require_restore_space(RESERVE + 10, 10).is_ok());
        assert!(require_restore_space(RESERVE + 9, 10).is_err());
        assert!(require_restore_space(u64::MAX, u64::MAX).is_err());
    }
    #[test]
    #[cfg(unix)]
    fn empty_profile_archive_previews_and_restores_without_resource_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let data = workspace.path().join("source");
        fs::create_dir(&data).unwrap();
        fs::create_dir(data.join("resources")).unwrap();
        let database_path = data.join("rhythm.sqlite3");
        database(&database_path, 0, None);
        let archive = workspace.path().join("empty.rhythm-backup");
        create(
            &archive,
            &database_path,
            &data.join("resources"),
            "test".into(),
            6,
            0,
        )
        .unwrap();
        let archive_file = File::open(&archive).unwrap();
        assert_eq!(ZipArchive::new(archive_file).unwrap().len(), 2);
        let preview = inspect(&archive).unwrap();
        assert_eq!(preview.resources, 0);
        assert_eq!(preview.resource_bytes, 0);

        let target = tempfile::tempdir().unwrap();
        database(&target.path().join("rhythm.sqlite3"), 0, None);
        let stage = target.path().join(STAGING);
        let extracted = extract_verified(&archive, &stage).unwrap();
        assert_eq!(extracted.resources, 0);
        fs::create_dir(stage.join("resources")).unwrap();
        validate_staged_database(&stage, 6, 0, 0).unwrap();
        let previous = format!("{PREVIOUS_PREFIX}fresh");
        write_journal(target.path(), &previous, false, &preview).unwrap();
        recover_pending_restore(target.path()).unwrap();
        validate_staged_database(target.path(), 6, 0, 0).unwrap();
        assert!(target.path().join("resources").is_dir());
        assert_eq!(
            fs::read_dir(target.path().join("resources"))
                .unwrap()
                .count(),
            0
        );
        assert!(target
            .path()
            .join(format!("{previous}/rhythm.sqlite3"))
            .is_file());
    }

    #[cfg(unix)]
    #[test]
    fn complete_restore_preserves_previous_data_models_and_reconciles_missing_weights() {
        let workspace = tempfile::tempdir().unwrap();
        let source_db = workspace.path().join("snapshot.db");
        let source_resources = workspace.path().join("resources");
        fs::create_dir(&source_resources).unwrap();
        database(&source_db, 1, Some(("resources/new.bin", b"new resource")));
        let source_connection = rusqlite::Connection::open(&source_db).unwrap();
        source_connection.execute_batch("CREATE TABLE ai_models(id TEXT,path TEXT,ownership TEXT,status TEXT,error TEXT,sha256 TEXT,bytes INTEGER); CREATE TABLE ai_preferences(active_model_id TEXT);").unwrap();
        let present_bytes = b"leave existing weights untouched";
        let absent_bytes = b"missing managed weights";
        source_connection.execute("INSERT INTO ai_models VALUES('managed','models/managed.gguf','managed','installed',NULL,?1,?2)", rusqlite::params![format!("{:x}", Sha256::digest(present_bytes)), present_bytes.len() as i64]).unwrap();
        source_connection.execute("INSERT INTO ai_models VALUES('missing','models/missing.gguf','managed','installed',NULL,?1,?2)", rusqlite::params![format!("{:x}", Sha256::digest(absent_bytes)), absent_bytes.len() as i64]).unwrap();
        source_connection.execute("INSERT INTO ai_models VALUES('external','/outside/model.gguf','external','installed',NULL,?1,?2)", rusqlite::params![format!("{:x}", Sha256::digest(absent_bytes)), absent_bytes.len() as i64]).unwrap();
        source_connection
            .execute("INSERT INTO ai_preferences VALUES('managed')", [])
            .unwrap();
        drop(source_connection);
        fs::write(source_resources.join("new.bin"), b"new resource").unwrap();
        let archive = workspace.path().join("backup.rhythm-backup");
        create(&archive, &source_db, &source_resources, "test".into(), 6, 1).unwrap();

        let root = tempfile::tempdir().unwrap();
        let prior_db = root.path().join("rhythm.sqlite3");
        database(&prior_db, 0, None);
        fs::create_dir(root.path().join("resources")).unwrap();
        fs::write(root.path().join("resources/old.bin"), b"old resource").unwrap();
        fs::create_dir(root.path().join("models")).unwrap();
        fs::write(
            root.path().join("models/managed.gguf"),
            b"leave existing weights untouched",
        )
        .unwrap();
        fs::write(
            root.path().join("models/unlisted.gguf"),
            b"preserve orphaned weights",
        )
        .unwrap();
        let previous_name = format!("{PREVIOUS_PREFIX}complete");
        let stage = root.path().join(STAGING);
        let preview = extract_verified(&archive, &stage).unwrap();
        let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
        reconcile_model_inventory(&stage, &root.path().join("models"), &cancellation).unwrap();
        validate_staged_database(
            &stage,
            preview.database_schema,
            preview.documents,
            preview.resources,
        )
        .unwrap();
        write_journal(root.path(), &previous_name, true, &preview).unwrap();

        recover_pending_restore(root.path()).unwrap();
        validate_staged_database(root.path(), 6, 1, 1).unwrap();
        let restored = rusqlite::Connection::open(root.path().join("rhythm.sqlite3")).unwrap();
        assert_eq!(
            restored
                .query_row(
                    "SELECT status FROM ai_models WHERE id='managed'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "installed"
        );
        assert_eq!(
            restored
                .query_row(
                    "SELECT status FROM ai_models WHERE id='missing'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "missing"
        );
        assert_eq!(
            restored
                .query_row("SELECT active_model_id FROM ai_preferences", [], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .unwrap(),
            Some("managed".into())
        );
        assert_eq!(
            restored
                .query_row(
                    "SELECT status FROM ai_models WHERE id='external'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "installed"
        );
        assert_eq!(
            fs::read(root.path().join("resources/new.bin")).unwrap(),
            b"new resource"
        );
        assert_eq!(
            fs::read(
                root.path()
                    .join(format!("{previous_name}/resources/old.bin"))
            )
            .unwrap(),
            b"old resource"
        );
        let previous_db =
            rusqlite::Connection::open(root.path().join(format!("{previous_name}/rhythm.sqlite3")))
                .unwrap();
        assert_eq!(
            previous_db
                .query_row("SELECT COUNT(*) FROM documents", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            fs::read(root.path().join("models/managed.gguf")).unwrap(),
            b"leave existing weights untouched"
        );
        assert_eq!(
            fs::read(root.path().join("models/unlisted.gguf")).unwrap(),
            b"preserve orphaned weights"
        );
    }

    #[cfg(unix)]
    #[test]
    fn backup_refuses_resource_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("snapshot.db");
        let resources = directory.path().join("resources");
        fs::create_dir(&resources).unwrap();
        fs::write(&db, b"db").unwrap();
        symlink(&db, resources.join("linked")).unwrap();
        assert!(create(
            &directory.path().join("backup.zip"),
            &db,
            &resources,
            "now".into(),
            6,
            0
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn restore_recovery_finishes_after_each_old_data_rename() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(STAGING);
        let previous_name = format!("{PREVIOUS_PREFIX}test");
        let previous = root.path().join(&previous_name);
        fs::create_dir_all(&previous).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::create_dir(root.path().join("resources")).unwrap();
        fs::write(previous.join("rhythm.sqlite3"), b"previous db").unwrap();
        fs::write(root.path().join("resources/old.bin"), b"previous resource").unwrap();
        fs::create_dir(stage.join("resources")).unwrap();
        database(
            &stage.join("rhythm.sqlite3"),
            0,
            Some(("resources/new.bin", b"restored resource")),
        );
        fs::write(stage.join("resources/new.bin"), b"restored resource").unwrap();
        write_journal(root.path(), &previous_name, true, &journal_preview(1)).unwrap();

        recover_pending_restore(root.path()).unwrap();
        let restored = rusqlite::Connection::open(root.path().join("rhythm.sqlite3")).unwrap();
        assert_eq!(
            restored
                .query_row("SELECT COUNT(*) FROM documents", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            fs::read(root.path().join("resources/new.bin")).unwrap(),
            b"restored resource"
        );
        assert_eq!(
            fs::read(previous.join("rhythm.sqlite3")).unwrap(),
            b"previous db"
        );
        assert_eq!(
            fs::read(previous.join("resources/old.bin")).unwrap(),
            b"previous resource"
        );
        assert!(!root.path().join(JOURNAL).exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_recovery_is_idempotent_after_install_before_journal_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join(STAGING);
        let previous_name = format!("{PREVIOUS_PREFIX}test");
        let previous = root.path().join(&previous_name);
        fs::create_dir_all(previous.join("resources")).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(previous.join("rhythm.sqlite3"), b"old").unwrap();
        fs::create_dir(root.path().join("resources")).unwrap();
        database(
            &root.path().join("rhythm.sqlite3"),
            0,
            Some(("resources/new.bin", b"new")),
        );
        fs::write(root.path().join("resources/new.bin"), b"new").unwrap();
        write_journal(root.path(), &previous_name, true, &journal_preview(1)).unwrap();

        recover_pending_restore(root.path()).unwrap();
        recover_pending_restore(root.path()).unwrap();
        let restored = rusqlite::Connection::open(root.path().join("rhythm.sqlite3")).unwrap();
        assert_eq!(
            restored
                .query_row("SELECT COUNT(*) FROM resources", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(fs::read(previous.join("rhythm.sqlite3")).unwrap(), b"old");
    }
    #[cfg(windows)]
    #[test]
    fn pending_restore_on_windows_is_refused_without_changing_live_data() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("rhythm.sqlite3"), b"existing database").unwrap();
        fs::create_dir(root.path().join(STAGING)).unwrap();
        write_journal(
            root.path(),
            &format!("{PREVIOUS_PREFIX}windows-test"),
            false,
            &journal_preview(0),
        )
        .unwrap();

        assert!(recover_pending_restore(root.path()).is_err());
        assert_eq!(
            fs::read(root.path().join("rhythm.sqlite3")).unwrap(),
            b"existing database"
        );
        assert!(root.path().join(JOURNAL).exists());
        assert!(root.path().join(STAGING).is_dir());
    }
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[tauri::command]
pub(crate) async fn backup_create(
    state: tauri::State<'_, crate::AppState>,
    destination: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    let registration = register_cancellation(operation_id)?;
    let cancellation = registration.cancellation.clone();
    let maintenance = state.exclusive_maintenance().await?;
    let destination = PathBuf::from(destination);
    if destination.as_os_str().is_empty() {
        return Err("백업 파일 위치를 선택해 주세요.".into());
    }
    let root = state.data_root.clone();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let _registration = registration;
        let _maintenance = maintenance;
        check_cancellation(Some(&cancellation))?;
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let snapshot = root.join(format!(
            ".rhythm-backup-{}-{}.sqlite3",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let cleanup = RemoveOnDrop(snapshot.clone());
        let (schema, documents) = {
            let store = store
                .lock()
                .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
            let (schema, documents) = store.backup_summary()?;
            store.backup_to(&snapshot)?;
            check_cancellation(Some(&cancellation))?;
            (schema, documents)
        };
        let result = create_cancellable(
            &destination,
            &snapshot,
            &root.join("resources"),
            timestamp(),
            schema,
            documents,
            &cancellation,
        );
        drop(cleanup);
        result
    })
    .await
    .map_err(|_| "백업 작업을 마치지 못했습니다.".to_owned())?
}

#[tauri::command]
pub(crate) async fn backup_inspect(
    source: String,
    operation_id: String,
) -> Result<BackupPreview, String> {
    let registration = register_cancellation(operation_id)?;
    let cancellation = registration.cancellation.clone();
    tokio::task::spawn_blocking(move || {
        let _registration = registration;
        inspect_cancellable(Path::new(&source), &cancellation)
    })
    .await
    .map_err(|_| "백업 파일을 검사하지 못했습니다.".to_owned())?
}

fn staged_resource_names(
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
        if kind.is_symlink() {
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
fn validate_staged_database(
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
fn validate_staged_database_cancellable(
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
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
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

fn reconcile_model_inventory(
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
    let mut query = connection
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
    let transaction = connection
        .unchecked_transaction()
        .map_err(|_| "백업 모델 상태를 갱신하지 못했습니다.")?;
    for id in missing {
        transaction.execute("UPDATE ai_models SET status='missing',error='해당 모델 파일이 없거나 무결성 확인에 실패했습니다. 다시 설치해 주세요.' WHERE id=?1", [&id])
            .map_err(|_| "백업 모델 상태를 갱신하지 못했습니다.")?;
        transaction
            .execute(
                "UPDATE ai_preferences SET active_model_id=NULL WHERE active_model_id=?1",
                [&id],
            )
            .map_err(|_| "백업에서 사용할 모델을 정리하지 못했습니다.")?;
    }
    transaction
        .commit()
        .map_err(|_| "백업 모델 상태를 저장하지 못했습니다.")?;
    Ok(())
}

fn managed_model_matches(
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
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let path = model_root.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("로컬 모델 파일을 확인하지 못했습니다.".into()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != expected_bytes.unwrap_or_default() as u64
    {
        return Ok(false);
    }
    let (bytes, hash) = hash_file(&path, Some(cancellation))
        .map_err(|_| "로컬 모델 파일의 무결성을 확인하지 못했습니다.".to_owned())?;
    Ok(bytes == expected_bytes.unwrap_or_default() as u64
        && expected_hash.is_some_and(|expected| hash.eq_ignore_ascii_case(expected)))
}

fn valid_model_filename(name: &str) -> bool {
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
struct RestoreJournal {
    version: u32,
    previous: String,
    had_resources: bool,
    schema: i64,
    documents: u64,
    resources: u64,
}
const STAGING: &str = ".rhythm-restore-staging";
const JOURNAL: &str = ".rhythm-restore-journal.json";
const PREVIOUS_PREFIX: &str = ".rhythm-restore-previous-";

fn write_journal(
    root: &Path,
    previous: &str,
    had_resources: bool,
    preview: &BackupPreview,
) -> Result<(), String> {
    write_journal_with_sync(root, previous, had_resources, preview, sync_directory)
}
fn write_journal_with_sync(
    root: &Path,
    previous: &str,
    had_resources: bool,
    preview: &BackupPreview,
    sync: impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let temp = root.join(".rhythm-restore-journal.tmp");
    let _cleanup = RemoveOnDrop(temp.clone());
    let bytes = serde_json::to_vec(&RestoreJournal {
        version: 1,
        previous: previous.to_owned(),
        had_resources,
        schema: preview.database_schema,
        documents: preview.documents,
        resources: preview.resources,
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
    fs::rename(&temp, root.join(JOURNAL)).map_err(|_| "복원 상태를 확정하지 못했습니다.")?;
    sync(root)
}
fn publish_without_replace(temporary: &Path, destination: &Path) -> Result<(), String> {
    fs::hard_link(temporary, destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            "같은 이름의 파일이 이미 있습니다. 다른 저장 위치를 선택해 주세요.".into()
        } else {
            "백업 파일을 안전하게 저장 위치로 옮기지 못했습니다. 저장 위치와 파일 시스템 권한을 확인해 주세요.".into()
        }
    })
}
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "복원 상태를 안정적으로 저장하지 못했습니다.".into())
}
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}
fn require_restore_durability_support(supported: bool) -> Result<(), String> {
    if supported {
        Ok(())
    } else {
        Err(
            "이 플랫폼은 복원 상태의 파일 시스템 내구성을 보장하지 않아 복원을 지원하지 않습니다."
                .into(),
        )
    }
}

#[cfg(test)]
mod restore_durability_tests {
    use super::require_restore_durability_support;

    #[test]
    fn restore_remains_blocked_when_durability_is_not_established() {
        let error = require_restore_durability_support(false).unwrap_err();
        assert!(error.contains("파일 시스템 내구성"));
    }

    #[test]
    fn restore_gate_allows_only_platforms_with_established_durability() {
        assert!(require_restore_durability_support(true).is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn windows_restore_stays_disabled_without_durable_directory_sync() {
        assert!(!super::restore_durability_supported());
        assert!(require_restore_durability_support(super::restore_durability_supported()).is_err());
    }
}

fn restore_durability_supported() -> bool {
    // The current protocol relies on syncing directory entries after each
    // rename. Only Unix targets have the required, documented directory fsync
    // behavior implemented here; Windows remains gated pending an equivalent.
    cfg!(unix)
}

fn sync_tree(_root: &Path, directory: &Path) -> Result<(), String> {
    for item in fs::read_dir(directory).map_err(|_| "복원 폴더를 저장하지 못했습니다.")?
    {
        let item = item.map_err(|_| "복원 폴더를 저장하지 못했습니다.")?;
        if item
            .file_type()
            .map_err(|_| "복원 폴더를 저장하지 못했습니다.")?
            .is_dir()
        {
            sync_tree(_root, &item.path())?;
        }
    }
    sync_directory(directory)
}

/// Called after the process lock is acquired and before Store::open. Each rename
/// is recoverable from the journal and previous files remain available.
pub(crate) fn recover_pending_restore(root: &Path) -> Result<(), String> {
    let journal = root.join(JOURNAL);
    if !restore_durability_supported() && journal.exists() {
        return Err("이 플랫폼은 복원 상태의 파일 시스템 내구성을 보장하지 않아 자동 복원을 중단했습니다. 원본 파일은 변경하지 않았습니다.".into());
    }
    if !journal.exists() {
        let _ = fs::remove_file(root.join(".rhythm-restore-journal.tmp"));
        let _ = fs::remove_dir_all(root.join(STAGING));
        return Ok(());
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
    let previous = root.join(data.previous);
    if fs::symlink_metadata(&previous).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("기존 데이터 복구 폴더가 안전하지 않습니다.".into());
    }
    fs::create_dir_all(&previous).map_err(|_| "기존 데이터 복구 폴더를 만들지 못했습니다.")?;
    sync_directory(root)?;
    let stage_metadata = fs::symlink_metadata(&stage);
    match stage_metadata {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err("복원 임시 폴더 경로가 안전하지 않습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("복원 임시 폴더를 확인하지 못했습니다.".into()),
    }
    let live_db = root.join("rhythm.sqlite3");
    let old_db = previous.join("rhythm.sqlite3");
    let stage_db = stage.join("rhythm.sqlite3");
    if stage_db.exists() {
        for suffix in ["-wal", "-shm", "-journal"] {
            let live = root.join(format!("rhythm.sqlite3{suffix}"));
            let old = previous.join(format!("rhythm.sqlite3{suffix}"));
            if live.exists() && !old.exists() {
                fs::rename(&live, &old)
                    .map_err(|_| "기존 데이터베이스 로그를 복구 폴더로 옮기지 못했습니다.")?;
                sync_directory(root)?;
                sync_directory(&previous)?;
            }
        }
    }
    if stage_db.exists() {
        if live_db.exists() && !old_db.exists() {
            fs::rename(&live_db, &old_db)
                .map_err(|_| "기존 데이터베이스를 복구 폴더로 옮기지 못했습니다.")?;
            sync_directory(root)?;
            sync_directory(&previous)?;
        }
        fs::rename(&stage_db, &live_db).map_err(|_| "복원 데이터베이스를 설치하지 못했습니다.")?;
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
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("복원 경로에 안전하지 않은 링크가 있습니다.".into());
        }
    }
    if stage_resources.exists() {
        if live_resources.exists() {
            if !data.had_resources || old_resources.exists() {
                return Err("복원 첨부 자료 상태가 일치하지 않습니다.".into());
            }
            fs::rename(&live_resources, &old_resources)
                .map_err(|_| "기존 첨부 자료를 복구 폴더로 옮기지 못했습니다.")?;
            sync_directory(root)?;
            sync_directory(&previous)?;
        }
        fs::rename(&stage_resources, &live_resources)
            .map_err(|_| "복원 첨부 자료를 설치하지 못했습니다.")?;
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
    File::open(&live_db)
        .and_then(|f| f.sync_all())
        .map_err(|_| "복원 데이터베이스를 저장하지 못했습니다.")?;
    let _ = fs::remove_dir_all(&stage);
    fs::remove_file(journal).map_err(|_| "복원 상태를 정리하지 못했습니다.")?;
    sync_directory(root)?;
    Ok(())
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
    require_restore_durability_support(restore_durability_supported())?;
    let maintenance = state.exclusive_maintenance().await?;
    let root = state.data_root.clone();
    let app_for_restart = app.clone();
    tokio::task::spawn_blocking(move || {
        let _registration = registration;
        check_cancellation(Some(&cancellation))?;
        let maintenance = maintenance;
        let output = (|| -> Result<(), String> {
            if root.join(STAGING).exists() || root.join(JOURNAL).exists() {
                return Err("이전 복원 작업의 복구 파일이 남아 있습니다. 앱을 다시 시작한 뒤 확인해 주세요.".into());
            }
            let previous_name = format!("{PREVIOUS_PREFIX}{}-{}", timestamp(), std::process::id());
            if root.join(&previous_name).exists() { return Err("기존 데이터 복구 폴더가 이미 있습니다.".into()); }
            let database_metadata = fs::symlink_metadata(root.join("rhythm.sqlite3")).map_err(|_| "현재 데이터베이스를 찾지 못했습니다.")?;
            if !database_metadata.is_file() || database_metadata.file_type().is_symlink() {
                return Err("현재 데이터베이스 경로가 안전하지 않습니다.".into());
            }
            let resources_metadata = fs::symlink_metadata(root.join("resources"));
            let had_resources = match resources_metadata {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
                Ok(_) => return Err("현재 첨부 자료 폴더 경로가 안전하지 않습니다.".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err("현재 첨부 자료 폴더를 확인하지 못했습니다.".into()),
            };
            let stage = root.join(STAGING);
            let result = (|| {
                let preview = extract_verified_cancellable(Path::new(&source), &stage, &cancellation)?;
                fs::create_dir_all(stage.join("resources")).map_err(|_| "첨부 자료 폴더를 준비하지 못했습니다.")?;
                reconcile_model_inventory(&stage, &root.join("models"), &cancellation)?;
                validate_staged_database_cancellable(
                    &stage,
                    preview.database_schema,
                    preview.documents,
                    preview.resources,
                    Some(&cancellation),
                )?;
                sync_directory(&stage.join("resources"))?;
                sync_directory(&stage)?;
                check_cancellation(Some(&cancellation))?;
                cancellation.begin_commit()?;
                write_journal(&root, &previous_name, had_resources, &preview)?;
                Ok(())
            })();
            if result.is_err() && !root.join(JOURNAL).exists() {
                let _ = fs::remove_dir_all(&stage);
            }
            result
        })();
        if output.is_ok() || root.join(JOURNAL).exists() {
            std::mem::forget(maintenance);
            app_for_restart.restart();
        }
        output
    }).await.map_err(|_| "복원 준비를 마치지 못했습니다.".to_owned())??;
    Ok(())
}
