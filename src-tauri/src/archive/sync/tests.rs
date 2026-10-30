use super::*;
use crate::archive::catalog;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::sync::Notify;

const MAIN: &str = "sogang-academic";
const COMPUTING: &str = "computing-academic";
const PNG_URL: &str = "https://www.sogang.ac.kr/file-fe-prd/editor/archive-fixture.png";
const PDF_URL: &str =
    "https://www.sogang.ac.kr/file-fe-prd/board/2/archive-fixture.pdf?sg=archive.pdf";
const PDF: &[u8] = b"%PDF-1.4\nlocal archive fixture\n%%EOF\n";

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U8AAAAASUVORK5CYII=").unwrap()
}
struct Gate {
    id: u64,
    entered: Notify,
    release: Notify,
}
type RecordedRequest = (String, Option<(Option<String>, Option<String>)>);

struct Fake {
    pages: Mutex<Vec<Vec<u64>>>,
    changed: Mutex<HashMap<u64, String>>,
    bad: Mutex<HashSet<u64>>,
    failed: Mutex<HashSet<u64>>,
    calls: Mutex<Vec<RecordedRequest>>,
    views: std::sync::atomic::AtomicU64,
    conditional: AtomicBool,
    resources: AtomicBool,
    missing_resources: AtomicBool,
    robots: Mutex<String>,
    redirects: Mutex<HashMap<String, String>>,
    limits: Mutex<Vec<(String, usize)>>,
    gate: Option<Arc<Gate>>,
    resource_gate: Option<Arc<Gate>>,
}
impl Fake {
    fn new(pages: Vec<Vec<u64>>) -> Self {
        Self {
            pages: Mutex::new(pages),
            changed: Mutex::new(HashMap::new()),
            bad: Mutex::new(HashSet::new()),
            failed: Mutex::new(HashSet::new()),
            calls: Mutex::new(Vec::new()),
            views: std::sync::atomic::AtomicU64::new(1),
            conditional: AtomicBool::new(false),
            resources: AtomicBool::new(false),
            missing_resources: AtomicBool::new(false),
            robots: Mutex::new("User-agent: *\nAllow: /\n".into()),
            redirects: Mutex::new(HashMap::new()),
            limits: Mutex::new(Vec::new()),
            gate: None,
            resource_gate: None,
        }
    }
    fn paths(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }
    fn detail(&self, id: u64) -> String {
        let mut value: Value =
            serde_json::from_str(include_str!("../fixtures/main-detail.json")).unwrap();
        let data = &mut value["data"];
        data["pkId"] = json!(id);
        data["title"] = json!(format!("장학금 AI 연구 공지 {id}"));
        data["content"] = json!(self
            .changed
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!(
                "<h2>보관 자료 {id}</h2><p>한국어 장학금과 AI 연구 안내입니다.</p>"
            )));
        data["viewCount"] = json!(self.views.load(Ordering::Acquire));
        if self.resources.load(Ordering::Acquire) {
            data["content"] = json!(format!(
                "{}<p><img src='{PNG_URL}' alt='안내 이미지'></p>",
                data["content"].as_str().unwrap()
            ));
            data["fileValue1"] = json!(PDF_URL);
        }
        value.to_string()
    }
}
impl Transport for Fake {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        _cancel: &AtomicBool,
    ) -> Result<network::Fetch, String> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), validators.clone()));
        self.limits.lock().unwrap().push((url.to_string(), limit));
        if let Some(target) = self.redirects.lock().unwrap().get(url.as_str()) {
            return Ok(network::Fetch::Redirect(Url::parse(target).unwrap()));
        }
        let body = |bytes: Vec<u8>, mime: &str, etag: Option<String>| {
            Ok(network::Fetch::Body {
                bytes,
                mime: Some(mime.into()),
                etag,
                last_modified: None,
            })
        };
        if url.path() == "/robots.txt" {
            return body(
                self.robots.lock().unwrap().as_bytes().to_vec(),
                "text/plain",
                None,
            );
        }
        if url.path().ends_with("/boardList") {
            let page = url
                .query_pairs()
                .find(|(key, _)| key == "pageNum")
                .unwrap()
                .1
                .parse::<usize>()
                .unwrap();
            let pages = self.pages.lock().unwrap();
            let ids = pages.get(page - 1).ok_or("없는 목록 페이지")?;
            let mut value: Value =
                serde_json::from_str(include_str!("../fixtures/main-list-exact.json")).unwrap();
            let template = value["data"]["list"][0].clone();
            let rows: Vec<_> = ids
                .iter()
                .map(|id| {
                    let mut row = template.clone();
                    row["pkId"] = json!(id);
                    row["title"] = json!(format!("장학금 AI 연구 공지 {id}"));
                    row["isTop"] = json!(if *id == 1 { "Y" } else { "N" });
                    row
                })
                .collect();
            value["data"]["pageNum"] = json!(page);
            value["data"]["pages"] = json!(pages.len());
            // Synthetic pages may be deliberately sparse to exercise checkpoints,
            // but must retain a coherent public API page-count/row-size envelope.
            let unique = pages.iter().flatten().collect::<HashSet<_>>().len();
            let implied = pages.len().saturating_sub(1) * 50
                + pages
                    .last()
                    .map_or(0, |ids| ids.iter().collect::<HashSet<_>>().len());
            value["data"]["total"] = json!(unique.max(implied));
            value["data"]["size"] = json!(ids.iter().collect::<HashSet<_>>().len());
            value["data"]["list"] = json!(rows);
            value["data"]["hasNextPage"] = json!(page < pages.len());
            return body(value.to_string().into_bytes(), "application/json", None);
        }
        if url.path() == "/api/api/v1/mainKo/BbsData" {
            let id = url
                .query_pairs()
                .find(|(key, _)| key == "pkId")
                .unwrap()
                .1
                .parse::<u64>()
                .unwrap();
            if let Some(gate) = &self.gate {
                if gate.id == id {
                    gate.entered.notify_one();
                    gate.release.notified().await;
                }
            }
            if self.failed.lock().unwrap().contains(&id) {
                return Err("모의 학교 서버 연결 실패".into());
            }
            if self.bad.lock().unwrap().contains(&id) {
                return body(
                    b"<html>changed markup</html>".to_vec(),
                    "application/json",
                    None,
                );
            }
            if self.conditional.load(Ordering::Acquire)
                && validators.as_ref().is_some_and(|(etag, _)| etag.is_some())
            {
                return Ok(network::Fetch::NotModified);
            }
            return body(
                self.detail(id).into_bytes(),
                "application/json",
                Some(format!("\"article-{id}\"")),
            );
        }
        if url.host_str() == Some("computing.sogang.ac.kr") && url.path().ends_with("/list") {
            return body(
                include_bytes!("../fixtures/computing-academic-list.html").to_vec(),
                "text/html",
                None,
            );
        }
        if url.host_str() == Some("computing.sogang.ac.kr") && url.path().ends_with("/detail/969") {
            return body(
                include_bytes!("../fixtures/computing-academic-detail.html").to_vec(),
                "text/html",
                None,
            );
        }
        if url.as_str() == PNG_URL {
            if let Some(gate) = &self.resource_gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
        }
        if self.missing_resources.load(Ordering::Acquire)
            && (url.as_str() == PNG_URL || url.as_str() == PDF_URL)
        {
            return Err("모의 첨부 파일 연결 실패".into());
        }
        if url.as_str() == PNG_URL {
            return body(png(), "image/png", None);
        }
        if url.as_str() == PDF_URL || url.path().ends_with(".pdf") {
            return body(PDF.to_vec(), "application/pdf", None);
        }
        Err(format!("예상하지 않은 모의 요청: {}", url.path()))
    }
}
fn setup(ids: &[&str]) -> (TempDir, SharedStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("rhythm.sqlite3")).unwrap();
    store.catalog(&catalog::definitions()).unwrap();
    store.set_enabled("sogang_notices", true).unwrap();
    for id in ids {
        store.subscribe(id, true).unwrap();
    }
    (dir, Arc::new(Mutex::new(store)))
}
fn sources(store: &SharedStore, ids: &[&str]) -> Vec<SourceInfo> {
    with_store(store, |s| ids.iter().map(|id| s.source(id)).collect()).unwrap()
}
async fn run(
    controller: &Controller,
    store: &SharedStore,
    root: &Path,
    fake: &Fake,
    ids: &[&str],
    mode: &str,
    budget: Budget,
) {
    let _ = controller
        .execute(store, root, fake, sources(store, ids), mode, budget)
        .await;
}
fn items(store: &SharedStore) -> Vec<DocumentItem> {
    with_store(store, |s| {
        s.archive_query(&ArchiveQuery {
            limit: Some(100),
            ..Default::default()
        })
    })
    .unwrap()
    .items
}

