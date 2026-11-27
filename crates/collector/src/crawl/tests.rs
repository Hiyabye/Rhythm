//! Ported from the app's `archive/sync/tests.rs`: the same synthetic main-board
//! pages built from the committed fixtures, served by a fake transport.
use super::*;
use rhythm_sources::transport::FetchError;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, Mutex},
};

/// 공지사항 (`sogang-general`, board 1): the second main-site board. The fake
/// serves it the same pages with IDs offset by `GENERAL_IDS`, so a detail
/// request (`pkId` only) names its board.
const GENERAL: &str = "sogang-general";
const GENERAL_IDS: u64 = 1_000_000_000;

const MAIN: &str = "sogang-academic";
const MECHANICAL: &str = "me-general";
const MECHANICAL_SITEMAP: &str = "https://me.sogang.ac.kr/sitemap.xml";
const MECHANICAL_DETAIL: &str = "https://me.sogang.ac.kr/ko/board/notice/1";

type Validators = Option<(Option<String>, Option<String>)>;

struct Fake {
    pages: Mutex<Vec<Vec<u64>>>,
    changed: Mutex<HashMap<u64, String>>,
    bad: Mutex<HashSet<u64>>,
    failed: Mutex<HashSet<u64>>,
    rate_limited: Mutex<HashSet<u64>>,
    /// Serve a page past the last one like the school: a valid envelope with
    /// no rows (which the parser rejects) instead of a transport error.
    past_end: AtomicBool,
    conditional: AtomicBool,
    views: AtomicU64,
    robots: Mutex<String>,
    /// Extra responses by exact URL: body and MIME type.
    extra: Mutex<HashMap<String, (String, String)>>,
    /// URLs answered with HTTP 404.
    not_found: Mutex<HashSet<String>>,
    calls: Mutex<Vec<(String, Validators)>>,
    /// `Transport::server` answers by host; others are unknown.
    servers: Mutex<HashMap<String, IpAddr>>,
}
impl Fake {
    fn new(pages: Vec<Vec<u64>>) -> Self {
        Self {
            pages: Mutex::new(pages),
            changed: Mutex::default(),
            bad: Mutex::default(),
            failed: Mutex::default(),
            rate_limited: Mutex::default(),
            past_end: AtomicBool::new(false),
            conditional: AtomicBool::new(false),
            views: AtomicU64::new(1),
            robots: Mutex::new("User-agent: *\nAllow: /\n".into()),
            extra: Mutex::default(),
            not_found: Mutex::default(),
            calls: Mutex::default(),
            servers: Mutex::default(),
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
            "../../../sources/src/fixtures/main-detail.json"
        ))
        .unwrap();
        let data = &mut value["data"];
        data["pkId"] = json!(id);
        data["configPkId"] = json!(if id >= GENERAL_IDS { 1 } else { 2 });
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
        value.to_string()
    }
    fn list(&self, page: usize, config: u64) -> Result<String, FetchError> {
        let offset = if config == 1 { GENERAL_IDS } else { 0 };
        let pages = self.pages.lock().unwrap();
        let past_end = self.past_end.load(Ordering::Acquire) && page > pages.len();
        let no_rows = Vec::new();
        let ids = if past_end {
            &no_rows
        } else {
            pages.get(page - 1).ok_or("없는 목록 페이지")?
        };
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../sources/src/fixtures/main-list-exact.json"
        ))
        .unwrap();
        let template = value["data"]["list"][0].clone();
        let rows: Vec<_> = ids
            .iter()
            .map(|id| {
                let mut row = template.clone();
                row["pkId"] = json!(id + offset);
                row["configId"] = json!(config);
                row["title"] = json!(format!("장학금 AI 연구 공지 {}", id + offset));
                row["isTop"] = json!(if *id == 1 { "Y" } else { "N" });
                row
            })
            .collect();
        value["data"]["pageNum"] = json!(page);
        value["data"]["pages"] = json!(pages.len());
        // Sparse synthetic pages keep a coherent page-count/row-size envelope.
        let unique = pages.iter().flatten().collect::<HashSet<_>>().len();
        let implied = pages.len().saturating_sub(1) * 50
            + pages
                .last()
                .map_or(0, |ids| ids.iter().collect::<HashSet<_>>().len());
        value["data"]["total"] = json!(unique.max(implied));
        value["data"]["size"] = json!(ids.iter().collect::<HashSet<_>>().len());
        value["data"]["list"] = json!(rows);
        value["data"]["hasNextPage"] = json!(page < pages.len());
        Ok(value.to_string())
    }
}
impl Transport for Fake {
    async fn server(&self, host: &str) -> Option<IpAddr> {
        self.servers.lock().unwrap().get(host).copied()
    }
    async fn fetch(
        &self,
        url: &Url,
        validators: Validators,
        limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        assert_eq!(access, Access::Source);
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), validators.clone()));
        let body = |text: String, mime: &str, etag: Option<String>| {
            // Like the real transport, a response over the limit is an error.
            if text.len() > limit {
                return Err(network::TOO_LARGE.into());
            }
            Ok(Fetch::Body {
                bytes: text.into_bytes(),
                mime: Some(mime.into()),
                etag,
                last_modified: None,
            })
        };
        if self.not_found.lock().unwrap().contains(url.as_str()) {
            return Err(FetchError::http(404, "없는 공지".into()));
        }
        if let Some((text, mime)) = self.extra.lock().unwrap().get(url.as_str()).cloned() {
            return body(text, &mime, None);
        }
        if url.path() == "/robots.txt" {
            return body(self.robots.lock().unwrap().clone(), "text/plain", None);
        }
        let query = |key: &str| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .unwrap()
                .1
                .into_owned()
        };
        if url.path().ends_with("/boardList") {
            return body(
                self.list(
                    query("pageNum").parse().unwrap(),
                    query("bbsConfigFk").parse().unwrap(),
                )?,
                "application/json",
                None,
            );
        }
        if url.path() == "/api/api/v1/mainKo/BbsData" {
            let id: u64 = query("pkId").parse().unwrap();
            if self.failed.lock().unwrap().contains(&id) {
                return Err("모의 학교 서버 연결 실패".into());
            }
            if self.rate_limited.lock().unwrap().contains(&id) {
                return Err(network::RATE_LIMITED.into());
            }
            if self.bad.lock().unwrap().contains(&id) {
                return body(
                    "<html>changed markup</html>".into(),
                    "application/json",
                    None,
                );
            }
            if self.conditional.load(Ordering::Acquire)
                && validators.as_ref().is_some_and(|(etag, _)| etag.is_some())
            {
                return Ok(Fetch::NotModified);
            }
            return body(
                self.detail(id),
                "application/json",
                Some(format!("\"article-{id}\"")),
            );
        }
        Err(format!("unexpected fake request: {url}").into())
    }
}

/// Serves the mechanical board: a sitemap with one notice changed at
/// `lastmod`, and that notice.
fn serve_mechanical(fake: &Fake, lastmod: &str) {
    let sitemap = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\
         <url><loc>{MECHANICAL_DETAIL}</loc><lastmod>{lastmod}</lastmod></url>\
         </urlset>"
    );
    let detail = format!(
        "<!doctype html><html><head><link rel=\"canonical\" href=\"{MECHANICAL_DETAIL}\"/></head>\
         <body><article class=\"container-site py-14 max-w-4xl\">\
         <header class=\"border-b-2 border-sg-ink pb-7\">\
         <span class=\"eyebrow\">일반공지</span><h1 class=\"mt-3\">기계공학과 공지</h1>\
         <div class=\"mt-5 flex\"><span>작성자 · 기계공학과</span>\
         <span>날짜 · 2026.09.28</span><span>조회 · 1</span></div></header>\
         <div class=\"prose-sg mt-8 min-h-[120px]\"><p>본문입니다.</p></div>\
         </article></body></html>"
    );
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        MECHANICAL_SITEMAP.into(),
        (sitemap, "application/xml".into()),
    );
    extra.insert(MECHANICAL_DETAIL.into(), (detail, "text/html".into()));
}

