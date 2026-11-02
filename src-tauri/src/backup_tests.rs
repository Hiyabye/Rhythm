//! Populated real-schema backup/restore, interruption recovery and hostile archives.
use super::*;
use crate::archive::model::{ArticleInput, ResourceInput};
use crate::storage::{
    intelligence::{AiModel, AnalysisKey},
    personalization::{PersonalInterestInput, PersonalizationProfile},
    proactive::{ProactiveEventKey, ProactiveEventKind, ProactiveSettings, ProactiveTaskInput},
    Store,
};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Deterministic incompressible bytes, so large entries exercise real copying.
fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn article(external: &str, title: &str, body: &str, resources: Vec<ResourceInput>) -> ArticleInput {
    ArticleInput {
        external_id: external.into(),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{external}"),
        title: title.into(),
        author: Some("학사지원팀".into()),
        category: Some("학사".into()),
        published_at: "2026-10-01".into(),
        modified_at: None,
        original: format!("<html><body>{body}</body></html>"),
        unsupported_body: false,
        body_html: format!("<p>{body}</p>"),
        body_text: body.into(),
        resources,
        etag: None,
        last_modified: None,
    }
}

fn resource(url: &str, filename: &str, kind: &str) -> ResourceInput {
    ResourceInput {
        url: url.into(),
        filename: filename.into(),
        kind: kind.into(),
    }
}

