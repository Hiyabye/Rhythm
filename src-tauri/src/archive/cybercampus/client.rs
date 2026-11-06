//! An authenticated eClass session. Requests go only to the exact HTTPS origin
//! and the routes the connector reads; the password is sent only to the login
//! route. Cookies are kept in memory for the process and never persisted.
use super::credentials::{Credentials, Vault};
use super::parser::ORIGIN;
use std::{
    collections::BTreeMap,
    future::Future,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

pub const PAGE_LIMIT: usize = 4 * 1024 * 1024;
const HOST: &str = "eclass.sogang.ac.kr";
const LOGIN: &str = "/ilos/lo/login.acl";
const INDEX: &str = "/ilos/index.acl";
/// `index.acl` redirects here; it is the page that opens the session cookie.
const LOGIN_FORM: &str = "/ilos/main/member/login_form.acl";
/// The read-only routes the connector uses. Every request path must be one of
/// these or an editor image below `/ilosfiles/editor-file/`.
const ROUTES: [&str; 15] = [
    INDEX,
    LOGIN_FORM,
    LOGIN,
    "/ilos/main/rg/regular_register_list_form.acl",
    "/ilos/main/rg/regular_register_list.acl",
    "/ilos/cls/st/co/eclass_room2.acl",
    "/ilos/cls/st/notice/notice_list.acl",
    "/ilos/cls/st/notice/notice_view_pop.acl",
    "/ilos/cls/st/co/efile_list.acl",
    "/ilos/co/efile_list.acl",
    "/ilos/cls/st/activity/activity_list.acl",
    "/ilos/cls/st/report/report_view_form.acl",
    "/ilos/mp/todo_list.acl",
    "/ilos/co/efile_download.acl",
    // `efile_download.acl` redirects the browser here for the file bytes.
    "/ilos/co/file_download_v2.acl",
];
pub const CANCELLED: &str = "동기화를 중단했습니다. 저장된 자료는 유지됩니다.";
pub const NO_LOGIN: &str = "설정의 정보원에서 사이버캠퍼스 로그인 정보를 먼저 저장해 주세요.";
pub const REJECTED: &str =
    "사이버캠퍼스 로그인에 실패했습니다. 아이디와 비밀번호를 확인해 다시 저장해 주세요.";

/// Validates a request target; only the exact origin and approved routes.
pub fn validate(url: &Url) -> Result<(), String> {
    let path = url.path();
    if url.scheme() == "https"
        && url.host_str() == Some(HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && (ROUTES.contains(&path)
            || (path.starts_with("/ilosfiles/editor-file/")
                && !path.contains("..")
                && !path.contains("//")))
    {
        Ok(())
    } else {
        Err("사이버캠퍼스의 허용된 주소가 아니어서 요청하지 않았습니다.".into())
    }
}

#[derive(Clone, Debug)]
pub enum Method {
    Get,
    Post(Vec<(String, String)>),
}
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub cookies: Vec<(String, String)>,
    pub mime: Option<String>,
    pub bytes: Vec<u8>,
}
/// The network boundary; tests replace it with a scripted server.
pub trait Http: Send + Sync {
    fn send<'a>(
        &'a self,
        url: &'a Url,
        method: &'a Method,
        cookie: Option<String>,
        limit: usize,
        cancel: &'a AtomicBool,
    ) -> impl Future<Output = Result<Response, String>> + Send + 'a;
}