#[tokio::test]
async fn historical_import_exceeds_thirty_and_deduplicates_repeated_pins() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![
        (1..=50).collect(),
        std::iter::once(1).chain(51..=62).collect(),
    ]);
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert_eq!(items(&store).len(), 62);
    assert!(items(&store).iter().all(|item| item.historical));
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .new_since_sync,
        0
    );
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert!(source.history_complete);
    assert_eq!(source.next_page, 3);
    assert_eq!(source.archived, 62);
    assert_eq!(
        fake.paths()
            .iter()
            .filter(|url| url.ends_with("?pkId=1"))
            .count(),
        1
    );
    let before = fake.paths().len();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert_eq!(
        fake.paths().len(),
        before,
        "finished history must not be fetched again"
    );
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert_eq!(items(&store).len(), 62);
    for item in items(&store) {
        assert_eq!(
            with_store(&store, |s| s.document(item.id, None))
                .unwrap()
                .revisions
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn incremental_sync_only_revises_meaningful_changes_and_preserves_user_state() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2]]);
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let original = items(&store)
        .into_iter()
        .find(|item| item.title.ends_with(" 1"))
        .unwrap();
    with_store(&store, |s| {
        s.set_document_state(original.id, Some(true), Some(true))
    })
    .unwrap();
    fake.views.store(999, Ordering::Release);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert_eq!(
        with_store(&store, |s| s.document(original.id, None))
            .unwrap()
            .revisions
            .len(),
        1,
        "view count is not a meaningful revision"
    );
    fake.changed.lock().unwrap().insert(
        1,
        "<h2>수정된 공지</h2><p>장학금 지급 일정이 변경되었습니다.</p>".into(),
    );
    fake.pages.lock().unwrap()[0].push(3);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let detail = with_store(&store, |s| s.document(original.id, None)).unwrap();
    assert_eq!(detail.revisions.len(), 2);
    assert!(detail.item.is_read && detail.item.saved);
    assert!(detail.body_text.contains("변경"));
    let old = with_store(&store, |s| {
        s.document(original.id, Some(detail.revisions[1].id))
    })
    .unwrap();
    assert!(old.body_text.contains("AI 연구"));
    assert!(!old.body_text.contains("변경"));
    assert_eq!(items(&store).len(), 3);
    assert!(
        !items(&store)
            .into_iter()
            .find(|item| item.title.ends_with(" 3"))
            .unwrap()
            .historical
    );
}