fn definition(id: &str) -> SourceDefinition {
    public_boards()
        .into_iter()
        .find(|source| source.id == id)
        .unwrap()
}
async fn run(state: &State, fake: &Fake, id: &str, budget: Budget) -> Result<(), String> {
    run_board(
        state,
        fake,
        &definition(id),
        budget,
        &AtomicBool::new(false),
    )
    .await
}
/// Historical (bootstrap) import only: the incremental pass reaches the end of
/// a short board and completes history on the way.
async fn historical(state: &State, fake: &Fake, budget: Budget) {
    let crawl = Crawl {
        state,
        transport: fake,
        cancel: &AtomicBool::new(false),
        budget,
        policies: &Policies::default(),
    };
    let mut board = state.board(MAIN).unwrap();
    let _ = crawl
        .pass(&definition(MAIN), &mut board, Pass::Historical)
        .await;
}

#[derive(Debug)]
struct Notice {
    external_id: String,
    title: String,
    body_text: String,
    body_state: String,
    historical: bool,
    revision_hash: String,
    revised_at: String,
}
fn notices(state: &State) -> Vec<Notice> {
    state
        .with(|c| {
            let mut statement = c.prepare(
                "SELECT external_id,title,body_text,body_state,historical,revision_hash,revised_at
                 FROM notices ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(Notice {
                    external_id: row.get(0)?,
                    title: row.get(1)?,
                    body_text: row.get(2)?,
                    body_state: row.get(3)?,
                    historical: row.get(4)?,
                    revision_hash: row.get(5)?,
                    revised_at: row.get(6)?,
                })
            })?;
            rows.collect()
        })
        .unwrap()
}
fn notice(state: &State, external_id: &str) -> Notice {
    notices(state)
        .into_iter()
        .find(|n| n.external_id == external_id)
        .unwrap()
}
fn skipped(state: &State) -> Vec<String> {
    state
        .with(|c| {
            let mut statement =
                c.prepare("SELECT external_id FROM skipped ORDER BY external_id")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .unwrap()
}

#[test]
fn public_boards_are_every_supported_board_but_cybercampus() {
    let boards = public_boards();
    assert_eq!(boards.len(), 172);
    assert!(boards
        .iter()
        .all(|b| b.family != catalog::CYBERCAMPUS_FAMILY));
}

#[tokio::test]
async fn history_import_deduplicates_repeated_pins_and_is_not_fetched_again() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![
        (1..=50).collect(),
        std::iter::once(1).chain(51..=62).collect(),
    ]);
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(notices(&state).len(), 62);
    assert!(notices(&state).iter().all(|n| n.historical));
    let board = state.board(MAIN).unwrap();
    assert!(board.history_complete);
    assert_eq!((board.next_page, board.archived), (3, 62));
    assert!(state.seed_complete(MAIN).unwrap());
    let pin_requests = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| url.ends_with("?pkId=1"))
            .count()
    };
    assert_eq!(pin_requests(&fake), 1);

    let before = fake.paths().len();
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(
        fake.paths().len(),
        before,
        "finished history is not refetched"
    );

    let hashes: Vec<_> = notices(&state)
        .into_iter()
        .map(|n| n.revision_hash)
        .collect();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let after: Vec<_> = notices(&state)
        .into_iter()
        .map(|n| n.revision_hash)
        .collect();
    assert_eq!(hashes, after, "an unchanged recheck is not a revision");
    assert_eq!(state.board(MAIN).unwrap().status, "completed");
}

#[tokio::test]
async fn only_meaningful_changes_revise_and_new_notices_are_not_historical() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2]]);
    historical(&state, &fake, Budget::default()).await;
    let original = notice(&state, "1");

    fake.views.store(999, Ordering::Release);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let viewed = notice(&state, "1");
    assert_eq!(viewed.revision_hash, original.revision_hash, "view count");
    assert_eq!(viewed.revised_at, original.revised_at);

    fake.changed.lock().unwrap().insert(
        1,
        "<h2>수정된 공지</h2><p>장학금 지급 일정이 변경되었습니다.</p>".into(),
    );
    fake.pages.lock().unwrap()[0].push(3);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let revised = notice(&state, "1");
    assert_ne!(revised.revision_hash, original.revision_hash);
    assert!(revised.body_text.contains("변경"));
    assert!(revised.historical, "a revision keeps its original marking");
    assert_eq!(notices(&state).len(), 3);
    assert!(!notice(&state, "3").historical);
}

#[tokio::test]
async fn an_iframe_only_body_never_replaces_readable_content() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![551681]]);
    let iframe = |src: &str| format!("<iframe src='https://embed.example.test/{src}'></iframe>");
    fake.changed.lock().unwrap().insert(551681, iframe("video"));
    historical(&state, &fake, Budget::default()).await;
    let initial = notice(&state, "551681");
    assert_eq!(initial.body_state, "unsupported");
    assert!(initial.body_text.is_empty());
    assert!(initial.title.ends_with("551681"));

    fake.changed
        .lock()
        .unwrap()
        .insert(551681, "<p>읽을 수 있는 기존 본문</p>".into());
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let readable = notice(&state, "551681");
    assert_eq!(readable.body_state, "normal");
    assert!(readable.body_text.contains("읽을 수 있는 기존 본문"));

    fake.changed
        .lock()
        .unwrap()
        .insert(551681, iframe("changed"));
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let after = notice(&state, "551681");
    assert_eq!(after.revision_hash, readable.revision_hash);
    assert!(after.body_text.contains("읽을 수 있는 기존 본문"));
    // Keeping the last good content is a checked notice, not a failure or a skip.
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("completed", 0));
    assert!(board.last_error.is_none() && skipped(&state).is_empty());
    assert!(!fake
        .paths()
        .iter()
        .any(|url| url.contains("embed.example.test")));
}

#[tokio::test]
async fn a_conditional_recheck_sends_the_saved_validator_and_keeps_the_notice() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![7]]);
    historical(&state, &fake, Budget::default()).await;
    let before = notice(&state, "7");
    fake.conditional.store(true, Ordering::Release);
    fake.calls.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let detail = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|(url, _)| url.ends_with("?pkId=7"))
        .cloned()
        .unwrap();
    assert_eq!(detail.1, Some((Some("\"article-7\"".into()), None)));
    assert_eq!(notice(&state, "7").revision_hash, before.revision_hash);
}

#[tokio::test]
async fn sitemap_entries_are_skipped_until_their_change_time_moves() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    serve_mechanical(&fake, "2026-09-28T15:31:48.462Z");
    let serve_sitemap = |lastmod: &str| serve_mechanical(&fake, lastmod);
    let detail_requests = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| *url == MECHANICAL_DETAIL)
            .count()
    };

    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert_eq!((notices(&state).len(), detail_requests(&fake)), (1, 1));
    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert_eq!(
        detail_requests(&fake),
        1,
        "unchanged lastmod is not refetched"
    );

    serve_sitemap("2026-09-29T09:00:00.000Z");
    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert_eq!((notices(&state).len(), detail_requests(&fake)), (1, 2));
    assert!(state
        .listing_unchanged(MECHANICAL, "1", "2026-09-29T09:00:00.000Z")
        .unwrap());
}

