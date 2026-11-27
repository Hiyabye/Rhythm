use super::client::{self, Method, Response};
use super::credentials::Credentials;
use super::*;
use crate::archive::model::ArchiveQuery;
use crate::storage::Store;
use std::sync::Arc;

use super::fake::*;

struct Setup {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    store: SharedStore,
    connector: Connector,
    fake: Fake,
}
fn setup(password: Option<&str>) -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    let store = Store::open(&root.join("rhythm.sqlite3")).unwrap();
    store.catalog(&crate::archive::definitions()).unwrap();
    store.set_enabled("sogang_notices", true).unwrap();
    store.subscribe(SOURCE_ID, true).unwrap();
    let connector = Connector::memory();
    if let Some(password) = password {
        connector
            .vault
            .save(&Credentials::new("20990001", password).unwrap())
            .unwrap();
    }
    Setup {
        _directory: directory,
        root,
        store: Arc::new(std::sync::Mutex::new(store)),
        connector,
        fake: Fake::default(),
    }
}
async fn run(setup: &Setup) -> Result<(), String> {
    let mut current = with_store(&setup.store, |s| s.source(SOURCE_ID)).unwrap();
    let bytes = AtomicU64::new(0);
    sync(
        &setup.connector,
        &setup.fake,
        &setup.store,
        &setup.root,
        &AtomicBool::new(false),
        &mut current,
        Progress { report: &|_| {} },
        &bytes,
    )
    .await
}

