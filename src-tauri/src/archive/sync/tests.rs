use super::*;
use crate::archive::catalog;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;

const MAIN: &str = "sogang-academic";
const PNG_URL: &str = "https://www.sogang.ac.kr/file-fe-prd/editor/archive-fixture.png";
const PDF_URL: &str =
    "https://www.sogang.ac.kr/file-fe-prd/board/2/archive-fixture.pdf?sg=archive.pdf";
const PDF: &[u8] = b"%PDF-1.4\nlocal archive fixture\n%%EOF\n";

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U8AAAAASUVORK5CYII=").unwrap()
}
type RecordedRequest = (String, Option<(Option<String>, Option<String>)>);
/// Body bytes and MIME type, or `None` for a failed connection.
type ExtraReply = Option<(Vec<u8>, String)>;

/// The school as the app sees it when a feed notice is opened: main-board
/// notice pages, their robots.txt, image and attachment.
struct Fake {
    changed: Mutex<HashMap<u64, String>>,
    failed: Mutex<HashSet<u64>>,
    attachment: Mutex<String>,
    calls: Mutex<Vec<RecordedRequest>>,
    accesses: Mutex<Vec<(String, network::Access)>>,
    /// Extra responses by exact URL.
    extra: Mutex<HashMap<String, ExtraReply>>,
    resources: AtomicBool,
    missing_resources: AtomicBool,
    robots: Mutex<String>,
    redirects: Mutex<HashMap<String, String>>,
    limits: Mutex<Vec<(String, usize)>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            changed: Mutex::new(HashMap::new()),
            failed: Mutex::new(HashSet::new()),
            attachment: Mutex::new(PDF_URL.into()),
            calls: Mutex::new(Vec::new()),
            accesses: Mutex::new(Vec::new()),
            extra: Mutex::new(HashMap::new()),
            resources: AtomicBool::new(false),
            missing_resources: AtomicBool::new(false),
            robots: Mutex::new("User-agent: *\nAllow: /\n".into()),
            redirects: Mutex::new(HashMap::new()),
            limits: Mutex::new(Vec::new()),
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
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../../crates/sources/src/fixtures/main-detail.json"
        ))
        .unwrap();
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
        if self.resources.load(Ordering::Acquire) {
            data["content"] = json!(format!(
                "{}<p><img src='{PNG_URL}' alt='안내 이미지'></p>",
                data["content"].as_str().unwrap()
            ));
            data["fileValue1"] = json!(*self.attachment.lock().unwrap());
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
        access: network::Access,
    ) -> Result<network::Fetch, network::FetchError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), validators.clone()));
        self.accesses
            .lock()
            .unwrap()
            .push((url.to_string(), access));
        self.limits.lock().unwrap().push((url.to_string(), limit));
        if let Some(target) = self.redirects.lock().unwrap().get(url.as_str()) {
            return Ok(network::Fetch::Redirect(Url::parse(target).unwrap()));
        }
        if let Some(reply) = self.extra.lock().unwrap().get(url.as_str()).cloned() {
            return match reply {
                Some((bytes, mime)) => Ok(network::Fetch::Body {
                    bytes,
                    mime: Some(mime),
                    etag: None,
                    last_modified: None,
                }),
                None => Err("모의 외부 서버 연결 실패".into()),
            };
        }
        let body = |bytes: Vec<u8>, mime: &str| {
            Ok(network::Fetch::Body {
                bytes,
                mime: Some(mime.into()),
                etag: None,
                last_modified: None,
            })
        };
        if url.path() == "/robots.txt" {
            return body(
                self.robots.lock().unwrap().as_bytes().to_vec(),
                "text/plain",
            );
        }
        if url.path() == "/api/api/v1/mainKo/BbsData" {
            let id = url
                .query_pairs()
                .find(|(key, _)| key == "pkId")
                .unwrap()
                .1
                .parse::<u64>()
                .unwrap();
            if self.failed.lock().unwrap().contains(&id) {
                return Err("모의 학교 서버 연결 실패".into());
            }
            return body(self.detail(id).into_bytes(), "application/json");
        }
        if self.missing_resources.load(Ordering::Acquire)
            && (url.as_str() == PNG_URL || url.as_str() == PDF_URL)
        {
            return Err("모의 첨부 파일 연결 실패".into());
        }
        if url.as_str() == PNG_URL {
            return body(png(), "image/png");
        }
        if url.as_str() == PDF_URL || url.path().ends_with(".pdf") {
            return body(PDF.to_vec(), "application/pdf");
        }
        Err(format!("예상하지 않은 모의 요청: {}", url.path()).into())
    }
}
fn setup() -> (TempDir, SharedStore) {
    let (dir, store) = crate::storage::temp_store();
    store.catalog(&catalog::definitions()).unwrap();
    store.set_enabled("sogang_notices", true).unwrap();
    store.subscribe(MAIN, true).unwrap();
    (dir, Arc::new(Mutex::new(store)))
}