#[tokio::test]
async fn iframe_only_article_is_archived_but_cannot_replace_existing_readable_revision() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![551681]]);
    *fake.changed.lock().unwrap() = HashMap::from([(
        551681,
        "<iframe src='https://embed.example.test/video'></iframe>".into(),
    )]);
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let item = items(&store).into_iter().next().unwrap();
    let initial = with_store(&store, |s| s.document(item.id, None)).unwrap();
    assert!(initial.body_html.is_empty());
    assert!(initial.body_text.is_empty());
    assert_eq!(initial.revisions.len(), 1);
    let original = with_store(&store, |s| s.latest_original(item.id)).unwrap();
    assert!(original.contains("iframe"));
    assert!(initial.item.title.ends_with("551681"));
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let repeated = with_store(&store, |s| s.document(item.id, None)).unwrap();
    assert_eq!(repeated.revisions.len(), 1);
    assert!(repeated.body_html.is_empty());
    fake.changed
        .lock()
        .unwrap()
        .insert(551681, "<p>읽을 수 있는 기존 본문</p>".into());
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let readable = with_store(&store, |s| s.document(item.id, None)).unwrap();
    assert!(readable.body_text.contains("읽을 수 있는 기존 본문"));
    assert_eq!(readable.revisions.len(), 2);
    let good_revision = readable.revisions[0].id;
    let readable_original = with_store(&store, |s| s.latest_original(item.id)).unwrap();
    fake.changed.lock().unwrap().insert(
        551681,
        "<iframe src='https://embed.example.test/changed'></iframe>".into(),
    );
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let after = with_store(&store, |s| s.document(item.id, None)).unwrap();
    assert!(after.body_text.contains("읽을 수 있는 기존 본문"));
    assert_eq!(after.revisions.len(), 2);
    assert_eq!(after.revisions[0].id, good_revision);
    assert_eq!(
        with_store(&store, |s| s.latest_original(item.id)).unwrap(),
        readable_original
    );
    assert_eq!(
        with_store(&store, |s| Ok(s.source(MAIN)?.status)).unwrap(),
        "failed"
    );
    assert!(!original.is_empty());
    assert!(!fake
        .paths()
        .iter()
        .any(|url| url.contains("embed.example.test")));
}

#[tokio::test]
async fn conditional_recheck_keeps_revision_and_sends_saved_validator() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![7]]);
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    fake.conditional.store(true, Ordering::Release);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let item = &items(&store)[0];
    assert_eq!(
        with_store(&store, |s| s.document(item.id, None))
            .unwrap()
            .revisions
            .len(),
        1
    );
    assert!(fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(url, validators)| url.ends_with("?pkId=7")
            && validators
                .as_ref()
                .is_some_and(|(etag, _)| etag.as_deref() == Some("\"article-7\""))));
}

#[tokio::test]
async fn source_failures_preserve_archive_and_other_sources_continue() {
    let (dir, store) = setup(&[MAIN, COMPUTING]);
    let fake = Fake::new(vec![vec![5]]);
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    fake.failed.lock().unwrap().insert(5);
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN, COMPUTING]),
            "incremental",
            Budget {
                articles: Some(1),
                pages: Some(1),
            },
        )
        .await;
    assert!(outcome.is_err(), "a failed source must not report success");
    assert_eq!(
        with_store(&store, |s| s.source(MAIN)).unwrap().status,
        "failed"
    );
    assert_eq!(
        with_store(&store, |s| s.document(id, None))
            .unwrap()
            .revisions
            .len(),
        1
    );
    assert!(items(&store).iter().any(|item| item.source_id == COMPUTING));
    assert_ne!(
        with_store(&store, |s| s.source(COMPUTING)).unwrap().status,
        "failed"
    );
    fake.failed.lock().unwrap().clear();
    fake.bad.lock().unwrap().insert(5);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert_eq!(
        with_store(&store, |s| s.source(MAIN)).unwrap().status,
        "failed"
    );
    assert!(with_store(&store, |s| s.document(id, None))
        .unwrap()
        .body_text
        .contains("장학금"));
}

