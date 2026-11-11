use super::*;

/// Explicit, isolated live-validation fixture export; never runs in CI.
#[test]
#[ignore = "exports an explicitly selected isolated validation fixture"]
fn export_existing_validation_archive() {
    let source = PathBuf::from(
        std::env::var_os("RHYTHM_BACKUP_SOURCE_DIR").expect("set RHYTHM_BACKUP_SOURCE_DIR"),
    );
    let destination =
        PathBuf::from(std::env::var_os("RHYTHM_BACKUP_EXPORT").expect("set RHYTHM_BACKUP_EXPORT"));
    assert!(
        source.is_absolute() && source.is_dir(),
        "source must be an existing absolute directory"
    );
    assert!(destination.is_absolute(), "destination must be absolute");
    assert_eq!(
        destination.extension().and_then(|s| s.to_str()),
        Some("rhythm-backup")
    );
    assert!(
        matches!(fs::symlink_metadata(&destination), Err(error) if error.kind() == io::ErrorKind::NotFound),
        "destination must not exist and its status must be accessible"
    );
    let source = fs::canonicalize(source).expect("resolve source directory");
    let output_parent = fs::canonicalize(destination.parent().expect("destination parent"))
        .expect("resolve existing destination parent");
    assert!(
        !output_parent.starts_with(&source),
        "destination must be outside the source directory"
    );

    let original = rusqlite::Connection::open_with_flags(
        source.join("rhythm.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open source database read-only");
    let temporary = tempfile::tempdir().unwrap();
    let snapshot = temporary.path().join("snapshot.sqlite3");
    let mut target = rusqlite::Connection::open(&snapshot).unwrap();
    {
        let copy = rusqlite::backup::Backup::new(&original, &mut target).unwrap();
        copy.run_to_completion(i32::MAX, std::time::Duration::ZERO, None)
            .unwrap();
    }
    drop(original);
    target.execute_batch("PRAGMA journal_mode=DELETE;").unwrap();
    let schema: i64 = target
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    let documents: i64 = target
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .unwrap();
    let (resources, resource_bytes): (i64, i64) = target
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM (SELECT local_path, MAX(bytes) AS bytes FROM resources WHERE status='downloaded' GROUP BY local_path)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let references: (i64, i64) = target
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM resources WHERE status='downloaded'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let documents = u64::try_from(documents).expect("nonnegative document count");
    let resources = u64::try_from(resources).expect("nonnegative resource count");
    let resource_bytes = u64::try_from(resource_bytes).expect("nonnegative resource bytes");
    let created_at: String = target
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
            r.get(0)
        })
        .unwrap();
    drop(target);
    assert_eq!(
        (documents, resources, resource_bytes),
        (10, 7, 4_096_347),
        "unexpected validation fixture"
    );
    assert_eq!(
        references,
        (11, 7_117_417),
        "unexpected resource references"
    );
    let created = create(
        &destination,
        &snapshot,
        &source.join("resources"),
        created_at,
        schema,
        documents,
    )
    .unwrap();
    let verified = inspect(&destination).unwrap();
    assert_eq!(
        (
            verified.database_schema,
            verified.documents,
            verified.resources,
            verified.resource_bytes
        ),
        (schema, documents, resources, resource_bytes)
    );
    assert_eq!(
        (created.documents, created.resources, created.resource_bytes),
        (documents, resources, resource_bytes)
    );
    println!(
        "VALIDATION_BACKUP path={} schema={schema} documents={documents} resources={resources} bytes={resource_bytes}",
        destination.display()
    );
}