/// Serves the WordPress board: a one-post list envelope with the post changed
/// at `modified_gmt`, and that post (the real fixture with the same time).
fn serve_wordpress(fake: &Fake, modified_gmt: &str) -> (String, String) {
    let board = definition("creative-news");
    let list = rhythm_sources::parser::list_url(&board, 1).unwrap();
    let entry = ListingEntry {
        external_id: "47154".into(),
        url: "https://creative.sogang.ac.kr/?p=47154".into(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let post = rhythm_sources::parser::detail_url(&board, &entry).unwrap();
    let envelope = json!({
        "body": [{"id": 47154, "modified_gmt": modified_gmt}],
        "status": 200,
        "headers": {"X-WP-Total": 1, "X-WP-TotalPages": 1},
    });
    let body = include_str!("../../../sources/src/fixtures/wordpress-creative-post-47154.json")
        .replace("2026-11-20T07:26:08", modified_gmt);
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        list.to_string(),
        (envelope.to_string(), "application/json".into()),
    );
    extra.insert(post.to_string(), (body, "application/json".into()));
    (list.to_string(), post.to_string())
}

#[tokio::test]
async fn wordpress_posts_are_archived_from_json_and_skipped_while_unchanged() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    let (_, post) = serve_wordpress(&fake, "2026-11-20T07:26:08");
    let post_requests = |fake: &Fake| fake.paths().iter().filter(|url| **url == post).count();

    run(&state, &fake, "creative-news", Budget::default())
        .await
        .unwrap();
    assert_eq!((notices(&state).len(), post_requests(&fake)), (1, 1));
    let archived = notice(&state, "47154");
    assert_eq!(
        archived.title,
        "2026 Art & Technology Conference (ATC) 《레시피 바꾸지 말 것 *》"
    );
    run(&state, &fake, "creative-news", Budget::default())
        .await
        .unwrap();
    assert_eq!(
        post_requests(&fake),
        1,
        "an unchanged post is not refetched"
    );

    serve_wordpress(&fake, "2026-11-21T01:00:00");
    run(&state, &fake, "creative-news", Budget::default())
        .await
        .unwrap();
    assert_eq!((notices(&state).len(), post_requests(&fake)), (1, 2));
    assert!(state
        .listing_unchanged("creative-news", "47154", "2026-11-21T01:00:00Z")
        .unwrap());
}

/// A board that shrank below its saved history cursor.
fn shrunk_board() -> (State, Fake) {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![
        (1..=50).collect(),
        (51..=100).collect(),
        (101..=110).collect(),
    ]);
    fake.past_end.store(true, Ordering::Release);
    let mut board = state.board(MAIN).unwrap();
    board.next_page = 5;
    state.progress(&board, "running", None, false).unwrap();
    (state, fake)
}

#[tokio::test]
async fn a_history_cursor_past_a_shrunk_board_resumes_at_its_last_page() {
    let (state, fake) = shrunk_board();
    historical(&state, &fake, Budget::default()).await;
    let board = state.board(MAIN).unwrap();
    assert!(board.history_complete);
    assert_eq!((board.status.as_str(), board.next_page), ("completed", 4));
    assert_eq!(notices(&state).len(), 10, "the last page was read");
    assert!(notices(&state).iter().all(|n| n.historical));
}

#[tokio::test]
async fn a_failed_page_one_probe_leaves_the_cursor_and_the_failure() {
    let (state, fake) = shrunk_board();
    // A block page instead of the first listing proves nothing about the end.
    let first = parser::list_url(&definition(MAIN), 1).unwrap().to_string();
    fake.extra.lock().unwrap().insert(
        first,
        ("<html>blocked</html>".into(), "application/json".into()),
    );
    let result = run(&state, &fake, MAIN, Budget::default()).await;
    assert!(result.is_err());
    let board = state.board(MAIN).unwrap();
    assert!(!board.history_complete);
    assert_eq!((board.status.as_str(), board.next_page), ("failed", 5));
    assert!(notices(&state).is_empty());
}

#[tokio::test]
async fn a_rejected_cursor_below_the_declared_pages_is_not_past_the_end() {
    let (state, fake) = shrunk_board();
    // Page 3 of 3 is rejected although the cursor is not past the end.
    let mut board = state.board(MAIN).unwrap();
    board.next_page = 3;
    state.progress(&board, "running", None, false).unwrap();
    let third = parser::list_url(&definition(MAIN), 3).unwrap().to_string();
    fake.extra.lock().unwrap().insert(
        third,
        ("<html>blocked</html>".into(), "application/json".into()),
    );
    historical(&state, &fake, Budget::default()).await;
    let board = state.board(MAIN).unwrap();
    assert!(!board.history_complete);
    assert_eq!(board.next_page, 3);
    assert!(notices(&state).is_empty());
}

#[tokio::test]
async fn bounded_budgets_never_claim_complete_history() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2], vec![3]]);
    let articles = Budget {
        articles: Some(1),
        ..Budget::default()
    };
    historical(&state, &fake, articles).await;
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.next_page), ("limited", 1));
    assert!(!board.history_complete);
    assert_eq!(notices(&state).len(), 1);

    let pages = Budget {
        pages: Some(1),
        ..Budget::default()
    };
    historical(&state, &fake, pages).await;
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.next_page), ("limited", 2));
    assert!(!board.history_complete);
    assert_eq!(notices(&state).len(), 2);

    historical(&state, &fake, Budget::default()).await;
    assert_eq!(notices(&state).len(), 3);
    assert!(state.board(MAIN).unwrap().history_complete);
}

#[tokio::test]
async fn a_burst_beyond_three_pages_is_discovered_without_pins_as_frontier() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2]]);
    historical(&state, &fake, Budget::default()).await;
    let mut pages: Vec<Vec<u64>> = (0..4)
        .map(|page| {
            std::iter::once(1)
                .chain((100 + page * 10)..(110 + page * 10))
                .collect()
        })
        .collect();
    pages.push(vec![1, 2]);
    *fake.pages.lock().unwrap() = pages;
    fake.calls.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(notices(&state).len(), 42);
    assert!(fake.paths().iter().any(|url| url.contains("pageNum=5")));
    assert_eq!(state.board(MAIN).unwrap().status, "completed");
}

#[tokio::test]
async fn known_notices_are_rechecked_only_within_thirty_regular_rows() {
    let state = State::open_in_memory().unwrap();
    // Three full 50-row pages, a pin (1) on the first.
    let fake = Fake::new(vec![
        (1..=50).collect(),
        (51..=100).collect(),
        (101..=150).collect(),
    ]);
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(notices(&state).len(), 150);
    fake.calls.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let fetched: HashSet<u64> = details(&fake).into_iter().map(|(_, id)| id).collect();
    // The pin and the first 30 regular rows (2..=31), nothing further down.
    assert_eq!(fetched, (1..=31).collect());
    let lists: Vec<_> = fake
        .paths()
        .into_iter()
        .filter(|url| url.contains("/boardList"))
        .collect();
    assert_eq!(lists.len(), 1, "page 1 holds the whole window: {lists:?}");
    assert_eq!(state.board(MAIN).unwrap().status, "completed");

    // New notices on top still arrive, however far they push the window.
    fake.pages.lock().unwrap()[0] = std::iter::once(1).chain(151..=185).chain(2..=15).collect();
    fake.calls.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(notices(&state).len(), 185);
    assert!(!notice(&state, "185").historical);
    let fetched: HashSet<u64> = details(&fake).into_iter().map(|(_, id)| id).collect();
    assert!(fetched.is_disjoint(&(2..=15).collect()), "{fetched:?}");
}

#[tokio::test]
async fn a_main_notice_over_four_mib_is_archived() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3]]);
    // Inline data: images make some main-site notices larger than 4 MiB.
    fake.changed.lock().unwrap().insert(
        2,
        format!(
            "<p>큰 이미지가 있는 공지</p><img src=\"data:image/png;base64,{}\">",
            "A".repeat(6 * 1024 * 1024)
        ),
    );
    historical(&state, &fake, Budget::default()).await;
    let board = state.board(MAIN).unwrap();
    assert!(board.history_complete, "{:?}", board.last_error);
    assert!(notice(&state, "2")
        .body_text
        .contains("큰 이미지가 있는 공지"));
    assert!(skipped(&state).is_empty());
}

#[tokio::test]
async fn the_first_incremental_pass_seeds_three_pages_as_history() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    let crawl = Crawl {
        state: &state,
        transport: &fake,
        cancel: &AtomicBool::new(false),
        budget: Budget::default(),
        policies: &Policies::default(),
    };
    let mut board = state.board(MAIN).unwrap();
    crawl
        .pass(&definition(MAIN), &mut board, Pass::Incremental)
        .await
        .unwrap();
    assert_eq!(notices(&state).len(), 4);
    assert!(notices(&state).iter().all(|n| n.historical));
    assert!(state.seed_complete(MAIN).unwrap());
    let board = state.board(MAIN).unwrap();
    assert!(!board.history_complete);
    assert_eq!(board.next_page, 1);
    assert!(!fake.paths().iter().any(|url| url.contains("pageNum=4")));
}