/// Mirrors main-board notice `id` from the feed as the collector saw `page`
/// (the fake's detail response), with only its excerpt archived.
fn mirror(store: &SharedStore, seq: u64, id: u64, page: &str) -> i64 {
    use rhythm_feed::record::{BodyState, Entry, Notice, Record, Resource, ResourceKind};
    let external = id.to_string();
    let definition = with_store(store, |s| s.source(MAIN)).unwrap().definition;
    let entry = ListingEntry {
        external_id: external.clone(),
        url: parser::main_canonical(&parser::MAIN_BOARDS[0], &external).to_string(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let article = parser::article(&definition, &entry, page).unwrap();
    let notice = Notice {
        source_id: MAIN.into(),
        external_id: external.clone(),
        revision_hash: crate::storage::feed::article_revision_hash(&article),
        canonical_url: article.canonical_url.clone(),
        title: article.title.clone(),
        author: article.author.clone(),
        category: article.category.clone(),
        published_at: article.published_at.clone(),
        modified_at: None,
        body_state: if article.unsupported_body {
            BodyState::Unsupported
        } else {
            BodyState::Normal
        },
        excerpt: rhythm_feed::excerpt(&article.body_text),
        body_hash: Some(rhythm_feed::body_hash(&article.body_text)),
        resources: article
            .resources
            .iter()
            .map(|r| Resource {
                url: r.url.clone(),
                filename: r.filename.clone(),
                kind: if r.kind == "image" {
                    ResourceKind::Image
                } else {
                    ResourceKind::Attachment
                },
            })
            .collect(),
        dates: vec![],
        dates_version: 1,
        historical: false,
        first_seen_at: "2026-11-18T01:00:00.000Z".into(),
        revised_at: "2026-11-18T01:00:00.000Z".into(),
    };
    let records = [Entry {
        seq,
        record: Some(Record::Notice(Box::new(notice))),
    }];
    with_store(store, |s| match s.feed_cursor()? {
        None => s.feed_apply_snapshot(seq, &records, &[]),
        Some(_) => s.feed_apply_segment(seq, seq, &records, &[], false, &Default::default()),
    })
    .unwrap();
    with_store(store, |s| s.document_id(MAIN, &external))
        .unwrap()
        .unwrap()
}
fn detail_requests(fake: &Fake) -> usize {
    fake.paths()
        .iter()
        .filter(|url| url.contains("/BbsData"))
        .count()
}
/// Mirrors main-board notice `id` from the feed and opens it: its body and
/// files come from the school as the reader fetches them.
async fn opened(store: &SharedStore, root: &Path, fake: &Fake, id: u64) -> i64 {
    let document = mirror(store, 1, id, &fake.detail(id));
    let bodies = body::Bodies::default();
    bodies.fetch(store, root, fake, document).await.unwrap();
    bodies.resources(store, root, fake, document).await.unwrap();
    document
}

#[tokio::test]
async fn downloaded_resources_and_states_survive_offline_reopen_and_detect_corruption() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let id = opened(&store, dir.path(), &fake, 1).await;
    with_store(&store, |s| s.set_document_state(id, Some(true), Some(true))).unwrap();
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
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
    let detail = with_store(&offline, |s| s.document(id, None)).unwrap();
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
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    let id = opened(&store, dir.path(), &fake, 1).await;
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail.body_text.contains("장학금"));
    assert_eq!(detail.item.content_status, "missing_resources");
    assert!(detail
        .resources
        .iter()
        .all(|resource| resource.status == "failed"));
    // A failed explicit retry is reported, and the notice stays archived.
    let resource = detail.resources[0].clone();
    let cancel = AtomicBool::new(false);
    let policy = Robots::load(
        &fake,
        &Url::parse(PNG_URL).unwrap(),
        &cancel,
        network::Access::Source,
    )
    .await
    .unwrap();
    let outcome = download_resource(
        &store,
        dir.path(),
        &fake,
        &cancel,
        &policy,
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0),
    )
    .await
    .unwrap();
    assert!(outcome.into_result().unwrap_err().contains("모의 첨부"));
    assert!(with_store(&store, |s| s.document(id, None))
        .unwrap()
        .body_text
        .contains("장학금"));

    fake.missing_resources.store(false, Ordering::Release);
    let total = AtomicU64::new(0);
    for resource in with_store(&store, |s| s.pending_resources(id)).unwrap() {
        download_resource(
            &store,
            dir.path(),
            &fake,
            &cancel,
            &policy,
            &RobotsCache::default(),
            &resource,
            &total,
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
async fn resource_retry_keeps_good_offline_copy_and_repairs_corrupt_or_oversized_copy() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let id = opened(&store, dir.path(), &fake, 2).await;
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
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0),
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
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0),
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
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0),
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