#[tokio::test]
async fn cancellation_after_checkpoint_resumes_without_reloading_completed_pages() {
    let (dir, store) = setup(&[MAIN]);
    let gate = Arc::new(Gate {
        id: 51,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let mut fake = Fake::new(vec![(1..=50).collect(), vec![1, 51, 52]]);
    fake.gate = Some(gate.clone());
    let controller = Controller::new();
    let selected = sources(&store, &[MAIN]);
    let execute = controller.execute(
        &store,
        dir.path(),
        &fake,
        selected,
        "historical",
        Budget::default(),
    );
    let cancel = async {
        gate.entered.notified().await;
        controller.cancel();
        gate.release.notify_one();
    };
    let (result, ()) = tokio::join!(execute, cancel);
    assert!(result.is_err());
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "cancelled");
    assert_eq!(source.next_page, 2);
    assert!(!source.history_complete);
    assert_eq!(items(&store).len(), 50);
    let path = dir.path().join("rhythm.sqlite3");
    drop(store);
    let resumed = Arc::new(Mutex::new(Store::open(&path).unwrap()));
    let fake = Fake::new(vec![(1..=50).collect(), vec![1, 51, 52]]);
    run(
        &Controller::new(),
        &resumed,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert_eq!(items(&resumed).len(), 52);
    assert!(!fake.paths().iter().any(|url| url.contains("pageNum=1&")));
    assert!(
        with_store(&resumed, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
}

#[tokio::test]
async fn disabling_extension_during_fetch_prevents_commit() {
    let (dir, store) = setup(&[MAIN]);
    let gate = Arc::new(Gate {
        id: 1,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let mut fake = Fake::new(vec![vec![1]]);
    fake.gate = Some(gate.clone());
    let controller = Controller::new();
    let selected = sources(&store, &[MAIN]);
    let execute = controller.execute(
        &store,
        dir.path(),
        &fake,
        selected,
        "historical",
        Budget::default(),
    );
    let disable = async {
        gate.entered.notified().await;
        with_store(&store, |s| s.set_enabled("sogang_notices", false)).unwrap();
        gate.release.notify_one();
    };
    let (result, ()) = tokio::join!(execute, disable);
    assert!(result.is_err());
    assert!(items(&store).is_empty());
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "cancelled");
    assert!(source.subscribed);
    assert!(controller
        .start(
            store.clone(),
            dir.path().into(),
            network::Client::new().unwrap(),
            None,
            "historical".into()
        )
        .is_err());
}

#[tokio::test]
async fn downloaded_resources_and_states_survive_offline_reopen_and_detect_corruption() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1]]);
    fake.resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let item = items(&store)[0].clone();
    with_store(&store, |s| {
        s.set_document_state(item.id, Some(true), Some(true))
    })
    .unwrap();
    let detail = with_store(&store, |s| s.document(item.id, None)).unwrap();
    assert_eq!(detail.resources.len(), 2);
    assert!(detail.resources.iter().all(|r| r.status == "downloaded"));
    assert_eq!(detail.item.content_status, "archived");
    let bytes = with_store(&store, |s| s.archive_summary())
        .unwrap()
        .resource_bytes;
    assert_eq!(bytes, (png().len() + PDF.len()) as u64);
    let path = dir.path().join("rhythm.sqlite3");
    drop(fake);
    drop(store);
    let offline = Arc::new(Mutex::new(Store::open(&path).unwrap()));
    assert_eq!(
        with_store(&offline, |s| s.archive_query(&ArchiveQuery {
            query: "장학금".into(),
            ..Default::default()
        }))
        .unwrap()
        .total,
        1
    );
    let detail = with_store(&offline, |s| s.document(item.id, None)).unwrap();
    assert!(detail.item.is_read && detail.item.saved);
    assert!(detail.body_text.contains("AI 연구"));
    for resource in &detail.resources {
        let (_, bytes) = local_resource(&offline, dir.path(), resource.id)
            .await
            .unwrap();
        assert_eq!(
            bytes,
            if resource.kind == "image" {
                png()
            } else {
                PDF.to_vec()
            }
        );
    }
    let image = detail.resources.iter().find(|r| r.kind == "image").unwrap();
    std::fs::write(
        dir.path().join(image.local_path.as_ref().unwrap()),
        b"corrupted",
    )
    .unwrap();
    assert!(local_resource(&offline, dir.path(), image.id)
        .await
        .unwrap_err()
        .contains("무결성"));
}

#[tokio::test]
async fn failed_resource_downloads_preserve_text_and_can_be_retried() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1]]);
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail.body_text.contains("장학금"));
    assert_eq!(detail.item.content_status, "missing_resources");
    assert!(detail
        .resources
        .iter()
        .all(|resource| resource.status == "failed"));
    fake.missing_resources.store(false, Ordering::Release);
    let cancel = AtomicBool::new(false);
    let policy = Robots::load(&fake, &Url::parse(PNG_URL).unwrap(), &cancel)
        .await
        .unwrap();
    let mut total = 0;
    for resource in with_store(&store, |s| s.pending_resources(id)).unwrap() {
        download_resource(
            &store,
            dir.path(),
            &fake,
            &cancel,
            &policy,
            &resource,
            &mut total,
        )
        .await
        .unwrap();
    }
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.item.content_status, "archived");
    for resource in &detail.resources {
        assert!(local_resource(&store, dir.path(), resource.id)
            .await
            .is_ok());
    }
}

#[tokio::test]
async fn source_ids_isolate_reused_external_identities() {
    let (dir, store) = setup(&[MAIN, COMPUTING]);
    let fake = Fake::new(vec![vec![969]]);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN, COMPUTING],
        "historical",
        Budget {
            articles: Some(1),
            pages: Some(1),
        },
    )
    .await;
    let rows = items(&store);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].id, rows[1].id);
    assert_ne!(rows[0].source_id, rows[1].source_id);
}

#[tokio::test]
async fn robots_disallow_prevents_article_fetch() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1]]);
    *fake.robots.lock().unwrap() = "User-agent: *\nDisallow: /api/\n".into();
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert!(items(&store).is_empty());
    assert_eq!(fake.paths().len(), 1);
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "failed");
    assert!(source.last_error.unwrap().contains("robots.txt"));
    assert!(!source.history_complete);
}