#[tokio::test]
async fn a_failed_seed_stays_pending_until_a_safe_completion() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    fake.failed.lock().unwrap().insert(3);
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    let board = state.board(MAIN).unwrap();
    assert_eq!(
        (board.status.as_str(), board.consecutive_failures),
        ("failed", 1)
    );
    assert!(!state.seed_complete(MAIN).unwrap());

    fake.failed.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert!(state.seed_complete(MAIN).unwrap());
    assert_eq!(notices(&state).len(), 5, "history continued after the seed");
    assert!(notices(&state).iter().all(|n| n.historical));
    let board = state.board(MAIN).unwrap();
    assert!(board.history_complete);
    assert_eq!(board.consecutive_failures, 0);
}

#[tokio::test]
async fn a_permanent_failure_is_skipped_but_a_transient_one_holds_the_page() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3], vec![4]]);
    fake.bad.lock().unwrap().insert(3);
    historical(&state, &fake, Budget::default()).await;
    let board = state.board(MAIN).unwrap();
    assert_eq!(board.status, "completed");
    assert!(board.history_complete, "the last public page was reached");
    assert_eq!((board.next_page, board.failed), (3, 0));
    assert_eq!(skipped(&state), ["3"]);
    let ids: Vec<_> = notices(&state).into_iter().map(|n| n.external_id).collect();
    assert_eq!(ids, ["2", "4"]);

    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3], vec![4]]);
    fake.failed.lock().unwrap().insert(3);
    historical(&state, &fake, Budget::default()).await;
    let board = state.board(MAIN).unwrap();
    assert!(!board.history_complete);
    assert_eq!(board.next_page, 1);
    assert!(skipped(&state).is_empty());

    // Passes leave a skipped notice alone; the daily retry that archives it
    // removes it from the skipped list.
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3], vec![4]]);
    fake.bad.lock().unwrap().insert(3);
    historical(&state, &fake, Budget::default()).await;
    fake.bad.lock().unwrap().clear();
    fake.pages.lock().unwrap()[0].insert(0, 5);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(
        requests(&fake, &main_detail(3)),
        1,
        "not refetched by the pass"
    );
    assert_eq!(skipped(&state), ["3"]);
    exec(
        &state,
        &format!("UPDATE skipped SET recorded_at={DAY_AGO} WHERE external_id='3'"),
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(skipped(&state), Vec::<String>::new());
    assert_eq!(notices(&state).len(), 4);
}

/// Runs of one notice's transient failures recorded so far.
fn transient_runs(state: &State, external_id: &str) -> Option<i64> {
    state
        .with(|c| {
            c.query_row(
                "SELECT runs FROM transient_failures WHERE external_id=?1",
                [external_id],
                |row| row.get(0),
            )
            .optional()
        })
        .unwrap()
}
/// Moves the last transient failure back past the one-run gap.
fn next_run(state: &State) {
    exec(
        state,
        "UPDATE transient_failures SET last_failed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','-11 minutes')",
    );
}

#[tokio::test]
async fn a_notice_failing_transiently_on_three_runs_is_skipped() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3, 4]]);
    fake.failed.lock().unwrap().insert(3);
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    // A second failure within the same run's time counts once.
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    assert_eq!(transient_runs(&state, "3"), Some(1));
    next_run(&state);
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    assert_eq!(transient_runs(&state, "3"), Some(2));
    assert!(skipped(&state).is_empty());
    assert!(
        !state.seed_complete(MAIN).unwrap(),
        "the page is still held"
    );

    next_run(&state);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(skipped(&state), ["3"]);
    assert_eq!(transient_runs(&state, "3"), None);
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("completed", 0));
    assert!(board.history_complete);
    assert!(state.seed_complete(MAIN).unwrap());

    // A failed daily retry uses an attempt and waits another day.
    let fetched = requests(&fake, &main_detail(3));
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(
        requests(&fake, &main_detail(3)),
        fetched,
        "not before a day"
    );
    exec(
        &state,
        &format!("UPDATE skipped SET recorded_at={DAY_AGO} WHERE external_id='3'"),
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, &main_detail(3)), fetched + 1);
    let attempts: i64 = state
        .with(|c| c.query_row("SELECT attempts FROM skipped", [], |row| row.get(0)))
        .unwrap();
    assert_eq!(attempts, 2);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, &main_detail(3)), fetched + 1);

    // Once the server answers, the retry archives it.
    fake.failed.lock().unwrap().clear();
    exec(
        &state,
        &format!("UPDATE skipped SET recorded_at={DAY_AGO} WHERE external_id='3'"),
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert!(skipped(&state).is_empty());
    assert_eq!(notices(&state).len(), 3);
}

#[tokio::test]
async fn a_success_resets_the_transient_failure_count() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3, 4]]);
    fake.failed.lock().unwrap().insert(3);
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    next_run(&state);
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    assert_eq!(transient_runs(&state, "3"), Some(2));
    fake.failed.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(transient_runs(&state, "3"), None);
    assert!(skipped(&state).is_empty());
    assert_eq!(notices(&state).len(), 3);
}

#[tokio::test]
async fn a_rate_limited_notice_stops_the_board_without_requesting_the_rest() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3, 4]]);
    fake.rate_limited.lock().unwrap().insert(3);
    let error = run(&state, &fake, MAIN, Budget::default())
        .await
        .unwrap_err();
    assert!(error.contains(network::RATE_LIMITED));
    assert!(!fake.paths().iter().any(|url| url.contains("pkId=4")));
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("failed", 1));
    assert!(
        skipped(&state).is_empty(),
        "a refusal is not a skipped notice"
    );
    assert_eq!(notices(&state).len(), 1);
}

#[tokio::test]
async fn robots_disallow_prevents_every_article_request() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2]]);
    *fake.robots.lock().unwrap() = "User-agent: *\nDisallow: /api/api/v1/mainKo/BbsData\n".into();
    assert!(run(&state, &fake, MAIN, Budget::default()).await.is_err());
    assert!(!fake.paths().iter().any(|url| url.contains("pkId=")));
    assert!(notices(&state).is_empty());
}

#[tokio::test]
async fn identical_pins_only_pages_are_not_a_repeated_listing() {
    // sociology-news: the 14 newest notices are all pinned, so pages 1 and 2
    // list the same pins and nothing else.
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1], vec![1], vec![1, 2, 3]]);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let ids: Vec<_> = notices(&state).into_iter().map(|n| n.external_id).collect();
    assert_eq!(ids, ["1", "2", "3"]);
    assert!(state.board(MAIN).unwrap().history_complete);

    // A repeated page of regular rows still stops the board.
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![2, 3], vec![2, 3], vec![4]]);
    let error = run(&state, &fake, MAIN, Budget::default())
        .await
        .unwrap_err();
    assert!(error.contains("반복"), "{error}");
    assert!(!state.board(MAIN).unwrap().history_complete);
}

#[tokio::test]
async fn a_page_of_only_pins_is_not_a_known_frontier() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2]]);
    historical(&state, &fake, Budget::default()).await;
    // Page 3 holds only the known pin; the burst continues on page 4.
    *fake.pages.lock().unwrap() = vec![
        std::iter::once(1).chain(100..110).collect(),
        std::iter::once(1).chain(110..120).collect(),
        vec![1],
        std::iter::once(1).chain(120..130).collect(),
        vec![1, 2],
    ];
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert!(fake.paths().iter().any(|url| url.contains("pageNum=4")));
    assert_eq!(notices(&state).len(), 32);
}