#[cfg(windows)]
#[tokio::test]
async fn unicode_data_path_recovers_a_temporarily_locked_resource() {
    use std::os::windows::fs::OpenOptionsExt;

    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("자료 보관 (한글)");
    std::fs::create_dir(&root).unwrap();
    let database = root.join("rhythm.sqlite3");
    let db = Store::open(&database).unwrap();
    db.catalog(&catalog::definitions()).unwrap();
    db.set_enabled("sogang_notices", true).unwrap();
    let store = Arc::new(Mutex::new(db));
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let id = opened(&store, &root, &fake, 2).await;
    let resource = with_store(&store, |s| s.document(id, None))
        .unwrap()
        .resources
        .into_iter()
        .find(|r| r.kind == "image")
        .unwrap();
    let path = root.join(resource.local_path.as_ref().unwrap());
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let policy = Robots::parse(&Url::parse(PNG_URL).unwrap(), "User-agent: *\nAllow: /\n").unwrap();
    assert!(download_resource(
        &store,
        &root,
        &fake,
        &AtomicBool::new(false),
        &policy,
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0)
    )
    .await
    .unwrap()
    .into_result()
    .is_err());
    drop(lock);
    assert_eq!(std::fs::read(&path).unwrap(), png());
    let failed = with_store(&store, |s| s.resource(resource.id)).unwrap();
    assert_eq!(failed.status, "failed");
    download_resource(
        &store,
        &root,
        &fake,
        &AtomicBool::new(false),
        &policy,
        &RobotsCache::default(),
        &failed,
        &AtomicU64::new(0),
    )
    .await
    .unwrap()
    .into_result()
    .unwrap();
    drop(store);
    let reopened = Arc::new(Mutex::new(Store::open(&database).unwrap()));
    assert_eq!(
        local_resource(&reopened, &root, resource.id)
            .await
            .unwrap()
            .1,
        png()
    );
}

