use super::intelligence::*;
use super::*;
use serde_json::json;

#[test]
fn execution_preference_migrates_schema_seven_and_persists_without_resetting_archive() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../../migrations/003_intelligence.sql"),
        include_str!("../../migrations/004_personalization.sql"),
        include_str!("../../migrations/005_proactive.sql"),
        include_str!("../../migrations/006_notification_attempts.sql"),
        include_str!("../../migrations/007_notification_activation.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch("UPDATE ai_preferences SET max_tokens=2048 WHERE id=1;")
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    assert_eq!(preferences.execution_mode, "auto");
    assert_eq!(preferences.model_directory, None);
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    for mode in ["cpu", "gpu", "auto"] {
        preferences.execution_mode = mode.into();
        store.ai_save_preferences(&preferences).unwrap();
        assert_eq!(store.ai_preferences().unwrap().execution_mode, mode);
    }
    preferences.execution_mode = "gpu".into();
    store.ai_save_preferences(&preferences).unwrap();
    preferences.execution_mode = "invalid".into();
    assert!(store.ai_save_preferences(&preferences).is_err());
    drop(store);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .ai_preferences()
            .unwrap()
            .execution_mode,
        "gpu"
    );
}

#[test]
fn execution_preference_old_payload_defaults_to_auto() {
    let preferences: AiPreferences = serde_json::from_value(json!({
        "active_model_id":null,"setup_completed":false,"processing_mode":"explicit",
        "automatic_limit":3,"max_tokens":1024
    }))
    .unwrap();
    assert_eq!(preferences.execution_mode, "auto");
    assert_eq!(preferences.model_directory, None);
    assert_eq!(preferences.embedding_model_id, None);
    assert!(preferences.classification_enabled);
}

#[test]
fn model_directory_migrates_schema_eight_without_relocating_models() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../../migrations/003_intelligence.sql"),
        include_str!("../../migrations/004_personalization.sql"),
        include_str!("../../migrations/005_proactive.sql"),
        include_str!("../../migrations/006_notification_attempts.sql"),
        include_str!("../../migrations/007_notification_activation.sql"),
        include_str!("../../migrations/008_ai_execution_mode.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch(
            "UPDATE ai_preferences SET execution_mode='gpu',max_tokens=2048;
         INSERT INTO ai_models(id,name,ownership,path,status,downloaded_bytes,resume_etag)
         VALUES('old','Existing model','managed','models/old.gguf','cancelled',47,'old-etag');",
        )
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let preferences = store.ai_preferences().unwrap();
    assert_eq!(preferences.model_directory, None);
    assert_eq!(preferences.execution_mode, "gpu");
    let old = store.ai_model("old").unwrap().unwrap();
    assert_eq!(old.storage_directory, None);
    assert_eq!(old.path, "models/old.gguf");
    assert_eq!(old.downloaded_bytes, 47);
    assert_eq!(old.resume_etag.as_deref(), Some("old-etag"));
    assert_eq!(store.document(1, None).unwrap().item.title, "장학 공지");
    integrity(&store);
}