#[tokio::test]
async fn sync_archives_course_notices_files_and_deadlines() {
    let setup = setup(Some("right"));
    run(&setup).await.unwrap();

    let store = setup.store.lock().unwrap();
    let page = store
        .archive_query(&ArchiveQuery {
            source_id: Some(SOURCE_ID.into()),
            ..ArchiveQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 2);
    let first = page
        .items
        .iter()
        .find(|item| item.title == "[중요] 중간고사 범위 안내")
        .unwrap();
    assert_eq!(first.category.as_deref(), Some("자료구조실습"));
    assert_eq!(first.author.as_deref(), Some("김조교"));
    assert_eq!(first.published_at, "2026-10-23");
    // The first sync is the history of the course, not a burst of new notices.
    assert!(first.historical);
    let detail = store.document(first.id, None).unwrap();
    assert!(detail.body_text.contains("1주차부터 7주차"));
    assert!(!detail.body_html.contains("script") && !detail.body_html.contains("alert"));
    let status = |url: &str| {
        detail
            .resources
            .iter()
            .find(|r| r.url.contains(url))
            .map(|r| r.status.clone())
            .unwrap()
    };
    assert_eq!(
        status("efile_download.acl?FILE_SEQ=SYNFILE01"),
        "downloaded"
    );
    assert_eq!(
        status("/ilosfiles/editor-file/0000/2026/SYNIMG01.png"),
        "downloaded"
    );
    // Images on other hosts are kept as references and never fetched; the
    // attachment list only yields eClass's own download route.
    assert_eq!(status("https://example.org/remote.png"), "unsupported");
    assert!(!detail.resources.iter().any(|r| r.url.contains("elsewhere")));
    let older = page
        .items
        .iter()
        .find(|i| i.title == "첫 수업 안내")
        .unwrap();
    assert_eq!(older.published_at, "2026-09-01");

    let courses = store.cybercampus_courses().unwrap();
    assert_eq!(
        courses.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["자료구조실습", "확률과통계"]
    );
    let tasks = store.cybercampus_tasks().unwrap();
    let task = |id: &str| tasks.iter().find(|t| t.item_id == id).unwrap();
    let submitted = task("7000010");
    assert_eq!(submitted.course_name, "자료구조실습");
    assert_eq!(submitted.week, Some(1));
    assert_eq!(submitted.due_at.as_deref(), Some("2026-09-15T23:59"));
    assert_eq!(submitted.late_due_at.as_deref(), Some("2026-09-17T23:59"));
    assert_eq!(submitted.status.as_deref(), Some("정상제출"));
    assert_eq!(submitted.submitted_at.as_deref(), Some("2026-09-14T16:05"));
    assert!(!submitted.pending);
    let open = task("7000020");
    assert_eq!(open.due_at.as_deref(), Some("2026-11-09T23:59"));
    assert_eq!(open.status.as_deref(), Some("미제출"));
    assert_eq!(open.late_due_at, None);
    assert!(open.pending);
    // A TODO item of another kind; its weekday puts January in the next year.
    let quiz = task("6000001");
    assert_eq!(
        (quiz.course_name.as_str(), quiz.label.as_str()),
        ("확률과통계", "퀴즈")
    );
    assert_eq!(quiz.due_at.as_deref(), Some("2027-01-05T10:00"));
    assert!(quiz.pending);
    // Items of courses outside the synced term are ignored.
    assert_eq!(tasks.len(), 3);
    drop(store);

    // The password goes only to the login route, and every request carried
    // only the session cookie of this origin.
    for request in setup.fake.requests() {
        let has_password = request.form.iter().any(|(name, _)| name == "usr_pwd");
        assert_eq!(
            has_password,
            request.path == "/ilos/lo/login.acl",
            "{request:?}"
        );
        assert!(client::validate(
            &Url::parse(&format!(
                "{}{}{}",
                parser::ORIGIN,
                request.path,
                request.query.map(|q| format!("?{q}")).unwrap_or_default()
            ))
            .unwrap()
        )
        .is_ok());
    }
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unchanged_notices_and_closed_assignments_are_not_fetched_again() {
    let setup = setup(Some("right"));
    run(&setup).await.unwrap();
    setup.fake.clear();
    run(&setup).await.unwrap();
    assert_eq!(
        setup.fake.count("/ilos/cls/st/notice/notice_view_pop.acl"),
        0
    );
    assert_eq!(setup.fake.count("/ilos/co/efile_download.acl"), 0);
    // Only the open assignment's page is read again.
    let details: Vec<_> = setup
        .fake
        .requests()
        .into_iter()
        .filter(|r| r.path == "/ilos/cls/st/report/report_view_form.acl")
        .map(|r| r.query.unwrap())
        .collect();
    assert_eq!(details, ["RT_SEQ=7000020"]);
    // The session is reused across syncs.
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 1);
    {
        let store = setup.store.lock().unwrap();
        assert_eq!(store.cybercampus_tasks().unwrap().len(), 3);
        // The second sync announces nothing as new: it found nothing new.
        assert!(store.seed_complete(SOURCE_ID).unwrap());
    }

    // An edited preview refetches that notice, and only that one.
    *setup.fake.notice_list.lock().unwrap() =
        Some(fixture("notice_list.html").replace("첫 수업은 온라인으로", "첫 수업은 대면으로"));
    setup.fake.clear();
    run(&setup).await.unwrap();
    let views: Vec<_> = setup
        .fake
        .requests()
        .into_iter()
        .filter(|r| r.path == "/ilos/cls/st/notice/notice_view_pop.acl")
        .map(|r| field(&r.form, "ARTL_NUM").to_owned())
        .collect();
    assert_eq!(views, ["9000001"]);
}

#[tokio::test]
async fn an_expired_session_logs_in_again_once() {
    let setup = setup(Some("right"));
    *setup.fake.expire_after.lock().unwrap() = Some(4);
    run(&setup).await.unwrap();
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 2);
    let store = setup.store.lock().unwrap();
    assert_eq!(store.cybercampus_tasks().unwrap().len(), 3);
}

#[tokio::test]
async fn a_notice_that_mentions_logging_in_is_not_an_expired_session() {
    let setup = setup(Some("right"));
    *setup.fake.first_view.lock().unwrap() = Some(
        r#"<div class="view_title"><span class="font_headline2">접속 안내</span></div>
<table><tr><th scope="row">작성자</th><td>김교수</td></tr><tr><th scope="row">게시일</th><td>9월 1일 (화) 09:00</td></tr>
<tr><td colspan="2"><div class="editor_content"><p>로그인이 필요합니다. login_form 과 member/login 주소를 확인하세요.</p></div></td></tr></table>"#
            .into(),
    );
    run(&setup).await.unwrap();
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 1);
    let store = setup.store.lock().unwrap();
    let page = store
        .archive_query(&ArchiveQuery {
            source_id: Some(SOURCE_ID.into()),
            ..ArchiveQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 2);
}