#[tokio::test]
async fn resource_budget_counts_failed_disk_writes_and_caps_remaining_request() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    let id = opened(&store, dir.path(), &fake, 2).await;
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
    let total = AtomicU64::new(IMPORT_RESOURCES - 100);
    let outcome = download_resource(
        &store,
        &bad_root,
        &fake,
        &AtomicBool::new(false),
        &policy,
        &RobotsCache::default(),
        &resource,
        &total,
    )
    .await
    .unwrap();
    assert!(outcome.into_result().is_err());
    assert!(total.into_inner() > IMPORT_RESOURCES - 100);
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
fn robots_ignores_an_unreadable_delay_of_a_group_for_another_agent() {
    let url = Url::parse(PNG_URL).unwrap();
    let policy = Robots::parse(
        &url,
        "User-agent: Yeti\nCrawl-delay: 10s\n\nUser-agent: Other\nCrawl-delay: 120\n\nUser-agent: *\nCrawl-delay: 2\nDisallow: /private\n",
    )
    .unwrap();
    assert_eq!(policy.delay, 2.0);
    assert!(policy.permits(&url));
    assert!(!policy.permits(&url.join("/private").unwrap()));
    assert!(Robots::parse(&url, "User-agent: Rhythm\nCrawl-delay: 10s\n").is_err());
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
    let fake = Fake::new();
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
    let result = fetch(
        &fake,
        &start,
        None,
        1024,
        &AtomicBool::new(false),
        &policy,
        network::Access::Source,
    )
    .await;
    assert!(result.unwrap_err().contains("robots.txt"));
    assert_eq!(fake.paths(), vec![start.to_string()]);
    fake.redirects.lock().unwrap().insert(
        start.to_string(),
        "https://computing.sogang.ac.kr/web/file/editor/next.png".into(),
    );
    assert!(fetch(
        &fake,
        &start,
        None,
        1024,
        &AtomicBool::new(false),
        &policy,
        network::Access::Source
    )
    .await
    .unwrap_err()
    .contains("robots.txt"));
    assert_eq!(fake.paths().len(), 2);
    fake.redirects
        .lock()
        .unwrap()
        .insert(start.to_string(), start.to_string());
    assert!(fetch(
        &fake,
        &start,
        None,
        1024,
        &AtomicBool::new(false),
        &policy,
        network::Access::Source
    )
    .await
    .unwrap_err()
    .contains("이동 경로"));
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
            _: network::Access,
        ) -> Result<network::Fetch, network::FetchError> {
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
            &AtomicBool::new(false),
            network::Access::Source
        )
        .await
        .is_err());
    }
}

#[tokio::test]
async fn allowed_same_origin_redirect_reapplies_policy_and_preserves_validator() {
    let fake = Fake::new();
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
        network::Access::Source,
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

const CDN_IMAGE: &str = "https://cdn.example.com/notice/a.png";
const CDN_OTHER: &str = "https://cdn.example.com/notice/b";

fn third_party_fixture(robots: &str) -> (TempDir, SharedStore, Fake) {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.changed.lock().unwrap().insert(
        1,
        format!("<p>본문</p><img src='{CDN_IMAGE}'><img src='{CDN_OTHER}'>"),
    );
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        "https://cdn.example.com/robots.txt".into(),
        Some((robots.as_bytes().to_vec(), "text/plain".into())),
    );
    extra.insert(CDN_IMAGE.into(), Some((png(), "image/png".into())));
    extra.insert(CDN_OTHER.into(), Some((png(), "image/png".into())));
    drop(extra);
    (dir, store, fake)
}

