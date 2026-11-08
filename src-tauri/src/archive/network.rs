use reqwest::{header, StatusCode};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

/// What an automatic request may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Verified board, article and attachment routes only (`validate_url`).
    Source,
    /// Also an image embedded in an archived notice on another public HTTPS host,
    /// and that host's robots.txt (user decision, #78). School hosts stay limited
    /// to their verified routes.
    EmbeddedImage,
}
/// Hosts a job collects from at once, and so requests one client has in flight.
pub const MAX_HOSTS: usize = 3;
/// Time of the last request to each host. Requests to one host stay 650 ms
/// apart; different hosts do not wait for each other.
type Gates = std::sync::Mutex<HashMap<String, Arc<Mutex<Instant>>>>;
#[derive(Clone)]
pub struct Client {
    gates: Arc<Gates>,
    inflight: Arc<Semaphore>,
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
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            gates: Arc::default(),
            inflight: Arc::new(Semaphore::new(MAX_HOSTS)),
        })
    }
    pub async fn get(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        max_bytes: usize,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, String> {
        validate(url, access)?;
        if max_bytes == 0 || max_bytes > 20 * 1024 * 1024 {
            return Err("허용된 다운로드 크기를 초과했습니다.".into());
        }
        let _permit = tokio::select! {value=self.inflight.acquire()=>value.map_err(|_|"다운로드 작업이 종료되었습니다.")?, _=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into())};
        for attempt in 0..3 {
            if cancel.load(Ordering::Acquire) {
                return Err("사용자가 동기화를 중단했습니다.".into());
            }
            let outcome = tokio::select! {
                result=tokio::time::timeout(Duration::from_secs(25),self.attempt(url,validators.clone(),max_bytes,cancel,access))=>match result {Ok(value)=>value,Err(_)=>Err(Failure::Retry("학교 서버 응답 시간이 초과되었습니다.".into()))},
                _=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into()),
            };
            match outcome {
                Ok(value) => return Ok(value),
                Err(Failure::Fatal(message)) => return Err(message),
                Err(Failure::Retry(message)) if attempt == 2 => return Err(message),
                Err(Failure::Retry(_)) => {
                    tokio::select! {_=tokio::time::sleep(Duration::from_secs(1<<attempt))=>{},_=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into())}
                }
            }
        }
        unreachable!()
    }
    /// Waits until 650 ms have passed since the last request to `host`.
    async fn pace(&self, host: &str, cancel: &AtomicBool) -> Result<(), Failure> {
        let gate = self
            .gates
            .lock()
            .map_err(|_| Failure::Fatal("요청 간격을 확인하지 못했습니다.".into()))?
            .entry(host.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1))))
            .clone();
        let mut last = gate.lock().await;
        let delay = Duration::from_millis(650).saturating_sub(last.elapsed());
        tokio::select! {_=tokio::time::sleep(delay)=>{},_=cancelled(cancel)=>return Err(Failure::Fatal("사용자가 동기화를 중단했습니다.".into()))};
        *last = Instant::now();
        Ok(())
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
        validate(url, access).map_err(Failure::Fatal)?;
        let host = url
            .host_str()
            .ok_or_else(|| Failure::Fatal("학교 서버 주소가 올바르지 않습니다.".into()))?;
        let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
            .await
            .map_err(|_| {
                Failure::Retry(
                    "학교 서버 주소를 찾을 수 없습니다. 인터넷 연결을 확인해 주세요.".into(),
                )
            })?
            .collect();
        if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
            return Err(Failure::Fatal(
                "공개 인터넷 주소가 아닌 다운로드 요청을 차단했습니다.".into(),
            ));
        }
        // Resolve once, validate every answer, and pin this connection to those
        // answers. Automatic redirects/proxies must not bypass the same policy.
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(super::tls::config().map_err(Failure::Fatal)?)
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(0)
            .user_agent(concat!(
                "Rhythm/",
                env!("CARGO_PKG_VERSION"),
                " (local university notice archive)"
            ))
            .build()
            .map_err(|_| Failure::Fatal("안전한 HTTPS 연결을 준비하지 못했습니다.".into()))?;
        self.pace(host, cancel).await?;
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
        let mut response=request.send().await.map_err(|error|{
                if error.is_builder(){Failure::Fatal("학교 응답 검증 값이 올바르지 않습니다.".into())}
                else{Failure::Retry("학교 서버에 안전하게 연결하지 못했습니다. 연결 상태나 인증서 구성을 확인해 주세요.".into())}
            })?;
        let status = response.status();
        if [301, 302, 307, 308].contains(&status.as_u16()) || status.as_u16() == 303 {
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into()))?;
            let target = url
                .join(location)
                .map_err(|_| Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into()))?;
            validate(&target, access).map_err(Failure::Fatal)?;
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
            return Err(Failure::Fatal(RATE_LIMITED.into()));
        }
        if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
            return Err(Failure::Retry(format!(
                "학교 서버에 일시적인 오류가 발생했습니다. (HTTP {})",
                status.as_u16()
            )));
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
            return Err(Failure::Fatal(format!(
                "공개 자료에 접근할 수 없습니다. (HTTP {})",
                status.as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes as u64)
        {
            return Err(Failure::Fatal(
                "파일이 다운로드 크기 제한을 초과했습니다.".into(),
            ));
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
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::Retry("자료를 내려받던 중 연결이 끊어졌습니다.".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(Failure::Fatal(
                    "파일이 다운로드 크기 제한을 초과했습니다.".into(),
                ));
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
    Fatal(String),
    Retry(String),
}
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
/// A 429 response. The sync stops the board on it instead of requesting the
/// rest of the page from a server that asked it to slow down.
pub const RATE_LIMITED: &str =
    "학교 서버가 요청을 제한했습니다. 잠시 기다린 후 다시 동기화해 주세요.";
const BLOCKED: &str = "검증된 학교 공개 자료 주소가 아니어서 자동 다운로드하지 않았습니다.";
pub fn validate(url: &Url, access: Access) -> Result<(), String> {
    if validate_url(url).is_ok() || (access == Access::EmbeddedImage && third_party_image(url)) {
        Ok(())
    } else {
        Err(BLOCKED.into())
    }
}
/// Any public HTTPS host name except the school's, on the default port. Bytes
/// are verified as a raster image after download; DNS answers must be public.
fn third_party_image(url: &Url) -> bool {
    let Some(url::Host::Domain(host)) = url.host() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && host.contains('.')
        && host != "sogang.ac.kr"
        && !host.ends_with(".sogang.ac.kr")
        && !path.contains(['\\', '\0'])
        && !path.contains("%2f")
        && !path.contains("%5c")
}
fn raster_name(name: &str) -> bool {
    name.len() <= 512
        && [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|extension| name.to_ascii_lowercase().ends_with(extension))
}
/// Automatic fetching is deliberately narrower than user-triggered external navigation.
pub fn validate_url(url: &Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or("");
    let path = url.path();
    let allowed = match host {
        "www.sogang.ac.kr" => {
            path == "/api/api/v1/mainKo/BbsData"
                || path == "/api/api/v1/mainKo/BbsData/boardList"
                || path.starts_with("/file-fe-prd/board/2/")
                || path.starts_with("/file-fe-prd/editor/")
                || path.starts_with("/file-fe-prd/upload/")
                // Inline notice images (user decision, #78), e.g.
                // /dataview/board/2/<uuid>.jpg and /dataview/board/551608/<uuid>.jpg.
                || {
                    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
                    parts.len() == 4
                        && parts[..2] == ["dataview", "board"]
                        && positive_id(parts[2])
                        && raster_name(parts[3])
                }
                || path == "/robots.txt"
        }
        "computing.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            let boards = [
                "academicNotice",
                "graduateNotice",
                "externalInfo",
                "news",
                "career",
            ];
            let article = parts.len() >= 4
                && parts[..2] == ["ko", "community"]
                && boards.contains(&parts[2])
                && ((parts.len() == 4 && parts[3] == "list")
                    || (parts.len() == 5 && parts[3] == "detail" && positive_id(parts[4])));
            let attachment = parts.len() == 6
                && parts[..3] == ["web", "file", "community"]
                && boards.contains(&parts[3])
                && positive_id(parts[4])
                && !parts[5].is_empty();
            article || attachment || path.starts_with("/web/file/editor/") || path == "/robots.txt"
        }
        "eng.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            let board = (parts.len() == 2 || parts.len() == 3)
                && parts[0] == "board"
                && ["notice", "recruit", "academic", "research"].contains(&parts[1])
                && (parts.len() == 2 || positive_id(parts[2]));
            let download = parts.len() == 2 && parts[0] == "download" && positive_id(parts[1]);
            // Observed official recruit article /board/recruit/676 embeds
            // /files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png. Do not allow
            // generic asset, application, or arbitrary nested file routes.
            let image = parts.len() == 3
                && parts[0] == "files"
                && !parts[1].is_empty()
                && parts[1].len() <= 128
                && parts[1]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                && parts[2].len() <= 512
                && [".png", ".jpg", ".jpeg", ".gif", ".webp"]
                    .iter()
                    .any(|extension| parts[2].to_ascii_lowercase().ends_with(extension));
            board || download || image || path == "/robots.txt"
        }
        "ee.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt disallows /se2/ (inline editor images) and /data/; the
            // /aseoul_bbs/ attachment route is not covered by any rule.
            let article = parts.len() == 3
                && ((parts[0] == "kor"
                    && parts[1] == "community"
                    && [
                        "notice01.php",
                        "notice02.php",
                        "notice03.php",
                        "seminar.php",
                        "employment.php",
                    ]
                    .contains(&parts[2]))
                    || parts[..2] == ["kor", "recruit"] && parts[2] == "recruit.php");
            let attachment = parts.len() == 2 && parts[0] == "aseoul_bbs" && parts[1] == "down.php";
            article || attachment || path == "/robots.txt"
        }
        "me.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt forbids every query string (`Disallow: /*?`), so board
            // paging is never requested. Discovery uses the query-free sitemap;
            // article pages stay under the verified `ko/board/<board>/<id>` route.
            let article = parts.len() == 4
                && parts[..2] == ["ko", "board"]
                && [
                    "notice",
                    "academic",
                    "research",
                    "award",
                    "scholarship",
                    "events",
                    "alumni_news",
                ]
                .contains(&parts[2])
                && positive_id(parts[3]);
            path == "/sitemap.xml" || article || path == "/robots.txt"
        }
        "se.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt allows every path, but only these exact routes are
            // requested: the board list/detail query routes, the filedown
            // attachment route and same-host editor images. Query keys and values
            // are bound so an arbitrary query never reaches another PHP route.
            let list = parts.len() == 2
                && parts[..2] == ["board", "board_list.php"]
                && exact_query(
                    url,
                    &[("board_id", semiconductor_board), ("page", positive_id)],
                );
            let article = parts.len() == 2
                && parts[..2] == ["board", "board_view.php"]
                && exact_query(
                    url,
                    &[("board_id", semiconductor_board), ("no", positive_id)],
                );
            let attachment = parts.len() == 2
                && parts[..2] == ["board", "filedown.php"]
                && exact_query(
                    url,
                    &[("filepath", bounded_value), ("filename", downloadable_name)],
                );
            let image = parts.len() == 3
                && parts[..2] == ["smart_editor", "upload"]
                && editor_image(parts[2]);
            list || article || attachment || image || path == "/robots.txt"
        }
        "sse.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt disallows /se2/, /data/ and /bbs/, so attachments and
            // inline images are never requested from this host.
            (parts.len() == 3
                && parts[0] == "kor"
                && parts[1] == "community"
                && ["notice.php", "news.php", "seminar.php"].contains(&parts[2]))
                || path == "/robots.txt"
        }
        "new-cms.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // Only the three verified dormitory menus and their board ids. Inline
            // editor images under /CrossEditor/ are the only article resources this
            // host serves; other school hosts' images stay blocked by policy.
            let menus = ["6975", "3947", "3951"];
            let boards = ["2231", "1644", "1648"];
            let list = parts.len() == 3
                && parts[0] == "sgdormitory"
                && menus.contains(&parts[1])
                && parts[2] == "subview.do";
            let board_file = parts.len() == 5
                && parts[..2] == ["bbs", "sgdormitory"]
                && boards.contains(&parts[2])
                && positive_id(parts[3])
                && ["artclView.do", "download.do"].contains(&parts[4]);
            let image = parts.len() == 5
                && parts[..3] == ["CrossEditor", "binary", "images"]
                && (1..=12).contains(&parts[3].len())
                && parts[3].bytes().all(|b| b.is_ascii_digit())
                && raster_name(parts[4]);
            list || board_file || image || path == "/robots.txt"
        }
        // The CMS file route behind the main board's PDF-viewer iframes
        // (user decision): a notice whose whole body is that viewer is archived
        // from the PDF's text. Article images on this host stay unsupported.
        "scc.sogang.ac.kr" => {
            let pdf = path == "/Download3"
                && exact_query(
                    url,
                    &[
                        ("pathStr", |v| {
                            bounded_value(v)
                                && v.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                        }),
                        ("fileName", |v| {
                            downloadable_name(v) && v.to_ascii_lowercase().ends_with(".pdf")
                        }),
                        ("gubun", |v| v == "cmsfile"),
                    ],
                );
            pdf || path == "/robots.txt"
        }
        _ => false,
    };
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !allowed
        || path.contains(['\\', '\0'])
        || path.to_ascii_lowercase().contains("%2f")
        || path.to_ascii_lowercase().contains("%5c")
    {
        return Err(BLOCKED.into());
    }
    Ok(())
}
fn positive_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id > 0)
}
fn semiconductor_board(value: &str) -> bool {
    [
        "notice",
        "notice_master",
        "news",
        "recruit",
        "notice_industry",
    ]
    .contains(&value)
}
fn bounded_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}
fn downloadable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.contains(['/', '\\'])
        && !value.chars().any(char::is_control)
        && {
            let lower = value.to_ascii_lowercase();
            [
                ".pdf", ".hwp", ".hwpx", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip",
                ".txt", ".png", ".jpg", ".jpeg", ".gif", ".webp",
            ]
            .iter()
            .any(|extension| lower.ends_with(extension))
        }
}
fn editor_image(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && raster_name(name)
}
type QueryRule = (&'static str, fn(&str) -> bool);
/// Requires exactly the listed query keys once each, with a validating predicate
/// per key. The order is not significant.
fn exact_query(url: &Url, allowed: &[QueryRule]) -> bool {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if pairs.len() != allowed.len() {
        return false;
    }
    for (name, check) in allowed {
        let mut matches = pairs.iter().filter(|(key, _)| key.as_str() == *name);
        let Some((_, value)) = matches.next() else {
            return false;
        };
        if matches.next().is_some() || !check(value) {
            return false;
        }
    }
    true
}
pub(super) fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            // Permit only ordinary global unicast; exclude documentation, local,
            // multicast and transition mechanisms which could tunnel private IPv4.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn address_policy_rejects_private_redirect_targets_and_unverified_domains() {
        for value in [
            "http://www.sogang.ac.kr/api/api/v1/mainKo/BbsData",
            "https://127.0.0.1/a",
            "https://www.sogang.ac.kr.evil.test/api/api/v1/mainKo/BbsData",
            "https://computing.sogang.ac.kr:8443/ko/community/news/list",
            "https://user@computing.sogang.ac.kr/ko/community/news/list",
            "https://computing.sogang.ac.kr/web/file/editor/a#x",
            "https://computing.sogang.ac.kr/admin",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
                "{value}"
            );
        }
        for value in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        assert!(public_ip("163.239.1.1".parse().unwrap()));
        assert!(validate_url(
            &Url::parse("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData?pkId=1").unwrap()
        )
        .is_ok());
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
    fn pdf_viewer_files_are_the_only_approved_scc_route() {
        let file = "https://scc.sogang.ac.kr/Download3?pathStr=MTAxIyMxMDg%3D&fileName=%EC%95%88%EB%82%B4.pdf&gubun=cmsfile";
        for value in [file, "https://scc.sogang.ac.kr/robots.txt"] {
            assert!(validate_url(&Url::parse(value).unwrap()).is_ok(), "{value}");
        }
        for value in [
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.hwp&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=other",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile&x=1",
            "https://scc.sogang.ac.kr/Download3?pathStr=..%2F..&fileName=a.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=..%2Fa.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download2?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file=x",
            "https://scc.sogang.ac.kr/dataview/temp/a.jpg",
            "http://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
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
    fn dormitory_policy_is_limited_to_verified_menus_boards_and_editor_images() {
        for path in [
            "/sgdormitory/6975/subview.do",
            "/sgdormitory/3947/subview.do",
            "/sgdormitory/3951/subview.do",
            "/bbs/sgdormitory/2231/313348/artclView.do",
            "/bbs/sgdormitory/1644/90922/download.do",
            "/bbs/sgdormitory/1648/374772/artclView.do",
            "/CrossEditor/binary/images/000035/20261102092906954_POL76LWU.png",
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

    #[test]
    fn engineering_policy_is_limited_to_verified_public_routes() {
        for path in [
            "/board/notice",
            "/board/recruit/676",
            "/board/academic/4",
            "/board/research/728",
            "/download/708",
            "/files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png",
            "/robots.txt",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://eng.sogang.ac.kr{path}")).unwrap())
                    .is_ok(),
                "{path}"
            );
        }
        for path in [
            "/adm",
            "/login",
            "/search",
            "/board/unknown",
            "/board/notice/0",
            "/board/notice/1/private",
            "/download/0",
            "/download/708/nested",
            "/files/token/page.html",
            "/files/token/nested/image.png",
            "/files/token/a%2fimage.png",
            "/assets/js/site.js",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://eng.sogang.ac.kr{path}")).unwrap())
                    .is_err(),
                "{path}"
            );
        }
    }
    #[tokio::test]
    async fn requests_are_paced_per_host() {
        let client = Client::new().unwrap();
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        assert!(client.pace("www.sogang.ac.kr", &cancel).await.is_ok());
        assert!(client.pace("eng.sogang.ac.kr", &cancel).await.is_ok());
        assert!(
            start.elapsed() < Duration::from_millis(300),
            "another host does not wait"
        );
        assert!(client.pace("www.sogang.ac.kr", &cancel).await.is_ok());
        assert!(
            start.elapsed() >= Duration::from_millis(650),
            "the same host waits"
        );
    }
    #[tokio::test]
    async fn cancelled_request_never_starts_network_work() {
        let client = Client::new().unwrap();
        let cancel = AtomicBool::new(true);
        let url = Url::parse("https://computing.sogang.ac.kr/ko/community/news/list").unwrap();
        assert!(client
            .get(&url, None, 1024, &cancel, Access::Source)
            .await
            .unwrap_err()
            .contains("중단"));
    }
}