#[test]
fn expiry_needs_a_redirect_an_empty_page_or_the_login_form() {
    let html = |body: &str| Response {
        status: 200,
        mime: Some("text/html".into()),
        bytes: body.as_bytes().to_vec(),
        ..Response::default()
    };
    let url = |path: &str| Url::parse(&format!("{}{path}", parser::ORIGIN)).unwrap();
    let notice = url("/ilos/cls/st/notice/notice_view_pop.acl");
    assert!(client::expired(&url("/ilos/index.acl"), &html("<p>x</p>")));
    assert!(client::expired(&notice, &html("  ")));
    assert!(client::expired(
        &notice,
        &html(r#"<form action="/ilos/lo/login.acl"><input type="password" name="usr_pwd"></form>"#)
    ));
    assert!(!client::expired(
        &notice,
        &html("<p>로그인이 필요합니다 login_form member/login usr_pwd</p>")
    ));
}

fn stopped(setup: &Setup) -> bool {
    Status::read(&setup.connector, &setup.store)
        .unwrap()
        .rejected
}
fn credentials(password: &str) -> Credentials {
    Credentials::new("20990001", password).unwrap()
}
/// A new connector (a relaunch) on the same store, with the login saved.
fn relaunch(setup: &mut Setup, password: &str) {
    setup.connector = Connector::memory();
    setup.connector.vault.save(&credentials(password)).unwrap();
}
const MAINTENANCE: &str = "<p>잠시 점검 중입니다.</p>";

#[tokio::test]
async fn a_rejected_login_is_not_retried_automatically() {
    let mut setup = setup(Some("wrong"));
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(stopped(&setup));
    setup.fake.clear();
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(setup.fake.requests().is_empty());
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 1);
    // The rejection survives a relaunch: no password is sent again.
    relaunch(&mut setup, "wrong");
    assert!(stopped(&setup));
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(setup.fake.requests().is_empty());
    // Saving the login again retries and clears the guard.
    let cancel = AtomicBool::new(false);
    setup
        .connector
        .save_login(&setup.fake, &setup.store, &credentials("right"), &cancel)
        .await
        .unwrap();
    assert!(!stopped(&setup));
    run(&setup).await.unwrap();
}

#[tokio::test]
async fn forgetting_the_login_clears_the_guard() {
    let setup = setup(Some("wrong"));
    run(&setup).await.unwrap_err();
    assert!(stopped(&setup));
    setup.connector.forget_login(&setup.store).await.unwrap();
    assert!(!stopped(&setup));
    assert_eq!(
        Status::read(&setup.connector, &setup.store).unwrap().id,
        None
    );
}

#[tokio::test]
async fn three_unrecognised_login_answers_in_a_row_stop_automatic_logins() {
    let mut setup = setup(Some("right"));
    setup.fake.login_pages.lock().unwrap().extend([
        MAINTENANCE.to_string(),
        MAINTENANCE.into(),
        MAINTENANCE.into(),
    ]);
    // Each answer comes after a relaunch: the count is persisted.
    assert!(run(&setup)
        .await
        .unwrap_err()
        .contains("확인하지 못했습니다"));
    relaunch(&mut setup, "right");
    assert!(run(&setup)
        .await
        .unwrap_err()
        .contains("확인하지 못했습니다"));
    assert!(!stopped(&setup));
    relaunch(&mut setup, "right");
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(stopped(&setup));
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 3);
    // No further login POST, also after another relaunch, until the user saves.
    setup.fake.clear();
    relaunch(&mut setup, "right");
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert_eq!(setup.fake.count("/ilos/lo/login.acl"), 0);
    let cancel = AtomicBool::new(false);
    setup
        .connector
        .save_login(&setup.fake, &setup.store, &credentials("right"), &cancel)
        .await
        .unwrap();
    assert!(!stopped(&setup));
    run(&setup).await.unwrap();
}

