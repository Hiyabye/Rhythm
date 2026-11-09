use super::*;

pub(super) fn validate_destination(url: &Url, origin: &Url) -> Result<(), String> {
    let approved = match url.host_str() {
        Some("huggingface.co") => {
            url.path() == origin.path() && url.query().is_none_or(|q| q == "download=true")
        }
        Some(
            "cdn-lfs.huggingface.co"
            | "cdn-lfs-us-1.huggingface.co"
            | "cdn-lfs-eu-1.huggingface.co"
            | "cas-bridge.xethub.hf.co"
            | "us.aws.cdn.hf.co",
        ) => true,
        _ => false,
    };
    if !approved
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 16384
    {
        return Err("검증되지 않은 모델 다운로드 이동 주소를 차단했습니다.".into());
    }
    Ok(())
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
                || (a == 192 && b == 0 && [0, 2].contains(&c))
                || (a == 198 && [18, 19].contains(&b))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}
// The fake transport changes only the network boundary; production disk/hash/resume logic is shared.
pub(super) trait Transport: Sync {
    fn response(
        &self,
        url: &Url,
        total: u64,
        offset: u64,
        etag: Option<&str>,
        cancel: &AtomicBool,
    ) -> impl std::future::Future<Output = Result<DownloadResponse, String>> + Send;
}
pub(super) struct DownloadResponse {
    pub(super) status: u16,
    pub(super) length: Option<u64>,
    pub(super) range: Option<String>,
    pub(super) etag: Option<String>,
    pub(super) body: Body,
}
pub(super) enum Body {
    Https(reqwest::Response),
    #[cfg(test)]
    Test(std::collections::VecDeque<Result<Vec<u8>, String>>),
}
impl Body {
    pub(super) async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Https(response) => response
                .chunk()
                .await
                .map(|v| v.map(|b| b.to_vec()))
                .map_err(|_| {
                    "모델 연결이 끊어졌습니다. 다시 선택하면 이어받을 수 있습니다.".into()
                }),
            #[cfg(test)]
            Self::Test(chunks) => chunks.pop_front().transpose(),
        }
    }
}
pub(super) struct HttpsTransport;
impl Transport for HttpsTransport {
    async fn response(
        &self,
        url: &Url,
        _total: u64,
        offset: u64,
        etag: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<DownloadResponse, String> {
        let origin = url.clone();
        let mut url = url.clone();
        for hop in 0..=5 {
            check_cancel(cancel)?;
            validate_destination(&url, &origin)?;
            let host = url.host_str().ok_or("모델 서버 주소가 없습니다.")?;
            let addresses: Vec<SocketAddr> = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::net::lookup_host((host, 443)),
            )
            .await
            .map_err(|_| "모델 서버 주소 조회 시간이 초과되었습니다.")?
            .map_err(|_| "모델 서버 주소를 찾지 못했습니다.")?
            .collect();
            if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
                return Err("공개 인터넷이 아닌 모델 서버 연결을 차단했습니다.".into());
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
                .user_agent(concat!("Rhythm/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| "모델 HTTPS 연결을 준비하지 못했습니다.")?;
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if offset > 0 {
                request = request.header(header::RANGE, format!("bytes={offset}-"));
                if let Some(etag) = etag {
                    request = request.header(header::IF_RANGE, etag);
                }
            }
            let response = tokio::select! {r=request.send()=>r.map_err(|_|"모델 서버에 안전하게 연결하지 못했습니다.")?,_=cancelled(cancel)=>return check_cancel(cancel).and(Err("설치를 취소했습니다.".into()))};
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err("모델 다운로드 이동 횟수가 너무 많습니다.".into());
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or("모델 이동 주소가 없습니다.")?;
                url = url
                    .join(location)
                    .map_err(|_| "모델 이동 주소가 올바르지 않습니다.")?;
                continue;
            }
            if response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|v| v != "identity")
            {
                return Err("압축된 모델 다운로드 응답을 거부했습니다.".into());
            }
            let text_header = |name| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .filter(|s| s.len() <= 4096)
                    .map(str::to_owned)
            };
            return Ok(DownloadResponse {
                status: response.status().as_u16(),
                length: response.content_length(),
                range: text_header(header::CONTENT_RANGE),
                etag: text_header(header::ETAG).filter(|v| strong_etag(v)),
                body: Body::Https(response),
            });
        }
        unreachable!()
    }
}
pub(super) fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 1024
        && value.starts_with('"')
        && value.ends_with('"')
        && !value.contains(['\r', '\n'])
}
pub(super) fn validate_response(
    response: &DownloadResponse,
    offset: u64,
    total: u64,
    expected_etag: Option<&str>,
) -> Result<bool, String> {
    match response.status {
        200 => {
            if response.length.is_some_and(|n| n != total) {
                return Err("모델 다운로드 크기가 고정 목록과 다릅니다.".into());
            }
            Ok(false)
        }
        206 if offset > 0 => {
            let expected = format!("bytes {offset}-{}/{total}", total - 1);
            if response.range.as_deref() != Some(expected.as_str())
                || response.length.is_some_and(|n| n != total - offset)
                || expected_etag.is_none()
                || response.etag.as_deref() != expected_etag
            {
                return Err(
                    "이어받기 응답의 범위나 파일 검증 값이 달라졌습니다. 다시 설치해 주세요."
                        .into(),
                );
            }
            Ok(true)
        }
        _ => Err(format!(
            "모델 다운로드에 실패했습니다. (HTTP {})",
            response.status
        )),
    }
}
