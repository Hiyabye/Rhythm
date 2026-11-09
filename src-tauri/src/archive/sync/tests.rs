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
/// Body bytes and MIME type, or `None` for a failed connection.
type ExtraReply = Option<(Vec<u8>, String)>;

struct Fake {
    pages: Mutex<Vec<Vec<u64>>>,
    changed: Mutex<HashMap<u64, String>>,
    bad: Mutex<HashSet<u64>>,
    failed: Mutex<HashSet<u64>>,
    rate_limited: Mutex<HashSet<u64>>,
    attachment: Mutex<String>,
    calls: Mutex<Vec<RecordedRequest>>,
    accesses: Mutex<Vec<(String, network::Access)>>,
    /// Extra responses by exact URL.
    extra: Mutex<HashMap<String, ExtraReply>>,
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
            rate_limited: Mutex::new(HashSet::new()),
            attachment: Mutex::new(PDF_URL.into()),
            calls: Mutex::new(Vec::new()),
            accesses: Mutex::new(Vec::new()),
            extra: Mutex::new(HashMap::new()),
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
    ) -> Result<network::Fetch, String> {
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
            if self.rate_limited.lock().unwrap().contains(&id) {
                return Err(network::RATE_LIMITED.into());
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

const MECHANICAL_SITEMAP: &str = "https://me.sogang.ac.kr/sitemap.xml";
const MECHANICAL_DETAIL: &str = "https://me.sogang.ac.kr/ko/board/notice/1";
fn mechanical_sitemap(lastmod: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\
         <url><loc>{MECHANICAL_DETAIL}</loc><lastmod>{lastmod}</lastmod></url>\
         </urlset>"
    )
}
fn mechanical_detail() -> String {
    format!(
        "<!doctype html><html><head><link rel=\"canonical\" href=\"{MECHANICAL_DETAIL}\"/></head>\
         <body><article class=\"container-site py-14 max-w-4xl\">\
         <header class=\"border-b-2 border-sg-ink pb-7\">\
         <span class=\"eyebrow\">일반공지</span><h1 class=\"mt-3\">기계공학과 공지</h1>\
         <div class=\"mt-5 flex\"><span>작성자 · 기계공학과</span>\
         <span>날짜 · 2026.09.28</span><span>조회 · 1</span></div></header>\
         <div class=\"prose-sg mt-8 min-h-[120px]\"><p>본문입니다.</p></div>\
         </article></body></html>"
    )
}

#[tokio::test]
async fn sitemap_listing_skips_an_unchanged_article_until_its_change_time_moves() {
    let (dir, store) = setup(&["me-general"]);
    let fake = Fake::new(vec![]);
    fake.extra.lock().unwrap().insert(
        MECHANICAL_SITEMAP.into(),
        Some((
            mechanical_sitemap("2026-09-28T15:31:48.462Z").into_bytes(),
            "application/xml".into(),
        )),
    );
    fake.extra.lock().unwrap().insert(
        MECHANICAL_DETAIL.into(),
        Some((mechanical_detail().into_bytes(), "text/html".into())),
    );
    let controller = Controller::new();
    let run_once = |mode: &'static str| {
        controller.execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &["me-general"]),
            mode,
            Budget::default(),
        )
    };
    assert!(run_once("incremental").await.is_ok());
    assert_eq!(items(&store).len(), 1);
    let detail_calls = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| *url == MECHANICAL_DETAIL)
            .count()
    };
    assert_eq!(detail_calls(&fake), 1);
    assert_eq!(
        with_store(&store, |s| s.source("me-general"))
            .unwrap()
            .status,
        "completed"
    );

    // The archive already carries this change time, so no article request is made.
    assert!(run_once("incremental").await.is_ok());
    assert_eq!(detail_calls(&fake), 1);
    assert_eq!(items(&store).len(), 1);

    // A later recorded change fetches and re-archives the notice.
    fake.extra.lock().unwrap().insert(
        MECHANICAL_SITEMAP.into(),
        Some((
            mechanical_sitemap("2026-09-29T09:00:00.000Z").into_bytes(),
            "application/xml".into(),
        )),
    );
    assert!(run_once("incremental").await.is_ok());
    assert_eq!(detail_calls(&fake), 2);
    assert_eq!(items(&store).len(), 1);
    assert!(with_store(&store, |s| s.listing_unchanged(
        "me-general",
        "1",
        "2026-09-29T09:00:00.000Z"
    ))
    .unwrap());
}

const SE_NOTICE_LIST_1: &str = include_str!("../fixtures/semiconductor-notice-list.html");
const SE_NOTICE_LIST_2: &str = include_str!("../fixtures/semiconductor-notice-page2.html");
const SE_NOTICE_LIST_3: &str = include_str!("../fixtures/semiconductor-notice-last.html");
const SE_IMAGE: &str = "https://se.sogang.ac.kr/smart_editor/upload/1-2.png";
const SE_ATTACHMENT: &str =
    "https://se.sogang.ac.kr/board/filedown.php?filepath=abc&filename=%EC%95%88%EB%82%B4.pdf";

fn semiconductor_detail(no: &str, revision: u64) -> String {
    let (files, image) = if no == "39" {
        (
            "<div class=\"view_file\"><dl class=\"attachments\"><dt>첨부파일</dt><dd><ul>\
             <li><a href=\"/board/filedown.php?filepath=abc&filename=안내.pdf\">안내.pdf</a></li>\
             </ul></dd></dl></div>",
            "<p><img src=\"/smart_editor/upload/1-2.png\" alt=\"안내\"></p>",
        )
    } else {
        (
            "<div class=\"view_file\"><dl class=\"attachments\"><dt>첨부파일</dt><dd><ul></ul></dd></dl></div>",
            "",
        )
    };
    format!(
        "<!doctype html><html lang=\"ko\"><body>\
         <h2 id=\"title\" class=\"pageName2\">학과 공지사항</h2>\
         <div class=\"view_tit\"><h3 class=\"v_tit\">공지 {no} rev {revision}</h3>\
         <p class=\"v_list\"><span>2026-10-12</span></p></div>\
         {files}\
         <div class=\"view_con\"><p>본문 {no} rev {revision}</p>{image}</div>\
         <p class=\"btn_list\"><a href=\"/board/board_list.php?board_id=notice&page=1\">목록으로</a></p>\
         </body></html>"
    )
}