#[tokio::test]
async fn third_party_images_download_under_their_hosts_robots_policy() {
    let (dir, store, fake) = third_party_fixture("User-agent: *\nAllow: /\n");
    let id = opened(&store, dir.path(), &fake, 1).await;
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.resources.len(), 2);
    assert!(detail.resources.iter().all(|r| r.status == "downloaded"));
    let accesses = fake.accesses.lock().unwrap().clone();
    let access = |url: &str| {
        accesses
            .iter()
            .filter(|(u, _)| u == url)
            .map(|(_, a)| *a)
            .collect::<Vec<_>>()
    };
    assert_eq!(access(CDN_IMAGE), [network::Access::EmbeddedImage]);
    // One robots.txt request per host per opened notice, not per image.
    assert_eq!(
        access("https://cdn.example.com/robots.txt"),
        [network::Access::EmbeddedImage]
    );
    assert!(accesses
        .iter()
        .filter(|(u, _)| u.contains("BbsData"))
        .all(|(_, a)| *a == network::Access::Source));

    let (dir, store, fake) = third_party_fixture("User-agent: *\nDisallow: /notice/\n");
    let id = opened(&store, dir.path(), &fake, 1).await;
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail.resources.iter().all(|r| r.status == "failed"));
    assert!(
        !fake.paths().iter().any(|u| u == CDN_IMAGE),
        "robots denial sends no request"
    );
}

#[tokio::test]
async fn exhausted_budget_leaves_files_pending_instead_of_too_large() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
    let id = opened(&store, dir.path(), &fake, 1).await;
    fake.missing_resources.store(false, Ordering::Release);
    let policy = Robots::parse(&Url::parse(PNG_URL).unwrap(), "User-agent: *\nAllow: /\n").unwrap();
    for total in [IMPORT_RESOURCES - 10, IMPORT_RESOURCES] {
        for resource in with_store(&store, |s| s.pending_resources(id)).unwrap() {
            let outcome = download_resource(
                &store,
                dir.path(),
                &fake,
                &AtomicBool::new(false),
                &policy,
                &RobotsCache::default(),
                &resource,
                &AtomicU64::new(total),
            )
            .await
            .unwrap();
            assert_eq!(outcome.into_result().unwrap_err(), BUDGET_REACHED);
            let after = with_store(&store, |s| s.resource(resource.id)).unwrap();
            assert_eq!(
                (after.status.as_str(), &after.error),
                ("failed", &resource.error)
            );
        }
    }
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

const VIEWER_PDF: &str =
    "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=%EC%95%88%EB%82%B4.pdf&gubun=cmsfile";
const VIEWER_SCAN: &str =
    "https://scc.sogang.ac.kr/Download3?pathStr=MTAy&fileName=scan.pdf&gubun=cmsfile";

fn viewer_iframe(file: &str) -> String {
    let file: String = url::form_urlencoded::byte_serialize(file.as_bytes()).collect();
    format!(
        "<div class=\"iframe-wrapper\"><iframe src=\"https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={file}\"></iframe></div>"
    )
}
/// `https://scc.sogang.ac.kr/robots.txt` as served (text/plain) on 2026-11-08:
/// unrendered ASP source choosing rules per host name.
const SCC_ROBOTS: &str = "<%\r\nResponse.ContentType=\"text/plain\";\r\nif(Request.ServerVariables(\"HTTP_HOST\") == \"nbsm.sogang.ac.kr\"){\r\n%>\r\nUser-agent: *\r\nallow:/\r\n<%\r\n}else{\r\n%>\r\n\r\nUser-agent: *\r\nDisallow:/cgsi/\r\n\r\n<%\r\n<%\r\n}\r\n%>\r\n";