#[test]
fn model_directory_persists_independently_from_each_models_original_location() {
    let (directory, store) = database();
    let original = directory
        .path()
        .join("한글 모델 폴더")
        .to_string_lossy()
        .into_owned();
    let next = directory
        .path()
        .join("next models")
        .to_string_lossy()
        .into_owned();
    store.ai_save_model_directory(Some(&original)).unwrap();
    let mut existing = model("resume", "managed");
    existing.path = Path::new(&original)
        .join("resume.gguf")
        .to_string_lossy()
        .into_owned();
    existing.storage_directory = Some(original.clone());
    existing.status = "cancelled".into();
    existing.downloaded_bytes = 47;
    existing.resume_etag = Some("continue-this-file".into());
    store.ai_upsert_model(&existing).unwrap();
    store.ai_save_model_directory(Some(&next)).unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.execution_mode = "cpu".into();
    store.ai_save_preferences(&preferences).unwrap();
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    assert_eq!(
        reopened
            .ai_preferences()
            .unwrap()
            .model_directory
            .as_deref(),
        Some(next.as_str())
    );
    let retained = reopened.ai_model("resume").unwrap().unwrap();
    assert_eq!(
        retained.storage_directory.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(retained.path, existing.path);
    assert_eq!(retained.downloaded_bytes, 47);
    assert_eq!(retained.resume_etag, existing.resume_etag);
    reopened.ai_save_model_directory(None).unwrap();
    assert_eq!(reopened.ai_preferences().unwrap().model_directory, None);
    assert_eq!(reopened.ai_preferences().unwrap().execution_mode, "cpu");
    assert_eq!(
        reopened.ai_models().unwrap()[0].storage_directory,
        Some(original)
    );
    let mut legacy = serde_json::to_value(&existing).unwrap();
    legacy.as_object_mut().unwrap().remove("storage_directory");
    assert_eq!(
        serde_json::from_value::<AiModel>(legacy)
            .unwrap()
            .storage_directory,
        None
    );
    integrity(&reopened);
}

#[test]
fn model_directory_rejects_malformed_input_without_changing_preferences_or_inventory() {
    let (directory, store) = database();
    let valid = directory
        .path()
        .join("models")
        .to_string_lossy()
        .into_owned();
    store.ai_save_model_directory(Some(&valid)).unwrap();
    let initial = store.ai_preferences().unwrap();
    let mut existing = model("unchanged", "managed");
    existing.storage_directory = Some(valid.clone());
    store.ai_upsert_model(&existing).unwrap();
    for invalid in [
        String::new(),
        "relative/models".into(),
        "D:relative".into(),
        format!("{valid}\0hidden"),
        format!("{valid}{}", "a".repeat(4097)),
    ] {
        assert!(store.ai_save_model_directory(Some(&invalid)).is_err());
        let mut preferences = initial.clone();
        preferences.model_directory = Some(invalid.clone());
        preferences.execution_mode = "gpu".into();
        assert!(store.ai_save_preferences(&preferences).is_err());
        assert_eq!(store.ai_preferences().unwrap(), initial);
        existing.storage_directory = Some(invalid);
        assert!(store.ai_upsert_model(&existing).is_err());
        assert_eq!(
            store
                .ai_model("unchanged")
                .unwrap()
                .unwrap()
                .storage_directory
                .as_deref(),
            Some(valid.as_str())
        );
    }
    integrity(&store);
}

fn populated_schema_two(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(include_str!("../../migrations/001_foundation.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../migrations/002_archive.sql"))
        .unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;
        UPDATE settings SET theme='dark',energy_profile='eco',interests='장학' WHERE id=1;
        INSERT INTO extension_preferences VALUES('sogang_notices',1);
        INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed,next_page) VALUES('test','학사','서강','공지','https://www.sogang.ac.kr/','main',1,1,12);
        INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical) VALUES(1,'test','notice','https://www.sogang.ac.kr/notice','장학 공지','2026-01-01','<p>신청 기한 3월 1일</p>','신청 기한 3월 1일','oldhash',1);
        INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(1,1,'https://www.sogang.ac.kr/notice','장학 공지','2026-01-01','original','<p>신청 기한 3월 1일</p>','신청 기한 3월 1일','oldhash');
        INSERT INTO document_state(document_id,is_read,saved) VALUES(1,1,1);
        INSERT INTO resources(document_id,revision_id,url,filename,kind,status,local_path,sha256,bytes) VALUES(1,1,'https://www.sogang.ac.kr/a.pdf','a.pdf','attachment','downloaded','resources/test.pdf','resourcehash',12);").unwrap();
}
fn database() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    (directory, Store::open(&path).unwrap())
}
fn model(id: &str, ownership: &str) -> AiModel {
    AiModel {
        id: id.into(),
        catalog_id: None,
        name: "Qwen test".into(),
        ownership: ownership.into(),
        kind: "generation".into(),
        path: format!("/tmp/{id}.gguf"),
        storage_directory: None,
        sha256: Some("a".repeat(64)),
        expected_sha256: None,
        expected_bytes: None,
        bytes: 123,
        source_revision: None,
        source_filename: None,
        status: "installed".into(),
        downloaded_bytes: 123,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
fn integrity(store: &Store) {
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(store
        .connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    store
        .connection
        .execute(
            "INSERT INTO documents_fts(documents_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .unwrap();
}
#[test]
fn populated_schema_two_upgrade_preserves_archive_and_settings() {
    let (directory, store) = database();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(store.settings().unwrap().theme, "dark");
    assert_eq!(store.settings().unwrap().energy_profile, "eco");
    assert_eq!(store.settings().unwrap().interests, "장학");
    assert!(store.enabled("sogang_notices").unwrap());
    let detail = store.document(1, None).unwrap();
    assert_eq!(detail.body_text, "신청 기한 3월 1일");
    assert_eq!(detail.revisions.len(), 1);
    assert!(detail.item.saved && detail.item.is_read);
    assert_eq!(
        detail.resources[0].local_path.as_deref(),
        Some("resources/test.pdf")
    );
    assert_eq!(store.sources().unwrap()[0].next_page, 12);
    assert!(store.ai_preferences().unwrap().classification_enabled);
    integrity(&store);
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    assert_eq!(
        reopened.document(1, None).unwrap().body_text,
        "신청 기한 3월 1일"
    );
    integrity(&reopened);
}
#[test]
fn schema_three_failure_rolls_back_without_touching_populated_archive() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE ai_results(sentinel TEXT); INSERT INTO ai_results VALUES('preserved');",
        )
        .unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM ai_results", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
    assert_eq!(connection.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('ai_models','ai_preferences','ai_jobs')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        connection
            .query_row("SELECT body_text FROM documents WHERE id=1", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "신청 기한 3월 1일"
    );
}
#[test]
fn restart_recovers_interrupted_downloads_without_losing_inventory() {
    let (directory, store) = database();
    let mut downloading = model("download", "managed");
    downloading.status = "downloading".into();
    downloading.downloaded_bytes = 17;
    downloading.resume_etag = Some("\"pinned\"".into());
    store.ai_upsert_model(&downloading).unwrap();
    store
        .ai_upsert_model(&model("installed", "external"))
        .unwrap();
    drop(store);
    let store = Store::open(&directory.path().join("db")).unwrap();
    let recovered = store.ai_model("download").unwrap().unwrap();
    assert_eq!(recovered.status, "cancelled");
    assert_eq!(recovered.downloaded_bytes, 17);
    assert_eq!(recovered.resume_etag, downloading.resume_etag);
    assert_eq!(
        store.ai_model("installed").unwrap().unwrap().status,
        "installed"
    );
    integrity(&store);
}
#[test]
fn model_unregistration_clears_selection_but_never_removes_file() {
    let (directory, store) = database();
    let path = directory.path().join("external.gguf");
    std::fs::write(&path, b"GGUF retained").unwrap();
    let mut external = model("local", "external");
    external.kind = "embedding".into();
    external.path = path.to_str().unwrap().into();
    store.ai_upsert_model(&external).unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.embedding_model_id = Some("local".into());
    store.ai_save_preferences(&preferences).unwrap();
    store.ai_remove_model("local").unwrap();
    assert!(path.exists());
    assert!(store.ai_preferences().unwrap().embedding_model_id.is_none());
    assert!(store.document(1, None).unwrap().item.saved);
    integrity(&store);
}

/// A real schema-32 profile: migrations 001–032 in order, then populated.
fn populated_schema_thirty_two(path: &Path) {
    let mut connection = Connection::open(path).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    for migration in &super::MIGRATIONS[..32] {
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(migration).unwrap();
        transaction.commit().unwrap();
    }
    let embedding = "e".repeat(64);
    let generation = "a".repeat(64);
    let remote = "f".repeat(64);
    connection.execute_batch(&format!("
        INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed) VALUES('test','학사','서강','공지','https://www.sogang.ac.kr/','main',1,1);
        INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical) VALUES(1,'test','notice','https://www.sogang.ac.kr/notice','장학 공지','2026-01-01','<p>본문</p>','본문','hash',0);
        INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(1,1,'https://www.sogang.ac.kr/notice','장학 공지','2026-01-01','original','<p>본문</p>','본문','hash');
        INSERT INTO ai_models(id,catalog_id,name,ownership,kind,path,sha256,bytes,status) VALUES
          ('qwen35-4b','qwen35-4b','Qwen3.5 4B','managed','generation','models/{generation}.gguf','{generation}',8,'installed'),
          ('qwen3-embedding-0-6b','qwen3-embedding-0-6b','Qwen3-Embedding 0.6B','managed','embedding','models/{embedding}.gguf','{embedding}',8,'installed');
        INSERT INTO ai_projectors(model_id,path,sha256,expected_sha256,expected_bytes,bytes,status) VALUES('qwen35-4b','models/{}.gguf','{}','{}',4,4,'installed');
        UPDATE ai_preferences SET active_model_id='qwen35-4b',setup_completed=1,processing_mode='backlog',
          execution_mode='cpu',model_directory='/Volumes/Models',embedding_model_id='qwen3-embedding-0-6b',
          classification_enabled=0,engine='openai-compatible',remote_base_url='https://API.Example.com:8443/v1',
          remote_model='summary',remote_embedding_model='embed',remote_consent=1,remote_automatic=1,remote_embeddings=1;
        INSERT INTO ai_jobs(id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,status)
          VALUES(1,1,1,'notice_analysis','{generation}','{{}}',5,3,'{generation}','completed');
        INSERT INTO ai_results(job_id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,result_json,partial,processed_chars,total_chars,duration_ms)
          VALUES(1,1,1,'notice_analysis','{generation}','{{}}',5,3,'{generation}','{{\"summary\":\"요약\"}}',0,2,2,1);
        INSERT INTO ai_remote_usage(day,attempts) VALUES('2026-11-13',3);
        INSERT INTO notice_embeddings(document_id,revision_id,model_sha256,dim,vector) VALUES(1,1,'{embedding}',1,x'0000803f'),(1,1,'{remote}',1,x'0000803f');
        INSERT INTO notice_classifications(document_id,revision_id,model_sha256,config_hash,category,score,margin,low_confidence)
          VALUES(1,1,'{embedding}','{generation}','장학',0.9,0.2,0),(1,1,'{remote}','{generation}','장학',0.9,0.2,0);
        INSERT INTO calendar_todos(title,day,document_id) VALUES('장학 신청','2026-03-01',1);
    ", "c".repeat(64), "c".repeat(64), "c".repeat(64))).unwrap();
}

#[test]
fn schema_thirty_three_retires_generation_and_remote_but_keeps_embedding_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_thirty_two(&path);
    DELETED_REMOTE_KEYS.with(|keys| keys.borrow_mut().clear());
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    // The key was deleted under the normalized origin before the column went.
    assert_eq!(
        DELETED_REMOTE_KEYS.with(|keys| keys.borrow().clone()),
        ["https://api.example.com:8443"]
    );
    for table in ["ai_jobs", "ai_results", "ai_remote_usage"] {
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(
        store.ai_preferences().unwrap(),
        AiPreferences {
            execution_mode: "cpu".into(),
            model_directory: Some("/Volumes/Models".into()),
            embedding_model_id: Some("qwen3-embedding-0-6b".into()),
            classification_enabled: false,
        }
    );
    // Installed generation models and their projectors stay until removed.
    assert_eq!(store.ai_models().unwrap().len(), 2);
    assert!(store.ai_projector("qwen35-4b").unwrap().is_some());
    // Only the remote-derived vectors are deleted.
    let hashes = |table: &str| -> Vec<String> {
        store
            .connection
            .prepare(&format!("SELECT model_sha256 FROM {table}"))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(hashes("notice_embeddings"), ["e".repeat(64)]);
    assert_eq!(hashes("notice_classifications"), ["e".repeat(64)]);
    assert_eq!(store.document(1, None).unwrap().body_text, "본문");
    let todo: Option<i64> = store
        .connection
        .query_row("SELECT document_id FROM calendar_todos", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(todo, Some(1));
    integrity(&store);
    drop(store);
    // Reopening runs no migration again.
    Store::open(&path).unwrap();
    assert!(DELETED_REMOTE_KEYS.with(|keys| keys.borrow().len()) == 1);
}

#[test]
fn schema_thirty_three_failure_rolls_back_to_thirty_two() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_thirty_two(&path);
    // A leftover rebuild table makes the migration fail midway.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE ai_preferences_033(id INTEGER);")
        .unwrap();
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        32
    );
    for (sql, expected) in [
        ("SELECT COUNT(*) FROM ai_results", 1),
        ("SELECT COUNT(*) FROM ai_jobs", 1),
        ("SELECT COUNT(*) FROM notice_embeddings", 2),
        (
            "SELECT COUNT(*) FROM ai_preferences WHERE engine='openai-compatible'",
            1,
        ),
    ] {
        assert_eq!(
            connection
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .unwrap(),
            expected,
            "{sql}"
        );
    }
}

#[test]
fn a_profile_without_a_remote_origin_deletes_no_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_thirty_two(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch("UPDATE ai_preferences SET remote_base_url=NULL;")
        .unwrap();
    DELETED_REMOTE_KEYS.with(|keys| keys.borrow_mut().clear());
    Store::open(&path).unwrap();
    assert!(DELETED_REMOTE_KEYS.with(|keys| keys.borrow().is_empty()));
}

#[test]
fn projector_inventory_is_separate_from_the_model_and_cascades() {
    let (directory, store) = database();
    let mut installed = model("test", "managed");
    installed.path = format!("models/{}.gguf", "a".repeat(64));
    store.ai_upsert_model(&installed).unwrap();
    let projector = AiProjector {
        model_id: "test".into(),
        path: format!("models/{}.gguf", "c".repeat(64)),
        sha256: Some("c".repeat(64)),
        expected_sha256: "c".repeat(64),
        expected_bytes: 1024,
        bytes: 1024,
        status: "installed".into(),
        downloaded_bytes: 1024,
        resume_etag: None,
        error: None,
    };
    store.ai_upsert_projector(&projector).unwrap();
    assert_eq!(store.ai_projectors().unwrap().len(), 1);
    assert_eq!(
        store.ai_projector("test").unwrap().unwrap().path,
        projector.path
    );
    // An "installed" claim needs the pinned identity, not just a status string.
    let mut wrong = projector.clone();
    wrong.sha256 = Some("d".repeat(64));
    assert!(store.ai_upsert_projector(&wrong).is_err());
    let mut orphan = projector.clone();
    orphan.model_id = "missing".into();
    assert!(store.ai_upsert_projector(&orphan).is_err());
    // A model without a projector row stays valid text-only inventory.
    assert!(store.ai_projector("other").unwrap().is_none());
    store.ai_remove_model("test").unwrap();
    assert!(store.ai_projector("test").unwrap().is_none());
    integrity(&store);
    drop(store);
    // The row survives only with its model.
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    assert!(reopened.ai_projectors().unwrap().is_empty());
}

#[test]
fn interrupted_projector_downloads_are_cancelled_on_reopen() {
    let (_directory, store) = database();
    store.ai_upsert_model(&model("test", "managed")).unwrap();
    let projector = AiProjector {
        model_id: "test".into(),
        path: format!("models/{}.gguf", "c".repeat(64)),
        sha256: None,
        expected_sha256: "c".repeat(64),
        expected_bytes: 1024,
        bytes: 0,
        status: "downloading".into(),
        downloaded_bytes: 512,
        resume_etag: Some("\"immutable\"".into()),
        error: None,
    };
    store.ai_upsert_projector(&projector).unwrap();
    store.reset_ai_interrupted().unwrap();
    let current = store.ai_projector("test").unwrap().unwrap();
    assert_eq!(current.status, "cancelled");
    assert_eq!(current.downloaded_bytes, 512);
    assert!(current.error.unwrap().contains("이미지 인식 모듈"));
}

#[test]
fn embedding_schema_upgrades_schema_fourteen_without_losing_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../../migrations/003_intelligence.sql"),
        include_str!("../../migrations/004_personalization.sql"),
        include_str!("../../migrations/005_proactive.sql"),
        include_str!("../../migrations/006_notification_attempts.sql"),
        include_str!("../../migrations/007_notification_activation.sql"),
        include_str!("../../migrations/008_ai_execution_mode.sql"),
        include_str!("../../migrations/009_ai_model_directory.sql"),
        include_str!("../../migrations/010_update_check.sql"),
        include_str!("../../migrations/011_skipped_articles.sql"),
        include_str!("../../migrations/012_resource_retry.sql"),
        include_str!("../../migrations/013_ai_projector.sql"),
        include_str!("../../migrations/014_ai_backlog_mode.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch("UPDATE ai_preferences SET execution_mode='gpu' WHERE id=1;")
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let preferences = store.ai_preferences().unwrap();
    assert_eq!(preferences.execution_mode, "gpu");
    assert_eq!(preferences.embedding_model_id, None);
    assert!(preferences.classification_enabled);
    // New tables and the model role column exist and default generation.
    let count: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM notice_embeddings e JOIN notice_classifications c ON 1=0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('ai_models') WHERE name='kind'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn classification_roundtrip_is_model_and_config_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    populated_schema_two(&path);
    let store = Store::open(&path).unwrap();
    let revision = store.document(1, None).unwrap().revision_id;
    let model_a = "a".repeat(64);
    let model_b = "b".repeat(64);
    let config = "c".repeat(64);
    store
        .ai_save_embedding(1, revision, &model_a, &[0.6, 0.8])
        .unwrap();
    let stored = store.ai_embedding(1, revision, &model_a).unwrap().unwrap();
    assert_eq!(stored.dim, 2);
    assert_eq!(stored.vector, vec![0.6, 0.8]);
    assert!(store.ai_embedding(1, revision, &model_b).unwrap().is_none());
    store
        .ai_save_classification(&ClassificationWrite {
            document_id: 1,
            revision_id: revision,
            model_sha256: &model_a,
            config_hash: &config,
            category: "장학",
            score: 0.8,
            margin: 0.1,
            low_confidence: false,
        })
        .unwrap();
    assert_eq!(
        store
            .ai_classification(1, revision, &model_a, &config)
            .unwrap()
            .unwrap()
            .category,
        "장학"
    );
    assert!(store
        .ai_classification(1, revision, &model_b, &config)
        .unwrap()
        .is_none());
    assert!(store
        .ai_embedding_candidates(&model_a, &config, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .ai_embedding_candidates(&model_b, &config, 10)
            .unwrap(),
        vec![(1, revision)]
    );
    let rows = store
        .ai_classifications_for_documents(&[1], &model_a, &config)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].category, "장학");
    assert!(!rows[0].low_confidence);

    // Removing the embedding model removes only its derived rows.
    let model = AiModel {
        id: "embedding-test".into(),
        catalog_id: Some("qwen3-embedding-0-6b".into()),
        name: "embedding".into(),
        ownership: "external".into(),
        kind: "embedding".into(),
        path: "/tmp/embedding-test.gguf".into(),
        storage_directory: None,
        sha256: Some(model_a.clone()),
        expected_sha256: None,
        expected_bytes: None,
        bytes: 8,
        source_revision: None,
        source_filename: None,
        status: "installed".into(),
        downloaded_bytes: 0,
        resume_etag: None,
        error: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    store.ai_upsert_model(&model).unwrap();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.embedding_model_id = Some(model.id.clone());
    store.ai_save_preferences(&preferences).unwrap();
    assert_eq!(
        store.ai_preferences().unwrap().embedding_model_id,
        Some(model.id.clone())
    );
    // A generation model cannot occupy the embedding slot.
    let mut generation = model.clone();
    generation.id = "generation-test".into();
    generation.catalog_id = None;
    generation.kind = "generation".into();
    generation.path = "/tmp/generation-test.gguf".into();
    store.ai_upsert_model(&generation).unwrap();
    preferences.embedding_model_id = Some(generation.id.clone());
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences.embedding_model_id = None;
    store.ai_save_preferences(&preferences).unwrap();
    store.ai_remove_model(&model.id).unwrap();
    assert!(store.ai_embedding(1, revision, &model_a).unwrap().is_none());
    assert!(store
        .ai_classification(1, revision, &model_a, &config)
        .unwrap()
        .is_none());
}

fn embedded_notice(
    store: &Store,
    source_id: &str,
    external_id: &str,
    title: &str,
    body: &str,
    vector: &[f32],
    model_sha256: &str,
) -> (i64, i64) {
    let mut article = notice_article(external_id, "2026-03-01", body);
    article.title = title.into();
    let (id, _) = store.upsert_article(source_id, &article, true).unwrap();
    let revision = store.document(id, None).unwrap().revision_id;
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    let unit = vector
        .iter()
        .map(|value| value / length)
        .collect::<Vec<_>>();
    store
        .ai_save_embedding(id, revision, model_sha256, &unit)
        .unwrap();
    (id, revision)
}

fn notice_article(
    external_id: &str,
    published_at: &str,
    body: &str,
) -> crate::archive::model::ArticleInput {
    crate::archive::model::ArticleInput {
        external_id: external_id.into(),
        canonical_url: format!("https://www.sogang.ac.kr/notice/{external_id}"),
        title: "공지".into(),
        author: None,
        category: None,
        published_at: published_at.into(),
        modified_at: None,
        original: body.into(),
        unsupported_body: false,
        body_html: format!("<p>{body}</p>"),
        body_text: body.into(),
        resources: vec![],
        etag: None,
        last_modified: None,
    }
}
#[test]
fn similar_notices_flag_only_exact_cross_posts_and_isolate_model_hashes() {
    let (_directory, store) = database();
    store
        .connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed,next_page) VALUES('test2','공과대학','서강','공지','https://www.sogang.ac.kr/','main',1,1,12)",
            [],
        )
        .unwrap();
    let model_a = "a".repeat(64);
    let model_b = "b".repeat(64);
    let title = "2026학년도 장학금 신청 안내";
    let body = "신청 기한 3월 10일 17시";
    let (target, _) = embedded_notice(&store, "test", "a", title, body, &[1.0, 0.0, 0.0], &model_a);
    // The same notice reposted on another board, with spacing differences.
    let (copy, _) = embedded_notice(
        &store,
        "test2",
        "b",
        " 2026학년도  장학금 신청 안내",
        body,
        &[1.0, 0.1, 0.0],
        &model_a,
    );
    // Near-identical text but another date: similar, never a duplicate.
    let (other_date, _) = embedded_notice(
        &store,
        "test2",
        "c",
        title,
        "신청 기한 3월 17일 17시",
        &[1.0, 0.12, 0.0],
        &model_a,
    );
    let (related, _) = embedded_notice(
        &store,
        "test",
        "d",
        "교내 장학 설명회",
        "설명회 안내",
        &[0.8, 0.6, 0.0],
        &model_a,
    );
    let (unrelated, _) = embedded_notice(
        &store,
        "test",
        "e",
        "도서관 휴관",
        "휴관 안내",
        &[0.0, 1.0, 0.0],
        &model_a,
    );
    // Same direction, but only under another model hash.
    let (other_model, _) =
        embedded_notice(&store, "test", "f", title, body, &[1.0, 0.0, 0.0], &model_b);
    // An embedding of an outdated revision is not compared.
    let (revised, _) =
        embedded_notice(&store, "test", "g", title, body, &[1.0, 0.0, 0.0], &model_a);
    let mut changed = notice_article("g", "2026-03-01", "변경된 본문 3월 10일 17시");
    changed.title = title.into();
    assert!(store.upsert_article("test", &changed, true).unwrap().1);

    let results = store.ai_similar_notices(&[target], &model_a, 5).unwrap();
    assert_eq!(results.len(), 1);
    let (id, similar) = &results[0];
    assert_eq!(*id, target);
    let ids = similar
        .iter()
        .map(|notice| notice.document_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![copy, other_date, related]);
    assert!(!ids.contains(&unrelated) && !ids.contains(&other_model) && !ids.contains(&revised));
    assert_eq!(
        similar
            .iter()
            .map(|notice| notice.duplicate)
            .collect::<Vec<_>>(),
        vec![true, false, false]
    );
    assert_eq!(similar[0].source_name, "공과대학");
    assert_eq!(similar[0].title, " 2026학년도  장학금 신청 안내");

    // Duplicates are symmetric and come first even when a non-duplicate scores
    // higher (`other_date` is closer to `copy` than `target` is).
    let results = store
        .ai_similar_notices(&[copy, related], &model_a, 1)
        .unwrap();
    assert_eq!(
        results[0]
            .1
            .iter()
            .map(|n| (n.document_id, n.duplicate))
            .collect::<Vec<_>>(),
        vec![(target, true)]
    );
    assert_eq!(
        results[1]
            .1
            .iter()
            .map(|n| (n.document_id, n.duplicate))
            .collect::<Vec<_>>(),
        vec![(other_date, false)]
    );
    // The other model only sees its own vectors, and a document without a
    // vector for the model gets nothing.
    assert!(store
        .ai_similar_notices(&[other_model], &model_b, 5)
        .unwrap()[0]
        .1
        .is_empty());
    assert!(store.ai_similar_notices(&[target], &model_b, 5).unwrap()[0]
        .1
        .is_empty());
    assert!(
        store.ai_similar_notices(&[revised], &model_a, 5).unwrap()[0]
            .1
            .is_empty()
    );
    assert!(store.ai_similar_notices(&[target], "bad", 5).is_err());
    assert!(store.ai_similar_notices(&[target], &model_a, 0).is_err());
    assert!(store
        .ai_similar_notices(&vec![1; 201], &model_a, 5)
        .is_err());
}

#[test]
fn duplicate_check_reads_full_width_digits() {
    let (_directory, store) = database();
    let model = "a".repeat(64);
    let title = "장학금 신청 안내";
    let (target, _) = embedded_notice(
        &store,
        "test",
        "a",
        title,
        "신청 기한 ３월 １０일 17시",
        &[1.0, 0.0],
        &model,
    );
    // The same date in ASCII digits is a copy.
    let (ascii, _) = embedded_notice(
        &store,
        "test",
        "b",
        title,
        "신청 기한 3월 10일 17시",
        &[1.0, 0.01],
        &model,
    );
    // Another date written only in full-width digits is not.
    let (other_date, _) = embedded_notice(
        &store,
        "test",
        "c",
        title,
        "신청 기한 ４월 １１일 17시",
        &[1.0, 0.02],
        &model,
    );
    let similar = &store.ai_similar_notices(&[target], &model, 5).unwrap()[0].1;
    assert_eq!(
        similar
            .iter()
            .map(|n| (n.document_id, n.duplicate))
            .collect::<Vec<_>>(),
        vec![(ascii, true), (other_date, false)]
    );
}

#[test]
fn duplicate_check_ignores_board_tags_and_contact_numbers() {
    let (_directory, store) = database();
    let model = "a".repeat(64);
    let body = "2025-2026학년도 장학금 신청 기한 3월 10일 17시";
    let (target, _) = embedded_notice(
        &store,
        "test",
        "target",
        "[학부] 장학금 신청 안내",
        body,
        &[1.0, 0.0],
        &model,
    );
    let cases = [
        ("장학금 신청 안내", body.to_string(), true),
        ("[공지][학부] 장학금 신청 안내", body.to_string(), true),
        ("<공지> [학부]  장학금 신청 안내", body.to_string(), true),
        (
            "[학부] 장학금 신청 안내",
            format!("{body}\n문의: 학생지원팀 02-705-8123 (내선 8123)"),
            true,
        ),
        (
            "【학부】 장학금 신청 안내",
            format!("{body}\n☎ 705-8123, Tel. 3274-1234"),
            true,
        ),
        // A conflicting tag can name another audience.
        ("[대학원] 장학금 신청 안내", body.to_string(), false),
        // Digits in a tag still count.
        ("[2027] 장학금 신청 안내", body.to_string(), false),
        // A changed date still blocks grouping despite the relaxed rules.
        (
            "[공지] 장학금 신청 안내",
            "2025-2026학년도 장학금 신청 기한 3월 17일 17시\n문의 02-705-8123".to_string(),
            false,
        ),
        // A year range is not a phone number.
        (
            "[학부] 장학금 신청 안내",
            "2026-2027학년도 장학금 신청 기한 3월 10일 17시".to_string(),
            false,
        ),
        // A tag alone is not a title.
        ("[학부]", body.to_string(), false),
    ];
    let mut expected = Vec::new();
    for (index, (title, body, duplicate)) in cases.iter().enumerate() {
        let (id, _) = embedded_notice(
            &store,
            "test",
            &format!("case-{index}"),
            title,
            body,
            &[1.0, 0.001 * (index + 1) as f32],
            &model,
        );
        expected.push((id, *duplicate));
    }
    let similar = &store.ai_similar_notices(&[target], &model, 20).unwrap()[0].1;
    let mut actual = similar
        .iter()
        .map(|n| (n.document_id, n.duplicate))
        .collect::<Vec<_>>();
    actual.sort();
    assert_eq!(actual, expected);
}

#[test]
fn similar_notice_candidates_are_bounded_to_the_newest_window() {
    let (_directory, store) = database();
    let model = "a".repeat(64);
    let body = "신청 기한 3월 10일";
    let (target, _) = embedded_notice(
        &store,
        "test",
        "old-a",
        "장학 공지",
        body,
        &[1.0, 0.0],
        &model,
    );
    let (copy, _) = embedded_notice(
        &store,
        "test",
        "old-b",
        "장학 공지",
        body,
        &[1.0, 0.0],
        &model,
    );
    assert_eq!(
        store.ai_similar_notices(&[target], &model, 5).unwrap()[0].1[0].document_id,
        copy
    );
    // Fill the window with newer, orthogonal notices; the old copy falls out.
    let unit = [0.0f32, 1.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    let transaction = store.connection.unchecked_transaction().unwrap();
    for index in 0..SIMILARITY_WINDOW as i64 {
        let id = 10_000 + index;
        transaction
            .execute(
                "INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical) VALUES(?1,'test',?2,'https://www.sogang.ac.kr/n','새 공지','2026-06-01','<p>x</p>','x','h',1)",
                params![id, format!("new-{index}")],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(?1,?1,'https://www.sogang.ac.kr/n','새 공지','2026-06-01','o','<p>x</p>','x','h')",
                [id],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO notice_embeddings(document_id,revision_id,model_sha256,dim,vector) VALUES(?1,?1,?2,2,?3)",
                params![id, model, unit],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    // The target itself is outside the window but still has its vector.
    assert!(store.ai_similar_notices(&[target], &model, 5).unwrap()[0]
        .1
        .is_empty());
}

fn ranked(query: &str) -> crate::archive::model::ArchiveQuery {
    crate::archive::model::ArchiveQuery {
        query: query.into(),
        limit: Some(30),
        ..Default::default()
    }
}

#[test]
fn ranked_search_fuses_text_and_meaning_and_respects_filters_and_model() {
    let (_directory, store) = database();
    store
        .connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed,next_page) VALUES('test2','공과대학','서강','공지','https://www.sogang.ac.kr/','main',1,1,12)",
            [],
        )
        .unwrap();
    let model = "a".repeat(64);
    let other_model = "b".repeat(64);
    // The term only in the body, then the term in the title.
    let (body_only, _) = embedded_notice(
        &store,
        "test",
        "a",
        "오픈소스 안내",
        "기술 세미나 개최",
        &[0.0, 1.0, 0.0],
        &model,
    );
    let (title_hit, _) = embedded_notice(
        &store,
        "test",
        "b",
        "기술 세미나 개최",
        "발표자 모집",
        &[0.0, 0.0, 1.0],
        &model,
    );
    // No search term, but close in meaning to the query vector.
    let (meaning, _) = embedded_notice(
        &store,
        "test",
        "c",
        "개발자 발표회",
        "발표 일정",
        &[1.0, 0.05, 0.0],
        &model,
    );
    // Both a text match and the closest meaning: fused first.
    let (both, _) = embedded_notice(
        &store,
        "test",
        "d",
        "학술 행사",
        "세미나 안내",
        &[1.0, 0.0, 0.0],
        &model,
    );
    // Close in meaning, but only on another board or under another model.
    let (other_board, _) = embedded_notice(
        &store,
        "test2",
        "e",
        "공학 발표회",
        "발표 일정",
        &[1.0, 0.02, 0.0],
        &model,
    );
    let (other_hash, _) = embedded_notice(
        &store,
        "test",
        "f",
        "발표 행사",
        "행사 일정",
        &[1.0, 0.0, 0.0],
        &other_model,
    );
    // Far from the best hit: outside the semantic margin.
    let (far, _) = embedded_notice(
        &store,
        "test",
        "g",
        "도서관 휴관",
        "휴관 안내",
        &[0.5, 0.9, 0.0],
        &model,
    );

    let query_vector = [1.0, 0.0, 0.0];
    // Text-only relevance: title hits before body-only hits.
    let text = store.archive_ranked_query(&ranked("세미나"), None).unwrap();
    let ids = text.items.iter().map(|item| item.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![title_hit, both, body_only]);
    assert_eq!(text.total, 3);
    assert!(!text.semantic && text.meaning_only.is_empty());

    let fused = store
        .archive_ranked_query(&ranked("세미나"), Some((&model, &query_vector)))
        .unwrap();
    let ids = fused.items.iter().map(|item| item.id).collect::<Vec<_>>();
    assert!(fused.semantic);
    assert_eq!(ids[0], both, "{ids:?}");
    assert_eq!(fused.total, ids.len() as u64);
    assert!(ids.contains(&meaning) && ids.contains(&other_board));
    assert!(!ids.contains(&other_hash) && !ids.contains(&far));
    assert_eq!(
        ids.iter()
            .filter(|id| [title_hit, body_only].contains(id))
            .count(),
        2,
        "every text match stays"
    );
    let mut meaning_only = fused.meaning_only.clone();
    meaning_only.sort();
    let mut expected = vec![meaning, other_board];
    expected.sort();
    assert_eq!(meaning_only, expected);

    let mut board = ranked("세미나");
    board.source_id = Some("test".into());
    let filtered = store
        .archive_ranked_query(&board, Some((&model, &query_vector)))
        .unwrap();
    assert!(filtered.items.iter().all(|item| item.source_id == "test"));
    assert_eq!(filtered.meaning_only, vec![meaning]);

    // A vector of another dimension matches nothing instead of failing.
    let mismatched = store
        .archive_ranked_query(&ranked("세미나"), Some((&model, &[1.0, 0.0])))
        .unwrap();
    assert_eq!(mismatched.total, 3);
    assert!(mismatched.meaning_only.is_empty());
    // Without a query the order is the ordinary date order.
    assert_eq!(
        store
            .archive_ranked_query(&ranked(""), Some((&model, &query_vector)))
            .unwrap()
            .total,
        store.archive_query(&ranked("")).unwrap().total
    );
    assert!(store
        .archive_ranked_query(&ranked("세미나"), Some(("bad", &query_vector)))
        .is_err());
}

#[test]
fn ranked_search_pages_cover_every_match_once_beyond_the_fused_head() {
    let (_directory, store) = database();
    let model = "a".repeat(64);
    for index in 0..205 {
        embedded_notice(
            &store,
            "test",
            &format!("t{index}"),
            &format!("행사 {index}"),
            "세미나 안내",
            &[0.0, 1.0],
            &model,
        );
    }
    let (meaning, _) = embedded_notice(
        &store,
        "test",
        "m",
        "발표회",
        "발표 일정",
        &[1.0, 0.0],
        &model,
    );
    let mut seen = Vec::new();
    let mut total = 0;
    for page in 0..8 {
        let mut query = ranked("세미나");
        query.offset = page * 30;
        let result = store
            .archive_ranked_query(&query, Some((&model, &[1.0, 0.0])))
            .unwrap();
        total = result.total;
        seen.extend(result.items.iter().map(|item| item.id));
    }
    assert_eq!(total, 206);
    assert_eq!(seen.len(), 206);
    let unique = seen.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), 206);
    // The best semantic hit is fused near the top, not appended after the tail.
    assert!(seen.iter().position(|id| *id == meaning).unwrap() < 30);
}