struct SemiconductorFake {
    revision: AtomicU64,
}
impl Transport for SemiconductorFake {
    async fn fetch<'a>(
        &'a self,
        url: &'a Url,
        _validators: Option<(Option<String>, Option<String>)>,
        _limit: usize,
        _cancel: &AtomicBool,
        _access: network::Access,
    ) -> Result<network::Fetch, String> {
        let body = |bytes: Vec<u8>, mime: &str| {
            Ok(network::Fetch::Body {
                bytes,
                mime: Some(mime.into()),
                etag: None,
                last_modified: None,
            })
        };
        if url.path() == "/robots.txt" {
            return body(b"User-agent: *\nAllow: /\n".to_vec(), "text/plain");
        }
        let query = |key: &str| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
        };
        if url.path() == "/board/board_list.php" {
            let fixture = match (query("board_id").as_deref(), query("page").as_deref()) {
                (Some("notice"), Some("1")) => SE_NOTICE_LIST_1,
                (Some("notice"), Some("2")) => SE_NOTICE_LIST_2,
                (Some("notice"), Some("3")) => SE_NOTICE_LIST_3,
                _ => return Err(format!("예상하지 않은 모의 목록: {url}")),
            };
            return body(fixture.as_bytes().to_vec(), "text/html");
        }
        if url.path() == "/board/board_view.php" {
            if query("board_id").as_deref() != Some("notice") {
                return Err(format!("예상하지 않은 모의 원문: {url}"));
            }
            let no = query("no").ok_or("모의 원문 번호 없음")?;
            return body(
                semiconductor_detail(&no, self.revision.load(Ordering::Acquire)).into_bytes(),
                "text/html",
            );
        }
        if url.as_str() == SE_IMAGE {
            return body(png(), "image/png");
        }
        if url.as_str() == SE_ATTACHMENT {
            return body(PDF.to_vec(), "application/pdf");
        }
        Err(format!("예상하지 않은 모의 요청: {url}"))
    }
}

#[tokio::test]
async fn semiconductor_paging_pins_revisions_and_resources_survive_reopen() {
    let (dir, store) = setup(&["se-notices"]);
    let database = dir.path().join("rhythm.sqlite3");
    let fake = SemiconductorFake {
        revision: AtomicU64::new(1),
    };
    let controller = Controller::new();
    let run_once = |mode: &'static str| {
        controller.execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &["se-notices"]),
            mode,
            Budget::default(),
        )
    };
    assert!(run_once("historical").await.is_ok());
    assert_eq!(items(&store).len(), 32);
    let source = with_store(&store, |s| s.source("se-notices")).unwrap();
    assert!(source.history_complete);
    assert_eq!(source.next_page, 4);
    assert_eq!(source.archived, 32);
    assert_eq!(source.failed, 0);

    let id = with_store(&store, |s| s.document_id("se-notices", "39"))
        .unwrap()
        .unwrap();
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.revisions.len(), 1);
    assert!(detail.body_text.contains("rev 1"));
    let attachments: Vec<_> = detail
        .resources
        .iter()
        .filter(|resource| resource.kind == "attachment")
        .collect();
    let images: Vec<_> = detail
        .resources
        .iter()
        .filter(|resource| resource.kind == "image")
        .collect();
    assert_eq!(attachments.len(), 1);
    assert_eq!(images.len(), 1);
    assert_eq!(attachments[0].status, "downloaded");
    assert_eq!(images[0].status, "downloaded");
    assert_eq!(
        local_resource(&store, dir.path(), attachments[0].id)
            .await
            .unwrap()
            .1,
        PDF
    );
    assert_eq!(
        local_resource(&store, dir.path(), images[0].id)
            .await
            .unwrap()
            .1,
        png()
    );

    // The repeated pins were fetched once, not once per page.
    assert!(with_store(&store, |s| s.document_id("se-notices", "34"))
        .unwrap()
        .is_some());

    // An unchanged incremental pass adds no revision.
    assert!(run_once("incremental").await.is_ok());
    assert_eq!(items(&store).len(), 32);
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.revisions.len(), 1);

    // A changed article body becomes a new revision.
    fake.revision.store(2, Ordering::Release);
    assert!(run_once("incremental").await.is_ok());
    let detail = with_store(&store, |s| s.document(id, None)).unwrap();
    assert_eq!(detail.revisions.len(), 2);
    assert!(detail.body_text.contains("rev 2"));
    assert_eq!(detail.resources.len(), 2);

    drop(store);
    let offline = Arc::new(Mutex::new(Store::open(&database).unwrap()));
    let detail = with_store(&offline, |s| s.document(id, None)).unwrap();
    assert!(detail.body_text.contains("rev 2"));
    assert_eq!(detail.revisions.len(), 2);
    for resource in &detail.resources {
        assert_eq!(resource.status, "downloaded");
        assert!(local_resource(&offline, dir.path(), resource.id)
            .await
            .is_ok());
    }
}