#[tokio::test]
async fn asp_source_robots_applies_only_to_its_host_and_html_still_fails() {
    for (host, body, mime, ok) in [
        ("scc.sogang.ac.kr", SCC_ROBOTS, "text/plain", true),
        (
            "scc.sogang.ac.kr",
            "<html>not available</html>",
            "text/html",
            false,
        ),
        ("www.sogang.ac.kr", SCC_ROBOTS, "text/plain", false),
    ] {
        let fake = Fake::new();
        fake.extra.lock().unwrap().insert(
            format!("https://{host}/robots.txt"),
            Some((body.as_bytes().to_vec(), mime.into())),
        );
        let url = Url::parse(&format!("https://{host}/robots.txt")).unwrap();
        let loaded = Robots::load(
            &fake,
            &url,
            &AtomicBool::new(false),
            network::Access::Source,
        )
        .await;
        assert_eq!(loaded.is_ok(), ok, "{host} {mime}");
    }
}
/// A board whose notices show viewer PDFs from the school's file host.
fn viewer_fake(ids: &[(u64, &str)]) -> Fake {
    let fake = Fake::new();
    for (id, file) in ids {
        fake.changed
            .lock()
            .unwrap()
            .insert(*id, viewer_iframe(file));
    }
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        "https://scc.sogang.ac.kr/robots.txt".into(),
        Some((SCC_ROBOTS.as_bytes().to_vec(), "text/plain".into())),
    );
    extra.insert(
        VIEWER_PDF.into(),
        Some((
            include_bytes!("../../../../crates/sources/src/fixtures/main-pdf-551680.pdf").to_vec(),
            "application/octet-stream".into(),
        )),
    );
    extra.insert(
        VIEWER_SCAN.into(),
        Some((PDF.to_vec(), "application/octet-stream".into())),
    );
    drop(extra);
    fake
}
fn requests(fake: &Fake, url: &str) -> usize {
    fake.paths().iter().filter(|u| *u == url).count()
}
fn document_by_external(store: &SharedStore, external: &str) -> DocumentDetail {
    let id = with_store(store, |s| s.document_id(MAIN, external))
        .unwrap()
        .unwrap();
    with_store(store, |s| s.document(id, None)).unwrap()
}

