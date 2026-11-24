//! Constrained HTTPS transport for automatic fetches: exact approved routes,
//! pinned public DNS answers, per-server pacing, bounded retries and sizes, and
//! redirects returned to the caller instead of followed.
use reqwest::{header, StatusCode};
use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OwnedMutexGuard, Semaphore};
use url::Url;

pub use crate::policy::{public_ip, validate_url};
use crate::policy::{third_party_image, BLOCKED};

/// What an automatic request may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Verified board, article and attachment routes only (`validate_url`).
    Source,
    /// Also an image embedded in an archived notice on another public HTTPS host,
    /// and that host's robots.txt (user decision, #78). School hosts stay limited
    /// to their verified routes.
    EmbeddedImage,
    /// Manual reading-room lookup, exact host/port/routes only. No retries.
    #[cfg(feature = "studium")]
    Studium,
}
/// Hosts a job collects from at once, and so requests one client has in flight.
pub const MAX_HOSTS: usize = 3;
/// Time of the last request to each server, keyed by `server_key`. One server
/// gets one request at a time, started at least 650 ms after the previous one,
/// whatever hostnames it answers for; different servers do not wait for each
/// other (#80, user decision 2026-11-22).
type Gates = std::sync::Mutex<HashMap<IpAddr, Arc<Mutex<Instant>>>>;
#[derive(Clone)]
pub struct Client {
    user_agent: &'static str,
    gates: Arc<Gates>,
    inflight: Arc<Semaphore>,
    // Tests replace only DNS and the wire response. URL validation, TLS client
    // construction, pacing, deadlines and response handling stay on this path.
    #[cfg(all(test, feature = "studium"))]
    test_io: Option<Arc<client_tests::Script>>,
}
/// A failed request. `status` is the HTTP status when the server answered
/// with one, so callers act on 429 or 401/403 without reading `message`, the
/// Korean text shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchError {
    pub status: Option<u16>,
    pub message: String,
}
impl FetchError {
    pub fn http(status: u16, message: String) -> Self {
        Self {
            status: Some(status),
            message,
        }
    }
    pub fn rate_limited(&self) -> bool {
        self.status == Some(429)
    }
    pub fn access_denied(&self) -> bool {
        matches!(self.status, Some(401 | 403))
    }
}
impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            status: None,
            message,
        }
    }
}
impl From<&str> for FetchError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<FetchError> for String {
    fn from(error: FetchError) -> Self {
        error.message
    }
}
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
#[derive(Debug)]
pub enum Fetch {
    /// Return redirects to the policy owner; transport never follows them implicitly.
    Redirect(Url),
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
        mime: Option<String>,
    },
}
impl Client {
    pub fn new(user_agent: &'static str) -> Result<Self, String> {
        Ok(Self {
            user_agent,
            gates: Arc::default(),
            inflight: Arc::new(Semaphore::new(MAX_HOSTS)),
            #[cfg(all(test, feature = "studium"))]
            test_io: None,
        })
    }
    pub async fn get(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        max_bytes: usize,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        validate(url, access)?;
        if max_bytes == 0 || max_bytes > 20 * 1024 * 1024 {
            return Err("허용된 다운로드 크기를 초과했습니다.".into());
        }
        let _permit = tokio::select! {value=self.inflight.acquire()=>value.map_err(|_|"다운로드 작업이 종료되었습니다.")?, _=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into())};
        let attempts = if is_studium(access) { 1 } else { 3 };
        for attempt in 0..attempts {
            if cancel.load(Ordering::Acquire) {
                return Err("사용자가 동기화를 중단했습니다.".into());
            }
            let outcome = tokio::select! {
                result=tokio::time::timeout(ATTEMPT_WAIT,self.attempt(url,validators.clone(),max_bytes,cancel,access))=>match result {Ok(value)=>value,Err(_)=>Err(Failure::Retry(TIMED_OUT.into()))},
                _=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into()),
            };
            match outcome {
                Ok(value) => return Ok(value),
                Err(Failure::Fatal(message)) => return Err(message),
                Err(Failure::Retry(message)) if attempt + 1 == attempts => return Err(message),
                Err(Failure::Retry(_)) => {
                    tokio::select! {_=tokio::time::sleep(Duration::from_secs(1<<attempt))=>{},_=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into())}
                }
            }
        }
        unreachable!()
    }
    /// Waits until `server` has no request in flight and 650 ms have passed
    /// since the last one started. The returned guard keeps the server busy.
    async fn pace(
        &self,
        server: IpAddr,
        cancel: &AtomicBool,
    ) -> Result<OwnedMutexGuard<Instant>, Failure> {
        let gate = self
            .gates
            .lock()
            .map_err(|_| Failure::Fatal("요청 간격을 확인하지 못했습니다.".into()))?
            .entry(server)
            .or_insert_with(|| Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1))))
            .clone();
        let mut last = tokio::select! {last=gate.lock_owned()=>last,_=cancelled(cancel)=>return Err(Failure::Fatal("사용자가 동기화를 중단했습니다.".into()))};
        let delay = Duration::from_millis(650).saturating_sub(last.elapsed());
        tokio::select! {_=tokio::time::sleep(delay)=>{},_=cancelled(cancel)=>return Err(Failure::Fatal("사용자가 동기화를 중단했습니다.".into()))};
        *last = Instant::now();
        Ok(last)
    }
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, Failure> {
        #[cfg(all(test, feature = "studium"))]
        if let Some(script) = &self.test_io {
            return script.resolve(host, port);
        }
        tokio::net::lookup_host((host, port))
            .await
            .map(|addresses| addresses.collect())
            .map_err(|_| {
                Failure::Retry(
                    "학교 서버 주소를 찾을 수 없습니다. 인터넷 연결을 확인해 주세요.".into(),
                )
            })
    }
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, Failure> {
        #[cfg(all(test, feature = "studium"))]
        if let Some(script) = &self.test_io {
            return script.send(request).await;
        }
        request.send().await.map_err(|error| {
            if error.is_builder() {
                Failure::Fatal("학교 응답 검증 값이 올바르지 않습니다.".into())
            } else {
                Failure::Retry(CONNECTION_FAILED.into())
            }
        })
    }
    async fn attempt(
        &self,
        original: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        max_bytes: usize,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, Failure> {
        let url = original;
        validate(url, access).map_err(|error| Failure::Fatal(error.into()))?;
        let host = url
            .host_str()
            .ok_or_else(|| Failure::Fatal("학교 서버 주소가 올바르지 않습니다.".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| Failure::Fatal(BLOCKED.into()))?;
        let addresses = self.resolve(host, port).await?;
        let server = server_key(&addresses).ok_or_else(|| {
            Failure::Fatal("공개 인터넷 주소가 아닌 다운로드 요청을 차단했습니다.".into())
        })?;
        // Resolve once, validate every answer, and pin this connection to those
        // answers. Automatic redirects/proxies must not bypass the same policy.
        // The seat server offers only TLS 1.2 CBC cipher suites, which rustls
        // does not implement. Only the exact Studium origin uses OS TLS, with
        // hostname/chain verification, TLS 1.2 as the minimum and the OS root
        // store replaced by the one root that issues the school's chain.
        let builder = reqwest::Client::builder();
        #[cfg(feature = "studium")]
        let builder = if access == Access::Studium {
            builder
                .use_native_tls()
                .min_tls_version(reqwest::tls::Version::TLS_1_2)
                .tls_built_in_root_certs(false)
                .add_root_certificate(studium_root().map_err(|error| Failure::Fatal(error.into()))?)
        } else {
            builder.use_preconfigured_tls(
                crate::tls::config().map_err(|error| Failure::Fatal(error.into()))?,
            )
        };
        #[cfg(not(feature = "studium"))]
        let builder = builder.use_preconfigured_tls(
            crate::tls::config().map_err(|error| Failure::Fatal(error.into()))?,
        );
        let client = builder
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_secs(8))
            // No total timeout: headers and body have their own deadlines
            // below. A body that stalls still ends after this idle time.
            .read_timeout(HEADER_WAIT)
            .pool_max_idle_per_host(0)
            .user_agent(self.user_agent)
            .build()
            .map_err(|_| Failure::Fatal("안전한 HTTPS 연결을 준비하지 못했습니다.".into()))?;
        // Held until the response headers arrive, so the server sees one
        // request at a time; the body streams without it.
        let gate = self.pace(server, cancel).await?;
        let mut request = client.get(url.clone()).header(
            header::ACCEPT,
            if url.path() == "/robots.txt" {
                "text/plain,*/*;q=0.1"
            } else if url.path().starts_with("/api/") {
                "application/json"
            } else {
                "text/html,application/pdf,image/*,application/octet-stream;q=0.8"
            },
        );
        if is_studium(access) {
            request = request.header(header::CACHE_CONTROL, "no-cache");
        }
        // Conditional headers are tied to the original document, never forwarded
        // across an origin-changing redirect.
        if url.origin() == original.origin() {
            if let Some((etag, last_modified)) = &validators {
                if let Some(etag) = etag {
                    request = request.header(header::IF_NONE_MATCH, etag);
                }
                if let Some(last_modified) = last_modified {
                    request = request.header(header::IF_MODIFIED_SINCE, last_modified);
                }
            }
        }
        let response = tokio::time::timeout(HEADER_WAIT, self.send(request))
            .await
            .unwrap_or_else(|_| Err(Failure::Retry(TIMED_OUT.into())));
        drop(gate);
        let mut response = response?;
        let status = response.status();
        if [301, 302, 307, 308].contains(&status.as_u16()) || status.as_u16() == 303 {
            if is_studium(access) {
                return Err(Failure::Fatal(
                    "좌석 서버의 이동 응답을 거부했습니다.".into(),
                ));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into()))?;
            let target = url
                .join(location)
                .map_err(|_| Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into()))?;
            validate(&target, access).map_err(|error| Failure::Fatal(error.into()))?;
            return Ok(Fetch::Redirect(target));
        }
        if status == StatusCode::NOT_MODIFIED {
            if !validators
                .as_ref()
                .is_some_and(|(etag, modified)| etag.is_some() || modified.is_some())
            {
                return Err(Failure::Fatal(
                    "검증 정보가 없는 문서에 변경 없음 응답이 도착했습니다.".into(),
                ));
            }
            return Ok(Fetch::NotModified);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Failure::Fatal(FetchError::http(429, RATE_LIMITED.into())));
        }
        if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
            return Err(Failure::Retry(FetchError::http(
                status.as_u16(),
                format!(
                    "학교 서버에 일시적인 오류가 발생했습니다. (HTTP {})",
                    status.as_u16()
                ),
            )));
        }
        // Only an absent policy means no restrictions for this exact origin.
        // Authentication failures and rate limits must never become allow-all.
        if is_studium(access) && url.path() == "/robots.txt" && matches!(status.as_u16(), 404 | 410)
        {
            return Ok(Fetch::Body {
                bytes: Vec::new(),
                etag: None,
                last_modified: None,
                mime: Some("text/plain".into()),
            });
        }
        // A school host that has no robots.txt at all (user decision, #80 step 4):
        // only these exact hosts, only for a missing file.
        if access == Access::Source
            && url.path() == "/robots.txt"
            && matches!(status.as_u16(), 404 | 410)
            && url
                .host_str()
                .is_some_and(|host| ABSENT_ROBOTS_HOSTS.contains(&host))
        {
            return Ok(Fetch::Body {
                bytes: Vec::new(),
                etag: None,
                last_modified: None,
                mime: Some("text/plain".into()),
            });
        }
        // RFC 9309: an unavailable (4xx) robots.txt places no restrictions. Only
        // other hosts get this reading; a school host's error stays an error.
        if access == Access::EmbeddedImage
            && url.path() == "/robots.txt"
            && status.is_client_error()
            && validate_url(url).is_err()
        {
            return Ok(Fetch::Body {
                bytes: Vec::new(),
                etag: None,
                last_modified: None,
                mime: Some("text/plain".into()),
            });
        }
        if !status.is_success() {
            return Err(Failure::Fatal(FetchError::http(
                status.as_u16(),
                http_refused(status.as_u16()),
            )));
        }
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes as u64)
        {
            return Err(Failure::Fatal(TOO_LARGE.into()));
        }
        let get_header = |name: header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .filter(|v| v.len() <= 4096)
                .map(str::to_owned)
        };
        let etag = get_header(header::ETAG);
        let last_modified = get_header(header::LAST_MODIFIED);
        let mime = get_header(header::CONTENT_TYPE).map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });
        let mut bytes = Vec::new();
        let body_deadline = tokio::time::Instant::now() + BODY_WAIT;
        while let Some(chunk) = tokio::time::timeout_at(body_deadline, response.chunk())
            .await
            .map_err(|_| Failure::Retry(TIMED_OUT.into()))?
            .map_err(|error| {
                Failure::Retry(
                    if error.is_timeout() {
                        TIMED_OUT
                    } else {
                        "자료를 내려받던 중 연결이 끊어졌습니다."
                    }
                    .into(),
                )
            })?
        {
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(Failure::Fatal(TOO_LARGE.into()));
            }
            bytes.extend_from_slice(&chunk);
            if cancel.load(Ordering::Acquire) {
                return Err(Failure::Fatal("사용자가 동기화를 중단했습니다.".into()));
            }
        }
        Ok(Fetch::Body {
            bytes,
            etag,
            last_modified,
            mime,
        })
    }
}
enum Failure {
    Fatal(FetchError),
    Retry(FetchError),
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
/// School hosts whose robots.txt is absent (HTTP 404), read as no rules:
/// 한국어교육원, observed 2026-11-24. Every other school host's robots error
/// stays an error, so a broken policy server never becomes allow-all.
const ABSENT_ROBOTS_HOSTS: [&str; 1] = ["klec.sogang.ac.kr"];
#[cfg(feature = "studium")]
fn is_studium(access: Access) -> bool {
    access == Access::Studium
}
#[cfg(not(feature = "studium"))]
fn is_studium(_: Access) -> bool {
    false
}
/// Sectigo Public Server Authentication Root R46, the root of the
/// `libseat.sogang.ac.kr` chain (provenance in `certificates/README.md`).
#[cfg(feature = "studium")]
const STUDIUM_ROOT: &[u8] = include_bytes!("certificates/sectigo-r46-root.pem");
#[cfg(feature = "studium")]
fn studium_root() -> Result<reqwest::Certificate, String> {
    reqwest::Certificate::from_pem(STUDIUM_ROOT)
        .map_err(|_| "좌석 서버 인증서 정보를 준비하지 못했습니다.".into())
}
/// A 429 response. The sync stops the board on it instead of requesting the
/// rest of the page from a server that asked it to slow down.
/// A response over the caller's byte limit.
pub const TOO_LARGE: &str = "파일이 다운로드 크기 제한을 초과했습니다.";
pub const RATE_LIMITED: &str =
    "학교 서버가 요청을 제한했습니다. 잠시 기다린 후 다시 동기화해 주세요.";
/// Message for an unsuccessful HTTP status.
pub fn http_refused(status: u16) -> String {
    format!("공개 자료에 접근할 수 없습니다. (HTTP {status})")
}
const TIMED_OUT: &str = "학교 서버 응답 시간이 초과되었습니다.";
/// Time for the response headers, and the longest pause within a body.
const HEADER_WAIT: Duration = Duration::from_secs(20);
/// Time for a whole body once the headers arrived. Pages of 1–2 MB took over
/// 20 s from the collector's GitHub runner (the old total timeout), so the
/// same notices timed out on every run.
const BODY_WAIT: Duration = Duration::from_secs(90);
/// Bounds one attempt, including the wait for the server's pacing gate.
const ATTEMPT_WAIT: Duration = Duration::from_secs(120);
const CONNECTION_FAILED: &str =
    "학교 서버에 안전하게 연결하지 못했습니다. 연결 상태나 인증서 구성을 확인해 주세요.";
pub fn validate(url: &Url, access: Access) -> Result<(), String> {
    #[cfg(feature = "studium")]
    if access == Access::Studium {
        return crate::studium::validate_url(url);
    }
    if validate_url(url).is_ok() || (access == Access::EmbeddedImage && third_party_image(url)) {
        Ok(())
    } else {
        Err(BLOCKED.into())
    }
}
/// The server a host's addresses belong to: the lowest address, so hostnames
/// with the same DNS answers share one key. `None` when any address is not
/// public (such a request is refused).
pub fn server_key(addresses: &[SocketAddr]) -> Option<IpAddr> {
    if addresses.iter().any(|address| !public_ip(address.ip())) {
        return None;
    }
    addresses.iter().map(SocketAddr::ip).min()
}
/// The request boundary the sync and seat checks use; tests replace it with
/// scripted responses.
pub trait Transport: Send + Sync {
    /// The server `host` resolves to (`server_key`), so a caller can keep one
    /// server's work together. `None` when unknown: the caller falls back to
    /// the hostname. Fakes keep the default.
    fn server<'a>(&'a self, _host: &'a str) -> impl Future<Output = Option<IpAddr>> + Send + 'a {
        std::future::ready(None)
    }
    fn fetch<'a>(
        &'a self,
        url: &'a Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &'a AtomicBool,
        access: Access,
    ) -> impl Future<Output = Result<Fetch, FetchError>> + Send + 'a;
}
impl Transport for Client {
    async fn server(&self, host: &str) -> Option<IpAddr> {
        server_key(&self.resolve(host, 443).await.ok()?)
    }
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        self.get(url, validators, limit, cancel, access).await
    }
}
#[cfg(test)]
const TEST_AGENT: &str = "Rhythm-test";
#[cfg(all(test, feature = "studium"))]
#[path = "transport/client_tests.rs"]
mod client_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "studium")]
    #[test]
    fn studium_pins_exactly_the_sectigo_r46_root() {
        use sha2::{Digest, Sha256};
        let certificates =
            crate::tls::decode_pem(std::str::from_utf8(STUDIUM_ROOT).unwrap()).unwrap();
        assert_eq!(certificates.len(), 1);
        assert_eq!(
            format!("{:x}", Sha256::digest(certificates[0].as_ref())),
            "7bb647a62aeeac88bf257aa522d01ffea395e0ab45c73f93f65654ec38f25a06"
        );
        studium_root().unwrap();
    }
    #[test]
    fn community_policy_is_limited_to_current_routes_and_robots_allowed_resources() {
        for value in [
            "https://ee.sogang.ac.kr/kor/community/notice01.php?pNo=2&code=news",
            "https://ee.sogang.ac.kr/kor/community/notice02.php?m=v&idx=2867&pNo=2&code=general",
            "https://ee.sogang.ac.kr/kor/recruit/recruit.php?pNo=1&code=recruit",
            "https://ee.sogang.ac.kr/aseoul_bbs/down.php?d=1&idx=2867",
            "https://ee.sogang.ac.kr/robots.txt",
            "https://sse.sogang.ac.kr/kor/community/notice.php?pNo=2",
            "https://sse.sogang.ac.kr/kor/community/news.php?m=v&idx=233&pNo=1",
            "https://sse.sogang.ac.kr/kor/community/seminar.php",
            "https://sse.sogang.ac.kr/robots.txt",
        ] {
            assert!(validate_url(&Url::parse(value).unwrap()).is_ok(), "{value}");
        }
        for value in [
            // robots.txt disallows these paths on both hosts.
            "https://ee.sogang.ac.kr/se2/photo_uploader/upload/1793606246.png",
            "https://sse.sogang.ac.kr/se2/photo_uploader/upload/1777513674.png",
            "https://sse.sogang.ac.kr/data/bbsData/0635011001792981167.png",
            "https://sse.sogang.ac.kr/bbs/down.php?d=1&idx=198",
            "https://ee.sogang.ac.kr/bbs/down.php?d=1&idx=198",
            "https://ee.sogang.ac.kr/aseoul/down.php?d=1&idx=1",
            "https://ee.sogang.ac.kr/aseoul_bbs/upload.php",
            "https://ee.sogang.ac.kr/aseoul_bbs/down.php/nested",
            "https://ee.sogang.ac.kr/kor/community/notice04.php",
            "https://ee.sogang.ac.kr/kor/community/notice01.php/extra",
            "https://sse.sogang.ac.kr/kor/community/notice02.php",
            "https://sse.sogang.ac.kr/kor/recruit/recruit.php",
            "http://ee.sogang.ac.kr/kor/community/notice01.php",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
                "{value}"
            );
            assert!(
                validate(&Url::parse(value).unwrap(), Access::EmbeddedImage).is_err(),
                "{value}"
            );
        }
    }
    #[test]
    fn embedded_images_may_use_other_public_https_hosts_but_school_hosts_stay_exact() {
        let check = |value: &str, access| validate(&Url::parse(value).unwrap(), access).is_ok();
        for value in [
            "https://www.sogang.ac.kr/dataview/board/2/d843c754-711a-4de1-8b84-86371fb562de.jpg",
            "https://www.sogang.ac.kr/dataview/board/551608/e41e62d2.PNG",
        ] {
            assert!(check(value, Access::Source), "{value}");
        }
        for value in [
            "https://www.sogang.ac.kr/dataview/board/2/page.html",
            "https://www.sogang.ac.kr/dataview/board/0/a.jpg",
            "https://www.sogang.ac.kr/dataview/board/2/nested/a.jpg",
            "https://www.sogang.ac.kr/dataview/temp/a.jpg",
        ] {
            assert!(!check(value, Access::EmbeddedImage), "{value}");
        }
        for value in [
            "https://lh3.googleusercontent.com/abc=w400",
            "https://cdn.example.com/a.png",
            "https://cdn.example.com:443/robots.txt",
        ] {
            assert!(check(value, Access::EmbeddedImage), "{value}");
            assert!(!check(value, Access::Source), "{value}");
        }
        for value in [
            "http://cdn.example.com/a.png",
            "https://cdn.example.com:58082/a.png",
            "https://163.239.27.155/a.png",
            "https://[2001:4860::1]/a.png",
            "https://localhost/a.png",
            "https://scc.sogang.ac.kr/dataview/temp/a.jpg",
            "https://sogang.ac.kr/a.png",
            "https://user@cdn.example.com/a.png",
            "https://cdn.example.com/a.png#x",
            "https://cdn.example.com/a%2Fb.png",
        ] {
            assert!(!check(value, Access::EmbeddedImage), "{value}");
        }
    }
    #[test]
    fn k2web_policy_is_limited_to_catalogued_boards_and_their_images() {
        for path in [
            "/sgdormitory/6975/subview.do",
            "/sgdormitory/3947/subview.do",
            "/sgdormitory/3951/subview.do",
            "/bbs/sgdormitory/2231/313348/artclView.do",
            "/bbs/sgdormitory/1644/90922/download.do",
            "/bbs/sgdormitory/1648/374772/artclView.do",
            "/CrossEditor/binary/images/000035/20261102092906954_POL76LWU.png",
            "/sites/sgdormitory/atchmnfl/bbs/2231/temp_1795392463736100.png",
            "/sites/sgdormitory/images/migrated/bbs/884755_1.JPG",
            "/sites/sgdormitory/%EB%B0%94%EB%A1%9C.png",
            "/gsot/3208/subview.do",
            "/bbs/gsot/908/420698/artclView.do",
            "/sites/gsot/atchmnfl/bbs/908/image.png",
            "/robots.txt",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://new-cms.sogang.ac.kr{path}")).unwrap())
                    .is_ok(),
                "{path}"
            );
        }
        for path in [
            "/sgdormitory/6976/subview.do",
            "/sgdormitory/6975/list.do",
            "/sgdormitory/6975/subview.do/extra",
            "/bbs/sgdormitory/2232/313348/artclView.do",
            "/bbs/sgdormitory/2231/0/artclView.do",
            "/bbs/sgdormitory/2231/313348/artclList.do",
            "/bbs/sgdormitory/2231/313348/artclView.do/nested",
            "/CrossEditor/binary/images/000035/page.html",
            "/CrossEditor/binary/images/000035/nested/image.png",
            "/CrossEditor/binary/other/000035/image.png",
            "/Web-home/_UI/css/common/jquery-ui.css",
            // A k2web site that is not in the catalog, and a left-out board
            // (시설 관련) of a catalogued site.
            "/sgarmy/6887/subview.do",
            "/bbs/sgarmy/2001/420698/artclView.do",
            "/sites/sgarmy/atchmnfl/bbs/2001/image.png",
            "/sgdormitory/3945/subview.do",
            "/bbs/sgdormitory/1646/313348/artclView.do",
            // A catalogued site with a menu or board id the catalog does not list,
            // and one site's menu under another site.
            "/history/3208/subview.do",
            "/gsot/3209/subview.do",
            "/bbs/gsot/909/420698/artclView.do",
            // Site folders hold only images the policy approves.
            "/sites/sgdormitory/atchmnfl/bbs/2231/file.pdf",
            "/sites/sgdormitory/a/b/c/d/e/f/image.png",
            "/sites/sgdormitory/.hidden/image.png",
            "/sites/sgdormitory/a%2Fb.png",
            "/sites/sgdormitory/a%5cb.png",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://new-cms.sogang.ac.kr{path}")).unwrap())
                    .is_err(),
                "{path}"
            );
            assert!(
                validate(
                    &Url::parse(&format!("https://new-cms.sogang.ac.kr{path}")).unwrap(),
                    Access::EmbeddedImage
                )
                .is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn mechanical_policy_is_limited_to_the_sitemap_and_article_routes() {
        for path in [
            "/sitemap.xml",
            "/robots.txt",
            "/ko/board/notice/2419",
            "/ko/board/academic/2395",
            "/ko/board/research/2425",
            "/ko/board/award/2427",
            "/ko/board/scholarship/2407",
            "/ko/board/events/1762",
            "/ko/board/alumni_news/1835",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://me.sogang.ac.kr{path}")).unwrap())
                    .is_ok(),
                "{path}"
            );
        }
        for path in [
            // robots.txt forbids the only paging route (`?page=`).
            "/ko/board/notice",
            "/ko/board/notice?page=2",
            "/ko/board/notice/",
            "/ko/board/notice/0",
            "/ko/board/notice/2419/extra",
            "/ko/board/unknown/1",
            "/en/board/notice/1",
            "/bbs/board.php?bo_table=notice",
            "/api/board/notice",
            "/sitemap.xml/extra",
            "/sitemap-index.xml",
        ] {
            let url = Url::parse(&format!("https://me.sogang.ac.kr{path}")).unwrap();
            assert!(validate_url(&url).is_err(), "{path}");
            assert!(validate(&url, Access::EmbeddedImage).is_err(), "{path}");
        }
    }

    #[test]
    fn semiconductor_policy_is_limited_to_exact_board_routes_and_resources() {
        for path in [
            "/robots.txt",
            "/board/board_list.php?board_id=notice&page=1",
            "/board/board_list.php?board_id=notice_master&page=3",
            "/board/board_list.php?board_id=news&page=2",
            "/board/board_list.php?board_id=recruit&page=1",
            "/board/board_list.php?board_id=notice_industry&page=1",
            "/board/board_view.php?board_id=notice&no=39",
            "/board/board_view.php?board_id=notice_master&no=7",
            "/board/board_view.php?board_id=news&no=10",
            "/board/board_view.php?board_id=recruit&no=11",
            "/board/board_view.php?board_id=notice_industry&no=7",
            "/board/filedown.php?filepath=wXJClutf9n%2FgtLi2zZE7x&filename=a.pdf",
            "/board/filedown.php?filepath=abc&filename=출입권한신청서.xlsx",
            // A literal `+` in a name (notice 29).
            "/board/filedown.php?filepath=wXJClutf9n%2FgtLi2zZE7x&filename=첨부1.+2026학년도+2학기+수강신청+안내.hwp",
            "/smart_editor/upload/1781851902-601781851902268.png",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://se.sogang.ac.kr{path}")).unwrap())
                    .is_ok(),
                "{path}"
            );
        }
        for path in [
            // Only the five catalogued boards and the exact list/detail queries.
            "/board/board_list.php?board_id=unknown&page=1",
            "/board/board_list.php?board_id=notice",
            "/board/board_list.php?board_id=notice&page=0",
            "/board/board_list.php?board_id=notice&page=x",
            "/board/board_list.php?board_id=notice&page=1&extra=1",
            "/board/board_view.php?board_id=notice",
            "/board/board_view.php?board_id=notice&no=0",
            "/board/board_view.php?board_id=notice&no=39&page=1",
            "/board/board_view.php?board_id=unknown&no=39",
            "/board/board_view.php?board_id=notice&no=39&no=40",
            "/board/board_view.php?board_id=notice&no=39&extra=1",
            // Attachment names must be bounded and use a supported extension.
            "/board/filedown.php?filepath=abc",
            "/board/filedown.php?filepath=&filename=a.pdf",
            "/board/filedown.php?filepath=abc&filename=a.exe",
            "/board/filedown.php?filepath=abc&filename=dir/a.pdf",
            "/board/filedown.php?filepath=abc&filename=a.pdf&extra=1",
            // Editor images stay inside the upload directory and are rasters.
            "/smart_editor/upload/nested/a.png",
            "/smart_editor/upload/a.svg",
            "/smart_editor/upload/../a.png",
            "/smart_editor/binary/a.png",
            // This host serves board pages only; /se2/ and /data/ are not routes
            // here and other school hosts' inline images remain blocked.
            "/se2/photo_uploader/upload/1792979823.png",
            "/data/bbsData/0635011001792981167.png",
            "/board/board_view.php/extra?board_id=notice&no=39",
        ] {
            let url = Url::parse(&format!("https://se.sogang.ac.kr{path}")).unwrap();
            assert!(validate_url(&url).is_err(), "{path}");
            assert!(validate(&url, Access::EmbeddedImage).is_err(), "{path}");
        }
        let insecure =
            Url::parse("http://se.sogang.ac.kr/board/board_list.php?board_id=notice&page=1")
                .unwrap();
        assert!(validate_url(&insecure).is_err());
        assert!(validate(&insecure, Access::EmbeddedImage).is_err());
    }

    #[tokio::test]
    async fn requests_are_paced_per_server() {
        let client = Client::new(TEST_AGENT).unwrap();
        let cancel = AtomicBool::new(false);
        let first: IpAddr = "163.239.3.91".parse().unwrap();
        let second: IpAddr = "163.239.1.17".parse().unwrap();
        let start = Instant::now();
        drop(client.pace(first, &cancel).await.ok().unwrap());
        drop(client.pace(second, &cancel).await.ok().unwrap());
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "another server does not wait"
        );
        drop(client.pace(first, &cancel).await.ok().unwrap());
        assert!(
            start.elapsed() >= Duration::from_millis(650),
            "the same server waits"
        );
    }
    #[tokio::test]
    async fn a_server_gets_one_request_at_a_time() {
        let client = Client::new(TEST_AGENT).unwrap();
        let cancel = AtomicBool::new(false);
        let server: IpAddr = "163.239.3.91".parse().unwrap();
        let busy = client.pace(server, &cancel).await.ok().unwrap();
        // Still in flight a second later: the next request waits for it.
        let waiting =
            tokio::time::timeout(Duration::from_millis(1000), client.pace(server, &cancel));
        assert!(waiting.await.is_err());
        drop(busy);
        let start = Instant::now();
        drop(client.pace(server, &cancel).await.ok().unwrap());
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "the gap has passed"
        );
    }
    #[test]
    fn hostnames_with_the_same_answers_share_a_server() {
        let address = |ip: &str| SocketAddr::new(ip.parse().unwrap(), 443);
        let ee = [address("115.68.52.186")];
        let sse = [address("115.68.52.186")];
        assert_eq!(server_key(&ee), server_key(&sse));
        let round_robin = [address("163.239.3.92"), address("163.239.3.91")];
        assert_eq!(
            server_key(&round_robin),
            Some("163.239.3.91".parse().unwrap())
        );
        assert_ne!(server_key(&ee), server_key(&round_robin));
        assert_eq!(server_key(&[address("127.0.0.1")]), None);
        assert_eq!(
            server_key(&[address("163.239.3.91"), address("10.0.0.1")]),
            None
        );
        assert_eq!(server_key(&[]), None);
    }
    #[tokio::test]
    async fn cancelled_request_never_starts_network_work() {
        let client = Client::new(TEST_AGENT).unwrap();
        let cancel = AtomicBool::new(true);
        let url = Url::parse("https://computing.sogang.ac.kr/ko/community/news/list").unwrap();
        assert!(client
            .get(&url, None, 1024, &cancel, Access::Source)
            .await
            .unwrap_err()
            .message
            .contains("중단"));
    }
}