#[tokio::test]
async fn refresh_checks_new_notices_then_continues_the_unfinished_backlog() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2], vec![3, 4]]);
    let controller = Controller::new();
    let first = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "refresh",
            Budget::default(),
        )
        .await;
    assert!(first.is_ok(), "{first:?}");
    assert_eq!(items(&store).len(), 4);
    // The first pass seeds historical records, so a refresh does not flood them
    // into new notices, and the following pass finishes the backlog.
    assert!(items(&store).iter().all(|item| item.historical));
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "completed");
    assert!(source.history_complete);
    assert_eq!(source.next_page, 3);
    let lists = |page: &str| fake.paths().iter().filter(|url| url.contains(page)).count();
    assert_eq!(lists("pageNum=1&"), 2, "each step read page one once");
    assert_eq!(
        lists("pageNum=2&"),
        2,
        "the backlog step resumed at page one"
    );

    fake.pages.lock().unwrap()[0].push(5);
    let before = fake.paths().len();
    let second = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "refresh",
            Budget::default(),
        )
        .await;
    assert!(second.is_ok(), "{second:?}");
    assert_eq!(items(&store).len(), 5);
    assert!(
        !items(&store)
            .into_iter()
            .find(|item| item.title.ends_with(" 5"))
            .unwrap()
            .historical
    );
    let after = &fake.paths()[before..];
    assert_eq!(
        after
            .iter()
            .filter(|url| url.contains("pageNum=1&"))
            .count(),
        1
    );
    assert!(
        after.iter().all(|url| !url.contains("pageNum=3&")),
        "completed history must not be requested again: {after:?}"
    );
}

/// Presses 중단 (a job-wide cancel) as the `cancel_on`-th request starts.
struct CancelAfter<'a> {
    fake: &'a Fake,
    controller: &'a Controller,
    calls: AtomicU64,
    cancel_on: u64,
}

impl Transport for CancelAfter<'_> {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: network::Access,
    ) -> Result<network::Fetch, String> {
        if self.calls.fetch_add(1, Ordering::AcqRel) + 1 == self.cancel_on {
            self.controller.cancel();
        }
        self.fake
            .fetch(url, validators, limit, cancel, access)
            .await
    }
}

#[tokio::test]
async fn refresh_cancelled_between_steps_keeps_new_notices_and_skips_the_backlog() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2], vec![3, 4]]);
    let controller = Controller::new();
    // The new-notices pass uses robots, two listings and four articles; the
    // cancellation is requested on the backlog pass's robots request.
    let transport = CancelAfter {
        fake: &fake,
        controller: &controller,
        calls: AtomicU64::new(0),
        cancel_on: 8,
    };
    let result = controller
        .execute(
            &store,
            dir.path(),
            &transport,
            sources(&store, &[MAIN]),
            "refresh",
            Budget::default(),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        items(&store).len(),
        4,
        "the saved seed survives cancellation"
    );
    assert!(items(&store).iter().all(|item| item.historical));
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "cancelled");
    assert_eq!(source.next_page, 1, "the backlog step never advanced");
    assert!(!source.history_complete);
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
    // Keeping the last good content is a checked notice, not a failure or a skip.
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!(source.status, "completed");
    assert_eq!((source.failed, source.skipped_count), (0, 0));
    assert!(source.last_error.is_none());
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
    // Unparseable markup is skipped and listed; the archived version stays.
    let source = with_store(&store, |s| s.sources())
        .unwrap()
        .into_iter()
        .find(|s| s.definition.id == MAIN)
        .unwrap();
    assert_eq!(source.status, "completed");
    assert_eq!(source.skipped_count, 1);
    assert_eq!(source.skipped[0].external_id, "5");
    assert!(source.skipped[0].archived);
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
        // The real disable path: the registry flips the flag and cancels
        // under the same store lock.
        let registry = crate::extensions::Registry::new(
            controller.clone(),
            crate::studium::controller::Controller::new(),
        );
        with_store(&store, |s| registry.set(s, "sogang_notices", false)).unwrap();
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
    let policy = Robots::load(
        &fake,
        &Url::parse(PNG_URL).unwrap(),
        &cancel,
        network::Access::Source,
    )
    .await
    .unwrap();
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
    db.subscribe(MAIN, true).unwrap();
    let store = Arc::new(Mutex::new(db));
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    run(
        &Controller::new(),
        &store,
        &root,
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
    // Public boards only: the Cybercampus connector would sign in with the
    // user's stored eClass login, which a live check needs explicit approval for.
    let ids: Vec<_> = catalog::definitions()
        .into_iter()
        .filter(|s| s.supported && s.family != cybercampus::FAMILY)
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
        &RobotsCache::default(),
        &resource,
        &AtomicU64::new(0),
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
            &AtomicBool::new(false),
            network::Access::Source
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

fn skipped(store: &SharedStore, source: &str) -> SourceInfo {
    with_store(store, |s| s.sources())
        .unwrap()
        .into_iter()
        .find(|s| s.definition.id == source)
        .unwrap()
}

#[tokio::test]
async fn permanent_failure_is_skipped_and_advances_history_but_transient_holds_the_page() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2, 3], vec![4]]);
    fake.bad.lock().unwrap().insert(3);
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
    let source = skipped(&store, MAIN);
    assert_eq!(source.status, "completed");
    assert!(source.history_complete, "the last public page was reached");
    assert_eq!((source.next_page, source.failed), (3, 0));
    assert_eq!(source.skipped_count, 1);
    assert_eq!(source.skipped[0].external_id, "3");
    assert!(!source.skipped[0].archived);
    assert!(!source.skipped[0].reason.is_empty());
    let mut archived = items(&store)
        .into_iter()
        .map(|item| item.title)
        .collect::<Vec<_>>();
    archived.sort();
    assert_eq!(archived, ["장학금 AI 연구 공지 2", "장학금 AI 연구 공지 4"]);

    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2, 3], vec![4]]);
    fake.failed.lock().unwrap().insert(3);
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
    let source = skipped(&store, MAIN);
    assert_eq!(source.status, "failed");
    assert!(!source.history_complete);
    assert_eq!((source.next_page, source.skipped_count), (1, 0));
}