#[tokio::test]
async fn a_retried_viewer_pdf_becomes_the_body_of_a_metadata_only_notice() {
    let (dir, store) = setup();
    let fake = viewer_fake(&[(551680, VIEWER_PDF)]);
    let source = with_store(&store, |s| s.source(MAIN)).unwrap().definition;
    // Archived by an earlier version that did not recognize the viewer PDF.
    let entry = ListingEntry {
        external_id: "551680".into(),
        url: parser::main_canonical(&parser::MAIN_BOARDS[0], "551680").to_string(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let mut article = parser::article(&source, &entry, &fake.detail(551680)).unwrap();
    article.resources.clear();
    with_store(&store, |s| s.upsert_article(MAIN, &article, true)).unwrap();
    let id = document_by_external(&store, "551680").item.id;
    let before = with_store(&store, |s| s.latest_original(id)).unwrap();
    let cancel = AtomicBool::new(false);

    // Without the PDF here the notice only gains the reference.
    fill_pdf_body(&store, dir.path(), &cancel, id)
        .await
        .unwrap();
    let detail = document_by_external(&store, "551680");
    assert!(detail.body_text.is_empty());
    let pdf = detail
        .resources
        .iter()
        .find(|r| r.url == VIEWER_PDF)
        .unwrap()
        .clone();
    assert_eq!(pdf.status, "pending");

    let policy = Robots::load(
        &fake,
        &Url::parse(VIEWER_PDF).unwrap(),
        &cancel,
        network::Access::Source,
    )
    .await
    .unwrap();
    download_resource(
        &store,
        dir.path(),
        &fake,
        &cancel,
        &policy,
        &RobotsCache::default(),
        &pdf,
        &AtomicU64::new(0),
    )
    .await
    .unwrap()
    .into_result()
    .unwrap();
    fill_pdf_body(&store, dir.path(), &cancel, id)
        .await
        .unwrap();
    let filled = document_by_external(&store, "551680");
    assert!(filled.body_text.contains("동계계절수업"));
    assert_eq!(
        filled.revisions.len(),
        1,
        "no revision, freshness or change notice"
    );
    assert_eq!(
        with_store(&store, |s| s.latest_original(id)).unwrap(),
        before
    );
    assert_eq!(requests(&fake, VIEWER_PDF), 1);
}

#[tokio::test]
async fn unsupported_attachment_is_recorded_without_requesting_it() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let exe = "https://www.sogang.ac.kr/file-fe-prd/board/2/setup.exe?sg=setup.exe";
    *fake.attachment.lock().unwrap() = exe.into();
    let id = opened(&store, dir.path(), &fake, 2).await;
    assert_eq!(requests(&fake, exe), 0, "{:?}", fake.paths());
    let resources = with_store(&store, |s| s.pending_resources(id)).unwrap();
    let attachment = resources.iter().find(|r| r.url == exe).unwrap();
    assert_eq!(attachment.status, "unsupported");
}

#[tokio::test]
async fn opening_a_feed_notice_fills_its_body_from_the_school() {
    let (dir, store) = setup();
    let fake = Fake::new();
    let bodies = body::Bodies::default();
    let same = mirror(&store, 1, 551700, &fake.detail(551700));
    let before = with_store(&store, |s| s.document(same, None)).unwrap();
    assert!(before.summary_only);

    let revised = bodies.fetch(&store, dir.path(), &fake, same).await.unwrap();
    assert!(!revised, "the school still shows the feed's revision");
    let detail = with_store(&store, |s| s.document(same, None)).unwrap();
    assert!(!detail.summary_only);
    assert!(detail.body_html.contains("<h2>보관 자료 551700</h2>"));
    assert_eq!(detail.revisions.len(), 1);
    // Archive routes and robots.txt as the crawler uses them.
    assert!(fake
        .accesses
        .lock()
        .unwrap()
        .iter()
        .all(|(_, access)| *access == network::Access::Source));
    // Opened again: the body is here, so nothing is requested.
    assert!(!bodies.fetch(&store, dir.path(), &fake, same).await.unwrap());
    assert_eq!(detail_requests(&fake), 1);

    // The school changed another notice after the collector saw it.
    let changed = mirror(&store, 2, 551701, &fake.detail(551701));
    fake.changed
        .lock()
        .unwrap()
        .insert(551701, "<p>수정된 안내입니다.</p>".into());
    assert!(bodies
        .fetch(&store, dir.path(), &fake, changed)
        .await
        .unwrap());
    let detail = with_store(&store, |s| s.document(changed, None)).unwrap();
    assert_eq!(detail.body_text, "수정된 안내입니다.");
    assert_eq!(detail.revisions.len(), 1);
    assert_eq!(
        fake.paths()
            .iter()
            .filter(|url| url.ends_with("/robots.txt"))
            .count(),
        1,
        "robots.txt is reused between opens"
    );
}

#[tokio::test]
async fn a_failed_body_fetch_keeps_the_excerpt_until_a_retry() {
    let (dir, store) = setup();
    let fake = Fake::new();
    let bodies = body::Bodies::default();
    let id = mirror(&store, 1, 551702, &fake.detail(551702));
    let excerpt = with_store(&store, |s| s.document(id, None))
        .unwrap()
        .body_text;
    fake.failed.lock().unwrap().insert(551702);
    assert!(bodies.fetch(&store, dir.path(), &fake, id).await.is_err());
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail.summary_only);
    assert_eq!(detail.body_text, excerpt);

    // With the notices extension off nothing is requested.
    with_store(&store, |s| s.set_enabled("sogang_notices", false)).unwrap();
    fake.failed.lock().unwrap().clear();
    let requests = detail_requests(&fake);
    assert!(bodies.fetch(&store, dir.path(), &fake, id).await.is_err());
    assert_eq!(detail_requests(&fake), requests);

    with_store(&store, |s| s.set_enabled("sogang_notices", true)).unwrap();
    assert!(!bodies.fetch(&store, dir.path(), &fake, id).await.unwrap());
    assert!(
        !with_store(&store, |s| s.document(id, None))
            .unwrap()
            .summary_only
    );
}