#[tokio::test]
async fn a_successful_login_resets_the_unrecognised_count() {
    let setup = setup(Some("right"));
    let cancel = AtomicBool::new(false);
    let right = credentials("right");
    let login = || {
        setup
            .connector
            .session
            .login(&setup.fake, &setup.store, &right, &cancel)
    };
    setup
        .fake
        .login_pages
        .lock()
        .unwrap()
        .extend([MAINTENANCE.to_string(), MAINTENANCE.into()]);
    login().await.unwrap_err();
    login().await.unwrap_err();
    login().await.unwrap();
    // Two more do not stop: the count started over.
    setup
        .fake
        .login_pages
        .lock()
        .unwrap()
        .extend([MAINTENANCE.to_string(), MAINTENANCE.into()]);
    login().await.unwrap_err();
    login().await.unwrap_err();
    assert!(!stopped(&setup));
}

/// An eClass that never answers: only transport errors.
struct Offline;
impl client::Http for Offline {
    async fn send(
        &self,
        _url: &Url,
        _method: &Method,
        _cookie: Option<String>,
        _limit: usize,
        _cancel: &AtomicBool,
    ) -> Result<Response, String> {
        Err("사이버캠퍼스에 안전하게 연결하지 못했습니다.".into())
    }
}
#[tokio::test]
async fn transport_failures_do_not_count_as_unrecognised_answers() {
    let setup = setup(Some("right"));
    let cancel = AtomicBool::new(false);
    for _ in 0..5 {
        setup
            .connector
            .session
            .login(&Offline, &setup.store, &credentials("right"), &cancel)
            .await
            .unwrap_err();
    }
    assert!(!stopped(&setup));
    run(&setup).await.unwrap();
}

#[tokio::test]
async fn sync_without_a_saved_login_makes_no_request() {
    let setup = setup(None);
    assert_eq!(run(&setup).await.unwrap_err(), client::NO_LOGIN);
    assert!(setup.fake.requests().is_empty());
}

#[test]
fn requests_stay_on_approved_eclass_routes() {
    for allowed in [
        "https://eclass.sogang.ac.kr/ilos/mp/todo_list.acl",
        "https://eclass.sogang.ac.kr/ilos/co/efile_download.acl?FILE_SEQ=A&CONTENT_SEQ=1",
        "https://eclass.sogang.ac.kr/ilosfiles/editor-file/0000/2026/A.png",
    ] {
        assert!(
            client::validate(&Url::parse(allowed).unwrap()).is_ok(),
            "{allowed}"
        );
    }
    for blocked in [
        "http://eclass.sogang.ac.kr/ilos/mp/todo_list.acl",
        "https://eclass.sogang.ac.kr:8443/ilos/mp/todo_list.acl",
        "https://saint.sogang.ac.kr/ilos/mp/todo_list.acl",
        "https://eclass.sogang.ac.kr/ilos/cls/st/report/report_insert.acl",
        "https://eclass.sogang.ac.kr/ilos/cls/st/report/report_delete.acl",
        "https://eclass.sogang.ac.kr/ilosfiles/editor-file/../../etc/passwd",
        "https://user:pw@eclass.sogang.ac.kr/ilos/mp/todo_list.acl",
    ] {
        assert!(
            client::validate(&Url::parse(blocked).unwrap()).is_err(),
            "{blocked}"
        );
    }
    assert!(owns(
        "https://eclass.sogang.ac.kr/ilos/co/efile_download.acl?FILE_SEQ=A"
    ));
    assert!(!owns("https://www.sogang.ac.kr/file.pdf"));
}

#[test]
fn credentials_are_validated_and_never_debug_printed() {
    assert!(Credentials::new("", "x").is_err());
    assert!(Credentials::new("2099 0001", "x").is_err());
    assert!(Credentials::new("20990001", "").is_err());
    let credentials = Credentials::new(" 20990001 ", "secret-password").unwrap();
    assert_eq!(credentials.id, "20990001");
    assert!(!format!("{credentials:?}").contains("secret-password"));
}