fn model(id: &str, ownership: &str, path: &str, bytes: &[u8]) -> AiModel {
    AiModel {
        id: id.into(),
        catalog_id: None,
        name: id.into(),
        ownership: ownership.into(),
        path: path.into(),
        sha256: Some(digest(bytes)),
        expected_sha256: None,
        expected_bytes: None,
        bytes: bytes.len() as u64,
        source_revision: None,
        source_filename: None,
        status: "installed".into(),
        downloaded_bytes: 0,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

const PRIMARY: &str = "sogang-academic";
const SECONDARY: &str = "eng-academic";

/// Weights a matching managed model has locally; never written into a backup.
const MANAGED_WEIGHTS: &[u8] = b"GGUF\x03\x00\x00\x00managed weights retained locally";

/// A profile with every kind of user state the backup must carry.
fn populated_profile(root: &Path) -> Store {
    let store = Store::open(&root.join("rhythm.sqlite3")).unwrap();
    // Real catalog identities, so a written native profile renders as usual.
    store.catalog(&crate::archive::definitions()).unwrap();
    store.subscribe(PRIMARY, true).unwrap();
    store.subscribe(SECONDARY, true).unwrap();
    let write = |relative: &str, bytes: &[u8]| {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    };
    let image = noise(4096, 1);
    let shared = b"%PDF-1.7 shared attachment".to_vec();
    let large = noise(8 * 1024 * 1024 + 17, 2);
    write("resources/aa/image.png", &image);
    write("resources/한글/수강 신청 안내.pdf", &shared);
    write("resources/bb/large.bin", &large);

    let download = |document: i64| {
        for item in store.pending_resources(document).unwrap() {
            let (path, bytes) = match item.filename.as_str() {
                "a.png" => ("resources/aa/image.png", &image),
                "수강 신청 안내.pdf" => ("resources/한글/수강 신청 안내.pdf", &shared),
                "large.bin" => ("resources/bb/large.bin", &large),
                _ => {
                    store
                        .resource_result(
                            item.id,
                            "failed",
                            None,
                            None,
                            None,
                            None,
                            Some("시험용 실패"),
                        )
                        .unwrap();
                    continue;
                }
            };
            store
                .resource_result(
                    item.id,
                    "downloaded",
                    Some("application/octet-stream"),
                    Some(bytes.len() as u64),
                    Some(path),
                    Some(&digest(bytes)),
                    None,
                )
                .unwrap();
        }
    };
    let (first, _) = store
        .upsert_article(
            PRIMARY,
            &article(
                "1",
                "2026학년도 장학금 신청 안내",
                "장학금 신청 기간은 10월 31일까지입니다.",
                vec![
                    resource("https://www.sogang.ac.kr/a.png", "a.png", "image"),
                    resource(
                        "https://www.sogang.ac.kr/수강.pdf",
                        "수강 신청 안내.pdf",
                        "attachment",
                    ),
                    resource(
                        "https://www.sogang.ac.kr/large.bin",
                        "large.bin",
                        "attachment",
                    ),
                    resource(
                        "https://www.sogang.ac.kr/failed.hwp",
                        "실패.hwp",
                        "attachment",
                    ),
                ],
            ),
            false,
        )
        .unwrap();
    download(first);
    // A meaningful edit creates a second revision of the same document; its
    // unchanged image reuses the downloaded file.
    store
        .upsert_article(
            PRIMARY,
            &article(
                "1",
                "2026학년도 장학금 신청 안내(수정)",
                "장학금 신청 기간은 11월 7일까지로 연장되었습니다.",
                vec![resource("https://www.sogang.ac.kr/a.png", "a.png", "image")],
            ),
            false,
        )
        .unwrap();
    let (second, _) = store
        .upsert_article(
            PRIMARY,
            &article(
                "2",
                "장학금 서류 제출 안내",
                "장학 서류는 학생지원팀에 제출합니다.",
                vec![resource(
                    "https://www.sogang.ac.kr/수강.pdf",
                    "수강 신청 안내.pdf",
                    "attachment",
                )],
            ),
            false,
        )
        .unwrap();
    download(first);
    download(second);
    let (third, _) = store
        .upsert_article(
            SECONDARY,
            &article("9", "취업 박람회", "박람회 일정", vec![]),
            true,
        )
        .unwrap();
    store
        .set_document_state(first, Some(true), Some(true))
        .unwrap();
    store
        .set_document_state(second, Some(true), Some(false))
        .unwrap();
    store.set_document_state(third, None, Some(true)).unwrap();

    store
        .personalization_save_profile(&PersonalizationProfile {
            university: "서강대학교".into(),
            department: "컴퓨터공학과".into(),
            study_level: "학부".into(),
        })
        .unwrap();
    for (name, polarity) in [("장학금", "positive"), ("취업", "negative")] {
        store
            .personalization_save_interest(&PersonalInterestInput {
                id: None,
                name: name.into(),
                aliases: vec![format!("{name} 공지")],
                priority: 3,
                polarity: polarity.into(),
            })
            .unwrap();
    }
    store
        .personalization_set_feedback(first, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(second, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(third, Some("not_interested"))
        .unwrap();
    // Remove a learned memory, if feedback produced one, so its suppression
    // baseline is part of the profile too.
    let snapshot = store.personalization_snapshot().unwrap();
    if let Some(learned) = snapshot
        .interests
        .iter()
        .find(|item| item.origin != "explicit")
    {
        store.personalization_remove_interest(learned.id).unwrap();
    }

    store
        .ai_upsert_model(&model(
            "managed",
            "managed",
            "models/managed.gguf",
            MANAGED_WEIGHTS,
        ))
        .unwrap();
    store
        .ai_upsert_model(&model(
            "absent",
            "managed",
            "models/absent.gguf",
            b"weights not present",
        ))
        .unwrap();
    store
        .ai_upsert_model(&model(
            "external",
            "external",
            "/Volumes/외장/model.gguf",
            b"external",
        ))
        .unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some("managed".into());
    preferences.setup_completed = true;
    store.ai_save_preferences(&preferences).unwrap();
    let revision = store.document(first, None).unwrap().revision_id;
    store
        .ai_enqueue(
            &AnalysisKey {
                document_id: first,
                revision_id: revision,
                task: "analysis".into(),
                model_sha256: digest(MANAGED_WEIGHTS),
                prompt_version: 3,
                schema_version: 1,
                configuration_hash: "0".repeat(64),
            },
            &serde_json::json!({"id": "managed"}),
            100,
        )
        .unwrap();
    store
        .proactive_save_settings(&ProactiveSettings {
            scheduler_enabled: true,
            notifications_enabled: true,
            ..ProactiveSettings::default()
        })
        .unwrap();
    store
        .proactive_save_task(&ProactiveTaskInput {
            task_id: format!("source:{PRIMARY}"),
            interval_minutes: 30,
            enabled: true,
        })
        .unwrap();
    // Enabling notifications set a baseline; only a later notice is eligible.
    let (fresh, _) = store
        .upsert_article(
            SECONDARY,
            &article("10", "장학금 추가 모집", "추가 모집 안내", vec![]),
            false,
        )
        .unwrap();
    store
        .proactive_claim_event(&ProactiveEventKey {
            document_id: fresh,
            revision_id: store.document(fresh, None).unwrap().revision_id,
            event_type: ProactiveEventKind::New,
        })
        .unwrap();
    store
}

fn complete_job(store: &mut Store) {
    let job = store.ai_claim_next().unwrap().unwrap();
    store
        .ai_complete_job(
            job.id,
            &serde_json::json!({"category": "장학", "summary": "신청 기간 연장"}),
            false,
            10,
            10,
            5,
        )
        .unwrap();
}

/// Every user table, row by row, in a stable order.
fn table_rows(path: &Path) -> std::collections::BTreeMap<String, Vec<String>> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'documents_fts%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1,2"))
                .or_else(|_| connection.prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1")))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|index| format!("{:?}", row.get_ref(index).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

fn assert_database_consistent(path: &Path) {
    let connection = rusqlite::Connection::open(path).unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(!connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
    connection
        .execute(
            "INSERT INTO documents_fts(documents_fts) VALUES('integrity-check')",
            [],
        )
        .unwrap();
}

#[cfg(unix)]
#[test]
fn populated_profile_round_trips_every_table_resource_and_model_decision() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("source");
    let mut store = populated_profile(&source);
    complete_job(&mut store);

    // Online snapshot while another connection keeps committing in WAL mode.
    let writer_path = source.join("rhythm.sqlite3");
    {
        let connection = rusqlite::Connection::open(&writer_path).unwrap();
        connection
            .execute_batch("CREATE TABLE snapshot_stress(n INTEGER PRIMARY KEY)")
            .unwrap();
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let connection = rusqlite::Connection::open(&writer_path).unwrap();
            connection
                .busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let mut n = 0;
            while !stop.load(Ordering::Acquire) {
                n += 1;
                connection
                    .execute("INSERT INTO snapshot_stress(n) VALUES(?1)", [n])
                    .unwrap();
            }
            n
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(20));
    let snapshot = workspace.path().join("snapshot.sqlite3");
    store.backup_to(&snapshot).unwrap();
    stop.store(true, Ordering::Release);
    let written = writer.join().unwrap();
    assert_database_consistent(&snapshot);
    let stress: Vec<i64> = rusqlite::Connection::open(&snapshot)
        .unwrap()
        .prepare("SELECT n FROM snapshot_stress ORDER BY n")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(stress.len() as i64 <= written);
    assert!(
        stress.iter().copied().eq(1..=stress.len() as i64),
        "snapshot is a committed prefix"
    );

    let (schema, documents) = store.backup_summary().unwrap();
    assert_eq!(schema, crate::storage::SCHEMA_VERSION);
    let source_rows = table_rows(&snapshot);
    for table in [
        "documents",
        "document_revisions",
        "resources",
        "document_state",
        "sources",
        "personalization_interests",
        "ai_models",
        "ai_results",
        "proactive_events",
        "proactive_tasks",
        "personalization_feedback",
    ] {
        assert!(
            source_rows.get(table).is_some_and(|rows| !rows.is_empty()),
            "fixture left {table} empty"
        );
    }
    drop(store);

    let archive = workspace.path().join("백업 파일.rhythm-backup");
    let created = create(
        &archive,
        &snapshot,
        &source.join("resources"),
        "1793500000".into(),
        schema,
        documents,
    )
    .unwrap();
    // Three distinct files; the shared PDF is referenced by two documents and
    // the failed attachment has no bytes to archive.
    assert_eq!((created.documents, created.resources), (4, 3));
    let preview = inspect(&archive).unwrap();
    assert_eq!(preview.resources, 3);
    assert_eq!(preview.resource_bytes, 4096 + 26 + 8 * 1024 * 1024 + 17);

    let mut zip = ZipArchive::new(File::open(&archive).unwrap()).unwrap();
    let manifest = read_manifest(&mut zip).unwrap();
    let names: Vec<_> = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().to_owned())
        .collect();
    assert!(names.iter().all(|name| !name.starts_with("models/")));
    for entry in &manifest.entries {
        let original = if entry.path == "rhythm.sqlite3" {
            snapshot.clone()
        } else {
            source.join(&entry.path)
        };
        let bytes = fs::read(original).unwrap();
        assert_eq!(
            (entry.bytes, entry.sha256.as_str()),
            (bytes.len() as u64, digest(&bytes).as_str())
        );
    }

    // Restore over a different existing profile that has its own data and a
    // locally matching managed model.
    let target = workspace.path().join("target");
    let prior = populated_profile(&target);
    drop(prior);
    let prior_rows = table_rows(&target.join("rhythm.sqlite3"));
    fs::create_dir_all(target.join("models")).unwrap();
    fs::write(target.join("models/managed.gguf"), MANAGED_WEIGHTS).unwrap();
    let stage = target.join(STAGING);
    let staged = extract_verified(&archive, &stage).unwrap();
    fs::create_dir_all(stage.join("resources")).unwrap();
    let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
    reconcile_model_inventory(&stage, &target.join("models"), &cancellation).unwrap();
    validate_staged_database(
        &stage,
        staged.database_schema,
        staged.documents,
        staged.resources,
    )
    .unwrap();
    let previous = format!("{PREVIOUS_PREFIX}populated");
    write_journal(&target, &previous, true, &staged).unwrap();
    let outcome = recover_pending_restore(&target).unwrap().unwrap();
    assert_eq!(
        outcome,
        RestoreOutcome {
            documents: 4,
            resources: 3,
            unavailable_resources: 1,
            models_needing_reinstall: 1,
        }
    );
    assert_eq!(recover_pending_restore(&target).unwrap(), None);

    let restored = target.join("rhythm.sqlite3");
    assert_database_consistent(&restored);
    let mut restored_rows = table_rows(&restored);
    let mut expected = source_rows.clone();
    // The only intended difference: a managed model without matching local
    // weights is marked missing and stops being the active model.
    let models = |rows: &mut std::collections::BTreeMap<String, Vec<String>>| {
        (
            rows.remove("ai_models").unwrap(),
            rows.remove("ai_preferences").unwrap(),
        )
    };
    let (restored_models, restored_preferences) = models(&mut restored_rows);
    let (source_models, source_preferences) = models(&mut expected);
    assert_eq!(restored_rows, expected);
    assert_eq!(
        restored_preferences, source_preferences,
        "active model had matching weights"
    );
    assert_eq!(restored_models.len(), source_models.len());
    let connection = rusqlite::Connection::open(&restored).unwrap();
    let status = |id: &str| -> String {
        connection
            .query_row("SELECT status FROM ai_models WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(
        (status("managed"), status("absent"), status("external")),
        ("installed".into(), "missing".into(), "installed".into())
    );
    drop(connection);

    for entry in manifest
        .entries
        .iter()
        .filter(|entry| entry.path != "rhythm.sqlite3")
    {
        assert_eq!(
            digest(&fs::read(target.join(&entry.path)).unwrap()),
            entry.sha256
        );
    }
    assert_eq!(
        fs::read(target.join("models/managed.gguf")).unwrap(),
        MANAGED_WEIGHTS
    );
    assert!(
        !target.join("models/absent.gguf").exists(),
        "restore never invents weights"
    );
    // The replaced profile is retained intact.
    let previous_root = target.join(&previous);
    assert_eq!(
        table_rows(&previous_root.join("rhythm.sqlite3")),
        prior_rows
    );
    assert!(previous_root.join("resources/bb/large.bin").is_file());
    // The restored profile opens through the normal store path.
    let reopened = Store::open(&restored).unwrap();
    assert_eq!(reopened.archive_summary().unwrap().documents, 4);
    assert_eq!(reopened.backup_summary().unwrap(), (schema, 4));
}

/// Mirrors the rename order of `recover_pending_restore` to leave the data
/// directory in each state a crash between two durable steps can produce.
#[cfg(unix)]
#[test]
fn recovery_resumes_from_every_interruption_point_and_is_idempotent() {
    const STEPS: usize = 7;
    for completed in 0..=STEPS {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().to_path_buf();
        fs::write(root.join("rhythm.sqlite3"), b"current database").unwrap();
        fs::write(root.join("rhythm.sqlite3-wal"), b"current wal").unwrap();
        fs::create_dir(root.join("resources")).unwrap();
        fs::write(root.join("resources/old.bin"), b"current resource").unwrap();
        let stage = root.join(STAGING);
        fs::create_dir_all(stage.join("resources")).unwrap();
        tests::database(
            &stage.join("rhythm.sqlite3"),
            2,
            Some(("resources/new.bin", b"restored resource")),
        );
        let restored_db = fs::read(stage.join("rhythm.sqlite3")).unwrap();
        fs::write(stage.join("resources/new.bin"), b"restored resource").unwrap();
        let previous_name = format!("{PREVIOUS_PREFIX}step-{completed}");
        let previous = root.join(&previous_name);
        let mut preview = tests::journal_preview(1);
        preview.documents = 2;
        write_journal(&root, &previous_name, true, &preview).unwrap();

        let steps: [&dyn Fn(); STEPS] = [
            &|| fs::create_dir(&previous).unwrap(),
            &|| {
                fs::rename(
                    root.join("rhythm.sqlite3-wal"),
                    previous.join("rhythm.sqlite3-wal"),
                )
                .unwrap()
            },
            &|| fs::rename(root.join("rhythm.sqlite3"), previous.join("rhythm.sqlite3")).unwrap(),
            &|| fs::rename(stage.join("rhythm.sqlite3"), root.join("rhythm.sqlite3")).unwrap(),
            &|| fs::rename(root.join("resources"), previous.join("resources")).unwrap(),
            &|| fs::rename(stage.join("resources"), root.join("resources")).unwrap(),
            &|| fs::remove_dir_all(&stage).unwrap(),
        ];
        for step in &steps[..completed] {
            step();
        }

        let first = recover_pending_restore(&root)
            .unwrap_or_else(|error| panic!("after {completed} steps: {error}"));
        assert_eq!(
            first.map(|outcome| outcome.documents),
            Some(2),
            "after {completed} steps"
        );
        assert_eq!(recover_pending_restore(&root).unwrap(), None);
        assert_eq!(fs::read(root.join("rhythm.sqlite3")).unwrap(), restored_db);
        assert_eq!(
            fs::read(root.join("resources/new.bin")).unwrap(),
            b"restored resource"
        );
        assert!(!root.join("resources/old.bin").exists());
        assert_eq!(
            fs::read(previous.join("rhythm.sqlite3")).unwrap(),
            b"current database"
        );
        assert_eq!(
            fs::read(previous.join("rhythm.sqlite3-wal")).unwrap(),
            b"current wal"
        );
        assert_eq!(
            fs::read(previous.join("resources/old.bin")).unwrap(),
            b"current resource"
        );
        assert!(!root.join(JOURNAL).exists() && !stage.exists());
    }
}

#[cfg(unix)]
#[test]
fn startup_removes_only_interrupted_backup_snapshots() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join(".rhythm-backup-12-1.sqlite3"),
        b"stale snapshot",
    )
    .unwrap();
    fs::write(root.path().join("rhythm.sqlite3"), b"live").unwrap();
    fs::write(
        root.path().join(".rhythm-backup-notes.txt"),
        b"not a snapshot",
    )
    .unwrap();
    remove_stale_backup_snapshots(root.path());
    assert!(!root.path().join(".rhythm-backup-12-1.sqlite3").exists());
    assert!(root.path().join("rhythm.sqlite3").exists());
    assert!(root.path().join(".rhythm-backup-notes.txt").exists());
}

enum Item<'a> {
    File(&'a str, &'a [u8]),
    Directory(&'a str),
    Symlink(&'a str, &'a str),
}

/// Writes an archive with an arbitrary manifest, bypassing `create`'s checks.
fn hostile_archive(path: &Path, items: &[Item], manifest: &Manifest) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    for item in items {
        match item {
            Item::File(name, bytes) => {
                zip.start_file(*name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
            Item::Directory(name) => zip.add_directory(*name, options).unwrap(),
            Item::Symlink(name, target) => zip.add_symlink(*name, *target, options).unwrap(),
        }
    }
    zip.start_file(MANIFEST, options).unwrap();
    serde_json::to_writer(&mut zip, manifest).unwrap();
    zip.finish().unwrap();
}

fn manifest_for(entries: &[(&str, &[u8])]) -> Manifest {
    let entries: Vec<Entry> = entries
        .iter()
        .map(|(path, bytes)| Entry {
            path: (*path).into(),
            bytes: bytes.len() as u64,
            sha256: digest(bytes),
        })
        .collect();
    let resources: Vec<_> = entries
        .iter()
        .filter(|entry| entry.path.starts_with("resources/"))
        .collect();
    Manifest {
        format_version: FORMAT_VERSION,
        created_at: "test".into(),
        database_schema: 6,
        documents: 0,
        resources: resources.len() as u64,
        resource_bytes: resources.iter().map(|entry| entry.bytes).sum(),
        entries,
    }
}

#[cfg(unix)]
#[test]
fn hostile_archives_are_rejected_without_writing_outside_staging() {
    let workspace = tempfile::tempdir().unwrap();
    let db_path = workspace.path().join("db.sqlite3");
    tests::database(&db_path, 0, None);
    let db = fs::read(&db_path).unwrap();
    let payload: &[u8] = b"payload";
    let mut tampered = manifest_for(&[("rhythm.sqlite3", &db), ("resources/a.bin", payload)]);
    tampered.entries[1].sha256 = digest(b"other");
    let mut oversized = manifest_for(&[("rhythm.sqlite3", &db), ("resources/a.bin", payload)]);
    oversized.entries[1].bytes = 3;
    let cases: Vec<(&str, Vec<Item>, Manifest)> = vec![
        (
            "absolute path",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("/tmp/rhythm-escape", payload),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("/tmp/rhythm-escape", payload)]),
        ),
        (
            "parent traversal",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("resources/../../escape", payload),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("resources/../../escape", payload)]),
        ),
        (
            "entry outside the manifest",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("../escape", payload),
            ],
            manifest_for(&[("rhythm.sqlite3", &db)]),
        ),
        (
            "unexpected top-level file",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("models/w.gguf", payload),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("models/w.gguf", payload)]),
        ),
        (
            "symlink entry",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::Symlink("resources/a.bin", "/etc/hosts"),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("resources/a.bin", b"/etc/hosts")]),
        ),
        (
            "directory entry",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::Directory("resources/a.bin"),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("resources/a.bin", b"")]),
        ),
        (
            "hash mismatch",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("resources/a.bin", payload),
            ],
            tampered,
        ),
        (
            "size mismatch",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("resources/a.bin", payload),
            ],
            oversized,
        ),
        (
            "missing database",
            vec![Item::File("resources/a.bin", payload)],
            manifest_for(&[("resources/a.bin", payload)]),
        ),
        (
            // Valid archive structure, but the database does not reference it.
            "unreferenced resource",
            vec![
                Item::File("rhythm.sqlite3", &db),
                Item::File("resources/a.bin", payload),
            ],
            manifest_for(&[("rhythm.sqlite3", &db), ("resources/a.bin", payload)]),
        ),
    ];
    for (case, items, manifest) in cases {
        let directory = workspace.path().join(case.replace(' ', "-"));
        fs::create_dir(&directory).unwrap();
        let archive = directory.join("hostile.rhythm-backup");
        hostile_archive(&archive, &items, &manifest);
        assert!(inspect(&archive).is_err(), "{case} was accepted");
        let stage = directory.join("stage");
        let staged = extract_verified(&archive, &stage);
        if let Ok(preview) = staged {
            assert!(
                validate_staged_database(
                    &stage,
                    preview.database_schema,
                    preview.documents,
                    preview.resources
                )
                .is_err(),
                "{case} passed staged validation"
            );
        }
        let mut listed: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|item| item.unwrap().file_name().into_string().unwrap())
            .collect();
        listed.sort();
        assert!(
            listed
                .iter()
                .all(|name| name == "hostile.rhythm-backup" || name == "stage"),
            "{case} wrote {listed:?}"
        );
    }
    assert!(!Path::new("/tmp/rhythm-escape").exists());
    assert!(!workspace.path().join("escape").exists());
}

