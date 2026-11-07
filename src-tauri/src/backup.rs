//! Versioned ZIP backup for Rhythm's database and downloaded archive resources.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, LazyLock, Mutex,
    },
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

#[cfg(windows)]
#[path = "backup_windows.rs"]
mod windows;

fn unsafe_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        windows::is_link(metadata)
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn restore_rename(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::rename(source, destination)
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

/// Check the actual data directory rather than implying every Windows volume
/// offers the recovery behavior this implementation requires.
pub(crate) fn restore_support(root: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::validate_root(root)
    }
    #[cfg(unix)]
    {
        let _ = root;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = root;
        Err("이 플랫폼에서는 복원을 지원하지 않습니다.".into())
    }
}

#[tauri::command]
pub(crate) fn backup_restore_support(
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    restore_support(&state.data_root)
}

const FORMAT_VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_ENTRY: u64 = 32 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024 * 1024;
const COPY_BUFFER: usize = 128 * 1024;

const CANCELLED: &str = "백업 또는 복원을 취소했습니다.";
const CANCELLABLE: u8 = 0;
const CANCEL_REQUESTED: u8 = 1;
const COMMITTING: u8 = 2;
static ACTIVE_CANCELLATIONS: LazyLock<Mutex<HashMap<String, Arc<BackupCancellation>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct BackupCancellation(AtomicU8);
impl BackupCancellation {
    fn check(&self) -> Result<(), String> {
        if self.0.load(Ordering::Acquire) == CANCEL_REQUESTED {
            Err(CANCELLED.into())
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
            .map_err(|_| CANCELLED.into())
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
/// A downloaded resource row with its database-recorded size and SHA-256.
struct ReferencedResource {
    name: String,
    path: PathBuf,
    bytes: u64,
    sha256: String,
}
/// Lists distinct downloaded resources and rejects unsafe or linked paths.
/// Content is verified against the recorded digest while it is archived.
fn referenced_resources(
    database: &Path,
    resources_root: &Path,
    cancellation: Option<&BackupCancellation>,
) -> Result<Vec<ReferencedResource>, String> {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "백업 데이터베이스를 확인하지 못했습니다.")?;
    let mut query = connection.prepare("SELECT local_path,bytes,sha256 FROM resources WHERE status='downloaded' ORDER BY local_path")
        .map_err(|_| "백업 첨부 자료 목록을 확인하지 못했습니다.")?;
    let mut rows = query
        .query([])
        .map_err(|_| "백업 첨부 자료 목록을 확인하지 못했습니다.")?;
    let mut files: Vec<ReferencedResource> = Vec::new();
    let data_root = resources_root.parent().unwrap_or(resources_root);
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
        // Rows are ordered by path, so repeated references are adjacent.
        if let Some(previous) = files.last().filter(|item| item.name == name) {
            if previous.bytes != expected_bytes as u64 || previous.sha256 != expected_hash {
                return Err("같은 첨부 경로를 가리키는 데이터베이스 정보가 서로 다릅니다.".into());
            }
            continue;
        }
        let mut current = data_root.to_path_buf();
        for part in Path::new(&name).components() {
            if let Component::Normal(component) = part {
                current.push(component);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|_| "데이터베이스가 참조하는 첨부 자료가 없습니다.")?;
                if unsafe_link(&metadata) {
                    return Err("첨부 자료 경로에 안전하지 않은 링크가 있습니다.".into());
                }
            }
        }
        files.push(ReferencedResource {
            path: data_root.join(&name),
            name,
            bytes: expected_bytes as u64,
            sha256: expected_hash,
        });
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
    let mut sources = vec![("rhythm.sqlite3".to_owned(), snapshot.to_owned(), None)];
    sources.extend(
        referenced_resources(snapshot, resources_root, Some(cancellation))?
            .into_iter()
            .map(|item| (item.name, item.path, Some((item.bytes, item.sha256)))),
    );
    if sources.len() > MAX_ENTRIES {
        return Err("백업에 포함된 파일이 너무 많습니다.".into());
    }
    let mut seen = HashSet::new();
    for (name, _, _) in &sources {
        if !valid_path(name) || !seen.insert(name.as_str()) {
            return Err("백업 파일 경로가 중복되었거나 올바르지 않습니다.".into());
        }
    }
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
    // Each file is read once: archived bytes are hashed as they are written and
    // resources must match the digest the database recorded for them. Readers
    // locate the manifest by name, so it is written last with the final digests.
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut entries = Vec::with_capacity(sources.len());
    let mut total = 0u64;
    let mut resource_bytes = 0u64;
    for (name, source, expected) in &sources {
        check_cancellation(Some(cancellation))?;
        let mut input = File::open(source)
            .map_err(|_| "백업할 파일을 열지 못했습니다. 파일 권한을 확인해 주세요.")?;
        zip.start_file(name.as_str(), options)
            .map_err(|_| "백업 압축 파일을 만들지 못했습니다.")?;
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
            bytes = bytes
                .checked_add(n as u64)
                .filter(|bytes| *bytes <= MAX_ENTRY)
                .ok_or("백업 파일 하나가 허용 크기를 초과했습니다.")?;
            hash.update(&buffer[..n]);
            zip.write_all(&buffer[..n])
                .map_err(|_| "백업 파일을 기록하지 못했습니다. 저장 공간을 확인해 주세요.")?;
        }
        let sha256 = format!("{:x}", hash.finalize());
        if let Some((expected_bytes, expected_hash)) = expected {
            if bytes != *expected_bytes || &sha256 != expected_hash {
                return Err("보관된 첨부 자료가 데이터베이스와 일치하지 않습니다.".into());
            }
            resource_bytes += bytes;
        }
        total = total
            .checked_add(bytes)
            .filter(|total| *total <= MAX_TOTAL)
            .ok_or("백업 전체 크기가 허용 범위를 초과했습니다.")?;
        entries.push(Entry {
            path: name.clone(),
            bytes,
            sha256,
        });
    }
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        created_at,
        database_schema,
        documents,
        resources: sources.len() as u64 - 1,
        resource_bytes,
        entries,
    };
    zip.start_file(MANIFEST, options)
        .map_err(|_| "백업 압축 파일을 만들지 못했습니다.")?;
    serde_json::to_writer(&mut zip, &manifest).map_err(|_| "백업 정보를 기록하지 못했습니다.")?;
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
            Some(
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|_| {
                        "복원 임시 파일을 만들지 못했습니다. 경로 중복 여부를 확인해 주세요."
                    })?,
            )
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
#[path = "backup_tests.rs"]
mod populated_tests;
#[cfg(test)]
mod tests;

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

mod restore;
pub(crate) use restore::*;
