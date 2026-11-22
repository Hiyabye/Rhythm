//! Request counts and times per host for the run log (#80), to tell many
//! requests from slow responses. The transport's own retries happen inside
//! one fetch, so a fetch longer than one attempt's timeout means a retry.

use rhythm_sources::transport::{Access, Fetch, FetchError, Transport};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::{atomic::AtomicBool, Mutex},
    time::{Duration, Instant},
};
use url::Url;

/// The transport's per-attempt timeout; a longer fetch was retried.
const ATTEMPT: Duration = Duration::from_secs(20);

/// A transport that records every fetch it passes on.
pub struct Measured<T> {
    inner: T,
    hosts: Mutex<BTreeMap<String, Host>>,
}

#[derive(Default)]
struct Host {
    requests: usize,
    failed: usize,
    not_modified: usize,
    retried: usize,
    bytes: usize,
    total: Duration,
    slowest: Duration,
    /// Count and time per path, digit runs folded to `N`.
    paths: BTreeMap<String, (usize, Duration)>,
}

impl<T> Measured<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            hosts: Mutex::new(BTreeMap::new()),
        }
    }

    /// One line per host, the host with the most request time first, naming
    /// its three most expensive paths.
    pub fn lines(&self) -> Vec<String> {
        let Ok(hosts) = self.hosts.lock() else {
            return Vec::new();
        };
        let mut hosts: Vec<_> = hosts.iter().collect();
        hosts.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.0.cmp(b.0)));
        hosts
            .into_iter()
            .map(|(name, host)| {
                let mut paths: Vec<_> = host.paths.iter().collect();
                paths.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(a.0.cmp(b.0)));
                let paths: Vec<String> = paths
                    .into_iter()
                    .take(3)
                    .map(|(path, (count, time))| format!("{path} {count}x {}s", time.as_secs()))
                    .collect();
                format!(
                    "requests to {name}: {} in {}s (average {:.1}s, slowest {:.1}s, {} retried, {} failed, {} not modified, {:.1} MB); most time: {}",
                    host.requests,
                    host.total.as_secs(),
                    host.total.as_secs_f64() / host.requests.max(1) as f64,
                    host.slowest.as_secs_f64(),
                    host.retried,
                    host.failed,
                    host.not_modified,
                    host.bytes as f64 / 1_000_000.0,
                    paths.join(", ")
                )
            })
            .collect()
    }

    fn record(&self, url: &Url, took: Duration, result: &Result<Fetch, FetchError>) {
        let Ok(mut hosts) = self.hosts.lock() else {
            return;
        };
        let host = hosts
            .entry(url.host_str().unwrap_or_default().to_owned())
            .or_default();
        host.requests += 1;
        host.total += took;
        host.slowest = host.slowest.max(took);
        if took > ATTEMPT {
            host.retried += 1;
        }
        match result {
            Ok(Fetch::Body { bytes, .. }) => host.bytes += bytes.len(),
            Ok(Fetch::NotModified) => host.not_modified += 1,
            Ok(Fetch::Redirect(_)) => {}
            Err(_) => host.failed += 1,
        }
        let path = host.paths.entry(fold_digits(url.path())).or_default();
        path.0 += 1;
        path.1 += took;
    }
}

/// `/ko/board/notice/123` → `/ko/board/notice/N`, so article paths add up.
fn fold_digits(path: &str) -> String {
    let mut folded = String::with_capacity(path.len());
    for c in path.chars() {
        if !c.is_ascii_digit() {
            folded.push(c);
        } else if !folded.ends_with('N') {
            folded.push('N');
        }
    }
    folded
}

impl<T: Transport> Transport for Measured<T> {
    async fn server(&self, host: &str) -> Option<IpAddr> {
        self.inner.server(host).await
    }

    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        let start = Instant::now();
        let result = self
            .inner
            .fetch(url, validators, limit, cancel, access)
            .await;
        self.record(url, start.elapsed(), &result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Answer;
    impl Transport for Answer {
        async fn fetch(
            &self,
            url: &Url,
            _: Option<(Option<String>, Option<String>)>,
            _: usize,
            _: &AtomicBool,
            _: Access,
        ) -> Result<Fetch, FetchError> {
            match url.path() {
                "/missing" => Err(FetchError::http(404, "없음".into())),
                "/same" => Ok(Fetch::NotModified),
                _ => Ok(Fetch::Body {
                    bytes: vec![0; 500_000],
                    etag: None,
                    last_modified: None,
                    mime: None,
                }),
            }
        }
    }

    #[tokio::test]
    async fn counts_requests_per_host_and_path() {
        let measured = Measured::new(Answer);
        let cancel = AtomicBool::new(false);
        for url in [
            "https://a.example/board/1?page=2",
            "https://a.example/board/22",
            "https://a.example/missing",
            "https://a.example/same",
            "https://b.example/robots.txt",
        ] {
            let url = Url::parse(url).unwrap();
            let _ = measured.fetch(&url, None, 1, &cancel, Access::Source).await;
        }
        let lines = measured.lines();
        assert_eq!(lines.len(), 2);
        let a = lines
            .iter()
            .find(|line| line.contains("a.example"))
            .unwrap();
        assert!(a.starts_with("requests to a.example: 4 in 0s"), "{a}");
        assert!(a.contains("1 failed, 1 not modified, 1.0 MB"), "{a}");
        assert!(a.contains("/board/N 2x"), "{a}");
        assert!(lines
            .iter()
            .any(|line| line.starts_with("requests to b.example: 1 in 0s")));
    }

    #[test]
    fn folds_digit_runs() {
        assert_eq!(fold_digits("/ko/board/notice/123"), "/ko/board/notice/N");
        assert_eq!(fold_digits("/bbs/a1b/22/x.do"), "/bbs/aNb/N/x.do");
    }
}