#[tokio::test]
async fn explicit_retry_refetches_only_skipped_notices_and_keeps_board_state() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2, 3]]);
    fake.bad.lock().unwrap().insert(3);
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
    let board = |store: &SharedStore| {
        let s = skipped(store, MAIN);
        (
            s.status,
            s.last_success,
            s.last_error,
            s.next_page,
            s.history_complete,
        )
    };
    let before = board(&store);
    let detail_calls = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| url.ends_with("?pkId=3"))
            .count()
    };

    // The same rejection keeps the row; only the skipped notice is requested.
    let calls = fake.paths().len();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "skipped",
        Budget::default(),
    )
    .await;
    let requested = &fake.paths()[calls..];
    assert!(requested.iter().all(|url| !url.contains("boardList")));
    assert_eq!(detail_calls(&fake), 2);
    assert_eq!(skipped(&store, MAIN).skipped_count, 1);
    assert_eq!(board(&store), before);

    // A transient failure reports an error but keeps the row and the board state.
    fake.bad.lock().unwrap().clear();
    fake.failed.lock().unwrap().insert(3);
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "skipped",
            Budget::default(),
        )
        .await;
    assert!(outcome.is_err());
    assert_eq!(skipped(&store, MAIN).skipped_count, 1);
    assert_eq!(board(&store), before);

    fake.failed.lock().unwrap().clear();
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "skipped",
            Budget::default(),
        )
        .await;
    assert!(outcome.is_ok());
    assert_eq!(skipped(&store, MAIN).skipped_count, 0);
    assert_eq!(board(&store), before);
    let retried = items(&store)
        .into_iter()
        .find(|item| item.title.ends_with(" 3"))
        .unwrap();
    assert!(retried.historical, "a backfilled notice stays historical");
}

#[tokio::test]
async fn retried_new_notice_keeps_its_original_non_historical_marking() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
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
    *fake.pages.lock().unwrap() = vec![vec![3, 2]];
    fake.bad.lock().unwrap().insert(3);
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
    let source = skipped(&store, MAIN);
    assert_eq!(
        (source.status.as_str(), source.skipped_count),
        ("completed", 1)
    );
    fake.bad.lock().unwrap().clear();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "skipped",
        Budget::default(),
    )
    .await;
    let retried = items(&store)
        .into_iter()
        .find(|item| item.title.ends_with(" 3"))
        .unwrap();
    assert!(!retried.historical, "a new notice must still count as new");
    assert_eq!(skipped(&store, MAIN).skipped_count, 0);
}

const CDN_IMAGE: &str = "https://cdn.example.com/notice/a.png";
const CDN_OTHER: &str = "https://cdn.example.com/notice/b";

fn third_party_fixture(robots: &str) -> (TempDir, SharedStore, Fake) {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1]]);
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
    // One robots.txt request per host per job, not per image.
    assert_eq!(
        access("https://cdn.example.com/robots.txt"),
        [network::Access::EmbeddedImage]
    );
    assert!(accesses
        .iter()
        .filter(|(u, _)| u.contains("BbsData"))
        .all(|(_, a)| *a == network::Access::Source));

    let (dir, store, fake) = third_party_fixture("User-agent: *\nDisallow: /notice/\n");
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
    assert!(detail.resources.iter().all(|r| r.status == "failed"));
    assert!(
        !fake.paths().iter().any(|u| u == CDN_IMAGE),
        "robots denial sends no request"
    );
}

#[tokio::test]
async fn exhausted_budget_leaves_files_pending_instead_of_too_large() {
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
    fake.missing_resources.store(false, Ordering::Release);
    let id = items(&store)[0].id;
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

#[tokio::test]
async fn resources_mode_downloads_missing_files_without_touching_board_state() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![1, 2]]);
    fake.resources.store(true, Ordering::Release);
    fake.missing_resources.store(true, Ordering::Release);
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
    let board = |store: &SharedStore| {
        let s = skipped(store, MAIN);
        (
            s.status,
            s.last_success,
            s.last_error,
            s.next_page,
            s.archived,
        )
    };
    let before = board(&store);
    let missing = || with_store(&store, |s| s.documents_missing_resources(MAIN)).unwrap();
    assert_eq!(missing().len(), 2);
    fake.missing_resources.store(false, Ordering::Release);
    let calls = fake.paths().len();
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "resources",
            Budget::default(),
        )
        .await;
    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(missing().is_empty());
    assert!(items(&store)
        .iter()
        .all(|item| item.content_status == "archived"));
    assert!(
        fake.paths()[calls..].iter().all(|u| !u.contains("BbsData")),
        "no article refetch"
    );
    assert_eq!(board(&store), before);
    // Nothing left: a second run makes no requests beyond none at all.
    let calls = fake.paths().len();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "resources",
        Budget::default(),
    )
    .await;
    assert_eq!(fake.paths().len(), calls);
}

fn community_listing(total: u32, rows: &[(&str, u32, &str, &str)]) -> String {
    let rows: String = rows
        .iter()
        .map(|(title, ordinal, date, idx)| {
            format!(
                "<tr><td>{ordinal}</td><td><a href=\"/kor/community/notice02.php?m=v&amp;idx={idx}&amp;pNo=1&amp;code=general\">{title}</a></td><td>{date}</td><td>1</td></tr>"
            )
        })
        .collect();
    format!(
        "<!DOCTYPE html><html><body><h3 id=\"sub_tit\">일반공지</h3><div id=\"board\">\
<div class=\"list_top\"><p>총 <span>{total}</span>건의 게시물</p></div>\
<table class=\"basic_list\"><tbody>{rows}</tbody></table>\
<div class=\"b_page_no\"><div>\
<a class='pageBtn pageBtn_first'>FIRST</a><a class='pageBtn pageBtn_prev'>PREV</a>\
<div><a class='on'>1</a></div>\
<a class='pageBtn pageBtn_next'>NEXT</a><a class='pageBtn pageBtn_last'>LAST</a>\
</div></div></div></body></html>"
    )
}