mod parsing {
    use super::super::parser::*;
    use super::fixture;

    #[test]
    fn terms_and_courses() {
        assert_eq!(
            terms(&fixture("terms.html"))
                .into_iter()
                .map(|t| format!("{}^{}", t.year, t.term))
                .collect::<Vec<_>>(),
            ["2026^3", "2026^1", "2025^3"]
        );
        assert_eq!(
            courses(&fixture("courses.html")),
            [
                Course {
                    key: "A202631099990101".into(),
                    name: "자료구조실습".into()
                },
                Course {
                    key: "A202631088880202".into(),
                    name: "확률과통계".into()
                }
            ]
        );
        assert!(entered(r#"{"isError":false}"#).is_ok());
        assert!(entered(r#"{"isError":true,"message":"권한 없음"}"#)
            .unwrap_err()
            .contains("권한 없음"));
        assert!(entered("<html>").is_err());
    }

    #[test]
    fn notice_list_and_view() {
        let list = notice_list(&fixture("notice_list.html")).unwrap();
        assert_eq!(list.total, 2);
        assert_eq!(list.rows[0].seq, "9000002");
        assert_eq!(list.rows[0].title, "[중요] 중간고사 범위 안내");
        assert!(list.rows[1].preview.starts_with("첫 수업은"));
        // A count without rows means the page changed shape, not an empty board.
        assert!(notice_list(r#"<script>.total_num").text('3');</script>"#).is_err());
        assert!(notice_list("<p>no count</p>").is_err());

        let view = notice_view(&fixture("notice_view.html")).unwrap();
        assert_eq!(view.title, "[중요] 중간고사 범위 안내");
        assert_eq!(view.author.as_deref(), Some("김조교"));
        assert_eq!(view.posted, "10월 23일 (금) 23:42");
        // Nested divs stay inside the body (eclass-cli's regex once cut them).
        assert!(view.body_html.contains("7주차"));
        let files = view.files.unwrap();
        assert_eq!(files.path, "/ilos/cls/st/co/efile_list.acl");
        assert_eq!(
            files.fields,
            [
                ("ud", "20990001".to_owned()),
                ("ky", "A202631099990101".to_owned()),
                ("pf_st_flag", "2".to_owned()),
                ("CONTENT_SEQ", "SYNTHETICSEQ01".to_owned())
            ]
        );
        let resources = attachments(&fixture("efile_list.html"));
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].filename, "중간고사_범위.pdf");
        assert_eq!(
            resources[0].url,
            "https://eclass.sogang.ac.kr/ilos/co/efile_download.acl?FILE_SEQ=SYNFILE01&CONTENT_SEQ=8000001"
        );
    }

    #[test]
    fn assignments_and_todo() {
        let rows = assignments(&fixture("activity.html"));
        assert_eq!(
            rows,
            [
                AssignmentRow {
                    seq: "7000010".into(),
                    title: "실습 과제 1".into(),
                    week: Some(1),
                    open: false
                },
                AssignmentRow {
                    seq: "7000020".into(),
                    title: "실습 과제 2".into(),
                    week: Some(9),
                    open: true
                }
            ]
        );
        let open = assignment(&fixture("report_open.html")).unwrap();
        assert_eq!(open.status.as_deref(), Some("미제출"));
        assert_eq!(open.submitted_at, None);
        assert_eq!(open.late_due_at, None);
        assert!(assignment("<p>not an assignment</p>").is_err());

        let items = todo(&fixture("todo.html"));
        // Items of other courses are parsed; the sync keeps only its own.
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].course_key, "A000000000000000");
        assert_eq!(items[0].title, "실습 과제 2");
        assert_eq!(items[0].label, "과제");
        assert_eq!(items[0].due, "11월 9일 (월) 23:59");
        assert_eq!(items[1].category, "test");
    }

    #[test]
    fn dates_take_the_year_whose_weekday_matches() {
        assert_eq!(
            full_datetime("2026.10.25 (일) 23:59").as_deref(),
            Some("2026-10-25T23:59")
        );
        assert_eq!(full_datetime("2026.02.30 (월) 10:00"), None);
        assert_eq!(full_datetime("무제한"), None);
        assert_eq!(
            month_day("10월 23일 (금) 23:42", 2026).as_deref(),
            Some("2026-10-23T23:42")
        );
        // 2026-01-05 is a Monday, 2027-01-05 a Tuesday.
        assert_eq!(
            month_day("1월 5일 (화) 10:00", 2026).as_deref(),
            Some("2027-01-05T10:00")
        );
        assert_eq!(
            month_day("1월 5일 (월)", 2026).as_deref(),
            Some("2026-01-05")
        );
        // An unmatched weekday falls back to the course year.
        assert_eq!(
            month_day("3월 2일 (일) 09:00", 2026).as_deref(),
            Some("2026-03-02T09:00")
        );
        assert_eq!(month_day("13월 2일", 2026), None);
    }

    #[test]
    fn twelve_hour_times_keep_the_afternoon() {
        // The forms eClass renders when the account's clock is set to 12 hours.
        assert_eq!(
            full_datetime("2026.09.22 (화) 오후 7:06").as_deref(),
            Some("2026-09-22T19:06")
        );
        assert_eq!(
            full_datetime("2026.10.05 (월) 오후 12:59").as_deref(),
            Some("2026-10-05T12:59")
        );
        assert_eq!(
            month_day("11월 22일 (일) 오후 11:59", 2026).as_deref(),
            Some("2026-11-22T23:59")
        );
        assert_eq!(
            full_datetime("2026.11.09 (월) 오후 02:59").as_deref(),
            Some("2026-11-09T14:59")
        );
        assert_eq!(
            full_datetime("2026.11.09 (월) 오전 2:59").as_deref(),
            Some("2026-11-09T02:59")
        );
        assert_eq!(
            month_day("11월 9일 (월) 오후 2:59", 2026).as_deref(),
            Some("2026-11-09T14:59")
        );
        assert_eq!(
            month_day("11월 9일 (월) 02:59 PM", 2026).as_deref(),
            Some("2026-11-09T14:59")
        );
        // Noon and midnight are 12 in the 12-hour clock.
        assert_eq!(
            full_datetime("2026.11.09 (월) 오후 12:00").as_deref(),
            Some("2026-11-09T12:00")
        );
        assert_eq!(
            full_datetime("2026.11.09 (월) 오전 12:30").as_deref(),
            Some("2026-11-09T00:30")
        );
        // A marker with a 24-hour value is contradictory.
        assert_eq!(
            full_datetime("2026.11.09 (월) 오후 14:59").as_deref(),
            Some("2026-11-09")
        );
    }
}

/// The scripted eClass behind the sync controller, calling `hook` before
/// each notice list request.
struct Hooked<'a> {
    fake: Fake,
    hook: Box<dyn Fn() + Send + Sync + 'a>,
}
impl client::Http for Hooked<'_> {
    async fn send(
        &self,
        url: &Url,
        method: &Method,
        cookie: Option<String>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<Response, String> {
        if url.path() == "/ilos/cls/st/notice/notice_list.acl" {
            (self.hook)();
        }
        self.fake.send(url, method, cookie, limit, cancel).await
    }
}
/// A sync controller whose eClass login is saved, with the Cybercampus
/// extension on.
fn controller(setup: &Setup) -> Arc<crate::archive::sync::Controller> {
    with_store(&setup.store, |s| s.set_enabled("cybercampus", true)).unwrap();
    let controller = crate::archive::sync::Controller::with_connector(Connector::memory());
    controller
        .cybercampus
        .vault
        .save(&Credentials::new("20990001", "right").unwrap())
        .unwrap();
    controller
}
fn source_status(setup: &Setup) -> String {
    with_store(&setup.store, |s| s.source(SOURCE_ID))
        .unwrap()
        .status
}