#[test]
fn duplicate_archive_entries_are_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let db_path = workspace.path().join("db.sqlite3");
    tests::database(&db_path, 0, None);
    let db = fs::read(&db_path).unwrap();
    let archive = workspace.path().join("duplicate.rhythm-backup");
    // Two stored entries whose names differ by one byte, then made identical
    // in both the local headers and the central directory.
    hostile_archive(
        &archive,
        &[
            Item::File("rhythm.sqlite3", &db),
            Item::File("resources/a.bin", b"first"),
            Item::File("resources/b.bin", b"other"),
        ],
        &manifest_for(&[
            ("rhythm.sqlite3", &db),
            ("resources/a.bin", b"first"),
            ("resources/b.bin", b"other"),
        ]),
    );
    let mut bytes = fs::read(&archive).unwrap();
    let mut replaced = 0;
    for index in 0..bytes.len().saturating_sub(15) {
        if &bytes[index..index + 15] == b"resources/b.bin" {
            bytes[index + 10] = b'a';
            replaced += 1;
        }
    }
    assert_eq!(replaced, 2, "local header and central directory");
    fs::write(&archive, bytes).unwrap();
    assert!(inspect(&archive).is_err());
}

#[test]
fn truncated_archive_is_rejected_and_staging_is_removed() {
    let workspace = tempfile::tempdir().unwrap();
    let db_path = workspace.path().join("db.sqlite3");
    tests::database(&db_path, 0, None);
    fs::create_dir(workspace.path().join("resources")).unwrap();
    let archive = workspace.path().join("complete.rhythm-backup");
    create(
        &archive,
        &db_path,
        &workspace.path().join("resources"),
        "test".into(),
        6,
        0,
    )
    .unwrap();
    let bytes = fs::read(&archive).unwrap();
    for length in [bytes.len() / 3, bytes.len() - 22] {
        let partial = workspace
            .path()
            .join(format!("partial-{length}.rhythm-backup"));
        fs::write(&partial, &bytes[..length]).unwrap();
        let stage = workspace.path().join(format!("stage-{length}"));
        assert!(extract_verified(&partial, &stage).is_err());
        assert!(!stage.exists());
    }
}