#[tokio::test]
async fn community_board_syncs_text_and_robots_allowed_attachment_then_reopens_offline() {
    let (dir, store) = setup(&["ee-general"]);
    let fake = Fake::new(vec![]);
    let list = "https://ee.sogang.ac.kr/kor/community/notice02.php?pNo=1&code=general";
    let detail =
        "https://ee.sogang.ac.kr/kor/community/notice02.php?m=v&idx=2867&pNo=1&code=general";
    let attachment = "https://ee.sogang.ac.kr/aseoul_bbs/down.php?d=1&idx=2867";
    fake.extra.lock().unwrap().insert(
        list.into(),
        Some((
            community_listing(
                1,
                &[(
                    "2026 서강대-LG이노텍 Track 상담부스 및 설명회 안내(11/3 화)",
                    1,
                    "2026.11.02",
                    "2867",
                )],
            )
            .into_bytes(),
            "text/html".into(),
        )),
    );
    fake.extra.lock().unwrap().insert(
        detail.into(),
        Some((
            include_str!("../fixtures/community-ee-general-detail-2867.html")
                .replace(
                    "/kor/community/notice02.php?pNo=2&amp;code=general",
                    "/kor/community/notice02.php?pNo=1&amp;code=general",
                )
                .into_bytes(),
            "text/html".into(),
        )),
    );
    fake.extra.lock().unwrap().insert(
        attachment.into(),
        Some((PDF.to_vec(), "application/pdf".into())),
    );
    let controller = Controller::new();
    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &["ee-general"],
        "historical",
        Budget::default(),
    )
    .await;
    let items = items(&store);
    assert_eq!(items.len(), 1);
    assert!(items[0].historical);
    assert!(items[0].title.contains("LG이노텍"));
    let detail = with_store(&store, |s| s.document(items[0].id, None)).unwrap();
    assert!(detail.body_text.contains("서강대"));
    assert_eq!(detail.resources.len(), 3);
    let downloaded = detail
        .resources
        .iter()
        .find(|resource| resource.kind == "attachment")
        .unwrap();
    assert_eq!(downloaded.status, "downloaded");
    assert!(downloaded.sha256.is_some());
    assert_eq!(
        detail
            .resources
            .iter()
            .filter(|resource| resource.kind == "image")
            .count(),
        2
    );
    assert!(detail
        .resources
        .iter()
        .filter(|resource| resource.kind == "image")
        .all(|resource| resource.status == "unsupported"));
    // robots.txt disallows /se2/; the policy rejects those URLs before any request.
    assert!(
        fake.paths().iter().all(|url| !url.contains("/se2/")),
        "{:?}",
        fake.paths()
    );
    drop(store);
    let reopened = Arc::new(Mutex::new(
        Store::open(&dir.path().join("rhythm.sqlite3")).unwrap(),
    ));
    let offline = with_store(&reopened, |s| s.document(items[0].id, None)).unwrap();
    assert_eq!(offline.resources.len(), 3);
    assert!(offline
        .resources
        .iter()
        .any(|resource| resource.kind == "attachment" && resource.status == "downloaded"));
}

#[derive(Clone)]
struct Offline;
impl Transport for Offline {
    async fn fetch(
        &self,
        _url: &Url,
        _validators: Option<(Option<String>, Option<String>)>,
        _limit: usize,
        _cancel: &AtomicBool,
        _access: network::Access,
    ) -> Result<network::Fetch, String> {
        Err("모의 연결 실패".into())
    }
}