pub struct Reqwest;
impl Http for Reqwest {
    async fn send(
        &self,
        url: &Url,
        method: &Method,
        cookie: Option<String>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<Response, String> {
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(30), send(url, method, cookie, limit, cancel)) => {
                result.map_err(|_| "사이버캠퍼스 응답 시간이 초과되었습니다.".to_string())?
            }
            _ = cancelled(cancel) => Err(CANCELLED.into()),
        }
    }
}
async fn send(
    url: &Url,
    method: &Method,
    cookie: Option<String>,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Response, String> {
    use reqwest::header;
    validate(url)?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((HOST, 443))
        .await
        .map_err(|_| "사이버캠퍼스 주소를 찾을 수 없습니다. 인터넷 연결을 확인해 주세요.")?
        .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !crate::archive::network::public_ip(address.ip()))
    {
        return Err("공개 인터넷 주소가 아닌 사이버캠퍼스 요청을 차단했습니다.".into());
    }
    // Same pinned-DNS, no-proxy, no-redirect policy as the archive transport.
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(crate::archive::tls::config()?)
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(HOST, &addresses)
        .connect_timeout(Duration::from_secs(8))
        .pool_max_idle_per_host(0)
        // eClass rejects requests without a browser user agent (eclass-cli).
        .user_agent(concat!(
            "Mozilla/5.0 (compatible) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36 Rhythm/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|_| "안전한 HTTPS 연결을 준비하지 못했습니다.")?;
    let request = match method {
        Method::Get => client.get(url.clone()),
        Method::Post(form) => client.post(url.clone()).form(form),
    }
    // Without a fixed locale the server mixes Korean and English dates.
    .header(header::ACCEPT_LANGUAGE, "ko-KR,ko;q=0.9");
    let request = match cookie {
        Some(cookie) => request.header(header::COOKIE, cookie),
        None => request,
    };
    let mut response = request
        .send()
        .await
        .map_err(|_| "사이버캠퍼스에 안전하게 연결하지 못했습니다. 연결 상태를 확인해 주세요.")?;
    let headers = response.headers();
    let text = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 4096)
            .map(str::to_owned)
    };
    let location = text(header::LOCATION);
    let mime = text(header::CONTENT_TYPE).map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    });
    let cookies = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(set_cookie)
        .collect();
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("파일이 다운로드 크기 제한을 초과했습니다.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "사이버캠퍼스 자료를 받던 중 연결이 끊어졌습니다.")?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("파일이 다운로드 크기 제한을 초과했습니다.".into());
        }
        bytes.extend_from_slice(&chunk);
        if cancel.load(Ordering::Acquire) {
            return Err(CANCELLED.into());
        }
    }
    Ok(Response {
        status,
        location,
        cookies,
        mime,
        bytes,
    })
}
/// The name and value of a `Set-Cookie` header; attributes are ignored because
/// every cookie is sent back only to this one origin.
fn set_cookie(value: &str) -> Option<(String, String)> {
    let (name, value) = value.split(';').next()?.split_once('=')?;
    let name = name.trim();
    (!name.is_empty()
        && name.len() <= 128
        && value.len() <= 4096
        && name.bytes().all(|b| b.is_ascii_graphic()))
    .then(|| (name.to_owned(), value.trim().to_owned()))
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct State {
    cookies: BTreeMap<String, String>,
    logged_in: bool,
    last_request: Option<Instant>,
}
/// The process-wide eClass session. One sync runs at a time (the archive
/// controller serializes jobs), and the lock is never held across a request
/// that another task waits on for long.
pub struct Session {
    state: Mutex<State>,
    /// Set when the server rejected the stored login. Automatic syncs stop
    /// retrying it (repeated failures can lock a school account) until the
    /// user saves the login again.
    rejected: AtomicBool,
    gap: Duration,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            gap: Duration::from_millis(700),
            state: Mutex::new(State {
                cookies: BTreeMap::new(),
                logged_in: false,
                last_request: None,
            }),
            rejected: AtomicBool::new(false),
        }
    }
}
impl Session {
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.cookies.clear();
        state.logged_in = false;
        self.rejected.store(false, Ordering::Release);
    }
    #[cfg(test)]
    pub fn unpaced() -> Self {
        Self {
            gap: Duration::ZERO,
            ..Self::default()
        }
    }
    pub fn rejected(&self) -> bool {
        self.rejected.load(Ordering::Acquire)
    }
    /// One request including same-origin redirects. Cookies set on any hop are
    /// kept; the final URL tells whether the server sent us to its login page.
    async fn exchange<H: Http>(
        &self,
        http: &H,
        url: &Url,
        method: &Method,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<(Url, Response), String> {
        let mut target = url.clone();
        let mut method = method.clone();
        for _ in 0..=3 {
            validate(&target)?;
            let cookie = {
                let mut state = self.state.lock().await;
                // eClass is not a public board; a fixed gap keeps the load at a
                // single browser's pace.
                if let Some(last) = state.last_request {
                    let delay = self.gap.saturating_sub(last.elapsed());
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancelled(cancel) => return Err(CANCELLED.into()),
                    }
                }
                state.last_request = Some(Instant::now());
                (!state.cookies.is_empty()).then(|| {
                    state
                        .cookies
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
            };
            if cancel.load(Ordering::Acquire) {
                return Err(CANCELLED.into());
            }
            let response = http.send(&target, &method, cookie, limit, cancel).await?;
            {
                let mut state = self.state.lock().await;
                for (name, value) in &response.cookies {
                    state.cookies.insert(name.clone(), value.clone());
                }
            }
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                let next = response
                    .location
                    .as_deref()
                    .and_then(|location| target.join(location).ok())
                    .ok_or("사이버캠퍼스 이동 주소가 올바르지 않습니다.")?;
                if next.origin().ascii_serialization() != ORIGIN {
                    return Err("사이버캠퍼스가 다른 사이트로 이동시켜 요청을 중단했습니다.".into());
                }
                // A login redirect ends the exchange; the caller logs in again.
                if [LOGIN, INDEX, "/"].contains(&next.path()) {
                    return Ok((next, response));
                }
                target = next;
                method = Method::Get;
                continue;
            }
            if response.status == 429 {
                return Err(
                    "사이버캠퍼스가 요청을 제한했습니다. 잠시 후 다시 시도해 주세요.".into(),
                );
            }
            if !(200..300).contains(&response.status) {
                return Err(format!(
                    "사이버캠퍼스에서 자료를 받지 못했습니다. (HTTP {})",
                    response.status
                ));
            }
            return Ok((target, response));
        }
        Err("사이버캠퍼스의 이동 경로가 너무 많습니다.".into())
    }
    /// Logs in with the given login. The response page reports success with a
    /// script redirect and failure with a message.
    pub async fn login<H: Http>(
        &self,
        http: &H,
        credentials: &Credentials,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        {
            let mut state = self.state.lock().await;
            state.cookies.clear();
            state.logged_in = false;
        }
        let index = Url::parse(&format!("{ORIGIN}{INDEX}")).expect("static URL");
        self.exchange(http, &index, &Method::Get, PAGE_LIMIT, cancel)
            .await?;
        let form = [
            ("usr_id", credentials.id.as_str()),
            ("usr_pwd", credentials.password.as_str()),
            ("returnURL", ""),
            ("challenge", ""),
            ("response", ""),
            ("auto_login", "N"),
            ("encoding", "utf-8"),
        ]
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
        let login = Url::parse(&format!("{ORIGIN}{LOGIN}")).expect("static URL");
        let (_, response) = self
            .exchange(http, &login, &Method::Post(form), PAGE_LIMIT, cancel)
            .await?;
        let body = String::from_utf8_lossy(&response.bytes);
        if body.contains("top.location.href") {
            self.state.lock().await.logged_in = true;
            self.rejected.store(false, Ordering::Release);
            Ok(())
        } else if body.contains("SAINT 인증에 실패")
            || body.contains("로긴에러")
            || body.contains("err_message")
        {
            self.rejected.store(true, Ordering::Release);
            Err(REJECTED.into())
        } else {
            Err(
                "사이버캠퍼스 로그인 응답을 확인하지 못했습니다. 잠시 후 다시 시도해 주세요."
                    .into(),
            )
        }
    }
    /// A logged-in request. An expired session is renewed once with the stored
    /// login and the request repeated.
    pub async fn request<H: Http>(
        &self,
        http: &H,
        vault: &Vault,
        path: &str,
        form: Option<Vec<(&str, String)>>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<Response, String> {
        let url = Url::parse(ORIGIN)
            .and_then(|origin| origin.join(path))
            .map_err(|_| "사이버캠퍼스 주소가 올바르지 않습니다.")?;
        validate(&url)?;
        let method = match form {
            Some(mut form) => {
                form.push(("encoding", "utf-8".into()));
                Method::Post(
                    form.into_iter()
                        .map(|(name, value)| (name.to_owned(), value))
                        .collect(),
                )
            }
            None => Method::Get,
        };
        for attempt in 0..2 {
            if attempt == 1 || !self.state.lock().await.logged_in {
                if self.rejected() {
                    return Err(REJECTED.into());
                }
                let credentials = vault.load()?.ok_or(NO_LOGIN)?;
                self.login(http, &credentials, cancel).await?;
            }
            let (final_url, response) = self.exchange(http, &url, &method, limit, cancel).await?;
            if !expired(&final_url, &response) {
                return Ok(response);
            }
            self.state.lock().await.logged_in = false;
        }
        Err("사이버캠퍼스 세션을 다시 열지 못했습니다. 잠시 후 다시 시도해 주세요.".into())
    }
}
/// eClass signals an expired session by sending the browser to its login page
/// or by answering a page request with an empty body or a login form.
fn expired(url: &Url, response: &Response) -> bool {
    if [LOGIN, INDEX, LOGIN_FORM, "/"].contains(&url.path()) {
        return true;
    }
    if !matches!(response.mime.as_deref(), Some("text/html") | None) {
        return false;
    }
    let body = String::from_utf8_lossy(&response.bytes);
    let body = body.trim();
    body.is_empty()
        || body.contains("login_form")
        || body.contains("member/login")
        || body.contains("로그인이 필요")
}