#[cfg(unix)]
#[test]
fn backup_rejects_a_resource_changed_after_its_digest_was_recorded() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("data");
    let mut store = populated_profile(&root);
    complete_job(&mut store);
    let snapshot = workspace.path().join("snapshot.sqlite3");
    store.backup_to(&snapshot).unwrap();
    let (schema, documents) = store.backup_summary().unwrap();
    drop(store);
    fs::write(root.join("resources/aa/image.png"), b"corrupted on disk").unwrap();
    let archive = workspace.path().join("corrupt-source.rhythm-backup");
    let error = create(
        &archive,
        &snapshot,
        &root.join("resources"),
        "test".into(),
        schema,
        documents,
    )
    .unwrap_err();
    assert!(error.contains("일치하지 않습니다"), "{error}");
    assert!(!archive.exists());
    assert!(fs::read_dir(workspace.path()).unwrap().all(|item| !item
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

/// Writes the populated fixture as a native test profile; never point it at
/// real app data. Run with RHYTHM_NATIVE_PROFILE_DIR=<fresh>/io.github.hiyabye.rhythm.
#[cfg(unix)]
#[test]
#[ignore = "writes an isolated native verification profile on request"]
fn write_populated_native_profile() {
    let root = PathBuf::from(
        std::env::var_os("RHYTHM_NATIVE_PROFILE_DIR").expect("set RHYTHM_NATIVE_PROFILE_DIR"),
    );
    assert!(
        !root.join("rhythm.sqlite3").exists(),
        "RHYTHM_NATIVE_PROFILE_DIR must name a fresh directory"
    );
    let mut store = populated_profile(&root);
    complete_job(&mut store);
    // Keep the history, but start no collection or OS notification.
    store
        .proactive_save_settings(&ProactiveSettings::default())
        .unwrap();
    fs::create_dir_all(root.join("models")).unwrap();
    fs::write(root.join("models/managed.gguf"), MANAGED_WEIGHTS).unwrap();
    println!("{:?}", store.backup_summary().unwrap());
}

/// Streams every user table into one digest per table, so a large restored
/// profile can be compared without holding its rows in memory.
fn table_digests(path: &Path) -> std::collections::BTreeMap<String, (u64, String)> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'documents_fts%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1,2"))
                .or_else(|_| connection.prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1")))
                .unwrap();
            let columns = statement.column_count();
            let mut rows = statement.query([]).unwrap();
            let (mut count, mut hash) = (0u64, Sha256::new());
            while let Some(row) = rows.next().unwrap() {
                count += 1;
                for index in 0..columns {
                    hash.update(format!("{:?}\u{1f}", row.get_ref(index).unwrap()));
                }
                hash.update(b"\x1e");
            }
            (table, (count, format!("{:x}", hash.finalize())))
        })
        .collect()
}