#[tokio::test]
async fn only_scheduler_jobs_keep_an_outcome_entry_until_taken() {
    let (dir, store) = setup(&["sogang-academic"]);
    let controller = Controller::new();
    let gate = Arc::new(crate::operation_gate::AdmissionGate::default());
    let start = |lease| {
        controller
            .start_tracked_with_lease(
                store.clone(),
                dir.path().to_path_buf(),
                Offline,
                Some(vec!["sogang-academic".into()]),
                "incremental".into(),
                lease,
            )
            .unwrap()
            .1
    };
    // A command-path job holds a lease and nobody takes its outcome.
    let untracked = start(Some(gate.shared().await.unwrap()));
    controller.wait().await;
    assert!(controller.take_task_outcome(untracked).is_err());
    assert!(controller.task_outcomes.lock().unwrap().is_empty());
    assert!(controller.task_notifications.lock().unwrap().is_empty());
    let tracked = start(None);
    controller.wait().await;
    let outcome = controller.take_task_outcome(tracked).unwrap().unwrap();
    assert!(outcome.result.is_err());
    assert!(controller.task_outcomes.lock().unwrap().is_empty());
    assert!(controller.task_notifications.lock().unwrap().is_empty());
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

#[test]
fn asp_source_robots_keeps_the_rules_of_every_branch() {
    let url = Url::parse(VIEWER_PDF).unwrap();
    let policy = Robots::parse(&url, &without_asp(SCC_ROBOTS).unwrap()).unwrap();
    assert!(policy.permits(&url));
    assert!(!policy.permits(&url.join("/cgsi/a").unwrap()));
    assert!(without_asp("User-agent: *\n<% if(x){ \nDisallow: /").is_err());
    assert!(
        Robots::parse(&url, SCC_ROBOTS).is_err(),
        "other hosts stay strict"
    );
}
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
        let fake = Fake::new(vec![]);
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
    let fake = Fake::new(vec![ids.iter().map(|(id, _)| *id).collect()]);
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
            include_bytes!("../fixtures/main-pdf-551680.pdf").to_vec(),
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
async fn viewer_pdf_text_becomes_the_body_of_a_new_notice() {
    let (dir, store) = setup(&[MAIN]);
    let fake = viewer_fake(&[(551680, VIEWER_PDF)]);
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
    let detail = document_by_external(&store, "551680");
    assert!(detail
        .body_text
        .contains("2026학년도 동계계절수업 수강신청 안내"));
    assert!(detail.body_html.contains("<p>"));
    assert_eq!(detail.revisions.len(), 1, "the body fill is not a revision");
    let pdf = detail
        .resources
        .iter()
        .find(|r| r.url == VIEWER_PDF)
        .unwrap();
    assert_eq!(
        (pdf.status.as_str(), pdf.filename.as_str()),
        ("downloaded", "안내.pdf")
    );
    assert_eq!(requests(&fake, VIEWER_PDF), 1);
    assert!(items(&store)[0].snippet.contains("동계계절수업"));

    // A later check of the same notice (only its view count changed) derives
    // the same body from the local PDF: no new revision and no new download.
    fake.views.store(9, Ordering::Release);
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
    let checked = document_by_external(&store, "551680");
    assert_eq!(checked.revisions.len(), 1);
    assert_eq!(checked.body_text, detail.body_text);
    assert_eq!(requests(&fake, VIEWER_PDF), 1);
    assert_eq!(
        with_store(&store, |s| s.source(MAIN)).unwrap().status,
        "completed"
    );
}

#[tokio::test]
async fn older_viewer_notices_are_backfilled_from_their_stored_response() {
    let (dir, store) = setup(&[MAIN]);
    let fake = viewer_fake(&[
        (551680, VIEWER_PDF),
        (551681, VIEWER_PDF),
        (551682, VIEWER_SCAN),
    ]);
    let source = with_store(&store, |s| s.source(MAIN)).unwrap().definition;
    // Archived by an earlier version that did not recognize the viewer PDF.
    for id in [551680, 551681, 551682] {
        let entry = ListingEntry {
            external_id: id.to_string(),
            url: parser::main_canonical(&id.to_string()).to_string(),
            pinned: false,
            modified_at: None,
        };
        let mut article = parser::article(&source, &entry, &fake.detail(id)).unwrap();
        article.resources.clear();
        with_store(&store, |s| s.upsert_article(MAIN, &article, true)).unwrap();
    }
    // An analysis already used 551681's empty revision.
    let analysed = document_by_external(&store, "551681");
    with_store(&store, |s| {
        s.ai_enqueue(
            &crate::storage::intelligence::AnalysisKey {
                document_id: analysed.item.id,
                revision_id: analysed.revision_id,
                task: "notice_analysis".into(),
                model_sha256: "a".repeat(64),
                prompt_version: 5,
                schema_version: 3,
                configuration_hash: "b".repeat(64),
            },
            &json!({"id": "test"}),
            1,
        )
        .map(|_| ())
    })
    .unwrap();
    let unfilled = document_by_external(&store, "551680").item.id;
    let before = with_store(&store, |s| s.latest_original(unfilled)).unwrap();

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
    assert!(
        fake.paths().iter().all(|u| !u.contains("pkId=")),
        "archived notices are not requested again"
    );
    let filled = document_by_external(&store, "551680");
    assert!(filled.body_text.contains("동계계절수업"));
    assert_eq!(
        filled.revisions.len(),
        1,
        "no revision, freshness or change notice"
    );
    assert_eq!(
        with_store(&store, |s| s.latest_original(filled.item.id)).unwrap(),
        before
    );
    let revised = document_by_external(&store, "551681");
    assert!(revised.body_text.contains("동계계절수업"));
    assert_eq!(
        revised.revisions.len(),
        2,
        "the analysed empty revision stays intact"
    );
    // A PDF without a text layer leaves the notice metadata-only, but stays
    // downloaded so later runs neither refetch nor fail on it.
    let scan = document_by_external(&store, "551682");
    assert!(scan.body_text.is_empty());
    assert_eq!(scan.resources[0].status, "downloaded");
    assert_eq!(requests(&fake, VIEWER_PDF), 2);
    assert_eq!(requests(&fake, VIEWER_SCAN), 1);
    let source = with_store(&store, |s| s.source(MAIN)).unwrap();
    assert_eq!((source.status.as_str(), source.failed), ("completed", 0));

    run(
        &controller,
        &store,
        dir.path(),
        &fake,
        &[MAIN],
        "resources",
        Budget::default(),
    )
    .await;
    assert_eq!(requests(&fake, VIEWER_SCAN), 1);
    assert_eq!(document_by_external(&store, "551682").revisions.len(), 1);
}

#[tokio::test]
async fn admission_requires_the_extension_that_owns_each_source() {
    let (dir, store) = setup(&["sogang-academic", "cybercampus"]);
    let controller = Controller::new();
    let start = |ids: &[&str]| {
        controller
            .start_tracked_with_lease(
                store.clone(),
                dir.path().to_path_buf(),
                Offline,
                Some(ids.iter().map(|id| id.to_string()).collect()),
                "incremental".into(),
                None,
            )
            .map(|_| ())
    };
    // The notice extension does not cover the Cybercampus source.
    assert!(start(&["cybercampus"]).is_err());
    assert!(start(&["sogang-academic", "cybercampus"]).is_err());
    start(&["sogang-academic"]).unwrap();
    controller.wait().await;
    // And the reverse: with only Cybercampus on, the public board is refused.
    with_store(&store, |s| {
        s.set_enabled("sogang_notices", false)?;
        s.set_enabled("cybercampus", true)
    })
    .unwrap();
    assert!(start(&["sogang-academic"]).is_err());
    let owners = with_store(&store, |s| s.sources()).unwrap();
    assert!(owners.iter().all(|source| source.extension.as_deref()
        == Some(
            if source.definition.family == crate::archive::cybercampus::FAMILY {
                "cybercampus"
            } else {
                "sogang_notices"
            }
        )));
}

#[tokio::test]
async fn rate_limited_notice_stops_the_board_without_requesting_the_rest() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2, 3, 4]]);
    fake.rate_limited.lock().unwrap().insert(3);
    let controller = Controller::new();
    let result = controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &[MAIN]),
            "historical",
            Budget::default(),
        )
        .await;
    assert!(result.unwrap_err().contains(network::RATE_LIMITED));
    assert!(
        !fake.paths().iter().any(|url| url.contains("pkId=4")),
        "{:?}",
        fake.paths()
    );
    let source = skipped(&store, MAIN);
    assert_eq!(source.status, "failed");
    assert_eq!((source.next_page, source.failed), (1, 1));
    assert!(!source.history_complete);
    assert_eq!(source.skipped_count, 0, "a refusal is not a skipped notice");
    assert_eq!(items(&store).len(), 1);
}

