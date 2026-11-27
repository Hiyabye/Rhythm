//! HTTPS transport for the signed feed (docs/FEED.md, #46/#177). It is
//! separate from the notice crawler's transport: the feed has no robots.txt,
//! pacing or school routes, only a base URL from the app or a signed
//! `moved_to` and the files the verified manifest names.
use reqwest::{header, StatusCode};
use rhythm_sources::transport::public_ip;
use std::{
    future::Future,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use url::{Host, Url};

/// Where the feed lives until a signed manifest moves it (#166 D22).
pub(crate) const DEFAULT_BASE: &str = "https://rhythm-feed.hiyabye0465.workers.dev/";
pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const SIGNATURE: &str = "manifest.json.sig";
/// The live manifest is about 10 KB; the signature is about 150 bytes.
pub(crate) const MANIFEST_LIMIT: u64 = 1 << 20;
pub(crate) const SIGNATURE_LIMIT: u64 = 4 << 10;
/// Any one listed file; the manifest gives its exact size (#172: the whole
/// feed was about 16 MB on 2026-11-18).
pub(crate) const FILE_LIMIT: u64 = 128 << 20;
const RETRY_AFTER_DEFAULT: Duration = Duration::from_secs(60);
const RETRY_AFTER_MAX: Duration = Duration::from_secs(60 * 60);

/// An HTTPS base URL ending in `/`, on a host name at port 443.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Base(Url);

impl Base {
    pub(crate) fn default_base() -> Self {
        Self::parse(DEFAULT_BASE).expect("the default feed base is valid")
    }

    /// Checks the app's base or a signed `moved_to`. The signature authorizes
    /// any host (user decision, 2026-11-18); the URL itself stays plain HTTPS.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let url = Url::parse(text).map_err(|_| INVALID_BASE.to_owned())?;
        let valid = url.scheme() == "https"
            && matches!(url.host(), Some(Host::Domain(_)))
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path().ends_with('/')
            && text.len() <= 2048;
        if valid {
            Ok(Self(url))
        } else {
            Err(INVALID_BASE.into())
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// `path` is the manifest, its signature or a path from a manifest that
    /// `Manifest::parse` accepted, so it is relative and never leaves the base.
    pub(crate) fn file(&self, path: &str) -> Result<Url, String> {
        self.0
            .join(path)
            .ok()
            .filter(|url| url.as_str().starts_with(self.0.as_str()))
            .ok_or_else(|| INVALID_BASE.into())
    }
}

const INVALID_BASE: &str = "공지 피드 주소가 올바르지 않습니다.";

/// Why a request failed. `Offline` is shown quietly (#46: an unreachable feed
/// keeps cached data without repeated errors).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Offline,
    RateLimited(Duration),
    Failed(String),
    Cancelled,
}

impl FetchError {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Offline => "공지 피드에 연결하지 못했습니다. 인터넷 연결을 확인해 주세요.".into(),
            Self::RateLimited(_) => {
                "공지 피드 서버가 요청을 제한했습니다. 잠시 후 다시 시도합니다.".into()
            }
            Self::Failed(message) => message.clone(),
            Self::Cancelled => "공지 피드 동기화를 중단했습니다.".into(),
        }
    }
}

/// The network boundary; tests replace only this.
pub trait Transport: Sync {
    /// GETs `url`, refusing a body larger than `limit` bytes.
    fn get(
        &self,
        url: &Url,
        limit: u64,
        cancel: &AtomicBool,
    ) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send;
}

/// Sends the app's `user_agent` (`Rhythm/<version>`).
pub struct HttpsTransport {
    user_agent: &'static str,
}

impl HttpsTransport {
    pub fn new(user_agent: &'static str) -> Self {
        Self { user_agent }
    }
}

impl Transport for HttpsTransport {
    async fn get(&self, url: &Url, limit: u64, cancel: &AtomicBool) -> Result<Vec<u8>, FetchError> {
        tokio::select! {
            result = fetch(url, limit, self.user_agent, cancel) => result,
            _ = cancelled(cancel) => Err(FetchError::Cancelled),
        }
    }
}

