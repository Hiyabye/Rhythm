//! Opt-in evaluation of already archived notices. Source archives are always read-only.
//! Local reports contain source-derived text and must not be committed or uploaded.
use super::{analysis, evaluation, Worker};
use crate::{
    archive::model::{ArticleInput, SourceDefinition},
    storage::Store,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

struct ArchivedSample {
    document_id: i64,
    revision_id: i64,
    content_hash: String,
    source: SourceDefinition,
    article: ArticleInput,
}

fn selection(value: &str) -> Result<Vec<i64>, String> {
    let ids = value
        .split(',')
        .map(|value| {
            value
                .trim()
                .parse::<i64>()
                .map_err(|_| "invalid document ID")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty()
        || ids.len() > 10
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err("select 1–10 distinct positive document IDs".into());
    }
    Ok(ids)
}

fn read_samples(path: &Path, ids: &[i64]) -> Result<Vec<ArchivedSample>, String> {
    // Store::open performs recovery/migrations, so it must never inspect this source.
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut statement = source
        .prepare(
            "SELECT d.id,r.id,r.content_hash,d.external_id,
             r.canonical_url,r.title,r.author,r.category,r.published_at,r.modified_at,
             r.original,r.body_html,r.body_text,
             s.id,s.name,s.organization,s.description,s.url,s.family,s.supported
             FROM documents d JOIN document_revisions r ON r.id=(
                 SELECT MAX(latest.id) FROM document_revisions latest WHERE latest.document_id=d.id
             ) JOIN sources s ON s.id=d.source_id WHERE d.id=?1",
        )
        .map_err(|error| error.to_string())?;
    ids.iter()
        .map(|id| {
            statement
                .query_row([id], |row| {
                    Ok(ArchivedSample {
                        document_id: row.get(0)?,
                        revision_id: row.get(1)?,
                        content_hash: row.get(2)?,
                        source: SourceDefinition {
                            id: row.get(13)?,
                            name: row.get(14)?,
                            organization: row.get(15)?,
                            description: row.get(16)?,
                            url: row.get(17)?,
                            family: row.get(18)?,
                            supported: row.get(19)?,
                        },
                        article: ArticleInput {
                            external_id: row.get(3)?,
                            canonical_url: row.get(4)?,
                            title: row.get(5)?,
                            author: row.get(6)?,
                            category: row.get(7)?,
                            published_at: row.get(8)?,
                            modified_at: row.get(9)?,
                            original: row.get(10)?,
                            body_html: row.get(11)?,
                            body_text: row.get(12)?,
                            // The selected text is evaluated, never attachment extraction/refetch.
                            resources: Vec::new(),
                            etag: None,
                            last_modified: None,
                        },
                    })
                })
                .map_err(|error| format!("archived document {id}: {error}"))
        })
        .collect()
}

#[tokio::test]
#[ignore = "explicit archived sample, installed model and local worker required"]
async fn actual_archived_notice_evaluation() {
    let archive = PathBuf::from(std::env::var_os("RHYTHM_AI_EVAL_ARCHIVE").expect("archive path"));
    let ids = selection(&std::env::var("RHYTHM_AI_ARCHIVE_IDS").unwrap_or_else(|_| "1,4,9".into()))
        .expect("bounded archived selection");
    let samples = read_samples(&archive, &ids).expect("read-only archived revisions");
    let executable = PathBuf::from(std::env::var_os("RHYTHM_WORKER_TEST_BIN").expect("worker"));
    let root = PathBuf::from(std::env::var_os("RHYTHM_MODEL_EVAL_ROOT").expect("model root"));
    let id = std::env::var("RHYTHM_MODEL_EVAL_ID").unwrap_or_else(|_| "qwen35-2b".into());
    let model = evaluation::installed_model(&root, &id);
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("archived-evaluation.sqlite3");
    let local_worker = directory.path().join(format!(
        "rhythm-archived-worker{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::copy(&executable, &local_worker).unwrap();
    let isolated = Store::open(&db).unwrap();
    isolated.ai_upsert_model(&model).unwrap();
    let mut preferences = isolated.ai_preferences().unwrap();
    preferences.active_model_id = Some(model.id.clone());
    preferences.processing_mode = "explicit".into();
    preferences.max_tokens = 1024;
    isolated.ai_save_preferences(&preferences).unwrap();
    let mut imported = Vec::new();
    for sample in &samples {
        isolated
            .catalog(std::slice::from_ref(&sample.source))
            .unwrap();
        let (document, _) = isolated
            .upsert_article(&sample.source.id, &sample.article, false)
            .unwrap();
        let revision = isolated.document(document, None).unwrap().revision_id;
        imported.push((document, revision));
    }
    let store = Arc::new(Mutex::new(isolated));
    let worker = Arc::new(AsyncMutex::new(Worker::launch(&local_worker)));
    let controller = analysis::Controller::default();
    let report_path = evaluation::report_path(&format!("archived-{id}"));
    let mut report = json!({
        "fixture_provenance":"selected latest revisions from an existing read-only local archive",
        "source_archive":archive,"source_document_ids":ids,"model":model,"worker":executable,
        "parent_debug_assertions":cfg!(debug_assertions),"prompt_version":analysis::PROMPT_VERSION,
        "schema_version":analysis::SCHEMA_VERSION,"max_tokens":1024,
        "decoding_mode":analysis::decoding_mode(),
        "network_note":"no HTTP client; not an OS-enforced network-denial test",
        "native_ui_verified":false,"human_review_required":true,
        "privacy_note":"local source-derived report; do not commit or upload", "samples":[]
    });
    let mut failures = Vec::new();
    let mut completed = Vec::new();
    for (sample, (document, revision)) in samples.iter().zip(imported) {
        let mut observation = json!({
            "source_id":sample.source.id,"original_document_id":sample.document_id,
            "original_revision_id":sample.revision_id,"original_content_hash":sample.content_hash,
            "canonical_url":sample.article.canonical_url,"source_title":sample.article.title,
            "source_body":sample.article.body_text,"original_representation_bytes":sample.article.original.len(),
            "original_representation_sha256":format!("{:x}",Sha256::digest(sample.article.original.as_bytes())),
            "imported_document_id":document,"imported_revision_id":revision,
            "human_review_required":true
        });
        let started = Instant::now();
        match controller.start(
            store.clone(),
            worker.clone(),
            directory.path().into(),
            document,
            revision,
            false,
        ) {
            Ok(admitted) => {
                let job_id = admitted.job.expect("fresh isolated evaluation job").id;
                if tokio::time::timeout(Duration::from_secs(1500), controller.wait())
                    .await
                    .is_err()
                {
                    observation["timeout"] = json!(true);
                    let _ = controller.cancel(&store, job_id);
                    failures.push(format!(
                        "document {}: evaluation deadline",
                        sample.document_id
                    ));
                    // Signal and join cleanup before reading/reusing a serial worker.
                    controller.wait().await;
                }
                observation["wall_ms"] = json!(started.elapsed().as_millis());
                observation["diagnostics"] = json!(controller.diagnostics(job_id));
                observation["worker_rss_kib"] = json!(evaluation::worker_rss_kib(&local_worker));
                let (job, results) = {
                    let store = store.lock().unwrap();
                    (
                        store.ai_job(job_id).unwrap().unwrap(),
                        store
                            .ai_document_results(document, Some(revision), 10)
                            .unwrap(),
                    )
                };
                observation["job"] = json!(job);
                if job.status == "completed" {
                    let result = results.first().expect("completed result transaction");
                    observation["result"] = json!(result);
                    let source =
                        format!("{}\n\n{}", sample.article.title, sample.article.body_text);
                    let (shape, grounded) =
                        evaluation::stored_result_checks(&result.result, &source);
                    observation["schema_valid"] = json!(shape);
                    observation["literal_grounding_valid"] = json!(grounded);
                    if !shape || !grounded {
                        failures.push(format!(
                            "document {}: stored result validation",
                            sample.document_id
                        ));
                    }
                    let cached_started = Instant::now();
                    let cached = controller.start(
                        store.clone(),
                        worker.clone(),
                        directory.path().into(),
                        document,
                        revision,
                        false,
                    );
                    observation["cached_wall_us"] = json!(cached_started.elapsed().as_micros());
                    let cache_reused = cached.is_ok_and(|response| {
                        response.job.is_none()
                            && response.cached.is_some_and(|value| value.id == result.id)
                    });
                    observation["cache_reused"] = json!(cache_reused);
                    if !cache_reused {
                        failures.push(format!("document {}: cache mismatch", sample.document_id));
                    }
                    completed.push((document, revision, result.id));
                } else {
                    failures.push(format!("document {}: {:?}", sample.document_id, job.error));
                }
            }
            Err(error) => {
                observation["admission_error"] = json!(error);
                failures.push(format!("document {}: admission error", sample.document_id));
            }
        }
        report["samples"].as_array_mut().unwrap().push(observation);
        evaluation::write_report(&report_path, &report);
    }
    controller.stop();
    controller.wait().await;
    worker.lock().await.shutdown().await;
    drop(controller);
    drop(store);
    let reopened = Store::open(&db).unwrap();
    let cached_reads = completed
        .iter()
        .filter(|(document, revision, result_id)| {
            reopened
                .ai_document_results(*document, Some(*revision), 10)
                .unwrap()
                .iter()
                .any(|result| result.id == *result_id)
        })
        .count();
    report["reopened_cache_reads_with_worker_stopped"] = json!(cached_reads);
    report["failures"] = json!(failures);
    evaluation::write_report(&report_path, &report);
    assert_eq!(cached_reads, completed.len());
    assert!(
        failures.is_empty(),
        "{failures:?}; see {}",
        report_path.display()
    );
}

#[test]
fn archived_import_reads_latest_exact_revision_without_migrating_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite3");
    let source = Connection::open(&path).unwrap();
    source
        .execute_batch(include_str!("../../migrations/001_foundation.sql"))
        .unwrap();
    source
        .execute_batch(include_str!("../../migrations/002_archive.sql"))
        .unwrap();
    source.execute_batch("PRAGMA user_version=2;
        INSERT INTO sources(id,name,organization,description,url,family,supported) VALUES('source','학사','대학','공지','https://www.sogang.ac.kr/','main',1);
        INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical) VALUES(1,'source','public-42','https://www.sogang.ac.kr/notice/42','현재 메타데이터','2026-01-01','<p>현재 본문</p>','현재 본문','current',1);
        INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(1,1,'https://www.sogang.ac.kr/notice/42','이전 제목','2026-01-01','old original','<p>이전 본문</p>','이전 본문','old');
        INSERT INTO document_revisions(id,document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash) VALUES(2,1,'https://www.sogang.ac.kr/notice/42','최신 보관 제목','2026-01-02','exact fetched representation','<p>최신 보관 본문</p>','최신 보관 본문','latest');").unwrap();
    drop(source);
    let before = std::fs::read(&path).unwrap();
    let samples = read_samples(&path, &[1]).unwrap();
    assert_eq!(samples.len(), 1);
    let sample = &samples[0];
    assert_eq!((sample.document_id, sample.revision_id), (1, 2));
    assert_eq!(sample.content_hash, "latest");
    assert_eq!(sample.source.id, "source");
    assert_eq!(sample.article.external_id, "public-42");
    assert_eq!(
        sample.article.canonical_url,
        "https://www.sogang.ac.kr/notice/42"
    );
    assert_eq!(sample.article.title, "최신 보관 제목");
    assert_eq!(sample.article.body_text, "최신 보관 본문");
    assert_eq!(sample.article.original, "exact fetched representation");
    let isolated = Store::open(&directory.path().join("isolated.sqlite3")).unwrap();
    isolated
        .catalog(std::slice::from_ref(&sample.source))
        .unwrap();
    let (document, _) = isolated
        .upsert_article(&sample.source.id, &sample.article, false)
        .unwrap();
    let imported = isolated.document(document, None).unwrap();
    assert_eq!(imported.item.source_id, "source");
    assert_eq!(imported.body_text, sample.article.body_text);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let read_only = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        read_only
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        2
    );
    assert!(read_only
        .execute("UPDATE documents SET title='forbidden'", [])
        .is_err());
    assert!(read_samples(&path, &[999]).is_err());
    let missing = directory.path().join("does-not-exist.sqlite3");
    assert!(read_samples(&missing, &[1]).is_err());
    assert!(!missing.exists());
}

#[test]
fn archived_selection_is_explicit_distinct_and_bounded() {
    assert_eq!(selection("1, 4,9").unwrap(), [1, 4, 9]);
    for invalid in ["", "1,1", "0", "-1", "1,no", "1,2,3,4,5,6,7,8,9,10,11"] {
        assert!(selection(invalid).is_err(), "{invalid}");
    }
}