fn directory_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    fs::read_dir(path)
        .unwrap()
        .map(|item| directory_bytes(&item.unwrap().path()))
        .sum()
}

fn bench_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .map(|value| value.parse().expect(name))
        .unwrap_or(default)
}

/// Deterministic Korean notice text of roughly `chars` characters.
fn korean_text(chars: usize, seed: u64) -> String {
    const WORDS: &[&str] = &[
        "장학금",
        "수강신청",
        "졸업요건",
        "학사일정",
        "등록금",
        "휴학",
        "복학",
        "계절학기",
        "성적",
        "교환학생",
        "취업",
        "인턴십",
        "연구실",
        "세미나",
        "공모전",
        "기숙사",
        "신청",
        "기간",
        "안내",
        "제출",
        "서류",
        "변경",
        "마감",
        "연장",
        "학생지원팀",
        "학부",
        "대학원",
        "컴퓨터공학과",
        "전자공학과",
        "확인하시기",
        "바랍니다",
    ];
    let mut state = seed | 1;
    let mut text = String::with_capacity(chars * 3);
    while text.chars().count() < chars {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(WORDS[(state % WORDS.len() as u64) as usize]);
        text.push(if state.is_multiple_of(11) { '.' } else { ' ' });
    }
    text
}

const BENCH_SOURCES: &[&str] = &[
    "sogang-academic",
    "computing-academic",
    "computing-career",
    "eng-academic",
    "eng-general",
    "eng-careers",
];

