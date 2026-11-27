//! A scripted eClass for tests here and in the app (`test-support`): a session
//! cookie from the index page, a login that accepts one password and the
//! synthetic fixture pages.
use super::client::{self, Method, Response};
use base64::Engine;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};
use url::Url;

pub const COURSE: &str = "A202631099990101";
pub const PDF: &[u8] = b"%PDF-1.4\nsynthetic course file\n%%EOF\n";

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/archive/cybercampus/fixtures")
            .join(name),
    )
    .unwrap()
}
pub fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j6U8AAAAASUVORK5CYII=").unwrap()
}
pub const FIRST_VIEW: &str = r#"<div class="view_title"><span class="font_headline2">첫 수업 안내</span></div>
<table><tr><th scope="row">작성자</th><td>김교수</td></tr><tr><th scope="row">게시일</th><td>9월 1일 (화) 09:00</td></tr>
<tr><td colspan="2"><div class="editor_content"><p>첫 수업은 온라인으로 진행합니다.</p></div></td></tr></table>"#;

#[derive(Clone, Debug)]
pub struct Recorded {
    pub path: String,
    pub query: Option<String>,
    pub form: Vec<(String, String)>,
}
/// A scripted eClass: a session cookie from the index page, a login that
/// accepts one password, and the fixture pages for two courses.
#[derive(Default)]
pub struct Fake {
    pub requests: Mutex<Vec<Recorded>>,
    pub logins: AtomicUsize,
    /// Number of further logged-in requests before the session expires.
    pub expire_after: Mutex<Option<usize>>,
    pub session: Mutex<Option<String>>,
    pub notice_list: Mutex<Option<String>>,
    /// Replaces the first notice's view page.
    pub first_view: Mutex<Option<String>>,
    /// Pages answered to the next login POSTs (one each, in order) instead of
    /// the password check.
    pub login_pages: Mutex<Vec<String>>,
}
impl Fake {
    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
    pub fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|r| r.path == path).count()
    }
    pub fn clear(&self) {
        self.requests.lock().unwrap().clear();
    }
}
pub fn page(body: impl Into<String>) -> Response {
    Response {
        status: 200,
        mime: Some("text/html".into()),
        bytes: body.into().into_bytes(),
        ..Response::default()
    }
}
pub fn field<'a>(form: &'a [(String, String)], name: &str) -> &'a str {
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
                {
                    let mut scripted = self.login_pages.lock().unwrap();
                    if !scripted.is_empty() {
                        return Ok(page(scripted.remove(0)));
                    }
                }
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
                "9000001" => page(
                    self.first_view
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(|| FIRST_VIEW.into()),
                ),
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
    pub fn last_course(&self) -> String {
        self.requests()
            .iter()
            .rev()
            .find(|r| r.path == "/ilos/cls/st/co/eclass_room2.acl")
            .map(|r| field(&r.form, "KJKEY").to_owned())
            .unwrap_or_default()
    }
}