#[tokio::test]
async fn the_controller_collects_cybercampus_and_keeps_only_scheduler_outcomes() {
    let setup = setup(None);
    let controller = controller(&setup);
    let start = |lease: Option<crate::operation_gate::AdmissionLease>| {
        let (store, root, http) = (
            setup.store.clone(),
            setup.root.clone(),
            Arc::new(Fake::default()),
        );
        match lease {
            Some(lease) => {
                controller.start_tracked_admitted(store, root, http, None, "refresh".into(), lease)
            }
            None => {
                controller.start_tracked_with_transport(store, root, http, None, "refresh".into())
            }
        }
        .unwrap()
        .1
    };
    let tracked = start(None);
    controller.wait().await;
    let outcome = controller.take_task_outcome(tracked).unwrap().unwrap();
    assert!(outcome.result.is_ok(), "{outcome:?}");
    assert_eq!(source_status(&setup), "completed");
    assert_eq!(
        with_store(&setup.store, |s| s.archive_query(&ArchiveQuery::default()))
            .unwrap()
            .total,
        2
    );
    // A command-path job holds a lease and nobody takes its outcome.
    let gate = Arc::new(crate::operation_gate::AdmissionGate::default());
    let untracked = start(Some(gate.shared().await.unwrap()));
    controller.wait().await;
    assert!(controller.take_task_outcome(untracked).is_err());

    // A failed sync is the job's failure.
    controller.cybercampus.vault.clear().unwrap();
    let failed = start(None);
    controller.wait().await;
    let outcome = controller.take_task_outcome(failed).unwrap().unwrap();
    assert!(outcome.result.unwrap_err().contains(client::NO_LOGIN));
    assert!(!outcome.cancelled);
    assert_eq!(source_status(&setup), "failed");
}