/// Answers every request slowly with a failure, recording how many requests
/// are in flight per server (its host unless `servers` names one) and in total.
#[derive(Default)]
struct Slow {
    inflight: Mutex<HashMap<String, usize>>,
    host_peak: AtomicU64,
    total_peak: AtomicU64,
    requests: AtomicU64,
    servers: HashMap<String, IpAddr>,
}
impl Transport for Slow {
    async fn server(&self, host: &str) -> Option<IpAddr> {
        self.servers.get(host).copied()
    }
    async fn fetch(
        &self,
        url: &Url,
        _validators: Validators,
        _limit: usize,
        _cancel: &AtomicBool,
        _access: Access,
    ) -> Result<Fetch, FetchError> {
        let host = url.host_str().unwrap().to_owned();
        let host = self
            .servers
            .get(&host)
            .map_or(host, |server| server.to_string());
        {
            let mut inflight = self.inflight.lock().unwrap();
            let count = inflight.entry(host.clone()).or_default();
            *count += 1;
            self.host_peak.fetch_max(*count as u64, Ordering::AcqRel);
            let total: usize = inflight.values().sum();
            self.total_peak.fetch_max(total as u64, Ordering::AcqRel);
        }
        self.requests.fetch_add(1, Ordering::AcqRel);
        tokio::time::sleep(Duration::from_millis(5)).await;
        *self.inflight.lock().unwrap().get_mut(&host).unwrap() -= 1;
        Err("모의 학교 서버 연결 실패".into())
    }
}

#[tokio::test]
async fn boards_run_three_hosts_at_once_and_one_board_per_host() {
    let state = State::open_in_memory().unwrap();
    let slow = Slow::default();
    let boards = public_boards();
    let outcomes = run_boards(
        &state,
        &slow,
        &boards,
        Budget::default(),
        &AtomicBool::new(false),
    )
    .await
    .outcomes;
    let ids: Vec<_> = outcomes.iter().map(|(id, _)| id.as_str()).collect();
    let expected: Vec<_> = boards.iter().map(|board| board.id.as_str()).collect();
    assert_eq!(ids, expected);
    assert!(outcomes.iter().all(|(_, outcome)| outcome.is_err()));
    assert_eq!(slow.host_peak.load(Ordering::Acquire), 1);
    assert_eq!(slow.total_peak.load(Ordering::Acquire), 3);
    assert_eq!(state.board(MAIN).unwrap().status, "failed");
}