#[tokio::test]
async fn opening_a_feed_viewer_notice_reads_its_pdf() {
    let (dir, store) = setup();
    let fake = viewer_fake(&[(551680, VIEWER_PDF)]);
    let bodies = body::Bodies::default();
    let id = mirror(&store, 1, 551680, &fake.detail(551680));
    assert!(!bodies.fetch(&store, dir.path(), &fake, id).await.unwrap());
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail
        .body_text
        .contains("2026학년도 동계계절수업 수강신청 안내"));
    assert_eq!(detail.revisions.len(), 1);
    let pdf = detail
        .resources
        .iter()
        .find(|r| r.url == VIEWER_PDF)
        .unwrap();
    assert_eq!(pdf.status, "downloaded");
    // The body keeps the feed's hash, so the feed's next record of that
    // revision leaves the PDF text in place.
    mirror(&store, 2, 551680, &fake.detail(551680));
    assert_eq!(
        with_store(&store, |s| s.document(id, None))
            .unwrap()
            .body_text,
        detail.body_text
    );
    assert!(fake
        .paths()
        .contains(&"https://scc.sogang.ac.kr/robots.txt".to_string()));
}

fn resource_requests(fake: &Fake) -> usize {
    fake.paths()
        .iter()
        .filter(|url| *url == PNG_URL || *url == PDF_URL)
        .count()
}

#[tokio::test]
async fn opening_a_feed_notice_downloads_its_files_once_the_body_is_here() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let bodies = body::Bodies::default();
    let id = mirror(&store, 1, 551703, &fake.detail(551703));

    // Before the body the feed's links may still change, so nothing is fetched.
    bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .unwrap();
    assert_eq!(resource_requests(&fake), 0);

    bodies.fetch(&store, dir.path(), &fake, id).await.unwrap();
    bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .unwrap();
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.resources.len(), 2);
    assert!(detail.resources.iter().all(|r| r.status == "downloaded"));
    let png = detail.resources.iter().find(|r| r.url == PNG_URL).unwrap();
    assert!(local_resource(&store, dir.path(), png.id).await.is_ok());
    let accesses = fake.accesses.lock().unwrap().clone();
    assert!(accesses
        .iter()
        .any(|(url, access)| url == PNG_URL && *access == network::Access::EmbeddedImage));
    assert!(accesses
        .iter()
        .any(|(url, access)| url == PDF_URL && *access == network::Access::Source));

    // Opened again: every file is here.
    bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .unwrap();
    assert_eq!(resource_requests(&fake), 2);
}

#[tokio::test]
async fn a_failed_file_waits_for_an_explicit_retry() {
    let (dir, store) = setup();
    let fake = Fake::new();
    fake.resources.store(true, Ordering::Release);
    let bodies = body::Bodies::default();
    let id = mirror(&store, 1, 551704, &fake.detail(551704));
    bodies.fetch(&store, dir.path(), &fake, id).await.unwrap();

    // With the notices extension off nothing is requested.
    with_store(&store, |s| s.set_enabled("sogang_notices", false)).unwrap();
    assert!(bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .is_err());
    assert_eq!(resource_requests(&fake), 0);
    with_store(&store, |s| s.set_enabled("sogang_notices", true)).unwrap();

    fake.missing_resources.store(true, Ordering::Release);
    bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .unwrap();
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert!(detail.resources.iter().all(|r| r.status == "failed"));
    assert_eq!(resource_requests(&fake), 2);

    // Opening the notice again does not repeat the failed requests.
    fake.missing_resources.store(false, Ordering::Release);
    bodies
        .resources(&store, dir.path(), &fake, id)
        .await
        .unwrap();
    assert_eq!(resource_requests(&fake), 2);
}