#[tokio::test]
async fn unsupported_attachment_is_recorded_without_requesting_it() {
    let (dir, store) = setup(&[MAIN]);
    let fake = Fake::new(vec![vec![2]]);
    fake.resources.store(true, Ordering::Release);
    let exe = "https://www.sogang.ac.kr/file-fe-prd/board/2/setup.exe?sg=setup.exe";
    *fake.attachment.lock().unwrap() = exe.into();
    let controller = Controller::new();
    for _ in 0..2 {
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
    }
    assert_eq!(requests(&fake, exe), 0, "{:?}", fake.paths());
    let id = items(&store)[0].id;
    let resources = with_store(&store, |s| s.pending_resources(id)).unwrap();
    let attachment = resources.iter().find(|r| r.url == exe).unwrap();
    assert_eq!(attachment.status, "unsupported");
    assert_eq!(skipped(&store, MAIN).status, "completed");
}

const COMPUTING_GRADUATE: &str = "computing-graduate";
type Hook<'a> = Box<dyn Fn(u64, &Url) + Send + Sync + 'a>;
/// Holds every request briefly so boards overlap, records the peak number of
/// requests in flight overall and per host, and runs `hook` with the
/// 1-based request number before each request.
struct Overlap<'a> {
    fake: &'a Fake,
    calls: AtomicU64,
    inflight: Mutex<HashMap<String, usize>>,
    peak_total: AtomicU64,
    peak_host: AtomicU64,
    hook: Hook<'a>,
}
impl<'a> Overlap<'a> {
    fn new(fake: &'a Fake, hook: impl Fn(u64, &Url) + Send + Sync + 'a) -> Self {
        Self {
            fake,
            calls: AtomicU64::new(0),
            inflight: Mutex::new(HashMap::new()),
            peak_total: AtomicU64::new(0),
            peak_host: AtomicU64::new(0),
            hook: Box::new(hook),
        }
    }
}
impl Transport for Overlap<'_> {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: network::Access,
    ) -> Result<network::Fetch, String> {
        (self.hook)(self.calls.fetch_add(1, Ordering::AcqRel) + 1, url);
        let host = url.host_str().unwrap().to_owned();
        {
            let mut inflight = self.inflight.lock().unwrap();
            let count = inflight.entry(host.clone()).or_default();
            *count += 1;
            self.peak_host.fetch_max(*count as u64, Ordering::AcqRel);
            let total = inflight.values().sum::<usize>() as u64;
            self.peak_total.fetch_max(total, Ordering::AcqRel);
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        *self.inflight.lock().unwrap().get_mut(&host).unwrap() -= 1;
        self.fake
            .fetch(url, validators, limit, cancel, access)
            .await
    }
}
fn status(store: &SharedStore, id: &str) -> String {
    with_store(store, |s| s.source(id)).unwrap().status
}
fn requested(fake: &Fake, host: &str) -> bool {
    fake.calls
        .lock()
        .unwrap()
        .iter()
        .any(|(url, _)| Url::parse(url).unwrap().host_str() == Some(host))
}

#[tokio::test]
async fn boards_on_different_hosts_collect_at_once_and_one_host_stays_serial() {
    let boards = [MAIN, COMPUTING, COMPUTING_GRADUATE];
    let (dir, store) = setup(&boards);
    let fake = Fake::new(vec![vec![1, 2]]);
    let controller = Controller::new();
    let transport = Overlap::new(&fake, |_, _| {});
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &transport,
            sources(&store, &boards),
            "historical",
            Budget::default(),
        )
        .await;
    // The fake serves only one computing notice, so those boards fail; that
    // failure must not stop the other host.
    assert!(outcome.is_err());
    assert_eq!(status(&store, MAIN), "completed");
    for id in [COMPUTING, COMPUTING_GRADUATE] {
        assert_eq!(status(&store, id), "failed", "{id}");
    }
    assert_eq!(
        transport.peak_total.load(Ordering::Acquire),
        2,
        "the www and computing hosts collect at once"
    );
    assert_eq!(
        transport.peak_host.load(Ordering::Acquire),
        1,
        "the two computing boards never overlap"
    );
}

#[tokio::test]
async fn unsubscribing_a_running_board_stops_only_that_board() {
    let (dir, store) = setup(&[MAIN, COMPUTING]);
    let fake = Fake::new(vec![vec![1, 2]]);
    let controller = Controller::new();
    let transport = Overlap::new(&fake, |_, url| {
        if url.host_str() == Some("computing.sogang.ac.kr") && url.path().ends_with("/list") {
            // What `archive_subscribe(false)` does.
            with_store(&store, |s| s.subscribe(COMPUTING, false)).unwrap();
            controller.cancel_source(COMPUTING);
        }
    });
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &transport,
            sources(&store, &[MAIN, COMPUTING]),
            "historical",
            Budget::default(),
        )
        .await;
    assert!(
        outcome.is_ok(),
        "an unsubscribed board is not a failure: {outcome:?}"
    );
    assert_eq!(status(&store, MAIN), "completed");
    assert_eq!(status(&store, COMPUTING), "cancelled");
    assert!(items(&store).iter().all(|item| item.source_id == MAIN));
}