#[tokio::test]
async fn a_cancelled_run_starts_no_further_board() {
    let state = State::open_in_memory().unwrap();
    let slow = Slow::default();
    let outcomes = run_boards(
        &state,
        &slow,
        &public_boards(),
        Budget::default(),
        &AtomicBool::new(true),
    )
    .await
    .outcomes;
    assert!(outcomes.is_empty());
    assert_eq!(slow.requests.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn boards_on_one_server_share_one_worker_whatever_their_hostnames() {
    let state = State::open_in_memory().unwrap();
    let shared: IpAddr = "163.239.3.91".parse().unwrap();
    let slow = Slow {
        servers: [
            "ee.sogang.ac.kr",
            "sse.sogang.ac.kr",
            "new-cms.sogang.ac.kr",
        ]
        .into_iter()
        .map(|host| (host.to_owned(), shared))
        .collect(),
        ..Slow::default()
    };
    let report = run_boards(
        &state,
        &slow,
        &public_boards(),
        Budget::default(),
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(report.outcomes.len(), public_boards().len());
    assert_eq!(slow.host_peak.load(Ordering::Acquire), 1);
    assert_eq!(slow.total_peak.load(Ordering::Acquire), 3);
    // The log names each server group once, with all three hostnames together
    // in catalog order.
    let boards: usize = report.groups.iter().map(|group| group.boards).sum();
    assert_eq!(boards, public_boards().len());
    let shared = report
        .groups
        .iter()
        .find(|group| group.hosts.iter().any(|host| host == "sse.sogang.ac.kr"))
        .unwrap();
    assert_eq!(
        shared.hosts,
        [
            "new-cms.sogang.ac.kr",
            "ee.sogang.ac.kr",
            "sse.sogang.ac.kr"
        ]
    );
}

/// Two main boards whose history is owed after their first incremental pass:
/// the seed stops at page 3 of 6, one notice per page.
fn owing_history() -> (State, Fake, Vec<SourceDefinition>) {
    let fake = Fake::new((5..=10).rev().map(|id| vec![id]).collect());
    (
        State::open_in_memory().unwrap(),
        fake,
        vec![definition(MAIN), definition(GENERAL)],
    )
}
/// Detail requests in order, as (board config, notice ID).
fn details(fake: &Fake) -> Vec<(u64, u64)> {
    fake.paths()
        .iter()
        .filter_map(|url| {
            url.split_once("pkId=")
                .map(|(_, id)| id.parse::<u64>().unwrap())
        })
        .map(|id| match id >= GENERAL_IDS {
            true => (1, id - GENERAL_IDS),
            false => (2, id),
        })
        .collect()
}

#[tokio::test]
async fn new_notices_on_every_board_come_before_any_history() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new((5..=10).rev().map(|id| vec![id]).collect());
    serve_mechanical(&fake, "2026-09-28T15:31:48.462Z");
    // One server: one worker takes both boards in turn.
    let server: IpAddr = "163.239.1.17".parse().unwrap();
    for host in ["www.sogang.ac.kr", "me.sogang.ac.kr"] {
        fake.servers.lock().unwrap().insert(host.into(), server);
    }
    let boards = [definition(MAIN), definition(MECHANICAL)];
    let report = run_boards(
        &state,
        &fake,
        &boards,
        Budget::default(),
        &AtomicBool::new(false),
    )
    .await;
    assert!(report.outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
    let paths = fake.paths();
    let position = |url: &str| paths.iter().position(|path| path == url).unwrap();
    // Notice 7 is on page 4, past the seed: only the history pass reaches it.
    assert!(position(MECHANICAL_SITEMAP) < position(&main_detail(7)));
    assert_eq!(report.history_owed, 0);
    assert!(state.board(MAIN).unwrap().history_complete);
}

#[tokio::test]
async fn the_deadline_stops_history_but_never_new_notices() {
    let (state, fake, boards) = owing_history();
    let past = Budget {
        history_until: Some(Instant::now()),
        ..Budget::default()
    };
    let report = run_boards(&state, &fake, &boards, past, &AtomicBool::new(false)).await;
    assert!(report.outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
    assert_eq!(report.history_owed, 2);
    // Each board's seed (pages 1 to 3) and nothing beyond it.
    let mut fetched = details(&fake);
    fetched.sort_unstable();
    assert_eq!(fetched, [(1, 8), (1, 9), (1, 10), (2, 8), (2, 9), (2, 10)]);

    // A history turn the deadline stops is limited, never a failure.
    run_board(
        &state,
        &fake,
        &definition(MAIN),
        past,
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    let board = state.board(MAIN).unwrap();
    assert_eq!(board.status, "limited");
    assert_eq!(board.last_error.as_deref(), Some(HISTORY_DEADLINE));
    assert_eq!(board.consecutive_failures, 0);
    assert_eq!(board.next_page, 1);
    assert!(!board.history_complete);
}

#[tokio::test]
async fn robots_txt_loads_once_per_origin_and_run() {
    // Two boards on one origin, each with an incremental pass, upkeep and
    // several history turns.
    let (state, fake, boards) = owing_history();
    let turns = Budget {
        articles: Some(1),
        history_until: Some(Instant::now() + Duration::from_secs(60)),
        ..Budget::default()
    };
    let report = run_boards(&state, &fake, &boards, turns, &AtomicBool::new(false)).await;
    assert!(report.outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
    assert_eq!(report.history_owed, 0);
    let robots = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| url.as_str() == "https://www.sogang.ac.kr/robots.txt")
            .count()
    };
    assert_eq!(robots(&fake), 1);

    // The next run loads it again.
    run_boards(&state, &fake, &boards, turns, &AtomicBool::new(false)).await;
    assert_eq!(robots(&fake), 2);
}

#[tokio::test]
async fn history_takes_turns_until_the_deadline_least_progress_first() {
    let (state, fake, boards) = owing_history();
    let turns = Budget {
        articles: Some(1),
        history_until: Some(Instant::now() + Duration::from_secs(60)),
        ..Budget::default()
    };
    let report = run_boards(&state, &fake, &boards, turns, &AtomicBool::new(false)).await;
    assert!(report.outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
    assert_eq!(report.history_owed, 0);
    assert!(state.board(MAIN).unwrap().history_complete);
    assert!(state.board(GENERAL).unwrap().history_complete);
    // One notice per turn, alternating between the boards (equal progress,
    // then source ID), instead of one board's whole backlog first.
    let history: Vec<_> = details(&fake)
        .into_iter()
        .filter(|(_, id)| *id < 10)
        .collect();
    assert_eq!(
        history,
        [
            (2, 9),
            (1, 9),
            (2, 8),
            (1, 8),
            (2, 7),
            (1, 7),
            (2, 6),
            (1, 6),
            (2, 5),
            (1, 5)
        ]
    );
}

#[tokio::test]
async fn every_server_gets_history_turns_before_the_deadline() {
    // Five servers with endless backlogs, three workers, room for 15 turns.
    let groups = (0..5)
        .map(|server| {
            vec![Turn {
                page: 1,
                id: format!("board-{server}"),
                index: server,
            }]
        })
        .collect();
    let taken = Mutex::new(Vec::new());
    take_turns(
        groups,
        || taken.lock().unwrap().len() >= 15,
        |index| {
            let taken = &taken;
            async move {
                taken.lock().unwrap().push(index);
                tokio::task::yield_now().await;
                Some(2)
            }
        },
    )
    .await;
    let taken = taken.into_inner().unwrap();
    for server in 0..5 {
        let turns = taken.iter().filter(|index| **index == server).count();
        assert!(turns >= 2, "server {server} got {turns} turns: {taken:?}");
    }
}

#[tokio::test]
async fn without_a_deadline_each_board_gets_one_history_turn() {
    let (state, fake, boards) = owing_history();
    let once = Budget {
        articles: Some(1),
        ..Budget::default()
    };
    let report = run_boards(&state, &fake, &boards, once, &AtomicBool::new(false)).await;
    assert_eq!(report.history_owed, 2);
    let history: Vec<_> = details(&fake)
        .into_iter()
        .filter(|(_, id)| *id < 10)
        .collect();
    assert_eq!(history, [(2, 9), (1, 9)]);
}

fn main_detail(id: u64) -> String {
    format!("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData?pkId={id}")
}
fn requests(fake: &Fake, url: &str) -> usize {
    fake.paths().iter().filter(|path| *path == url).count()
}
fn exec(state: &State, sql: &str) {
    state.with(|c| c.execute_batch(sql)).unwrap();
}
/// `missing_since` and `removed_at` of a notice.
fn deletion(state: &State, external_id: &str) -> (Option<String>, Option<String>) {
    state
        .with(|c| {
            c.query_row(
                "SELECT missing_since,removed_at FROM notices WHERE external_id=?1",
                [external_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .unwrap()
}
const DAY_AGO: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now','-25 hours')";

#[tokio::test]
async fn a_notice_is_removed_only_after_two_not_found_answers_a_day_apart() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![4, 3, 2]]);
    historical(&state, &fake, Budget::default()).await;
    // The board now lists only a new notice. 2 and 3 are recent and due for
    // their weekly audit; 4 is older than the 90-day window.
    *fake.pages.lock().unwrap() = vec![vec![5]];
    exec(
        &state,
        "UPDATE notices SET checked_at='2026-01-01T00:00:00.000Z',published_at=date('now');
         UPDATE notices SET published_at=date('now','-91 days') WHERE external_id='4';",
    );
    fake.not_found.lock().unwrap().insert(main_detail(2));
    fake.failed.lock().unwrap().insert(3);

    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    let first = deletion(&state, "2");
    assert!(first.0.is_some() && first.1.is_none(), "{first:?}");
    // A connection failure is not an answer.
    assert_eq!(deletion(&state, "3"), (None, None));
    assert_eq!(requests(&fake, &main_detail(4)), 1, "outside the window");
    let board = state.board(MAIN).unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("completed", 0));

    // Within a day the notice is not asked again and stays.
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, &main_detail(2)), 2);
    assert_eq!(deletion(&state, "2"), first);

    exec(
        &state,
        &format!("UPDATE notices SET missing_since={DAY_AGO} WHERE external_id='2'"),
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert!(
        deletion(&state, "2").1.is_some(),
        "second answer a day later"
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(
        requests(&fake, &main_detail(2)),
        3,
        "a removed notice is not audited"
    );

    // Listed and answered again, it is restored.
    fake.not_found.lock().unwrap().clear();
    *fake.pages.lock().unwrap() = vec![vec![5, 2]];
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(deletion(&state, "2"), (None, None));
}

#[tokio::test]
async fn the_mechanical_audit_checks_only_notices_missing_from_the_sitemap() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    let url = |id: u32| format!("https://me.sogang.ac.kr/ko/board/notice/{id}");
    let sitemap = |ids: &[u32]| {
        let urls: String = ids
            .iter()
            .map(|id| {
                format!(
                    "<url><loc>{}</loc><lastmod>2026-09-28T15:31:48.462Z</lastmod></url>",
                    url(*id)
                )
            })
            .collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{urls}</urlset>"
        )
    };
    let detail = |id: u32| {
        format!(
            "<!doctype html><html><head><link rel=\"canonical\" href=\"{}\"/></head>\
             <body><article class=\"container-site py-14 max-w-4xl\">\
             <header class=\"border-b-2 border-sg-ink pb-7\">\
             <span class=\"eyebrow\">일반공지</span><h1 class=\"mt-3\">기계공학과 공지 {id}</h1>\
             <div class=\"mt-5 flex\"><span>작성자 · 기계공학과</span>\
             <span>날짜 · 2026.09.28</span><span>조회 · 1</span></div></header>\
             <div class=\"prose-sg mt-8 min-h-[120px]\"><p>본문입니다.</p></div>\
             </article></body></html>",
            url(id)
        )
    };
    let serve = |ids: &[u32]| {
        fake.extra.lock().unwrap().insert(
            MECHANICAL_SITEMAP.into(),
            (sitemap(ids), "application/xml".into()),
        );
    };
    for id in [1, 2] {
        fake.extra
            .lock()
            .unwrap()
            .insert(url(id), (detail(id), "text/html".into()));
    }
    serve(&[1, 2]);
    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert_eq!(notices(&state).len(), 2);

    // 2 leaves the sitemap; both are due for the weekly audit, at any age.
    serve(&[1]);
    fake.not_found.lock().unwrap().insert(url(2));
    exec(
        &state,
        "UPDATE notices SET checked_at='2026-01-01T00:00:00.000Z',published_at='2020-01-01'",
    );
    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert_eq!(
        requests(&fake, &url(1)),
        1,
        "a listed notice is not audited"
    );
    assert!(deletion(&state, "2").0.is_some());
    exec(
        &state,
        &format!("UPDATE notices SET missing_since={DAY_AGO} WHERE external_id='2'"),
    );
    run(&state, &fake, MECHANICAL, Budget::default())
        .await
        .unwrap();
    assert!(deletion(&state, "2").1.is_some());
    assert_eq!(deletion(&state, "1"), (None, None));
}

#[tokio::test]
async fn skipped_notices_are_retried_daily_within_bounded_attempts() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![6, 3, 2]]);
    fake.bad.lock().unwrap().extend([2, 6]);
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(skipped(&state), ["2", "6"]);
    // Both fall off the listing, so only the retry reaches them.
    *fake.pages.lock().unwrap() = vec![vec![3]];
    let attempts = |state: &State| -> (i64, String) {
        state
            .with(|c| {
                c.query_row(
                    "SELECT attempts,collector_version FROM skipped WHERE external_id='2'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            })
            .unwrap()
    };
    assert_eq!(attempts(&state), (1, crate::VERSION.to_string()));
    let due = |state: &State| {
        exec(
            state,
            &format!("UPDATE skipped SET recorded_at={DAY_AGO} WHERE external_id='2'"),
        )
    };

    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, &main_detail(2)), 1, "not before a day");

    due(&state);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(attempts(&state).0, 2);

    exec(
        &state,
        "UPDATE skipped SET attempts=7 WHERE external_id='2'",
    );
    due(&state);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, &main_detail(2)), 2, "attempts used up");

    // A new collector version gets fresh attempts.
    exec(
        &state,
        "UPDATE skipped SET collector_version='0.0.1' WHERE external_id='2'",
    );
    due(&state);
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert_eq!(attempts(&state), (1, crate::VERSION.to_string()));

    // Parsed at last, it is archived; answered "not found", it is dropped.
    fake.bad.lock().unwrap().remove(&2);
    fake.not_found.lock().unwrap().insert(main_detail(6));
    due(&state);
    exec(
        &state,
        &format!("UPDATE skipped SET recorded_at={DAY_AGO} WHERE external_id='6'"),
    );
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    assert!(skipped(&state).is_empty());
    assert_eq!(notice(&state, "2").title, "장학금 AI 연구 공지 2");
    assert!(notices(&state).iter().all(|n| n.external_id != "6"));
}

