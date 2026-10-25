use super::intelligence::*;
use super::*;
use serde_json::json;

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
        path: format!("/tmp/{id}.gguf"),
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
    assert_eq!(store.ai_document_jobs(1, 1, 1000).unwrap().len(), 100);
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
#[test]
fn populated_schema_two_upgrade_preserves_archive_and_settings() {
    let (directory, store) = database();
    assert_eq!(store.info().unwrap().schema_version, 3);
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
