use super::client::{self, Method, Response};
use super::credentials::Credentials;
use super::*;
use crate::archive::model::ArchiveQuery;
use crate::storage::Store;
use base64::Engine;
use std::sync::{atomic::AtomicUsize, Arc, Mutex};

const COURSE: &str = "A202631099990101";
const PDF: &[u8] = b"%PDF-1.4\nsynthetic course file\n%%EOF\n";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/archive/cybercampus/fixtures")
            .join(name),
    )
    .unwrap()
}
fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U8AAAAASUVORK5CYII=").unwrap()
}
const FIRST_VIEW: &str = r#"<div class="view_title"><span class="font_headline2">첫 수업 안내</span></div>
<table><tr><th scope="row">작성자</th><td>김교수</td></tr><tr><th scope="row">게시일</th><td>9월 1일 (화) 09:00</td></tr>
<tr><td colspan="2"><div class="editor_content"><p>첫 수업은 온라인으로 진행합니다.</p></div></td></tr></table>"#;

#[derive(Clone, Debug)]
struct Recorded {
    path: String,
    query: Option<String>,
    form: Vec<(String, String)>,
}
/// A scripted eClass: a session cookie from the index page, a login that
/// accepts one password, and the fixture pages for two courses.
#[derive(Default)]
struct Fake {
    requests: Mutex<Vec<Recorded>>,
    logins: AtomicUsize,
    /// Number of further logged-in requests before the session expires.
    expire_after: Mutex<Option<usize>>,
    session: Mutex<Option<String>>,
    notice_list: Mutex<Option<String>>,
}
impl Fake {
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
    fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|r| r.path == path).count()
    }
    fn clear(&self) {
        self.requests.lock().unwrap().clear();
    }
}
fn page(body: impl Into<String>) -> Response {
    Response {
        status: 200,
        mime: Some("text/html".into()),
        bytes: body.into().into_bytes(),
        ..Response::default()
    }
}
fn field<'a>(form: &'a [(String, String)], name: &str) -> &'a str {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}
impl client::Http for Fake {
    async fn send(
        &self,
        url: &Url,
        method: &Method,
        cookie: Option<String>,
        _limit: usize,
        _cancel: &AtomicBool,
    ) -> Result<Response, String> {
        let form = match method {
            Method::Get => Vec::new(),
            Method::Post(form) => form.clone(),
        };
        self.requests.lock().unwrap().push(Recorded {
            path: url.path().into(),
            query: url.query().map(str::to_owned),
            form: form.clone(),
        });
        match url.path() {
            // The real index redirects to the login form, which opens the session.
            "/ilos/index.acl" => {
                return Ok(Response {
                    status: 302,
                    location: Some("/ilos/main/member/login_form.acl".into()),
                    ..Response::default()
                });
            }
            "/ilos/main/member/login_form.acl" => {
                let session = format!("S{}", self.logins.load(Ordering::SeqCst));
                *self.session.lock().unwrap() = Some(session.clone());
                return Ok(Response {
                    cookies: vec![("JSESSIONID".into(), session)],
                    ..page("<html>login</html>")
                });
            }
            "/ilos/lo/login.acl" => {
                self.logins.fetch_add(1, Ordering::SeqCst);
                return Ok(if field(&form, "usr_pwd") == "right" {
                    page("<script>top.location.href='/ilos/main/main_form.acl';</script>")
                } else {
                    page("<script>$('#err_message').text(\"로긴에러\");</script>")
                });
            }
            _ => {}
        }
        let expected = self
            .session
            .lock()
            .unwrap()
            .as_ref()
            .map(|session| format!("JSESSIONID={session}"));
        let mut expire = self.expire_after.lock().unwrap();
        let expired = match expire.as_mut() {
            Some(0) => {
                *expire = None;
                *self.session.lock().unwrap() = None;
                true
            }
            Some(left) => {
                *left -= 1;
                false
            }
            None => false,
        };
        if expired || cookie.is_none() || cookie != expected {
            return Ok(Response {
                status: 302,
                location: Some("/ilos/index.acl".into()),
                ..Response::default()
            });
        }
        drop(expire);
        Ok(match url.path() {
            "/ilos/main/rg/regular_register_list_form.acl" => page(fixture("terms.html")),
            "/ilos/main/rg/regular_register_list.acl" => {
                if field(&form, "YEAR") == "2026" && field(&form, "TERM") == "3" {
                    page(fixture("courses.html"))
                } else {
                    page("<ul></ul>")
                }
            }
            "/ilos/cls/st/co/eclass_room2.acl" => page(r#"{"isError":false,"message":"성공"}"#),
            "/ilos/cls/st/notice/notice_list.acl" => {
                if self.last_course() == COURSE {
                    page(
                        self.notice_list
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| fixture("notice_list.html")),
                    )
                } else {
                    page(
                        r#"<script>$(".board_count_wrap > .total_num").text('0');</script><div class="empty_content">조회할 자료가 없습니다</div>"#,
                    )
                }
            }
            "/ilos/cls/st/notice/notice_view_pop.acl" => match field(&form, "ARTL_NUM") {
                "9000002" => page(fixture("notice_view.html")),
                "9000001" => page(FIRST_VIEW),
                _ => page(""),
            },
            "/ilos/cls/st/co/efile_list.acl" => page(fixture("efile_list.html")),
            "/ilos/co/efile_download.acl" => Response {
                status: 302,
                location: Some(format!(
                    "/ilos/co/file_download_v2.acl?{}",
                    url.query().unwrap_or("")
                )),
                ..Response::default()
            },
            "/ilos/co/file_download_v2.acl" => Response {
                status: 200,
                mime: Some("application/octet-stream".into()),
                bytes: PDF.to_vec(),
                ..Response::default()
            },
            path if path.starts_with("/ilosfiles/editor-file/") => Response {
                status: 200,
                mime: Some("image/png".into()),
                bytes: png(),
                ..Response::default()
            },
            "/ilos/cls/st/activity/activity_list.acl" => {
                if self.last_course() == COURSE {
                    page(fixture("activity.html"))
                } else {
                    page("<div class=\"activity_list\"></div>")
                }
            }
            "/ilos/cls/st/report/report_view_form.acl" => match url.query() {
                Some("RT_SEQ=7000010") => page(fixture("report_submitted.html")),
                Some("RT_SEQ=7000020") => page(fixture("report_open.html")),
                _ => page(""),
            },
            "/ilos/mp/todo_list.acl" => page(fixture("todo.html")),
            other => panic!("unexpected request {other}"),
        })
    }
}
impl Fake {
    fn last_course(&self) -> String {
        self.requests()
            .iter()
            .rev()
            .find(|r| r.path == "/ilos/cls/st/co/eclass_room2.acl")
            .map(|r| field(&r.form, "KJKEY").to_owned())
            .unwrap_or_default()
    }
}

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
    let mut bytes = 0;
    sync(
        &setup.connector,
        &setup.fake,
        &setup.store,
        &setup.root,
        &AtomicBool::new(false),
        &mut current,
        Progress { report: &|_| {} },
        &mut bytes,
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
async fn a_rejected_login_is_not_retried_automatically() {
    let setup = setup(Some("wrong"));
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(setup.connector.session.rejected());
    setup.fake.clear();
    assert_eq!(run(&setup).await.unwrap_err(), client::REJECTED);
    assert!(setup.fake.requests().is_empty());
    assert_eq!(setup.fake.logins.load(Ordering::SeqCst), 1);
    // Saving the login again (the settings command resets the session) retries.
    setup.connector.session.reset().await;
    setup
        .connector
        .vault
        .save(&Credentials::new("20990001", "right").unwrap())
        .unwrap();
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