const INNOV: &str = "innov-newsletter";
const INNOV_LIST: &str = "https://new-cms.sogang.ac.kr/innov/10558/subview.do?page=1";
const INNOV_DETAIL: &str = "https://new-cms.sogang.ac.kr/bbs/innov/3184/405464/artclView.do";

/// Serves the k2web gallery fixture (five posts on one page): the captured
/// notice 405464, and the CMS's missing-post page for the other four.
fn serve_innov(fake: &Fake, title: &str) {
    let html = |text: &str| {
        (
            text.replace("서강의 혁신을 기록하다 Vol.5", title),
            "text/html".to_string(),
        )
    };
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        INNOV_LIST.into(),
        html(include_str!(
            "../../../sources/src/fixtures/k2web-innov-list.html"
        )),
    );
    extra.insert(
        INNOV_DETAIL.into(),
        html(include_str!(
            "../../../sources/src/fixtures/k2web-innov-detail-405464.html"
        )),
    );
    for id in [374715, 374716, 374718, 374719] {
        extra.insert(
            format!("https://new-cms.sogang.ac.kr/bbs/innov/3184/{id}/artclView.do"),
            html(
                "<html><body><div id=\"errorMessege\"><p class=\"txt\">\
                  <strong>게시물이(가) 없습니다.</strong></p></div></body></html>",
            ),
        );
    }
}

#[tokio::test]
async fn a_k2web_notice_is_refetched_only_when_its_list_title_changes() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    serve_innov(&fake, "서강의 혁신을 기록하다 Vol.5");
    let detail_requests = |fake: &Fake| {
        fake.paths()
            .iter()
            .filter(|url| *url == INNOV_DETAIL)
            .count()
    };

    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert!(state.seed_complete(INNOV).unwrap());
    let title = notice(&state, "405464").title;
    assert_eq!(detail_requests(&fake), 1);

    // The list still shows the archived title: no request.
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(detail_requests(&fake), 1);

    // A changed title, or a notice reported missing, is fetched again.
    serve_innov(&fake, "서강의 혁신을 기록하다 Vol.5 (수정)");
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(detail_requests(&fake), 2);
    assert_eq!(notice(&state, "405464").title, format!("{title} (수정)"));
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(detail_requests(&fake), 2);
    exec(
        &state,
        "UPDATE notices SET missing_since='2026-11-20T00:00:00Z' WHERE external_id='405464'",
    );
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(detail_requests(&fake), 3);
    assert_eq!(deletion(&state, "405464"), (None, None));
}

#[tokio::test]
async fn a_k2web_list_title_cut_short_matches_the_archived_title() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    serve_innov(&fake, "서강의 혁신을 기록하다 Vol.5");
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, INNOV_DETAIL), 1);
    let list = |title: &str| {
        let mut extra = fake.extra.lock().unwrap();
        let page = extra.get_mut(INNOV_LIST).unwrap();
        page.0 = include_str!("../../../sources/src/fixtures/k2web-innov-list.html")
            .replace("서강의 혁신을 기록하다 Vol.5", title);
    };

    // The CMS cuts long titles in its lists; the article shows them whole.
    list("서강의 혁신을 기록하다 V");
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, INNOV_DETAIL), 1);

    // Any other listed title is a change.
    list("서강의 혁신을 기록하다 Vol.6");
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(requests(&fake, INNOV_DETAIL), 2);
}

#[tokio::test]
async fn new_boards_start_a_bounded_number_per_run_in_catalog_order() {
    let state = State::open_in_memory().unwrap();
    let boards = public_boards();
    // Every board is new: only the first ones start.
    let (started, waiting) = admitted(&state, boards.clone()).unwrap();
    assert_eq!(started.len(), NEW_BOARDS_PER_RUN);
    assert_eq!(waiting, boards.len() - NEW_BOARDS_PER_RUN);
    assert!(started
        .iter()
        .zip(&boards)
        .all(|(started, board)| started.id == board.id));

    // Boards past their first pass always run, wherever they are listed; a
    // started board that has not finished keeps its turn.
    let last = boards.last().unwrap().id.clone();
    for board in boards.iter().take(5).chain(boards.last()) {
        state.board(&board.id).unwrap();
        if board.id != boards[0].id {
            state.mark_seed_complete(&board.id).unwrap();
        }
    }
    let (started, waiting) = admitted(&state, boards.clone()).unwrap();
    let ids: Vec<_> = started.iter().map(|board| board.id.as_str()).collect();
    // The unfinished first board, four done ones, the next nineteen new ones
    // and the last board.
    let mut expected: Vec<_> = boards[..4 + NEW_BOARDS_PER_RUN]
        .iter()
        .map(|board| board.id.as_str())
        .collect();
    expected.push(&last);
    assert_eq!(ids, expected);
    assert_eq!(waiting, boards.len() - ids.len());
}

#[tokio::test]
async fn a_k2web_notice_over_its_limit_is_skipped_without_holding_the_board() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    serve_innov(&fake, "서강의 혁신을 기록하다 Vol.5");
    let detail = include_str!("../../../sources/src/fixtures/k2web-innov-detail-405464.html");
    // Over 4 MiB is still a k2web notice (inline images the sanitizer drops).
    let padded = detail.replacen(
        "</body>",
        &format!("<!--{}--></body>", "x".repeat(5 << 20)),
        1,
    );
    fake.extra
        .lock()
        .unwrap()
        .insert(INNOV_DETAIL.into(), (padded, "text/html".into()));
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert_eq!(notices(&state).len(), 1);

    // Over 16 MiB the same page would come back every time: skipped, and the
    // board completes instead of failing on it.
    let state = State::open_in_memory().unwrap();
    let huge = detail.replacen(
        "</body>",
        &format!("<!--{}--></body>", "x".repeat(17 << 20)),
        1,
    );
    fake.extra
        .lock()
        .unwrap()
        .insert(INNOV_DETAIL.into(), (huge, "text/html".into()));
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    assert!(notices(&state).is_empty());
    assert_eq!(skipped(&state), ["405464"]);
    let board = state.board(INNOV).unwrap();
    assert_eq!(board.status, "completed");
    assert!(state.seed_complete(INNOV).unwrap());
}