#[tokio::test]
async fn bounded_budgets_never_claim_complete_history() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2], vec![3]]);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget {
            articles: Some(1),
            pages: None,
        },
    )
    .await;
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "limited");
    assert_eq!(source.next_page, 1);
    assert!(!source.history_complete);
    assert_eq!(items(&store).len(), 1);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget {
            pages: Some(1),
            articles: None,
        },
    )
    .await;
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "limited");
    assert_eq!(source.next_page, 2);
    assert!(!source.history_complete);
    assert_eq!(items(&store).len(), 2);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert_eq!(items(&store).len(), 3);
    assert!(
        with_store(&store, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
}

#[tokio::test]
async fn article_budget_does_not_mask_prior_article_failure() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2]]);
    fake.failed.lock().unwrap().insert(1);

    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget {
            articles: Some(1),
            pages: None,
        },
    )
    .await;

    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "failed");
    assert_eq!(source.failed, 1);
    assert_eq!(source.archived, 0);
    assert_eq!(source.next_page, 1);
    assert!(!source.history_complete);
    assert!(source
        .last_error
        .unwrap()
        .contains("모의 학교 서버 연결 실패"));
    assert!(items(&store).is_empty());
}

#[test]
fn single_job_admission_rejects_concurrent_reservations() {
    let controller = Controller::new();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let controller = controller.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                controller.reserve().is_ok()
            })
        })
        .collect();
    barrier.wait();
    let accepted = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter(|accepted| *accepted)
        .count();
    assert_eq!(accepted, 1);
    controller.finish("완료".into());
    assert!(controller.reserve().is_ok());
    controller.finish("완료".into());
}

#[tokio::test]
async fn incremental_discovers_a_burst_beyond_three_pages_without_using_pins_as_frontier() {
    let (dir, store) = setup(&[MAIN]);
    let controller = Controller::new();
    let fake = Fake::new(vec![vec![1, 2]]);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let mut pages = Vec::new();
    for page in 0..4 {
        let mut ids = vec![1];
        ids.extend((100 + page * 10)..(110 + page * 10));
        pages.push(ids);
    }
    pages.push(vec![1, 2]);
    *fake.pages.lock().unwrap() = pages;
    fake.calls.lock().unwrap().clear();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .documents,
        42
    );
    assert!(fake.paths().iter().any(|url| url.contains("pageNum=5")));
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "completed");
    assert!(source.history_complete);
}

#[tokio::test]
async fn first_incremental_seeds_three_pages_without_calling_old_records_new() {
    let (dir, store) = setup(&[MAIN]);
    let controller = Controller::new();
    let fake = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let summary = with_store(&store, |s| s.archive_summary()).unwrap();
    assert_eq!(summary.documents, 4);
    assert_eq!(summary.new_since_sync, 0);
    assert!(items(&store).iter().all(|item| item.historical));
    assert!(with_store(&store, |s| s.seed_complete(MAIN)).unwrap());
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert!(!source.history_complete);
    assert_eq!(source.next_page, 1);
    assert!(!fake.paths().iter().any(|url| url.contains("pageNum=4")));
}