#[tokio::test]
async fn admission_takes_only_cybercampus_with_its_extension_on() {
    let setup = setup(Some("right"));
    let controller = crate::archive::sync::Controller::with_connector(Connector::memory());
    let start = |ids: Option<&[&str]>, mode: &str| {
        let started = controller.start_tracked_with_transport(
            setup.store.clone(),
            setup.root.clone(),
            Arc::new(Fake::default()),
            ids.map(|ids| ids.iter().map(|id| id.to_string()).collect()),
            mode.into(),
        );
        started.map(|_| ())
    };
    // The notices extension does not cover the Cybercampus source.
    assert!(start(Some(&[SOURCE_ID]), "refresh").is_err());
    with_store(&setup.store, |s| s.set_enabled("cybercampus", true)).unwrap();
    // Public boards come from the feed, and crawl-only modes are gone.
    with_store(&setup.store, |s| s.subscribe("sogang-academic", true)).unwrap();
    assert!(start(Some(&["sogang-academic"]), "refresh").is_err());
    assert!(start(Some(&[SOURCE_ID, "sogang-academic"]), "refresh").is_err());
    assert!(start(None, "historical").is_err());
    for mode in ["refresh", "incremental", "skipped", "resources"] {
        start(None, mode).unwrap();
        controller.wait().await;
    }
    start(Some(&[SOURCE_ID]), "refresh").unwrap();
    controller.wait().await;
}

#[tokio::test]
async fn unsubscribing_stops_the_sync_and_cancelling_cancels_the_job() {
    for unsubscribe in [true, false] {
        let setup = setup(None);
        let controller = controller(&setup);
        let source = with_store(&setup.store, |s| s.source(SOURCE_ID)).unwrap();
        let http = Hooked {
            fake: Fake::default(),
            hook: Box::new(|| {
                if unsubscribe {
                    with_store(&setup.store, |s| s.subscribe(SOURCE_ID, false)).unwrap();
                    controller.cancel_source(SOURCE_ID);
                } else {
                    controller.cancel();
                }
            }),
        };
        let outcome = controller
            .execute(&setup.store, &setup.root, &http, source, "refresh")
            .await;
        assert_eq!(outcome.is_ok(), unsubscribe, "{outcome:?}");
        assert_eq!(source_status(&setup), "cancelled");
        assert_eq!(http.fake.count("/ilos/cls/st/notice/notice_list.acl"), 1);
    }
}