#[tokio::test]
async fn a_k2web_poster_only_notice_is_archived_metadata_only() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    serve_innov(&fake, "서강의 혁신을 기록하다 Vol.5");
    // The whole body one inline `data:` image, and no files.
    let detail = include_str!("../../../sources/src/fixtures/k2web-innov-detail-405464.html");
    let body = detail.find("<div class=\"txt\">").unwrap() + "<div class=\"txt\">".len();
    let files = detail.find("<div class=\"attachment\">").unwrap();
    let list_end = files + detail[files..].find("</ul>").unwrap() + "</ul>".len();
    let poster = format!(
        "{}<p><img src=\"data:image/png;base64,iVBORw0KGgo=\"></p></div>\
         <div class=\"attachment\"><span class=\"no-file\">첨부파일이(가) 없습니다.</span>{}",
        &detail[..body],
        &detail[list_end..]
    );
    fake.extra
        .lock()
        .unwrap()
        .insert(INNOV_DETAIL.into(), (poster, "text/html".into()));
    run(&state, &fake, INNOV, Budget::default()).await.unwrap();
    let archived = notice(&state, "405464");
    assert_eq!(archived.body_state, "unsupported");
    assert!(archived.body_text.is_empty());
    assert!(skipped(&state).is_empty());
    assert_eq!(state.board(INNOV).unwrap().failed, 0);
}

#[tokio::test]
async fn a_skipped_notice_counts_as_known_for_the_frontier() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![(1..=50).collect()]);
    fake.bad.lock().unwrap().insert(40);
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(skipped(&state), ["40"]);
    // Older notices the history import has not reached yet appear below.
    fake.pages
        .lock()
        .unwrap()
        .extend([(51..=100).collect(), (101..=150).collect()]);
    fake.calls.lock().unwrap().clear();
    run(&state, &fake, MAIN, Budget::default()).await.unwrap();
    // Page 1 is the known frontier: the pass neither pages on nor fetches the
    // older notices as new ones.
    assert!(
        !fake.paths().iter().any(|url| url.contains("pageNum=2")),
        "{:?}",
        fake.paths()
    );
    assert_eq!(notices(&state).len(), 49);
    assert!(notices(&state).iter().all(|notice| notice.historical));
    assert_eq!(
        requests(&fake, &main_detail(40)),
        0,
        "not refetched by the pass"
    );
}

#[tokio::test]
async fn history_retries_a_notice_an_older_collector_version_skipped() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![vec![1, 2], vec![1, 3], vec![1, 4], vec![1, 5]]);
    fake.bad.lock().unwrap().insert(3);
    let crawl = Crawl {
        state: &state,
        transport: &fake,
        cancel: &AtomicBool::new(false),
        budget: Budget::default(),
        policies: &Policies::default(),
    };
    let mut board = state.board(MAIN).unwrap();
    crawl
        .pass(&definition(MAIN), &mut board, Pass::Incremental)
        .await
        .unwrap();
    assert_eq!(skipped(&state), ["3"]);
    fake.bad.lock().unwrap().clear();

    // Skipped by this version: history leaves it to the daily retry.
    historical(
        &state,
        &fake,
        Budget {
            pages: Some(3),
            ..Budget::default()
        },
    )
    .await;
    assert_eq!(requests(&fake, &main_detail(3)), 1);

    // Skipped by an older version (a parser fix since): the next pass of
    // history over its page retries it, the incremental pass does not.
    exec(&state, "UPDATE boards SET next_page=1");
    exec(&state, "UPDATE skipped SET collector_version='0.0.1'");
    let mut board = state.board(MAIN).unwrap();
    crawl
        .pass(&definition(MAIN), &mut board, Pass::Incremental)
        .await
        .unwrap();
    assert_eq!(requests(&fake, &main_detail(3)), 1);
    historical(&state, &fake, Budget::default()).await;
    assert_eq!(requests(&fake, &main_detail(3)), 2);
    assert!(skipped(&state).is_empty());
    assert!(notice(&state, "3").historical);
}

/// A one-page 미래교육원 board made from the captured last page (three rows):
/// the total and pager say page 1 of 1. Notice 45763 is the captured article;
/// the other two answer with the site's empty article frame for a gone post.
fn serve_scec(fake: &Fake) -> String {
    let fixture = include_str!("../../../sources/src/fixtures/vendor-scec-notice-last.html");
    let start = fixture.find("<div class=\"bbs-paging\">").unwrap();
    let end = start + fixture[start..].find("</div>").unwrap();
    let list = format!(
        "{}<div class=\"bbs-paging\"><ul>\
         <li class=\"arrow prev\"><a href=\"#\"></a></li>\
         <li class=\"on\"><a href=\"#none\">1</a></li>\
         <li class=\"arrow next\"><a href=\"#\"></a></li></ul>{}",
        &fixture[..start],
        &fixture[end..]
    )
    .replace("total-numb\">383", "total-numb\">3");
    let board = definition("scec-notice");
    let html = |text: &str| (text.to_string(), "text/html".to_string());
    let detail = |id: &str| {
        format!("https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?smode=view&bbs_idx={id}&bbsid=f_notice")
    };
    let mut extra = fake.extra.lock().unwrap();
    extra.insert(
        rhythm_sources::parser::list_url(&board, 1).unwrap().into(),
        html(&list),
    );
    extra.insert(
        detail("45763"),
        html(include_str!(
            "../../../sources/src/fixtures/vendor-scec-notice-detail-45763.html"
        )),
    );
    for id in ["84854", "84834"] {
        extra.insert(
            detail(id),
            html(include_str!(
                "../../../sources/src/fixtures/vendor-scec-notice-missing.html"
            )),
        );
    }
    detail("45763")
}

#[tokio::test]
async fn a_vendor_board_archives_its_notices_and_reports_gone_posts_missing() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    let detail = serve_scec(&fake);

    run(&state, &fake, "scec-notice", Budget::default())
        .await
        .unwrap();
    let archived = notices(&state);
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].title, "International Culture Festival");
    assert!(fake.paths().contains(&detail));
    // A gone post is neither a failure nor a skip, and history is complete.
    let board = state.board("scec-notice").unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("completed", 0));
    assert!(board.last_error.is_none() && skipped(&state).is_empty());
    assert!(state.seed_complete("scec-notice").unwrap());
}

/// The 입학처 board from both captured list pages: notices 29514 and 24472 are
/// the captured articles, every other row answers with the site's missing-post
/// script, and robots.txt is the site's HTML "not found" page.
#[tokio::test]
async fn the_admission_board_reads_through_its_html_robots_page() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(vec![]);
    let board = definition("admission-notice");
    let html = |text: &str| (text.to_string(), "text/html".to_string());
    let detail = |id: &str| {
        format!("https://admission.sogang.ac.kr/enter/html/counsel/noticeView.asp?idx={id}")
    };
    {
        let mut extra = fake.extra.lock().unwrap();
        extra.insert(
            "https://admission.sogang.ac.kr/robots.txt".into(),
            html("<html><body>페이지를 찾을 수 없습니다</body></html>"),
        );
        for (page, body) in [
            (
                1,
                include_str!("../../../sources/src/fixtures/admission-notice-list.html"),
            ),
            (
                2,
                include_str!("../../../sources/src/fixtures/admission-notice-last.html"),
            ),
        ] {
            let url = rhythm_sources::parser::list_url(&board, page).unwrap();
            extra.insert(url.into(), html(body));
            for id in body
                .split("viewData('")
                .skip(1)
                .map(|rest| &rest[..rest.find('\'').unwrap()])
            {
                extra.insert(
                    detail(id),
                    html(include_str!(
                        "../../../sources/src/fixtures/admission-notice-missing.html"
                    )),
                );
            }
        }
        for id in ["29514", "24472"] {
            let body = match id {
                "29514" => {
                    include_str!("../../../sources/src/fixtures/admission-notice-detail-29514.html")
                }
                _ => {
                    include_str!("../../../sources/src/fixtures/admission-notice-detail-24472.html")
                }
            };
            extra.insert(detail(id), html(body));
        }
    }

    run(&state, &fake, "admission-notice", Budget::default())
        .await
        .unwrap();
    let mut archived: Vec<_> = notices(&state).into_iter().map(|n| n.external_id).collect();
    archived.sort();
    assert_eq!(archived, ["24472", "29514"]);
    let board = state.board("admission-notice").unwrap();
    assert_eq!((board.status.as_str(), board.failed), ("completed", 0));
    assert!(board.last_error.is_none() && skipped(&state).is_empty());
    assert!(state.seed_complete("admission-notice").unwrap());
}
