use reqwest::{header, StatusCode};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

#[derive(Clone)]
pub struct Client {
    gate: Arc<Mutex<Instant>>,
    inflight: Arc<Semaphore>,
}
#[derive(Debug)]
pub enum Fetch {
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
            gate: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1))),
            inflight: Arc::new(Semaphore::new(2)),
        })
    }
    pub async fn get(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Fetch, String> {
        validate_url(url)?;
        if max_bytes == 0 || max_bytes > 20 * 1024 * 1024 {
            return Err("허용된 다운로드 크기를 초과했습니다.".into());
        }
        let _permit = tokio::select! {value=self.inflight.acquire()=>value.map_err(|_|"다운로드 작업이 종료되었습니다.")?, _=cancelled(cancel)=>return Err("사용자가 동기화를 중단했습니다.".into())};
        for attempt in 0..3 {
            if cancel.load(Ordering::Acquire) {
                return Err("사용자가 동기화를 중단했습니다.".into());
            }
            let outcome = tokio::select! {
                result=tokio::time::timeout(Duration::from_secs(25),self.attempt(url,validators.clone(),max_bytes,cancel))=>match result {Ok(value)=>value,Err(_)=>Err(Failure::Retry("학교 서버 응답 시간이 초과되었습니다.".into()))},
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
    async fn attempt(
        &self,
        original: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Fetch, Failure> {
        let mut url = original.clone();
        for redirect in 0..=3 {
            validate_url(&url).map_err(Failure::Fatal)?;
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
                .user_agent("Rhythm/0.2.0 (local university notice archive)")
                .build()
                .map_err(|_| Failure::Fatal("안전한 HTTPS 연결을 준비하지 못했습니다.".into()))?;
            {
                let mut last = self.gate.lock().await;
                let delay = Duration::from_millis(650).saturating_sub(last.elapsed());
                tokio::select! {_=tokio::time::sleep(delay)=>{},_=cancelled(cancel)=>return Err(Failure::Fatal("사용자가 동기화를 중단했습니다.".into()))};
                *last = Instant::now();
            }
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
            if [301, 302, 303, 307, 308].contains(&status.as_u16()) {
                if redirect == 3 {
                    return Err(Failure::Fatal(
                        "학교 서버의 이동 경로가 너무 많습니다.".into(),
                    ));
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into())
                    })?;
                url = url.join(location).map_err(|_| {
                    Failure::Fatal("학교 서버 이동 주소가 올바르지 않습니다.".into())
                })?;
                continue;
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
                return Err(Failure::Fatal(
                    "학교 서버가 요청을 제한했습니다. 잠시 기다린 후 다시 동기화해 주세요.".into(),
                ));
            }
            if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
                return Err(Failure::Retry(format!(
                    "학교 서버에 일시적인 오류가 발생했습니다. (HTTP {})",
                    status.as_u16()
                )));
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
            return Ok(Fetch::Body {
                bytes,
                etag,
                last_modified,
                mime,
            });
        }
        unreachable!()
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
        return Err("검증된 학교 공개 자료 주소가 아니어서 자동 다운로드하지 않았습니다.".into());
    }
    Ok(())
}
fn positive_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id > 0)
}
fn public_ip(address: IpAddr) -> bool {
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
    #[tokio::test]
    async fn cancelled_request_never_starts_network_work() {
        let client = Client::new().unwrap();
        let cancel = AtomicBool::new(true);
        let url = Url::parse("https://computing.sogang.ac.kr/ko/community/news/list").unwrap();
        assert!(client
            .get(&url, None, 1024, &cancel)
            .await
            .unwrap_err()
            .contains("중단"));
    }
}