pub(super) fn database(path: &Path, document_count: u64, resource: Option<(&str, &[u8])>) {
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
pub(super) fn journal_preview(resources: u64) -> BackupPreview {
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
#[cfg(any(unix, windows))]
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

#[cfg(any(unix, windows))]
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
    source_connection.execute_batch("CREATE TABLE ai_projectors(model_id TEXT PRIMARY KEY,path TEXT,sha256 TEXT,expected_sha256 TEXT,expected_bytes INTEGER,bytes INTEGER,status TEXT,downloaded_bytes INTEGER,resume_etag TEXT,error TEXT);").unwrap();
    let present_projector = b"leave existing projector untouched";
    source_connection.execute("INSERT INTO ai_projectors VALUES('managed','models/present-projector.gguf',?1,?1,?2,?2,'installed',?2,NULL,NULL)", rusqlite::params![format!("{:x}", Sha256::digest(present_projector)), present_projector.len() as i64]).unwrap();
    source_connection.execute("INSERT INTO ai_projectors VALUES('missing','models/missing-projector.gguf',?1,?1,?2,?2,'installed',?2,NULL,NULL)", rusqlite::params![format!("{:x}", Sha256::digest(absent_bytes)), absent_bytes.len() as i64]).unwrap();
    source_connection.execute("INSERT INTO ai_projectors VALUES('external','models/external-projector.gguf',?1,?1,?2,?2,'installed',?2,NULL,NULL)", rusqlite::params![format!("{:x}", Sha256::digest(absent_bytes)), absent_bytes.len() as i64]).unwrap();
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
    fs::write(
        root.path().join("models/present-projector.gguf"),
        b"leave existing projector untouched",
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
    for (model, expected) in [
        ("managed", "installed"),
        ("missing", "missing"),
        ("external", "missing"),
    ] {
        assert_eq!(
            restored
                .query_row(
                    "SELECT status FROM ai_projectors WHERE model_id=?1",
                    [model],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            expected,
            "projector state for {model}"
        );
    }
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

#[cfg(any(unix, windows))]
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
fn windows_restore_with_missing_prepared_database_is_refused_without_changing_live_data() {
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

#[cfg(windows)]
#[test]
fn windows_unjournaled_restore_artifacts_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("rhythm.sqlite3"), b"original").unwrap();
    fs::create_dir(root.path().join(STAGING)).unwrap();
    fs::write(root.path().join(STAGING).join("unverified"), b"keep").unwrap();
    assert!(recover_pending_restore(root.path()).is_err());
    assert_eq!(
        fs::read(root.path().join("rhythm.sqlite3")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(root.path().join(STAGING).join("unverified")).unwrap(),
        b"keep"
    );
}

#[test]
fn restored_activation_tokens_are_cleared_in_staging_only() {
    let root = tempfile::tempdir().unwrap();
    let stage = root.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let db = stage.join("rhythm.sqlite3");
    database(&db, 0, None);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TABLE notification_activation_tokens(token TEXT); INSERT INTO notification_activation_tokens VALUES('old');").unwrap();
    drop(connection);
    clear_restored_activation_tokens(&stage).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM notification_activation_tokens",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn restored_seat_tokens_are_cleared_without_resetting_latches_or_rate_accounting() {
    let root = tempfile::tempdir().unwrap();
    let stage = root.path().join("stage");
    fs::create_dir(&stage).unwrap();
    let live = root.path().join("rhythm.sqlite3");
    let store = crate::storage::Store::open(&live).unwrap();
    let connection = rusqlite::Connection::open(&live).unwrap();
    connection.execute_batch(
        "UPDATE studium_alert_rules SET armed=0 WHERE room_id='5';
         INSERT INTO studium_alert_attempts(attempted_at) VALUES(strftime('%Y-%m-%dT%H:%M:%fZ','now'));
         INSERT INTO studium_alert_tokens(token_hash,room_id,rule_version,expires_at,created_at)
         VALUES(printf('%064d',1),'5',1,strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 minutes'),strftime('%Y-%m-%dT%H:%M:%fZ','now'));",
    ).unwrap();
    drop(connection);
    store.backup_to(&stage.join("rhythm.sqlite3")).unwrap();
    drop(store);
    clear_restored_activation_tokens(&stage).unwrap();
    for (path, expected_tokens) in [(live, 1), (stage.join("rhythm.sqlite3"), 0)] {
        let connection = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM studium_alert_tokens", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            expected_tokens
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT armed FROM studium_alert_rules WHERE room_id='5'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM studium_alert_attempts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_restore_refuses_prepared_database_tampering() {
    let root = tempfile::tempdir().unwrap();
    database(&root.path().join("rhythm.sqlite3"), 0, None);
    let before = fs::read(root.path().join("rhythm.sqlite3")).unwrap();
    let stage = root.path().join(STAGING);
    fs::create_dir(&stage).unwrap();
    database(&stage.join("rhythm.sqlite3"), 1, None);
    let mut preview = journal_preview(0);
    preview.documents = 1;
    write_journal(
        root.path(),
        &format!("{PREVIOUS_PREFIX}tamper"),
        false,
        &preview,
    )
    .unwrap();
    fs::write(stage.join("rhythm.sqlite3"), b"tampered").unwrap();
    assert!(recover_pending_restore(root.path()).is_err());
    assert_eq!(
        fs::read(root.path().join("rhythm.sqlite3")).unwrap(),
        before
    );
    assert!(root.path().join(JOURNAL).exists());
    assert!(stage.join("rhythm.sqlite3").exists());
}

#[cfg(windows)]
#[test]
fn windows_restore_fault_child() {
    let Some(root) = std::env::var_os("RHYTHM_RESTORE_TEST_ROOT") else {
        return;
    };
    recover_pending_restore(Path::new(&root)).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_restore_recovers_after_process_abort_at_each_transition() {
    for point in [
        "old-db",
        "new-db",
        "old-resources",
        "new-resources",
        "verified",
        "marked",
        "stage-removed",
    ] {
        let root = tempfile::tempdir().unwrap();
        database(&root.path().join("rhythm.sqlite3"), 0, None);
        fs::create_dir(root.path().join("resources")).unwrap();
        fs::write(root.path().join("resources/old.bin"), b"previous").unwrap();
        let stage = root.path().join(STAGING);
        fs::create_dir(&stage).unwrap();
        fs::create_dir(stage.join("resources")).unwrap();
        database(
            &stage.join("rhythm.sqlite3"),
            1,
            Some(("resources/new.bin", b"restored")),
        );
        fs::write(stage.join("resources/new.bin"), b"restored").unwrap();
        let mut preview = journal_preview(1);
        preview.documents = 1;
        let previous = format!("{PREVIOUS_PREFIX}fault-{point}");
        write_journal(root.path(), &previous, true, &preview).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("backup::tests::windows_restore_fault_child")
            .env("RHYTHM_RESTORE_TEST_ROOT", root.path())
            .env("RHYTHM_RESTORE_TEST_FAULT", point)
            .status()
            .unwrap();
        assert!(
            !status.success(),
            "fault point {point} did not abort the subprocess"
        );
        let outcome = recover_pending_restore(root.path()).unwrap().unwrap();
        assert_eq!((outcome.documents, outcome.resources), (1, 1), "{point}");
        assert_eq!(
            fs::read(root.path().join("resources/new.bin")).unwrap(),
            b"restored"
        );
        assert_eq!(
            fs::read(root.path().join(&previous).join("resources/old.bin")).unwrap(),
            b"previous"
        );
        validate_staged_database(root.path(), 6, 1, 1).unwrap();
        assert_eq!(recover_pending_restore(root.path()).unwrap(), None);
    }
}