/// Builds a large synthetic profile through the real Store APIs.
fn generate_large_profile(root: &Path, documents: u64, resource_mib: u64) {
    let mut store = Store::open(&root.join("rhythm.sqlite3")).unwrap();
    store.catalog(&crate::archive::definitions()).unwrap();
    for source in BENCH_SOURCES {
        store.subscribe(source, true).unwrap();
    }
    let large_size = 8 * 1024 * 1024 + 4099;
    let small_budget = documents * 12 * 1024;
    let large_count = (resource_mib * 1024 * 1024).saturating_sub(small_budget) / large_size;
    let large_every = (documents / large_count.max(1)).max(1);
    let mut written_large = 0u64;
    let notification_start = documents - documents / 50;
    for index in 0..documents {
        if index == notification_start {
            // Events are eligible only for notices archived after opt-in.
            store
                .proactive_save_settings(&ProactiveSettings {
                    scheduler_enabled: true,
                    notifications_enabled: true,
                    ..ProactiveSettings::default()
                })
                .unwrap();
        }
        let source = BENCH_SOURCES[(index as usize) % BENCH_SOURCES.len()];
        let external = format!("bench-{index}");
        let mut files: Vec<(ResourceInput, String, Vec<u8>)> = vec![(
            resource(
                &format!("https://www.sogang.ac.kr/img/{index}.png"),
                &format!("{index}.png"),
                "image",
            ),
            format!("resources/{:02x}/{index}.png", index % 256),
            noise(4096 + (index as usize % 8) * 1024, index + 1),
        )];
        if index % 10 == 0 {
            files.push((
                resource(
                    &format!("https://www.sogang.ac.kr/files/{index}.pdf"),
                    &format!("{index} 수강 신청 안내문.pdf"),
                    "attachment",
                ),
                format!("resources/한글/{index} 수강 신청 안내문.pdf"),
                noise(32 * 1024, index + 7),
            ));
        }
        if index % large_every == 0 && written_large < large_count {
            written_large += 1;
            files.push((
                resource(
                    &format!("https://www.sogang.ac.kr/files/{index}.zip"),
                    &format!("{index} 첨부 자료 모음.zip"),
                    "attachment",
                ),
                format!("resources/large/{index} 첨부 자료 모음.zip"),
                noise(large_size as usize, index + 13),
            ));
        }
        let body = korean_text(1500 + (index as usize % 7) * 200, index + 3);
        let title = format!("{} 공지 {index}", korean_text(6, index + 5).trim());
        let (document, _) = store
            .upsert_article(
                source,
                &article(
                    &external,
                    &title,
                    &body,
                    files.iter().map(|(input, _, _)| input.clone()).collect(),
                ),
                index < notification_start,
            )
            .unwrap();
        for item in store.pending_resources(document).unwrap() {
            let (_, path, bytes) = files
                .iter()
                .find(|(input, _, _)| input.filename == item.filename)
                .unwrap();
            let full = root.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, bytes).unwrap();
            store
                .resource_result(
                    item.id,
                    "downloaded",
                    Some("application/octet-stream"),
                    Some(bytes.len() as u64),
                    Some(path),
                    Some(&digest(bytes)),
                    None,
                )
                .unwrap();
        }
        if index % 5 == 0 {
            // A meaningful edit adds a revision that keeps the same image.
            let revised = article(
                &external,
                &format!("{title}(수정)"),
                &format!("{body} 일정이 변경되었습니다."),
                vec![files[0].0.clone()],
            );
            store
                .upsert_article(source, &revised, index < notification_start)
                .unwrap();
        }
        if index % 3 == 0 {
            store
                .set_document_state(document, Some(true), Some(index % 9 == 0))
                .unwrap();
        }
        if index % 40 == 0 {
            let feedback = if index % 80 == 0 {
                "interested"
            } else {
                "not_interested"
            };
            store
                .personalization_set_feedback(document, Some(feedback))
                .unwrap();
        }
        if index % 25 == 0 {
            let revision = store.document(document, None).unwrap().revision_id;
            store
                .ai_enqueue(
                    &AnalysisKey {
                        document_id: document,
                        revision_id: revision,
                        task: "analysis".into(),
                        model_sha256: digest(MANAGED_WEIGHTS),
                        prompt_version: 3,
                        schema_version: 1,
                        configuration_hash: "0".repeat(64),
                    },
                    &serde_json::json!({"id": "managed"}),
                    100,
                )
                .unwrap();
            complete_job(&mut store);
        }
        if index >= notification_start && index % 2 == 0 {
            store
                .proactive_claim_event(&ProactiveEventKey {
                    document_id: document,
                    revision_id: store.document(document, None).unwrap().revision_id,
                    event_type: ProactiveEventKind::New,
                })
                .unwrap();
        }
    }
    store
        .personalization_save_profile(&PersonalizationProfile {
            university: "서강대학교".into(),
            department: "컴퓨터공학과".into(),
            study_level: "학부".into(),
        })
        .unwrap();
    for (name, polarity) in [
        ("장학금", "positive"),
        ("인턴십", "positive"),
        ("기숙사", "negative"),
    ] {
        store
            .personalization_save_interest(&PersonalInterestInput {
                id: None,
                name: name.into(),
                aliases: vec![format!("{name} 공지")],
                priority: 3,
                polarity: polarity.into(),
            })
            .unwrap();
    }
    for (id, ownership, path, bytes) in [
        ("managed", "managed", "models/managed.gguf", MANAGED_WEIGHTS),
        (
            "absent",
            "managed",
            "models/absent.gguf",
            &b"weights not present"[..],
        ),
        (
            "external",
            "external",
            "/Volumes/외장/model.gguf",
            &b"external"[..],
        ),
        (
            "external-2",
            "external",
            "/Users/시험/models/other.gguf",
            &b"other"[..],
        ),
    ] {
        store
            .ai_upsert_model(&model(id, ownership, path, bytes))
            .unwrap();
    }
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some("managed".into());
    preferences.setup_completed = true;
    store.ai_save_preferences(&preferences).unwrap();
    // Keep history but leave collection and OS notifications off.
    store
        .proactive_save_settings(&ProactiveSettings::default())
        .unwrap();
    drop(store);
    // Settle WAL into the main file so the reported database size is honest.
    rusqlite::Connection::open(root.join("rhythm.sqlite3"))
        .unwrap()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
}

