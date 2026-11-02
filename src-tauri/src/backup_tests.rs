//! Populated real-schema backup/restore, interruption recovery and hostile archives.
use super::*;
use crate::archive::model::{ArticleInput, ResourceInput, SourceDefinition};
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

/// Weights a matching managed model has locally; never written into a backup.
const MANAGED_WEIGHTS: &[u8] = b"GGUF\x03\x00\x00\x00managed weights retained locally";

/// A profile with every kind of user state the backup must carry.
fn populated_profile(root: &Path) -> Store {
    let store = Store::open(&root.join("rhythm.sqlite3")).unwrap();
    store
        .catalog(&[
            SourceDefinition {
                id: "one".into(),
                name: "학사 공지".into(),
                organization: "서강대학교".into(),
                description: "공지".into(),
                url: "https://www.sogang.ac.kr/".into(),
                family: "main".into(),
                supported: true,
            },
            SourceDefinition {
                id: "two".into(),
                name: "장학 공지".into(),
                organization: "서강대학교".into(),
                description: "공지".into(),
                url: "https://www.sogang.ac.kr/".into(),
                family: "main".into(),
                supported: true,
            },
        ])
        .unwrap();
    store.subscribe("one", true).unwrap();
    store.subscribe("two", true).unwrap();
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
            "one",
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
            "one",
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
            "one",
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
            "two",
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
            task_id: "source:one".into(),
            interval_minutes: 30,
            enabled: true,
        })
        .unwrap();
    // Enabling notifications set a baseline; only a later notice is eligible.
    let (fresh, _) = store
        .upsert_article(
            "two",
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
