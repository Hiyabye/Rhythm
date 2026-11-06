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
    assert_eq!(preferences.max_tokens, 2048);
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
    assert_eq!(preferences.engine, "local");
    assert_eq!(preferences.remote_base_url, None);
    assert_eq!(preferences.remote_model, None);
    assert_eq!(preferences.remote_embedding_model, None);
    assert_eq!(preferences.remote_timeout_seconds, 120);
    assert!(!preferences.remote_consent);
    assert!(!preferences.remote_automatic);
    assert_eq!(preferences.remote_daily_limit, 20);
    assert!(!preferences.remote_embeddings);
}

#[test]
fn remote_engine_migrates_schema_fifteen_and_persists_disabled() {
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
        include_str!("../../migrations/015_ai_embedding.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let mut preferences = store.ai_preferences().unwrap();
    // Upgrading never opts into remote inference or its cost-bearing modes.
    assert_eq!(preferences.engine, "local");
    assert!(!preferences.remote_consent);
    assert!(!preferences.remote_automatic);
    assert!(!preferences.remote_embeddings);
    assert_eq!(preferences.remote_timeout_seconds, 120);
    assert_eq!(preferences.remote_daily_limit, 20);

    preferences.engine = "openai-compatible".into();
    preferences.remote_base_url = Some("https://api.example.com/v1".into());
    preferences.remote_model = Some("test-model".into());
    preferences.remote_embedding_model = Some("test-embedding".into());
    preferences.remote_timeout_seconds = 45;
    preferences.remote_consent = true;
    preferences.remote_automatic = true;
    preferences.remote_daily_limit = 7;
    preferences.remote_embeddings = true;
    store.ai_save_preferences(&preferences).unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    let stored = reopened.ai_preferences().unwrap();
    assert_eq!(stored.engine, "openai-compatible");
    assert_eq!(
        stored.remote_base_url.as_deref(),
        Some("https://api.example.com/v1")
    );
    assert_eq!(stored.remote_model.as_deref(), Some("test-model"));
    assert_eq!(
        stored.remote_embedding_model.as_deref(),
        Some("test-embedding")
    );
    assert_eq!(stored.remote_timeout_seconds, 45);
    assert!(stored.remote_consent);
    assert!(stored.remote_automatic);
    assert_eq!(stored.remote_daily_limit, 7);
    assert!(stored.remote_embeddings);
}

#[test]
fn api_key_is_not_a_persisted_preference_or_backup_field() {
    let (directory, store) = database();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.engine = "openai-compatible".into();
    preferences.remote_base_url = Some("https://api.example.com".into());
    preferences.remote_model = Some("model".into());
    preferences.remote_consent = true;
    store.ai_save_preferences(&preferences).unwrap();
    // The credential-store value is never passed to SQLite. A distinctive
    // marker verifies the database and its WAL contain no key material.
    let marker = "sk-rhythm-must-not-persist-7f4a";
    let raw = std::fs::read(directory.path().join("db")).unwrap();
    assert!(!raw
        .windows(marker.len())
        .any(|window| window == marker.as_bytes()));
    let wal = std::fs::read(directory.path().join("db-wal")).unwrap_or_default();
    assert!(!wal
        .windows(marker.len())
        .any(|window| window == marker.as_bytes()));
}

#[test]
fn remote_automatic_usage_is_bounded_and_releasable() {
    let (_directory, store) = database();
    assert_eq!(store.ai_remote_usage_today().unwrap(), 0);
    assert!(store.ai_reserve_remote_automatic(2).unwrap());
    assert!(store.ai_reserve_remote_automatic(2).unwrap());
    assert!(!store.ai_reserve_remote_automatic(2).unwrap());
    assert_eq!(store.ai_remote_usage_today().unwrap(), 2);
    store.ai_release_remote_automatic().unwrap();
    assert_eq!(store.ai_remote_usage_today().unwrap(), 1);
    assert!(store.ai_reserve_remote_automatic(2).unwrap());
    assert!(!store.ai_reserve_remote_automatic(0).unwrap());
}

#[test]
fn invalid_remote_preferences_are_rejected_without_persisting() {
    let (_directory, store) = database();
    let initial = store.ai_preferences().unwrap();
    let mut preferences = initial.clone();
    preferences.engine = "cloud".into();
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences = initial.clone();
    preferences.remote_timeout_seconds = 5;
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences = initial.clone();
    preferences.remote_daily_limit = 201;
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences = initial.clone();
    preferences.remote_base_url = Some("https://api.example.com\n".into());
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences = initial.clone();
    preferences.remote_model = Some("   ".into());
    assert!(store.ai_save_preferences(&preferences).is_err());
    assert_eq!(store.ai_preferences().unwrap(), initial);
}

#[test]
fn restore_disables_remote_preferences_until_a_new_consent() {
    let (_directory, store) = database();
    let mut preferences = store.ai_preferences().unwrap();
    preferences.engine = "openai-compatible".into();
    preferences.remote_base_url = Some("https://api.example.com".into());
    preferences.remote_model = Some("model".into());
    preferences.remote_consent = true;
    preferences.remote_automatic = true;
    preferences.remote_embeddings = true;
    store.ai_save_preferences(&preferences).unwrap();
    store.reset_remote_after_restore().unwrap();
    let restored = store.ai_preferences().unwrap();
    assert_eq!(restored.engine, "local");
    assert!(!restored.remote_consent);
    assert!(!restored.remote_automatic);
    assert!(!restored.remote_embeddings);
    // The address and model remain as inert configuration; only authorization
    // is revoked, so the user can review what will be re-enabled.
    assert_eq!(
        restored.remote_base_url.as_deref(),
        Some("https://api.example.com")
    );
    assert_eq!(restored.remote_model.as_deref(), Some("model"));
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
    assert_eq!(preferences.max_tokens, 2048);
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

#[test]
fn actual_backend_provenance_commits_with_result_and_survives_reopen() {
    let (directory, mut store) = database();
    let job = store
        .ai_enqueue(&key(), &json!({"id":"test","execution_mode":"auto"}), 1)
        .unwrap();
    store.ai_claim_next().unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_gpu_result BEFORE INSERT ON ai_results BEGIN SELECT RAISE(ABORT,'test result failure'); END;").unwrap();
    assert!(store
        .ai_complete_job_with_backend(
            job.id,
            &json!({"summary":"tested"}),
            false,
            4,
            4,
            10,
            Some("vulkan"),
            None
        )
        .is_err());
    assert!(store
        .ai_job(job.id)
        .unwrap()
        .unwrap()
        .model_identity
        .get("loaded_backend")
        .is_none());
    assert!(store.ai_cached_result(&key()).unwrap().is_none());
    store
        .connection
        .execute_batch("DROP TRIGGER reject_gpu_result;")
        .unwrap();
    let result = store
        .ai_complete_job_with_backend(
            job.id,
            &json!({"summary":"tested"}),
            false,
            4,
            4,
            10,
            Some("vulkan"),
            None,
        )
        .unwrap();
    assert_eq!(result.model_identity["loaded_backend"], "vulkan");
    assert_eq!(
        store.ai_job(job.id).unwrap().unwrap().model_identity["loaded_backend"],
        "vulkan"
    );
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    assert_eq!(
        reopened
            .ai_cached_result(&key())
            .unwrap()
            .unwrap()
            .model_identity["loaded_backend"],
        "vulkan"
    );
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
fn key() -> AnalysisKey {
    AnalysisKey {
        document_id: 1,
        revision_id: 1,
        task: "notice_analysis".into(),
        model_sha256: "a".repeat(64),
        prompt_version: 1,
        schema_version: 1,
        configuration_hash: "b".repeat(64),
    }
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
fn active_jobs_and_document_history_are_independent_of_global_history_window() {
    let (_directory, mut store) = database();
    let active_key = key();
    let active = store
        .ai_enqueue(&active_key, &json!({"id":"old-model"}), 1)
        .unwrap();
    assert_eq!(store.ai_claim_next().unwrap().unwrap().id, active.id);
    let pending_key = AnalysisKey {
        prompt_version: 2,
        ..key()
    };
    let pending = store
        .ai_enqueue(&pending_key, &json!({"id":"old-model"}), 50)
        .unwrap();
    for version in 3..=105 {
        let newer_key = AnalysisKey {
            prompt_version: version,
            model_sha256: "c".repeat(64),
            ..key()
        };
        let newer = store
            .ai_enqueue(&newer_key, &json!({"id":"other-model"}), 100)
            .unwrap();
        store.ai_finish_job(newer.id, "failed", None, None).unwrap();
    }
    assert!(!store
        .ai_jobs(100)
        .unwrap()
        .iter()
        .any(|job| job.id == active.id || job.id == pending.id));
    let visible = store.ai_active_jobs().unwrap();
    assert_eq!(
        visible.iter().map(|job| job.id).collect::<Vec<_>>(),
        [pending.id, active.id]
    );
    assert_eq!(visible[0].status, "pending");
    assert_eq!(visible[1].status, "running");
    assert!(store.ai_has_model_jobs(&active_key.model_sha256).unwrap());
    assert!(!store.ai_has_model_jobs(&"c".repeat(64)).unwrap());
    assert!(!store.ai_has_model_jobs(&"d".repeat(64)).unwrap());

    store.connection.execute_batch("INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical) VALUES(2,'test','second','https://www.sogang.ac.kr/second','다른 공지','2026-01-01','<p>본문</p>','본문','secondhash',1); INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(2,2,'https://www.sogang.ac.kr/second','다른 공지','2026-01-01','original','<p>본문</p>','본문','secondhash');").unwrap();
    let other = store
        .ai_enqueue(
            &AnalysisKey {
                document_id: 2,
                revision_id: 2,
                ..key()
            },
            &json!({}),
            1,
        )
        .unwrap();
    assert_eq!(
        store.ai_document_jobs(1, 1, 1).unwrap()[0]
            .key
            .prompt_version,
        105
    );
    let local_history = store.ai_document_jobs(1, 1, 20).unwrap();
    assert!(local_history.iter().any(|job| job.id == active.id));
    assert!(local_history.iter().any(|job| job.id == pending.id));
    assert_eq!(local_history.len(), 22);
    assert_eq!(store.ai_document_jobs(1, 1, 1000).unwrap().len(), 102);
    assert_eq!(store.ai_document_jobs(2, 2, 10).unwrap()[0].id, other.id);
    assert!(store.ai_document_jobs(1, 2, 10).unwrap().is_empty());
    assert!(store.ai_document_jobs(1, 1, 0).unwrap().is_empty());
    store
        .ai_finish_job(active.id, "cancelled", None, None)
        .unwrap();
    store
        .ai_finish_job(pending.id, "cancelled", None, None)
        .unwrap();
    store
        .ai_finish_job(other.id, "cancelled", None, None)
        .unwrap();
    assert!(!store.ai_has_model_jobs(&active_key.model_sha256).unwrap());
    assert!(store.ai_active_jobs().unwrap().is_empty());
    integrity(&store);
}
fn recent_article(
    external_id: &str,
    published_at: &str,
    body: &str,
) -> crate::archive::model::ArticleInput {
    crate::archive::model::ArticleInput {
        external_id: external_id.into(),
        canonical_url: format!("https://www.sogang.ac.kr/notice/{external_id}"),
        title: "분석할 공지".into(),
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
fn recent_record(
    store: &Store,
    external_id: &str,
    published_at: &str,
    historical: bool,
    collected_at: &str,
) -> (i64, i64) {
    let (id, _) = store
        .upsert_article(
            "test",
            &recent_article(external_id, published_at, "신청 안내"),
            historical,
        )
        .unwrap();
    let revision = store.document(id, None).unwrap().revision_id;
    store
        .connection
        .execute(
            "UPDATE documents SET collected_at=?2 WHERE id=?1",
            params![id, collected_at],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE document_revisions SET collected_at=?2 WHERE id=?1",
            params![revision, collected_at],
        )
        .unwrap();
    (id, revision)
}
#[test]
fn recent_candidates_follow_ingestion_not_publication_and_exclude_startup_backfill() {
    let (_directory, store) = database();
    let boundary = "2026-10-25T12:00:00.500Z";
    store.connection.execute_batch("UPDATE documents SET collected_at='2026-10-01T00:00:00.000Z'; UPDATE document_revisions SET collected_at='2026-10-01T00:00:00.000Z';").unwrap();
    for index in 0..25 {
        recent_record(
            &store,
            &format!("newer-publication-{index}"),
            "2026-10-25",
            false,
            "2026-10-25T11:59:59.000Z",
        );
    }
    let expected = recent_record(
        &store,
        "late-arrival",
        "2025-01-01",
        false,
        "2026-10-25T12:00:00.501Z",
    );
    recent_record(
        &store,
        "backfill",
        "2026-12-31",
        true,
        "2026-10-25T12:00:00.900Z",
    );
    recent_record(&store, "at-boundary", "2026-12-30", false, boundary);
    assert_eq!(
        store.ai_recent_candidates(boundary, 20, false).unwrap(),
        [expected]
    );
    assert!(store
        .ai_recent_candidates(boundary, 0, false)
        .unwrap()
        .is_empty());
    integrity(&store);
}
#[test]
fn recent_candidates_include_meaningful_revisions_of_old_and_historical_documents() {
    let (_directory, store) = database();
    let boundary = "2026-10-25T12:00:00.500Z";
    store.connection.execute_batch("UPDATE documents SET collected_at='2026-10-01T00:00:00.000Z'; UPDATE document_revisions SET collected_at='2026-10-01T00:00:00.000Z';").unwrap();
    let mut expected = Vec::new();
    for (external, historical) in [("old-arrival", false), ("old-history", true)] {
        let (id, original_revision) = recent_record(
            &store,
            external,
            "2025-01-01",
            historical,
            "2026-10-01T00:00:00.000Z",
        );
        let old_attempt = store
            .ai_enqueue(
                &AnalysisKey {
                    document_id: id,
                    revision_id: original_revision,
                    ..key()
                },
                &json!({}),
                1,
            )
            .unwrap();
        store
            .ai_finish_job(old_attempt.id, "failed", None, None)
            .unwrap();
        let changed = recent_article(
            external,
            "2025-01-01",
            "정정: 신청 마감은 2026년 10월 30일입니다.",
        );
        assert!(store.upsert_article("test", &changed, false).unwrap().1);
        let revision = store.document(id, None).unwrap().revision_id;
        assert_ne!(revision, original_revision);
        store
            .connection
            .execute(
                "UPDATE document_revisions SET collected_at='2026-10-25T12:00:00.600Z' WHERE id=?1",
                [revision],
            )
            .unwrap();
        assert_eq!(
            store.document(id, None).unwrap().item.historical,
            historical
        );
        // A validator/timestamp-only check does not create another revision.
        assert!(!store.upsert_article("test", &changed, false).unwrap().1);
        assert_eq!(store.document(id, None).unwrap().revision_id, revision);
        expected.insert(0, (id, revision));
    }
    recent_record(
        &store,
        "fresh-backfill",
        "2026-12-31",
        true,
        "2026-10-25T12:00:00.900Z",
    );
    assert_eq!(
        store.ai_recent_candidates(boundary, 20, false).unwrap(),
        expected
    );
    integrity(&store);
}
#[test]
fn recent_candidates_never_retry_cached_failed_cancelled_or_active_revisions() {
    let (_directory, mut store) = database();
    let boundary = "2026-10-25T12:00:00.500Z";
    store.connection.execute_batch("UPDATE documents SET collected_at='2026-10-01T00:00:00.000Z'; UPDATE document_revisions SET collected_at='2026-10-01T00:00:00.000Z';").unwrap();
    let candidate_key = |(document_id, revision_id)| AnalysisKey {
        document_id,
        revision_id,
        ..key()
    };
    let cached = recent_record(
        &store,
        "cached",
        "2025-01-01",
        false,
        "2026-10-25T12:00:00.600Z",
    );
    let job = store
        .ai_enqueue(&candidate_key(cached), &json!({}), 100)
        .unwrap();
    store.ai_claim_next().unwrap().unwrap();
    store
        .ai_complete_job(job.id, &json!({"summary":"저장된 결과"}), false, 1, 1, 1)
        .unwrap();
    let first_job = job.id;
    for status in ["failed", "cancelled", "stale"] {
        let identity = recent_record(
            &store,
            status,
            "2025-01-01",
            false,
            "2026-10-25T12:00:00.600Z",
        );
        let job = store
            .ai_enqueue(&candidate_key(identity), &json!({}), 100)
            .unwrap();
        store.ai_finish_job(job.id, status, None, None).unwrap();
    }
    let running = recent_record(
        &store,
        "running",
        "2025-01-01",
        false,
        "2026-10-25T12:00:00.600Z",
    );
    let running_job = store
        .ai_enqueue(&candidate_key(running), &json!({}), 100)
        .unwrap();
    assert_eq!(store.ai_claim_next().unwrap().unwrap().id, running_job.id);
    let pending = recent_record(
        &store,
        "pending",
        "2025-01-01",
        false,
        "2026-10-25T12:00:00.600Z",
    );
    store
        .ai_enqueue(&candidate_key(pending), &json!({}), 100)
        .unwrap();
    for version in 10..120 {
        let noise = store
            .ai_enqueue(
                &AnalysisKey {
                    prompt_version: version,
                    ..key()
                },
                &json!({}),
                1,
            )
            .unwrap();
        store.ai_finish_job(noise.id, "failed", None, None).unwrap();
    }
    assert!(store
        .ai_jobs(100)
        .unwrap()
        .iter()
        .all(|job| job.id != first_job));
    let untouched = recent_record(
        &store,
        "unprocessed",
        "2024-01-01",
        false,
        "2026-10-25T12:00:00.600Z",
    );
    assert_eq!(
        store.ai_recent_candidates(boundary, 20, false).unwrap(),
        [untouched]
    );
    assert!(store
        .ai_cached_result(&candidate_key(cached))
        .unwrap()
        .is_some());
    integrity(&store);
}
#[test]
fn recent_candidate_window_is_bounded_and_has_stable_ingestion_order() {
    let (_directory, store) = database();
    let boundary = "2026-10-25T12:00:00.500Z";
    store.connection.execute_batch("UPDATE documents SET collected_at='2026-10-01T00:00:00.000Z'; UPDATE document_revisions SET collected_at='2026-10-01T00:00:00.000Z';").unwrap();
    let mut identities = Vec::new();
    for index in 0..105 {
        identities.push(recent_record(
            &store,
            &format!("candidate-{index}"),
            "2025-01-01",
            false,
            "2026-10-25T12:00:00.600Z",
        ));
    }
    identities.reverse();
    assert_eq!(
        store.ai_recent_candidates(boundary, 3, false).unwrap(),
        identities[..3]
    );
    assert_eq!(
        store.ai_recent_candidates(boundary, 1000, false).unwrap(),
        identities[..100]
    );
    integrity(&store);
}
#[test]
fn backlog_candidates_include_backfill_and_order_by_publication_newest_first() {
    let (_directory, store) = database();
    store
        .connection
        .execute("DELETE FROM documents", [])
        .unwrap();
    let old = recent_record(
        &store,
        "backfill-old",
        "2024-03-01",
        true,
        "2026-10-01T00:00:00.000Z",
    );
    let new = recent_record(
        &store,
        "newest",
        "2026-09-01",
        false,
        "2026-10-01T00:00:00.000Z",
    );
    let attempted = recent_record(
        &store,
        "attempted",
        "2026-10-01",
        false,
        "2026-10-01T00:00:00.000Z",
    );
    store
        .ai_enqueue(
            &AnalysisKey {
                document_id: attempted.0,
                revision_id: attempted.1,
                ..key()
            },
            &json!({}),
            1,
        )
        .unwrap();
    let empty = store
        .upsert_article(
            "test",
            &recent_article("empty-body", "2026-11-01", ""),
            false,
        )
        .unwrap()
        .0;
    assert!(store.document(empty, None).unwrap().body_text.is_empty());
    assert_eq!(store.ai_backlog_candidates(20, false).unwrap(), [new, old]);
    assert_eq!(store.ai_backlog_candidates(1, false).unwrap(), [new]);
    assert!(store.ai_backlog_candidates(0, false).unwrap().is_empty());
    integrity(&store);
}
#[test]
fn remote_candidates_never_include_cybercampus_documents() {
    let (_directory, store) = database();
    store.catalog(&crate::archive::definitions()).unwrap();
    store
        .connection
        .execute("DELETE FROM documents", [])
        .unwrap();
    let public = recent_record(
        &store,
        "public",
        "2026-10-01",
        false,
        "2026-10-02T00:00:00.000Z",
    );
    let (id, _) = store
        .upsert_article(
            "cybercampus",
            &recent_article("A202631099990101-9000001", "2026-10-03", "과제 안내"),
            false,
        )
        .unwrap();
    let course = (id, store.document(id, None).unwrap().revision_id);
    store
        .connection
        .execute_batch(
            "UPDATE documents SET collected_at='2026-10-02T00:00:00.000Z';
             UPDATE document_revisions SET collected_at='2026-10-02T00:00:00.000Z';",
        )
        .unwrap();
    let boundary = "2026-10-01T00:00:00.000Z";
    // The local engine may analyze the user's course notices.
    assert_eq!(
        store.ai_recent_candidates(boundary, 20, false).unwrap(),
        [course, public]
    );
    assert_eq!(
        store.ai_backlog_candidates(20, false).unwrap(),
        [course, public]
    );
    // A remote engine never receives them, automatically or by classification.
    assert_eq!(
        store.ai_recent_candidates(boundary, 20, true).unwrap(),
        [public]
    );
    assert_eq!(store.ai_backlog_candidates(20, true).unwrap(), [public]);
    let model = "a".repeat(64);
    let config = "b".repeat(64);
    assert_eq!(
        store
            .ai_embedding_candidates(&model, &config, 10, false)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .ai_embedding_candidates(&model, &config, 10, true)
            .unwrap(),
        [public]
    );
}
#[test]
fn backlog_mode_migrates_schema_thirteen_and_persists_across_reopen() {
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
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch(
            "UPDATE ai_preferences SET processing_mode='recent',setup_completed=1,
             automatic_limit=7,max_tokens=2048;",
        )
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let mut preferences = store.ai_preferences().unwrap();
    assert_eq!(preferences.processing_mode, "recent");
    assert!(preferences.setup_completed);
    assert_eq!(preferences.automatic_limit, 7);
    assert_eq!(preferences.max_tokens, 2048);
    preferences.processing_mode = "backlog".into();
    store.ai_save_preferences(&preferences).unwrap();
    assert_eq!(store.ai_preferences().unwrap().processing_mode, "backlog");
    preferences.processing_mode = "invalid".into();
    assert!(store.ai_save_preferences(&preferences).is_err());
    drop(store);
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .ai_preferences()
            .unwrap()
            .processing_mode,
        "backlog"
    );
}
#[test]
fn queue_claims_direct_requests_before_idle_backlog_work() {
    let (_directory, mut store) = database();
    let backlog = recent_record(
        &store,
        "backlog",
        "2026-05-01",
        true,
        "2026-10-01T00:00:00.000Z",
    );
    let direct = recent_record(
        &store,
        "direct",
        "2026-05-02",
        false,
        "2026-10-01T00:00:00.000Z",
    );
    let backlog_job = store
        .ai_enqueue(
            &AnalysisKey {
                document_id: backlog.0,
                revision_id: backlog.1,
                ..key()
            },
            &json!({}),
            10,
        )
        .unwrap();
    let direct_job = store
        .ai_enqueue(
            &AnalysisKey {
                document_id: direct.0,
                revision_id: direct.1,
                ..key()
            },
            &json!({}),
            100,
        )
        .unwrap();
    assert_eq!(store.ai_claim_next().unwrap().unwrap().id, direct_job.id);
    assert_eq!(store.ai_claim_next().unwrap().unwrap().id, backlog_job.id);
    assert!(store.ai_claim_next().unwrap().is_none());
    integrity(&store);
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
    assert_eq!(store.ai_preferences().unwrap().processing_mode, "explicit");
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
fn active_dedup_and_cache_identity_preserve_good_result_across_regeneration() {
    let (_directory, mut store) = database();
    let identity = json!({"name":"Qwen test","sha256":"a".repeat(64)});
    let first = store.ai_enqueue(&key(), &identity, 1).unwrap();
    let promoted = store.ai_enqueue(&key(), &identity, 10).unwrap();
    assert_eq!(promoted.id, first.id);
    assert_eq!(promoted.priority, 10);
    assert_eq!(store.ai_jobs(10).unwrap().len(), 1);
    assert!(store.ai_cached_result(&key()).unwrap().is_none());
    assert_eq!(store.ai_claim_next().unwrap().unwrap().id, first.id);
    assert!(store.ai_claim_next().unwrap().is_none());
    let saved = store
        .ai_complete_job(first.id, &json!({"summary":"장학 신청"}), false, 20, 20, 42)
        .unwrap();
    assert_eq!(store.ai_job(first.id).unwrap().unwrap().status, "completed");
    assert_eq!(
        store.ai_cached_result(&key()).unwrap().unwrap().id,
        saved.id
    );
    for changed_key in [
        AnalysisKey {
            model_sha256: "c".repeat(64),
            ..key()
        },
        AnalysisKey {
            prompt_version: 2,
            ..key()
        },
        AnalysisKey {
            schema_version: 2,
            ..key()
        },
        AnalysisKey {
            configuration_hash: "d".repeat(64),
            ..key()
        },
        AnalysisKey {
            revision_id: 2,
            ..key()
        },
        AnalysisKey {
            task: "other".into(),
            ..key()
        },
    ] {
        assert!(store.ai_cached_result(&changed_key).unwrap().is_none());
    }
    let retry = store.ai_enqueue(&key(), &identity, 10).unwrap();
    assert_ne!(first.id, retry.id);
    store
        .ai_finish_job(retry.id, "cancelled", None, None)
        .unwrap();
    assert!(store
        .ai_complete_job(retry.id, &json!({"summary":"no"}), false, 1, 1, 1)
        .is_err());
    assert_eq!(
        store.ai_cached_result(&key()).unwrap().unwrap().id,
        saved.id
    );
    assert_eq!(store.ai_document_results(1, Some(1), 10).unwrap().len(), 1);
    assert_eq!(store.ai_recent_results(10).unwrap().len(), 1);
    integrity(&store);
}
#[test]
fn restart_recovers_work_without_losing_inventory_or_results() {
    let (directory, mut store) = database();
    let running = store.ai_enqueue(&key(), &json!({}), 1).unwrap();
    store.ai_claim_next().unwrap().unwrap();
    let pending = store
        .ai_enqueue(
            &AnalysisKey {
                prompt_version: 2,
                ..key()
            },
            &json!({}),
            1,
        )
        .unwrap();
    let mut downloading = model("download", "managed");
    downloading.status = "downloading".into();
    downloading.downloaded_bytes = 17;
    downloading.resume_etag = Some("\"pinned\"".into());
    store.ai_upsert_model(&downloading).unwrap();
    drop(store);
    let store = Store::open(&directory.path().join("db")).unwrap();
    assert_eq!(store.ai_job(running.id).unwrap().unwrap().status, "failed");
    assert_eq!(store.ai_job(pending.id).unwrap().unwrap().status, "pending");
    let recovered = store.ai_model("download").unwrap().unwrap();
    assert_eq!(recovered.status, "cancelled");
    assert_eq!(recovered.downloaded_bytes, 17);
    assert_eq!(recovered.resume_etag, downloading.resume_etag);
    integrity(&store);
}
#[test]
fn model_unregistration_clears_selection_but_never_removes_file_or_results() {
    let (directory, mut store) = database();
    let path = directory.path().join("external.gguf");
    std::fs::write(&path, b"GGUF retained").unwrap();
    let mut external = model("local", "external");
    external.path = path.to_str().unwrap().into();
    store.ai_upsert_model(&external).unwrap();
    assert_eq!(store.ai_models().unwrap().len(), 1);
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some("local".into());
    preferences.setup_completed = true;
    store.ai_save_preferences(&preferences).unwrap();
    let job = store
        .ai_enqueue(&key(), &json!({"name":"external"}), 1)
        .unwrap();
    store.ai_claim_next().unwrap();
    store
        .ai_complete_job(job.id, &json!({"summary":"saved"}), false, 3, 3, 1)
        .unwrap();
    store.ai_remove_model("local").unwrap();
    assert!(path.exists());
    assert!(store.ai_preferences().unwrap().active_model_id.is_none());
    assert!(store.ai_preferences().unwrap().setup_completed);
    assert!(store.ai_cached_result(&key()).unwrap().is_some());
    store.set_enabled("sogang_notices", false).unwrap();
    assert!(store.ai_cached_result(&key()).unwrap().is_some());
    assert!(store.document(1, None).unwrap().item.saved);
    integrity(&store);
}
#[test]
fn boundaries_reject_wrong_revision_invalid_inventory_and_late_result_failure() {
    let (_directory, mut store) = database();
    assert!(store
        .ai_enqueue(
            &AnalysisKey {
                document_id: 99,
                ..key()
            },
            &json!({}),
            1
        )
        .is_err());
    let mut invalid = model("invalid", "external");
    invalid.sha256 = None;
    assert!(store.ai_upsert_model(&invalid).is_err());
    let mut preferences = store.ai_preferences().unwrap();
    preferences.active_model_id = Some("missing".into());
    assert!(store.ai_save_preferences(&preferences).is_err());
    let job = store.ai_enqueue(&key(), &json!({}), 1).unwrap();
    store.ai_claim_next().unwrap();
    assert!(store
        .ai_complete_job(job.id, &json!({}), false, 2, 3, 1)
        .is_err());
    store.connection.execute_batch("CREATE TRIGGER reject_ai_complete BEFORE UPDATE OF status ON ai_jobs WHEN new.status='completed' BEGIN SELECT RAISE(ABORT,'test completion failure'); END;").unwrap();
    assert!(store
        .ai_complete_job(job.id, &json!({"summary":"no commit"}), false, 3, 3, 1)
        .is_err());
    assert!(store.ai_cached_result(&key()).unwrap().is_none());
    assert_eq!(store.ai_job(job.id).unwrap().unwrap().status, "running");
    integrity(&store);
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
        .execute_batch(
            "UPDATE ai_preferences SET processing_mode='backlog',max_tokens=2048 WHERE id=1;",
        )
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let preferences = store.ai_preferences().unwrap();
    assert_eq!(preferences.processing_mode, "backlog");
    assert_eq!(preferences.max_tokens, 2048);
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
        .ai_embedding_candidates(&model_a, &config, 10, false)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .ai_embedding_candidates(&model_b, &config, 10, false)
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
    // A generation model cannot occupy the embedding slot, and vice versa.
    preferences.active_model_id = Some(model.id.clone());
    assert!(store.ai_save_preferences(&preferences).is_err());
    preferences.active_model_id = None;
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
    let mut article = recent_article(external_id, "2026-03-01", body);
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
    let mut changed = recent_article("g", "2026-03-01", "변경된 본문 3월 10일 17시");
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