#[tokio::test]
async fn interrupted_initial_seed_resumes_historical_and_keeps_later_items_eligible() {
    let (dir, store) = setup(&[MAIN]);
    let gate = Arc::new(Gate {
        id: 2,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let mut gated = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    gated.gate = Some(gate.clone());
    let gated = Arc::new(gated);
    let controller = Controller::new();
    let task = {
        let controller = controller.clone();
        let gated = gated.clone();
        let store = store.clone();
        let root = dir.path().to_owned();
        tokio::spawn(async move {
            controller
                .execute(
                    &store,
                    &root,
                    &*gated,
                    sources(&store, &[MAIN]),
                    "incremental",
                    Budget::default(),
                )
                .await
        })
    };
    gate.entered.notified().await;
    controller.cancel();
    gate.release.notify_one();
    assert!(task.await.unwrap().is_err());
    assert!(!with_store(&store, |s| s.seed_complete(MAIN)).unwrap());
    assert!(
        !with_store(&store, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
    assert_eq!(
        with_store(&store, |s| s.source(MAIN)).unwrap().status,
        "cancelled"
    );

    // The restart must keep treating the unfinished bootstrap as historical
    // even though the first interrupted run already stored some documents.
    let resumed = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &resumed,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert!(with_store(&store, |s| s.seed_complete(MAIN)).unwrap());
    assert!(
        !with_store(&store, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
    assert_eq!(items(&store).len(), 4);
    assert!(items(&store).iter().all(|item| item.historical));
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .new_since_sync,
        0
    );
    assert!(!resumed.paths().iter().any(|url| url.contains("pageNum=4")));

    // Only after the seed is durably complete may a genuinely new record be
    // reported as newly published.
    let later = Fake::new(vec![vec![1, 2, 6], vec![1, 3], vec![1, 4], vec![1, 5]]);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &later,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let fresh = items(&store)
        .into_iter()
        .find(|item| item.title.ends_with(" 6"))
        .unwrap();
    assert!(!fresh.historical);
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .new_since_sync,
        1
    );
}

#[tokio::test]
async fn failed_initial_seed_stays_pending_until_a_safe_completion() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    fake.failed.lock().unwrap().insert(3);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "failed");
    assert!(!source.history_complete);
    assert!(!with_store(&store, |s| s.seed_complete(MAIN)).unwrap());

    fake.failed.lock().unwrap().clear();
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    assert!(with_store(&store, |s| s.seed_complete(MAIN)).unwrap());
    assert!(
        !with_store(&store, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
    assert_eq!(items(&store).len(), 4);
    assert!(items(&store).iter().all(|item| item.historical));
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .new_since_sync,
        0
    );
}

#[tokio::test]
async fn historical_resume_completes_resources_after_text_commit_without_refetching_detail() {
    let (dir, store) = setup(&[MAIN]);
    let controller = Controller::new();
    let gate = Arc::new(Gate {
        id: 0,
        entered: Notify::new(),
        release: Notify::new(),
    });
    let mut fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    fake.resource_gate = Some(gate.clone());
    let fake = Arc::new(fake);
    let task = {
        let controller = controller.clone();
        let fake = fake.clone();
        let store = store.clone();
        let root = dir.path().to_owned();
        tokio::spawn(async move {
            controller
                .execute(
                    &store,
                    &root,
                    &*fake,
                    sources(&store, &[MAIN]),
                    "historical",
                    Budget::default(),
                )
                .await
        })
    };
    gate.entered.notified().await;
    assert_eq!(
        with_store(&store, |s| s.archive_summary())
            .unwrap()
            .documents,
        1
    );
    controller.cancel();
    gate.release.notify_one();
    assert!(task.await.unwrap().is_err());
    let id = items(&store)[0].id;
    assert!(!with_store(&store, |s| s.pending_resources(id))
        .unwrap()
        .is_empty());
    let resumed = Fake::new(vec![vec![2]]);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &resumed,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    assert!(with_store(&store, |s| s.pending_resources(id))
        .unwrap()
        .is_empty());
    assert!(!resumed.paths().iter().any(|url| url.contains("pkId=")));
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.item.content_status, "archived");
    for resource in detail.resources {
        assert!(local_resource(&store, dir.path(), resource.id)
            .await
            .is_ok());
    }
    assert!(
        with_store(&store, |s| s.source(MAIN))
            .unwrap()
            .history_complete
    );
}

#[tokio::test]
async fn resource_retry_keeps_good_offline_copy_and_repairs_corrupt_or_oversized_copy() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    let resource = detail
        .resources
        .into_iter()
        .find(|r| r.kind == "image")
        .unwrap();
    let policy = Robots::parse(&Url::parse(PNG_URL).unwrap(), "User-agent: *\nAllow: /\n").unwrap();
    fake.missing_resources.store(true, Ordering::Release);
    let error = download_resource(
        &store,
        dir.path(),
        &fake,
        &AtomicBool::new(false),
        &policy,
        &resource,
        &mut 0,
    )
    .await
    .unwrap()
    .into_result()
    .unwrap_err();
    assert!(error.contains("기존 로컬 자료"));
    assert_eq!(
        local_resource(&store, dir.path(), resource.id)
            .await
            .unwrap()
            .1,
        png()
    );
    assert_eq!(
        with_store(&store, |s| s.resource(resource.id))
            .unwrap()
            .local_path,
        resource.local_path
    );
    fake.missing_resources.store(false, Ordering::Release);
    let path = dir.path().join(resource.local_path.as_ref().unwrap());
    tokio::fs::write(&path, b"corrupt").await.unwrap();
    assert!(local_resource(&store, dir.path(), resource.id)
        .await
        .is_err());
    download_resource(
        &store,
        dir.path(),
        &fake,
        &AtomicBool::new(false),
        &policy,
        &resource,
        &mut 0,
    )
    .await
    .unwrap();
    assert_eq!(
        local_resource(&store, dir.path(), resource.id)
            .await
            .unwrap()
            .1,
        png()
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(FILE_LIMIT as u64 + 1)
        .unwrap();
    download_resource(
        &store,
        dir.path(),
        &fake,
        &AtomicBool::new(false),
        &policy,
        &resource,
        &mut 0,
    )
    .await
    .unwrap();
    assert_eq!(
        local_resource(&store, dir.path(), resource.id)
            .await
            .unwrap()
            .1,
        png()
    );
}

#[tokio::test]
#[ignore = "explicit bounded live university import; never run in ordinary CI"]
async fn bounded_live_import_and_offline_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let root = std::env::var_os("RHYTHM_LIVE_ARCHIVE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_path_buf());
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("rhythm.sqlite3");
    let db = Store::open(&path).unwrap();
    assert_eq!(
        db.archive_summary().unwrap().documents,
        0,
        "Use a fresh RHYTHM_LIVE_ARCHIVE_DIR for a bounded live import"
    );
    db.catalog(&catalog::definitions()).unwrap();
    db.set_enabled("sogang_notices", true).unwrap();
    let ids: Vec<_> = catalog::definitions()
        .into_iter()
        .filter(|s| s.supported)
        .map(|s| s.id)
        .collect();
    for id in &ids {
        db.subscribe(id, true).unwrap();
    }
    let store = Arc::new(Mutex::new(db));
    let selected = with_store(&store, |s| s.sources())
        .unwrap()
        .into_iter()
        .filter(|s| s.subscribed && s.definition.supported)
        .collect();
    let client = network::Client::new().unwrap();
    Controller::new()
        .execute(
            &store,
            &root,
            &client,
            selected,
            "historical",
            Budget {
                pages: Some(1),
                articles: Some(1),
            },
        )
        .await
        .unwrap();
    for source in with_store(&store, |s| s.sources())
        .unwrap()
        .into_iter()
        .filter(|s| s.subscribed)
    {
        eprintln!(
            "LIVE source={} archived={} status={} next_page={} complete={} error={:?}",
            source.definition.id,
            source.archived,
            source.status,
            source.next_page,
            source.history_complete,
            source.last_error
        );
        assert_ne!(
            source.status, "failed",
            "{}: {:?}",
            source.definition.id, source.last_error
        );
        assert!(!source.history_complete);
    }
    assert_eq!(
        items(&store).len(),
        ids.len(),
        "fresh bounded live archive must contain one article per supported source"
    );
    let first = items(&store)[0].id;
    with_store(&store, |s| {
        s.set_document_state(first, Some(true), Some(true))
    })
    .unwrap();
    drop(client);
    drop(store);
    let offline = Arc::new(Mutex::new(Store::open(&path).unwrap()));
    let mut downloaded = 0;
    let mut missing = 0;
    let mut local_bytes = 0;
    for item in items(&offline) {
        let detail = with_store(&offline, |s| s.document(item.id, None)).unwrap();
        let has_image = detail
            .resources
            .iter()
            .any(|resource| resource.kind == "image");
        assert!(!detail.body_html.contains("<iframe"));
        if detail.body_html.is_empty() && detail.body_text.is_empty() && !has_image {
            assert_eq!(item.source_id, MAIN);
            let original = with_store(&offline, |s| s.latest_original(item.id)).unwrap();
            let response: Value = serde_json::from_str(&original).unwrap();
            let content = response["data"]["content"].as_str().unwrap();
            assert!(content.contains("<iframe"));
            assert!(!content.contains("<script"));
            assert!(!content.contains("<img"));
        } else {
            assert!(
                !detail.body_text.is_empty() || has_image,
                "readable live notice must retain text or an image"
            );
        }
        let term = detail
            .item
            .title
            .split_whitespace()
            .next()
            .unwrap()
            .to_string();
        assert!(
            with_store(&offline, |s| s.archive_query(&ArchiveQuery {
                query: term,
                ..Default::default()
            }))
            .unwrap()
            .total
                > 0
        );
        for resource in &detail.resources {
            if resource.status == "downloaded" {
                let (_, bytes) = local_resource(&offline, &root, resource.id).await.unwrap();
                local_bytes += bytes.len();
                downloaded += 1;
            } else {
                missing += 1;
            }
        }
        eprintln!(
            "OFFLINE source={} document={} body_chars={} resources={} status={}",
            item.source_id,
            item.id,
            detail.body_text.chars().count(),
            detail.resources.len(),
            detail.item.content_status
        );
    }
    let saved = with_store(&offline, |s| s.document(first, None)).unwrap();
    assert!(saved.item.is_read && saved.item.saved);
    eprintln!("LIVE_IMPORT root={} documents={} downloaded_resources={} missing_resources={} local_resource_bytes={} historical_coverage=partial_only",root.display(),ids.len(),downloaded,missing,local_bytes);
}

#[tokio::test]
async fn conditional_recheck_resumes_pending_resources_without_new_revision() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    let before = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(
        before
            .resources
            .iter()
            .filter(|r| r.status != "downloaded")
            .count(),
        2
    );
    fake.missing_resources.store(false, Ordering::Release);
    fake.conditional.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "incremental",
        Budget::default(),
    )
    .await;
    let after = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(after.revisions.len(), before.revisions.len());
    assert!(with_store(&store, |s| s.pending_resources(id))
        .unwrap()
        .is_empty());
    for resource in after.resources {
        assert!(local_resource(&store, dir.path(), resource.id)
            .await
            .is_ok());
    }
}

