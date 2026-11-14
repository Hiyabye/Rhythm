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
    // Generation models were retired with summaries (#226).
    assert_eq!(ids, ["qwen3-embedding-0-6b"]);
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
    assert!(error.contains("다른 설치 폴더"), "{error}");
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
    assert_eq!(entries.len(), 1);
    for e in &entries {
        validate_manifest(e).unwrap();
        let origin = download_url(e).unwrap();
        validate_destination(&origin, &origin).unwrap();
        assert_eq!(e.kind, "embedding");
        assert_eq!(e.quantization, "Q8_0");
        assert!(e.pooling.is_some() && e.embedding_dim == Some(1024));
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
async fn managed_delete_clears_files_and_rejects_path_escape() {
    let (dir, store, e, bytes) = fixture();
    let mut m = inventory(&e);
    let cancel = Arc::new(AtomicBool::new(false));
    install(&store, dir.path(), &e, &mut m, &cancel, &Fake::new(bytes))
        .await
        .unwrap();
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

#[test]
fn gguf_architecture_is_found_after_values_that_span_the_read_buffer() {
    let entry = |key: &str, kind: u32, value: &[u8]| {
        let mut bytes = (key.len() as u64).to_le_bytes().to_vec();
        bytes.extend(key.as_bytes());
        bytes.extend(kind.to_le_bytes());
        bytes.extend(value);
        bytes
    };
    let text = |value: &[u8]| {
        let mut bytes = (value.len() as u64).to_le_bytes().to_vec();
        bytes.extend(value);
        bytes
    };
    let mut array = 8u32.to_le_bytes().to_vec();
    array.extend(3u64.to_le_bytes());
    for item in [&b"a"[..], &[b'b'; 12_000][..], b"c"] {
        array.extend(text(item));
    }
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3u32.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend(5u64.to_le_bytes());
    bytes.extend(entry("general.alignment", 4, &32u32.to_le_bytes()));
    bytes.extend(entry("general.description", 8, &text(&[b'x'; 20_000])));
    bytes.extend(entry("tokenizer.ggml.tokens", 9, &array));
    bytes.extend(entry("general.architecture", 8, &text(b"gemma4")));
    bytes.extend(entry("after", 4, &0u32.to_le_bytes()));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.gguf");
    std::fs::write(&path, &bytes).unwrap();
    assert!(verify_file(&path, None, &AtomicBool::new(false)).is_ok());
    // The same metadata with an unsupported architecture is still rejected,
    // and a skip that runs past the end of the file is a bounded error.
    let other = bytes
        .windows(6)
        .position(|window| window == b"gemma4")
        .unwrap();
    let mut unsupported = bytes.clone();
    unsupported[other..other + 6].copy_from_slice(b"gemma9");
    std::fs::write(&path, &unsupported).unwrap();
    assert!(verify_file(&path, None, &AtomicBool::new(false)).is_err());
    std::fs::write(&path, &bytes[..30_000]).unwrap();
    assert!(verify_file(&path, None, &AtomicBool::new(false)).is_err());
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
    let (dir, store, entry, bytes) = fixture();
    std::fs::create_dir_all(dir.path().join("models")).unwrap();
    let mut model = inventory(&entry);
    std::fs::write(dir.path().join(&model.path), &bytes).unwrap();
    model.status = "installed".into();
    model.sha256 = Some(entry.sha256.clone());
    model.bytes = entry.bytes;
    model.downloaded_bytes = entry.bytes;
    with_store(&store, |s| s.ai_upsert_model(&model)).unwrap();
    (dir, store, entry, model)
}
#[tokio::test]
async fn removing_a_model_also_removes_its_retired_projector() {
    // Vision projectors are no longer installed (#226), but a profile may
    // still hold one beside a generation model.
    let (dir, store, _entry, model) = installed_fixture();
    let projector_bytes = gguf("clip");
    let sha256 = format!("{:x}", Sha256::digest(&projector_bytes));
    let relative = format!("models/{sha256}.gguf");
    std::fs::write(dir.path().join(&relative), &projector_bytes).unwrap();
    with_store(&store, |s| {
        s.ai_upsert_projector(&crate::storage::intelligence::AiProjector {
            model_id: model.id.clone(),
            path: relative.clone(),
            sha256: Some(sha256.clone()),
            expected_sha256: sha256.clone(),
            expected_bytes: projector_bytes.len() as u64,
            bytes: projector_bytes.len() as u64,
            status: "installed".into(),
            downloaded_bytes: projector_bytes.len() as u64,
            resume_etag: None,
            error: None,
        })
    })
    .unwrap();
    remove_with_policy(
        store.clone(),
        dir.path().into(),
        model.id.clone(),
        true,
        false,
    )
    .await
    .unwrap();
    assert!(!dir.path().join(&relative).exists());
    assert!(with_store(&store, |s| s.ai_projector(&model.id))
        .unwrap()
        .is_none());
    assert!(with_store(&store, |s| s.ai_model(&model.id))
        .unwrap()
        .is_none());
}