async fn fetch(
    url: &Url,
    limit: u64,
    user_agent: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, FetchError> {
    let host = url
        .host_str()
        .ok_or_else(|| FetchError::Failed(INVALID_BASE.into()))?;
    // Resolve once, require public answers and pin the connection to them, so
    // a proxy or a rebinding answer cannot reach a local address.
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::lookup_host((host, 443)),
    )
    .await
    .map_err(|_| FetchError::Offline)?
    .map_err(|_| FetchError::Offline)?
    .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(FetchError::Failed(
            "공개 인터넷이 아닌 공지 피드 연결을 차단했습니다.".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(0)
        .user_agent(user_agent)
        .build()
        .map_err(|_| FetchError::Failed("공지 피드 HTTPS 연결을 준비하지 못했습니다.".into()))?;
    let mut response = client
        .get(url.clone())
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|_| FetchError::Offline)?;
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(FetchError::RateLimited(retry_after(
            response.headers().get(header::RETRY_AFTER),
        )));
    }
    if !status.is_success() {
        // The Worker never redirects; a move comes only through `moved_to`.
        return Err(FetchError::Failed(format!(
            "공지 피드를 받지 못했습니다. (HTTP {})",
            status.as_u16()
        )));
    }
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(FetchError::Failed(
            "압축된 공지 피드 응답을 거부했습니다.".into(),
        ));
    }
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Offline)? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
        if cancel.load(Ordering::Acquire) {
            return Err(FetchError::Cancelled);
        }
    }
    Ok(bytes)
}

fn too_large() -> FetchError {
    FetchError::Failed("공지 피드 파일이 허용된 크기를 넘었습니다.".into())
}

/// Seconds only (the Worker sends `retry-after: 60`); anything else waits the
/// default, and a huge value is capped so a bad header cannot stop syncing.
fn retry_after(value: Option<&header::HeaderValue>) -> Duration {
    value
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(RETRY_AFTER_DEFAULT)
        .min(RETRY_AFTER_MAX)
}

pub(crate) async fn cancelled(cancel: &AtomicBool) {
    crate::job::cancelled(cancel, Duration::from_millis(50)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_accepts_only_plain_https_host_names() {
        for good in [
            DEFAULT_BASE,
            "https://feed.example.org/",
            // A bare origin is its root.
            "https://feed.example.org",
            "https://feed.example.org:443/v2/",
        ] {
            assert!(Base::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "http://feed.example.org/",
            "https://feed.example.org/v2",
            "https://feed.example.org:8443/",
            "https://user@feed.example.org/",
            "https://feed.example.org/?x=1",
            "https://feed.example.org/#x",
            "https://203.0.113.7/",
            "https://[2001:db8::1]/",
            "file:///etc/",
        ] {
            assert!(Base::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn files_stay_under_the_base() {
        let base = Base::parse("https://feed.example.org/v2/").unwrap();
        assert_eq!(
            base.file("segments/run-1-2-ab.jsonl.zst").unwrap().as_str(),
            "https://feed.example.org/v2/segments/run-1-2-ab.jsonl.zst"
        );
        assert_eq!(
            base.file(MANIFEST).unwrap().as_str(),
            "https://feed.example.org/v2/manifest.json"
        );
        for escape in [
            "../x",
            "/x",
            "https://other.example.org/x",
            "//other.example.org/x",
        ] {
            assert!(base.file(escape).is_err(), "{escape}");
        }
    }

    #[test]
    fn retry_after_reads_seconds_with_a_cap() {
        let value = |text: &str| header::HeaderValue::from_str(text).unwrap();
        assert_eq!(retry_after(Some(&value("60"))), Duration::from_secs(60));
        assert_eq!(retry_after(Some(&value(" 5 "))), Duration::from_secs(5));
        assert_eq!(retry_after(None), RETRY_AFTER_DEFAULT);
        assert_eq!(
            retry_after(Some(&value("Wed, 21 Oct 2015 07:28:00 GMT"))),
            RETRY_AFTER_DEFAULT
        );
        assert_eq!(retry_after(Some(&value("999999"))), RETRY_AFTER_MAX);
    }
}