#[tokio::test]
async fn failed_explicit_retry_is_not_success_but_article_stays_archived() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    let resource = with_store(&store, |s| s.document(id, None))
        .unwrap()
        .resources
        .remove(0);
    let policy = Robots::parse(
        &Url::parse(&resource.url).unwrap(),
        "User-agent: *\nAllow: /\n",
    )
    .unwrap();
    let outcome = download_resource(
        &store,
        dir.path(),
        &fake,
        &AtomicBool::new(false),
        &policy,
        &resource,
        &mut 0,
    )
    .await
    .unwrap();
    assert!(outcome.into_result().unwrap_err().contains("모의 첨부"));
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "completed");
    assert!(source.history_complete);
    assert_eq!(source.failed, 0);
    assert_eq!(items(&store).len(), 1);
}

#[tokio::test]
async fn resource_budget_counts_failed_disk_writes_and_caps_remaining_request() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let id = items(&store)[0].id;
    let resource = with_store(&store, |s| s.document(id, None))
        .unwrap()
        .resources
        .remove(0);
    fake.missing_resources.store(false, Ordering::Release);
    let bad_root = dir.path().join("not-a-directory");
    std::fs::write(&bad_root, b"blocking path").unwrap();
    let policy = Robots::parse(
        &Url::parse(&resource.url).unwrap(),
        "User-agent: *\nAllow: /\n",
    )
    .unwrap();
    let mut total = IMPORT_RESOURCES - 100;
    let outcome = download_resource(
        &store,
        &bad_root,
        &fake,
        &AtomicBool::new(false),
        &policy,
        &resource,
        &mut total,
    )
    .await
    .unwrap();
    assert!(outcome.into_result().is_err());
    assert!(total > IMPORT_RESOURCES - 100);
    assert_eq!(fake.limits.lock().unwrap().last().unwrap().1, 100);
    assert_eq!(
        with_store(&store, |s| s.resource(resource.id))
            .unwrap()
            .status,
        "failed"
    );
}

#[test]
fn robots_validates_delays_and_html_without_rejecting_empty_policy() {
    let url = Url::parse(PNG_URL).unwrap();
    for delay in ["NaN", "inf", "-2", "61"] {
        assert!(
            Robots::parse(&url, &format!("User-agent: *\nCrawl-delay: {delay}\n")).is_err(),
            "{delay}"
        );
    }
    assert!(Robots::parse(&url, "<!doctype html><html>error</html>").is_err());
    assert!(Robots::parse(&url, "\u{feff} <html>error</html>").is_err());
    assert!(Robots::parse(&url, "# valid empty policy\n")
        .unwrap()
        .permits(&url));
}