/// Measured backup and restore of a large synthetic profile. Run each phase
/// as its own process (generate, backup, restore, verify, cancel) so
/// `/usr/bin/time -l` reports that phase's peak memory. Never point
/// RHYTHM_BACKUP_BENCH_DIR at real app data.
#[cfg(unix)]
#[test]
#[ignore = "writes a large synthetic profile; run explicitly with RHYTHM_BACKUP_BENCH_DIR"]
fn large_profile_backup_restore_benchmark() {
    use std::time::Instant;
    let dir = PathBuf::from(
        std::env::var_os("RHYTHM_BACKUP_BENCH_DIR").expect("set RHYTHM_BACKUP_BENCH_DIR"),
    );
    let phase = std::env::var("RHYTHM_BACKUP_BENCH_PHASE").unwrap_or_else(|_| "all".into());
    let source = dir.join("source");
    let archive = dir.join("벤치마크 백업.rhythm-backup");
    let target = dir.join("target");
    let run = |name: &str| phase == "all" || phase == name;
    if run("generate") {
        assert!(!source.exists(), "RHYTHM_BACKUP_BENCH_DIR must be fresh");
        let documents = bench_env("RHYTHM_BACKUP_BENCH_DOCUMENTS", 5000);
        let resource_mib = bench_env("RHYTHM_BACKUP_BENCH_RESOURCE_MIB", 256);
        let started = Instant::now();
        generate_large_profile(&source, documents, resource_mib);
        println!(
            "generate: {documents} documents in {:.1}s; database {} bytes, resources {} bytes",
            started.elapsed().as_secs_f64(),
            directory_bytes(&source.join("rhythm.sqlite3")),
            directory_bytes(&source.join("resources")),
        );
    }
    if run("backup") {
        let store = Store::open(&source.join("rhythm.sqlite3")).unwrap();
        let snapshot = dir.join("snapshot.sqlite3");
        // Mirrors `backup_create`: the store mutex is held only for these two calls.
        let locked = Instant::now();
        let (schema, documents) = store.backup_summary().unwrap();
        store.backup_to(&snapshot).unwrap();
        let lock = locked.elapsed();
        drop(store);
        let snapshot_bytes = directory_bytes(&snapshot);
        let started = Instant::now();
        let preview = create(
            &archive,
            &snapshot,
            &source.join("resources"),
            timestamp(),
            schema,
            documents,
        )
        .unwrap();
        let archived = started.elapsed();
        fs::remove_file(&snapshot).unwrap();
        println!(
            "backup: lock+snapshot {:.2}s, archive {:.2}s; {} documents, {} resources, {} resource bytes; snapshot {snapshot_bytes} bytes, archive {} bytes",
            lock.as_secs_f64(),
            archived.as_secs_f64(),
            preview.documents,
            preview.resources,
            preview.resource_bytes,
            directory_bytes(&archive),
        );
    }
    if run("restore") {
        assert!(!target.exists(), "restore target must be fresh");
        fs::create_dir_all(&target).unwrap();
        drop(populated_profile(&target));
        fs::create_dir_all(target.join("models")).unwrap();
        fs::write(target.join("models/managed.gguf"), MANAGED_WEIGHTS).unwrap();
        let before = directory_bytes(&target);
        let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
        let started = Instant::now();
        stage_restore(&target, &archive, &cancellation).unwrap();
        let staged = started.elapsed();
        let staging = directory_bytes(&target.join(STAGING));
        let started = Instant::now();
        let outcome = recover_pending_restore(&target).unwrap().unwrap();
        println!(
            "restore: verify+stage {:.2}s, finalize on next launch {:.2}s; prior profile {before} bytes, staging {staging} bytes; {outcome:?}",
            staged.as_secs_f64(),
            started.elapsed().as_secs_f64(),
        );
    }
    if run("verify") {
        let restored = target.join("rhythm.sqlite3");
        let started = Instant::now();
        assert_database_consistent(&restored);
        let mut expected = table_digests(&source.join("rhythm.sqlite3"));
        let mut actual = table_digests(&restored);
        // As in the populated round trip, only model inventory may differ.
        for rows in [&mut expected, &mut actual] {
            rows.remove("ai_models");
            rows.remove("ai_preferences");
        }
        assert_eq!(actual, expected);
        let connection = rusqlite::Connection::open(&restored).unwrap();
        let status = |id: &str| -> String {
            connection
                .query_row("SELECT status FROM ai_models WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        assert_eq!(
            [
                status("managed"),
                status("absent"),
                status("external"),
                status("external-2")
            ],
            ["installed", "missing", "installed", "installed"]
        );
        let mut statement = connection
            .prepare("SELECT DISTINCT local_path,sha256 FROM resources WHERE status='downloaded'")
            .unwrap();
        let resources: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (path, sha256) in &resources {
            let (_, actual) = hash_file(&target.join(path), None).unwrap();
            assert_eq!(&actual, sha256, "{path}");
        }
        drop(statement);
        drop(connection);
        let store = Store::open(&restored).unwrap();
        let searched = Instant::now();
        let page = store
            .archive_query(&crate::archive::model::ArchiveQuery {
                query: "장학금".into(),
                limit: Some(30),
                ..Default::default()
            })
            .unwrap();
        let search = searched.elapsed();
        assert!(page.total > 0);
        assert!(target.join("models/managed.gguf").is_file());
        println!(
            "verify: integrity/foreign keys/FTS, {} table digests, {} resource SHA-256 checks in {:.1}s; search '장학금' {} results in {:.1} ms",
            actual.len(),
            resources.len(),
            started.elapsed().as_secs_f64(),
            page.total,
            search.as_secs_f64() * 1000.0,
        );
    }
    if let Some(full) = std::env::var_os("RHYTHM_BACKUP_BENCH_FULL_DIR").map(PathBuf::from) {
        // A small dedicated volume (for example an attached disk image), never the host disk.
        let snapshot = dir.join("full-snapshot.sqlite3");
        let store = Store::open(&source.join("rhythm.sqlite3")).unwrap();
        let (schema, documents) = store.backup_summary().unwrap();
        store.backup_to(&snapshot).unwrap();
        drop(store);
        let destination = full.join("가득 찬 백업.rhythm-backup");
        let error = create(
            &destination,
            &snapshot,
            &source.join("resources"),
            timestamp(),
            schema,
            documents,
        )
        .unwrap_err();
        fs::remove_file(&snapshot).unwrap();
        let leftovers: Vec<_> = fs::read_dir(&full)
            .unwrap()
            .map(|item| item.unwrap().file_name())
            .filter(|name| !name.to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert!(fs::read_dir(&full).unwrap().all(|item| !item
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        // Restore onto the full volume is refused before anything is staged.
        let root = full.join("profile");
        fs::create_dir_all(root.join("resources")).unwrap();
        tests::database(&root.join("rhythm.sqlite3"), 0, None);
        let prior = fs::read(root.join("rhythm.sqlite3")).unwrap();
        let cancellation = BackupCancellation(AtomicU8::new(CANCELLABLE));
        let restore_error = stage_restore(&root, &archive, &cancellation).unwrap_err();
        assert!(!root.join(STAGING).exists() && !root.join(JOURNAL).exists());
        assert_eq!(fs::read(root.join("rhythm.sqlite3")).unwrap(), prior);
        fs::remove_dir_all(&root).unwrap();
        println!("disk full: backup refused ({error}) with no partial file; restore refused ({restore_error}) without staging");
    }
    if run("cancel") {
        // Cancel each operation mid-flight and confirm nothing is published.
        let cancelled_archive = dir.join("cancelled.rhythm-backup");
        let snapshot = dir.join("cancel-snapshot.sqlite3");
        let store = Store::open(&source.join("rhythm.sqlite3")).unwrap();
        let (schema, documents) = store.backup_summary().unwrap();
        store.backup_to(&snapshot).unwrap();
        drop(store);
        type Work = Box<dyn FnOnce(&BackupCancellation) -> Result<(), String> + Send>;
        let measure = |work: Work| {
            let cancellation = Arc::new(BackupCancellation(AtomicU8::new(CANCELLABLE)));
            let worker = {
                let cancellation = cancellation.clone();
                std::thread::spawn(move || work(&cancellation))
            };
            std::thread::sleep(std::time::Duration::from_millis(500));
            let requested = Instant::now();
            assert!(cancellation.request());
            let error = worker.join().unwrap().unwrap_err();
            (requested.elapsed(), error)
        };
        let (backup_latency, backup_error) = {
            let (archive, snapshot, resources) = (
                cancelled_archive.clone(),
                snapshot.clone(),
                source.join("resources"),
            );
            measure(Box::new(move |cancellation| {
                create_cancellable(
                    &archive,
                    &snapshot,
                    &resources,
                    timestamp(),
                    schema,
                    documents,
                    cancellation,
                )
                .map(|_| ())
            }))
        };
        assert!(!cancelled_archive.exists(), "{backup_error}");
        fs::remove_file(&snapshot).unwrap();
        let cancel_target = dir.join("cancel-target");
        fs::create_dir_all(&cancel_target).unwrap();
        drop(populated_profile(&cancel_target));
        let prior = table_digests(&cancel_target.join("rhythm.sqlite3"));
        let (restore_latency, restore_error) = {
            let (root, archive) = (cancel_target.clone(), archive.clone());
            measure(Box::new(move |cancellation| {
                stage_restore(&root, &archive, cancellation)
            }))
        };
        assert!(!cancel_target.join(STAGING).exists() && !cancel_target.join(JOURNAL).exists());
        assert_eq!(table_digests(&cancel_target.join("rhythm.sqlite3")), prior);
        assert_eq!(recover_pending_restore(&cancel_target).unwrap(), None);
        println!(
            "cancel: backup stopped {:.0} ms after request ({backup_error}); restore stopped {:.0} ms after request ({restore_error}); no archive, staging or journal remained",
            backup_latency.as_secs_f64() * 1000.0,
            restore_latency.as_secs_f64() * 1000.0,
        );
    }
}