#[tokio::test]
async fn cancelling_the_job_stops_every_board() {
    let (dir, store) = setup(&[MAIN, COMPUTING]);
    let fake = Fake::new(vec![vec![1, 2], vec![3, 4]]);
    let controller = Controller::new();
    let transport = Overlap::new(&fake, |call, _| {
        if call == 3 {
            controller.cancel();
        }
    });
    let outcome = controller
        .execute(
            &store,
            dir.path(),
            &transport,
            sources(&store, &[MAIN, COMPUTING]),
            "historical",
            Budget::default(),
        )
        .await;
    assert!(outcome.is_err());
    assert_eq!(status(&store, MAIN), "cancelled");
    assert_eq!(status(&store, COMPUTING), "cancelled");
}

#[tokio::test]
async fn a_board_subscribed_during_a_job_of_every_board_joins_it() {
    for joinable in [true, false] {
        let (dir, store) = setup(&[MAIN]);
        let fake = Fake::new(vec![vec![1, 2]]);
        let controller = Controller::new();
        let joined = AtomicBool::new(false);
        let transport = Overlap::new(&fake, |call, _| {
            if call == 1 {
                // What `archive_subscribe(true)` does.
                let source = with_store(&store, |s| {
                    s.subscribe(COMPUTING, true)?;
                    s.source(COMPUTING)
                })
                .unwrap();
                joined.store(controller.join(source), Ordering::Release);
            }
        });
        let extensions = if joinable {
            HashSet::from([Some("sogang_notices".to_owned())])
        } else {
            HashSet::new()
        };
        controller.reserve_task(0, extensions).unwrap();
        let outcome = controller
            .execute(
                &store,
                dir.path(),
                &transport,
                sources(&store, &[MAIN]),
                "historical",
                Budget::default(),
            )
            .await;
        controller.finish(String::new());
        assert_eq!(joined.load(Ordering::Acquire), joinable);
        assert_eq!(status(&store, MAIN), "completed");
        assert_eq!(
            requested(&fake, "computing.sogang.ac.kr"),
            joinable,
            "joinable job: {joinable}"
        );
        // The fake serves only one computing notice, so the joined board fails.
        assert_eq!(outcome.is_ok(), !joinable, "{outcome:?}");
        if joinable {
            assert_eq!(status(&store, COMPUTING), "failed");
        }
        // A finished job takes no more boards.
        assert!(!controller.join(with_store(&store, |s| s.source(COMPUTING)).unwrap()));
    }
}

struct Recording<'a, T> {
    inner: &'a T,
    urls: Mutex<Vec<String>>,
}
impl<T: Transport + Sync> Transport for Recording<'_, T> {
    async fn fetch<'a>(
        &'a self,
        url: &'a Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: network::Access,
    ) -> Result<network::Fetch, String> {
        self.urls.lock().unwrap().push(url.to_string());
        self.inner
            .fetch(url, validators, limit, cancel, access)
            .await
    }
}

#[tokio::test]
async fn trashed_notices_stay_in_sync_and_deleted_ones_are_never_fetched_again() {
    let (dir, store) = setup(&["se-notices"]);
    let fake = SemiconductorFake {
        revision: AtomicU64::new(1),
    };
    let controller = Controller::new();
    assert!(controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &["se-notices"]),
            "historical",
            Budget::default()
        )
        .await
        .is_ok());
    assert_eq!(items(&store).len(), 32);
    let id = with_store(&store, |s| s.document_id("se-notices", "39"))
        .unwrap()
        .unwrap();
    let shared = with_store(&store, |s| s.document(id, None))
        .unwrap()
        .resources;
    assert!(shared.iter().all(|r| r.status == "downloaded"));

    // In 휴지통 the notice keeps its content and still counts as archived.
    with_store(&store, |s| s.trash_documents(&[id])).unwrap();
    assert_eq!(items(&store).len(), 31);
    assert!(controller
        .execute(
            &store,
            dir.path(),
            &fake,
            sources(&store, &["se-notices"]),
            "incremental",
            Budget::default()
        )
        .await
        .is_ok());
    assert_eq!(items(&store).len(), 31);
    assert!(
        with_store(&store, |s| s.document(id, None))
            .unwrap()
            .item
            .trashed
    );

    // Its downloaded files are handed back for removal; other notices keep theirs.
    let mut orphaned = with_store(&store, |s| s.purge_trashed(Some(&[id]))).unwrap();
    let mut expected = shared
        .iter()
        .filter_map(|r| r.local_path.clone())
        .collect::<Vec<_>>();
    orphaned.sort();
    expected.sort();
    expected.dedup();
    assert_eq!(orphaned, expected);
    for item in items(&store) {
        for resource in with_store(&store, |s| s.document(item.id, None))
            .unwrap()
            .resources
        {
            assert!(!orphaned.contains(resource.local_path.as_ref().unwrap()));
        }
    }

    let recording = Recording {
        inner: &fake,
        urls: Mutex::new(Vec::new()),
    };
    assert!(controller
        .execute(
            &store,
            dir.path(),
            &recording,
            sources(&store, &["se-notices"]),
            "incremental",
            Budget::default()
        )
        .await
        .is_ok());
    let urls = recording.urls.into_inner().unwrap();
    assert!(urls.iter().any(|url| url.contains("board_list.php")));
    assert!(!urls
        .iter()
        .any(|url| url.contains("board_view.php") && url.contains("no=39")));
    assert!(with_store(&store, |s| s.document_id("se-notices", "39"))
        .unwrap()
        .is_none());
    assert_eq!(items(&store).len(), 31);
    let source = with_store(&store, |s| s.source("se-notices")).unwrap();
    assert_eq!(source.failed, 0);
}