#[test]
fn robots_wildcards_end_anchors_queries_and_agent_precedence() {
    let origin = Url::parse("https://computing.sogang.ac.kr").unwrap();
    let policy = Robots::parse(&origin, "User-agent: *\nDisallow: /web/file/*a$\nDisallow: /web/file/private*\nAllow: /web/file/private/public\nDisallow: /query?*x=x$\n").unwrap();
    for path in [
        "/web/file/editor/aa",
        "/web/file/private/a",
        "/query?a=1&x=x",
    ] {
        assert!(!policy.permits(&origin.join(path).unwrap()), "{path}");
    }
    for path in [
        "/web/file/editor/ab",
        "/web/file/private/public/a",
        "/query?x=y",
    ] {
        assert!(policy.permits(&origin.join(path).unwrap()), "{path}");
    }
    let specific = Robots::parse(
        &origin,
        "User-agent: *\nDisallow: /\nUser-agent: Rhythm\nDisallow: /private\nAllow: /private\n",
    )
    .unwrap();
    assert!(specific.permits(&origin.join("/private").unwrap()));
    assert!(specific.permits(&origin.join("/other").unwrap()));
    assert!(!specific.permits(&Url::parse("https://www.sogang.ac.kr/other").unwrap()));
}

#[tokio::test]
async fn redirect_targets_are_checked_against_robots_before_requesting() {
    let fake = Fake::new(vec![]);
    let start = Url::parse(PNG_URL).unwrap();
    let blocked = "https://www.sogang.ac.kr/file-fe-prd/editor/blocked.png";
    fake.redirects
        .lock()
        .unwrap()
        .insert(start.to_string(), blocked.into());
    let policy = Robots::parse(
        &start,
        "User-agent: *\nDisallow: /file-fe-prd/editor/blocked.png\n",
    )
    .unwrap();
    let result = fetch(&fake, &start, None, 1024, &AtomicBool::new(false), &policy).await;
    assert!(result.unwrap_err().contains("robots.txt"));
    assert_eq!(fake.paths(), vec![start.to_string()]);
    fake.redirects.lock().unwrap().insert(
        start.to_string(),
        "https://computing.sogang.ac.kr/web/file/editor/next.png".into(),
    );
    assert!(
        fetch(&fake, &start, None, 1024, &AtomicBool::new(false), &policy)
            .await
            .unwrap_err()
            .contains("robots.txt")
    );
    assert_eq!(fake.paths().len(), 2);
    fake.redirects
        .lock()
        .unwrap()
        .insert(start.to_string(), start.to_string());
    assert!(
        fetch(&fake, &start, None, 1024, &AtomicBool::new(false), &policy)
            .await
            .unwrap_err()
            .contains("이동 경로")
    );
    assert_eq!(fake.paths().len(), 6);
}

#[tokio::test]
async fn robots_html_mime_and_redirect_are_not_an_allow_all_policy() {
    struct RobotsReply(bool);
    impl Transport for RobotsReply {
        async fn fetch(
            &self,
            url: &Url,
            _: Option<(Option<String>, Option<String>)>,
            _: usize,
            _: &AtomicBool,
        ) -> Result<network::Fetch, String> {
            if self.0 {
                Ok(network::Fetch::Body {
                    bytes: b"User-agent: *\nAllow: /\n".to_vec(),
                    mime: Some("text/html".into()),
                    etag: None,
                    last_modified: None,
                })
            } else {
                Ok(network::Fetch::Redirect(
                    url.join("/file-fe-prd/editor/robots.txt").unwrap(),
                ))
            }
        }
    }
    for reply in [RobotsReply(true), RobotsReply(false)] {
        assert!(Robots::load(
            &reply,
            &Url::parse(PNG_URL).unwrap(),
            &AtomicBool::new(false)
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn allowed_same_origin_redirect_reapplies_policy_and_preserves_validator() {
    let fake = Fake::new(vec![]);
    let start = Url::parse("https://www.sogang.ac.kr/file-fe-prd/editor/previous.png").unwrap();
    fake.redirects
        .lock()
        .unwrap()
        .insert(start.to_string(), PNG_URL.into());
    let policy = Robots::parse(&start, "User-agent: *\nAllow: /\n").unwrap();
    let validator = Some((Some("\"test\"".into()), None));
    let response = fetch(
        &fake,
        &start,
        validator.clone(),
        1024,
        &AtomicBool::new(false),
        &policy,
    )
    .await
    .unwrap();
    let network::Fetch::Body { bytes, .. } = response else {
        panic!("expected image")
    };
    assert_eq!(bytes, png());
    assert_eq!(
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v)| v)
            .collect::<Vec<_>>(),
        vec![&validator, &validator]
    );
}

#[tokio::test]
async fn historical_resume_counters_describe_only_current_attempt() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2, 3]]);
    fake.failed.lock().unwrap().insert(3);
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let first = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!((first.discovered, first.archived, first.failed), (2, 1, 1));
    fake.failed.lock().unwrap().clear();
    run(
        &Controller::new(),
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "historical",
        Budget::default(),
    )
    .await;
    let second = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(
        (second.discovered, second.archived, second.failed),
        (1, 1, 0)
    );
    assert_eq!(items(&store).len(), 2);
}
