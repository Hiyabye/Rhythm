//! Exercise Client::get/attempt with DNS and wire responses replaced in memory.
//! No live school traffic; the injected connection failure is not native TLS
//! handshake evidence. R46/native compatibility still needs a platform live run.
use super::*;
use std::{collections::VecDeque, sync::Mutex as StdMutex};

enum Reply {
    Http(u16),
    /// A 302 to this `Location`.
    Redirect(&'static str),
    /// A 200 whose body of this many bytes arrives at once, with its length known.
    Body(usize),
    ConnectionFailure,
    #[cfg(feature = "studium")]
    Pending,
    /// A 200 whose body arrives as `chunks` blocks of `CHUNK` bytes, one
    /// every `every`.
    Trickle {
        chunks: usize,
        every: Duration,
    },
}

static CHUNK: [u8; 64 * 1024] = [b'a'; 64 * 1024];

struct Trickle {
    left: usize,
    every: Duration,
    sleep: std::pin::Pin<Box<tokio::time::Sleep>>,
}

impl http_body::Body for Trickle {
    type Data = &'static [u8];
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if self.left == 0 {
            return std::task::Poll::Ready(None);
        }
        std::task::ready!(self.sleep.as_mut().poll(cx));
        self.left -= 1;
        let next = tokio::time::Instant::now() + self.every;
        self.sleep.as_mut().reset(next);
        std::task::Poll::Ready(Some(Ok(http_body::Frame::data(&CHUNK[..]))))
    }
}

pub(super) struct Script {
    addresses: Vec<SocketAddr>,
    lookups: StdMutex<Vec<(String, u16)>>,
    requests: StdMutex<Vec<reqwest::Request>>,
    replies: StdMutex<VecDeque<Reply>>,
}

impl Script {
    fn client(addresses: Vec<SocketAddr>, replies: Vec<Reply>) -> (Client, Arc<Self>) {
        let script = Arc::new(Self {
            addresses,
            lookups: StdMutex::new(Vec::new()),
            requests: StdMutex::new(Vec::new()),
            replies: StdMutex::new(replies.into()),
        });
        let mut client = Client::new(TEST_AGENT).unwrap();
        client.test_io = Some(script.clone());
        (client, script)
    }

    pub(super) fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, Failure> {
        self.lookups.lock().unwrap().push((host.into(), port));
        Ok(self.addresses.clone())
    }

    pub(super) async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, Failure> {
        self.requests.lock().unwrap().push(request.build().unwrap());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("Client made an unexpected extra attempt");
        match reply {
            Reply::Http(status) => Ok(http::Response::builder()
                .status(status)
                .header("Location", "/SEAT/roomview5.asp?room_no=5")
                .header("Content-Type", "text/html; charset=euc-kr")
                .body("synthetic response")
                .unwrap()
                .into()),
            Reply::Redirect(location) => Ok(http::Response::builder()
                .status(302)
                .header("Location", location)
                .body("")
                .unwrap()
                .into()),
            Reply::Body(size) => Ok(http::Response::builder()
                .status(200)
                .header("Content-Type", "text/html")
                .header("ETag", "\"v2\"")
                .body(vec![b'a'; size])
                .unwrap()
                .into()),
            Reply::ConnectionFailure => Err(Failure::Retry(CONNECTION_FAILED.into())),
            #[cfg(feature = "studium")]
            Reply::Pending => std::future::pending().await,
            Reply::Trickle { chunks, every } => Ok(http::Response::builder()
                .status(200)
                .header("Content-Type", "text/html")
                .body(reqwest::Body::wrap(Trickle {
                    left: chunks,
                    every,
                    sleep: Box::pin(tokio::time::sleep(every)),
                }))
                .unwrap()
                .into()),
        }
    }

    #[cfg(feature = "studium")]
    fn assert_one_request(&self, url: &Url) {
        assert_eq!(
            self.lookups.lock().unwrap().as_slice(),
            &[("libseat.sogang.ac.kr".into(), 444)]
        );
        let requests = self.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url(), url);
        assert_eq!(requests[0].headers()[header::CACHE_CONTROL], "no-cache");
    }
}

#[cfg(feature = "studium")]
fn public_addresses() -> Vec<SocketAddr> {
    vec!["8.8.8.8:444".parse().unwrap()]
}

#[cfg(feature = "studium")]
fn status_url() -> Url {
    Url::parse(crate::studium::STATUS_URL).unwrap()
}

#[cfg(feature = "studium")]
async fn get(client: &Client, url: &Url) -> Result<Fetch, FetchError> {
    client
        .get(url, None, 1024, &AtomicBool::new(false), Access::Studium)
        .await
}

#[cfg(feature = "studium")]
#[tokio::test]
async fn studium_client_makes_one_attempt_on_retriable_http_or_connection_failure() {
    for reply in [
        Reply::ConnectionFailure,
        Reply::Http(408),
        Reply::Http(500),
        Reply::Http(503),
    ] {
        let (client, script) = Script::client(public_addresses(), vec![reply]);
        assert!(get(&client, &status_url()).await.is_err());
        script.assert_one_request(&status_url());
    }
}

#[cfg(feature = "studium")]
#[tokio::test]
async fn studium_client_refuses_each_redirect_without_a_second_request() {
    for status in [301, 302, 303, 307, 308] {
        let (client, script) = Script::client(public_addresses(), vec![Reply::Http(status)]);
        assert!(get(&client, &status_url())
            .await
            .unwrap_err()
            .message
            .contains("이동 응답을 거부"));
        script.assert_one_request(&status_url());
    }
}

#[cfg(feature = "studium")]
#[tokio::test]
async fn studium_client_only_missing_robots_policies_become_empty() {
    let url = Url::parse("https://libseat.sogang.ac.kr:444/robots.txt").unwrap();
    for status in [404, 410] {
        let (client, script) = Script::client(public_addresses(), vec![Reply::Http(status)]);
        match get(&client, &url).await.unwrap() {
            Fetch::Body { bytes, mime, .. } => {
                assert!(bytes.is_empty());
                assert_eq!(mime.as_deref(), Some("text/plain"));
            }
            _ => panic!("missing robots policy must be an empty body"),
        }
        script.assert_one_request(&url);
    }
    for status in [400, 401, 403, 429, 500] {
        let (client, script) = Script::client(public_addresses(), vec![Reply::Http(status)]);
        let error = get(&client, &url).await.unwrap_err();
        assert_eq!(error.status, Some(status));
        if status == 429 {
            assert_eq!(error.message, RATE_LIMITED);
        } else {
            assert!(error.message.contains(&format!("HTTP {status}")), "{error}");
        }
        assert_eq!(error.rate_limited(), status == 429);
        assert_eq!(error.access_denied(), matches!(status, 401 | 403));
        script.assert_one_request(&url);
    }
    // A 404 page is not a missing robots policy.
    let (client, _) = Script::client(public_addresses(), vec![Reply::Http(404)]);
    assert!(get(&client, &status_url()).await.is_err());
}

#[cfg(feature = "studium")]
#[tokio::test]
async fn studium_client_resolves_port_444_and_checks_every_dns_answer_before_send() {
    for addresses in [
        vec![],
        vec!["127.0.0.1:444".parse().unwrap()],
        vec!["[::1]:444".parse().unwrap()],
        vec![
            "8.8.8.8:444".parse().unwrap(),
            "192.168.0.1:444".parse().unwrap(),
        ],
    ] {
        let (client, script) = Script::client(addresses, vec![]);
        assert!(get(&client, &status_url())
            .await
            .unwrap_err()
            .message
            .contains("공개 인터넷 주소"));
        assert_eq!(
            script.lookups.lock().unwrap().as_slice(),
            &[("libseat.sogang.ac.kr".into(), 444)]
        );
        assert!(script.requests.lock().unwrap().is_empty());
    }
    let (client, script) = Script::client(public_addresses(), vec![Reply::Http(200)]);
    assert!(matches!(
        get(&client, &status_url()).await,
        Ok(Fetch::Body { .. })
    ));
    script.assert_one_request(&status_url());
}

#[cfg(feature = "studium")]
#[tokio::test]
async fn studium_client_rejects_unapproved_routes_before_resolving() {
    let (client, script) = Script::client(public_addresses(), vec![]);
    for value in [
        "https://libseat.sogang.ac.kr/SEAT/domian5.asp",
        "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=5&extra=1",
        "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=999",
        "https://example.com:444/SEAT/domian5.asp",
    ] {
        assert!(get(&client, &Url::parse(value).unwrap()).await.is_err());
    }
    assert!(script.lookups.lock().unwrap().is_empty());
    assert!(script.requests.lock().unwrap().is_empty());
}

#[cfg(feature = "studium")]
#[tokio::test(start_paused = true)]
async fn studium_client_deadline_ends_a_hung_attempt_without_retry() {
    let (client, script) = Script::client(public_addresses(), vec![Reply::Pending]);
    let started = tokio::time::Instant::now();
    assert!(get(&client, &status_url())
        .await
        .unwrap_err()
        .message
        .contains("응답 시간이 초과"));
    assert_eq!(started.elapsed(), HEADER_WAIT);
    script.assert_one_request(&status_url());
}

#[cfg(feature = "studium")]
#[tokio::test(start_paused = true)]
async fn studium_client_cancellation_stops_a_hung_attempt_without_retry() {
    let (client, script) = Script::client(public_addresses(), vec![Reply::Pending]);
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let task = tokio::spawn(async move {
        client
            .get(&status_url(), None, 1024, &signal, Access::Studium)
            .await
    });
    // Poll into the request before setting the cancellation flag.
    for _ in 0..10 {
        if !script.requests.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    assert_eq!(script.requests.lock().unwrap().len(), 1);
    cancel.store(true, Ordering::Release);
    tokio::time::advance(Duration::from_millis(100)).await;
    assert!(task.await.unwrap().unwrap_err().message.contains("중단"));
    script.assert_one_request(&status_url());
}

#[cfg(feature = "studium")]
async fn get_large(client: &Client, url: &Url) -> Result<Fetch, FetchError> {
    client
        .get(url, None, 4 << 20, &AtomicBool::new(false), Access::Studium)
        .await
}

#[cfg(feature = "studium")]
#[tokio::test(start_paused = true)]
async fn a_slow_body_within_its_deadline_completes() {
    // 1.25 MB over 60 s: what the collector's runner saw on se.sogang.ac.kr,
    // past the old 20 s total timeout.
    let (client, script) = Script::client(
        public_addresses(),
        vec![Reply::Trickle {
            chunks: 20,
            every: Duration::from_secs(3),
        }],
    );
    match get_large(&client, &status_url()).await.unwrap() {
        Fetch::Body { bytes, .. } => assert_eq!(bytes.len(), 20 * CHUNK.len()),
        _ => panic!("a slow body must still arrive"),
    }
    script.assert_one_request(&status_url());
}

#[cfg(feature = "studium")]
#[tokio::test(start_paused = true)]
async fn a_body_past_its_deadline_is_reported_as_a_timeout() {
    let (client, script) = Script::client(
        public_addresses(),
        vec![Reply::Trickle {
            // Under the byte limit, but it would take 120 s.
            chunks: 60,
            every: Duration::from_secs(2),
        }],
    );
    let started = tokio::time::Instant::now();
    let error = get_large(&client, &status_url()).await.unwrap_err();
    assert_eq!(error.message, TIMED_OUT);
    assert_eq!(started.elapsed(), BODY_WAIT);
    script.assert_one_request(&status_url());
}

#[tokio::test]
async fn only_klec_missing_robots_policy_becomes_empty() {
    let source = |client: &Client, url: &Url| {
        let url = url.clone();
        let client = client.clone();
        async move {
            client
                .get(&url, None, 1024, &AtomicBool::new(false), Access::Source)
                .await
        }
    };
    let addresses = vec!["8.8.8.8:443".parse().unwrap()];
    let klec = Url::parse("https://klec.sogang.ac.kr/robots.txt").unwrap();
    for status in [404, 410] {
        let (client, script) = Script::client(addresses.clone(), vec![Reply::Http(status)]);
        match source(&client, &klec).await.unwrap() {
            Fetch::Body { bytes, mime, .. } => {
                assert!(bytes.is_empty());
                assert_eq!(mime.as_deref(), Some("text/plain"));
            }
            _ => panic!("missing robots policy must be an empty body"),
        }
        let requests = script.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url(), &klec);
    }
    let scec = Url::parse("https://scec.sogang.ac.kr/robots.txt").unwrap();
    let page =
        Url::parse("https://klec.sogang.ac.kr/index.php?bbs_idx=1&pagekind=c&bbsid=00010").unwrap();
    for (url, status) in [
        (&klec, 401),
        (&klec, 403),
        (&klec, 429),
        (&scec, 404),
        (&page, 404),
    ] {
        let (client, _) = Script::client(addresses.clone(), vec![Reply::Http(status)]);
        assert_eq!(source(&client, url).await.unwrap_err().status, Some(status));
    }
}

#[tokio::test]
async fn only_the_calendar_hosts_missing_robots_policy_becomes_empty() {
    let fetch = |client: &Client, url: &Url, access: Access| {
        let url = url.clone();
        let client = client.clone();
        async move {
            client
                .get(&url, None, 1024, &AtomicBool::new(false), access)
                .await
        }
    };
    let addresses = vec!["8.8.8.8:443".parse().unwrap()];
    let robots = Url::parse("https://outlook.office365.com/robots.txt").unwrap();
    // Observed: 302 to the mail login, which answers 417.
    for status in [302, 404, 417] {
        let (client, script) = Script::client(addresses.clone(), vec![Reply::Http(status)]);
        match fetch(&client, &robots, Access::Calendar).await.unwrap() {
            Fetch::Body { bytes, mime, .. } => {
                assert!(bytes.is_empty());
                assert_eq!(mime.as_deref(), Some("text/plain"));
            }
            _ => panic!("missing robots policy must be an empty body"),
        }
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }
    // Denied or rate limited is never allow-all, and the ICS's own errors stay errors.
    let ics = Url::parse(crate::calendar::ACADEMIC_CALENDAR_URL).unwrap();
    for (url, status) in [(&robots, 401), (&robots, 403), (&robots, 429), (&ics, 404)] {
        let (client, _) = Script::client(addresses.clone(), vec![Reply::Http(status)]);
        assert_eq!(
            fetch(&client, url, Access::Calendar)
                .await
                .unwrap_err()
                .status,
            Some(status)
        );
    }
    // School-source access cannot reach the calendar host.
    let (client, script) = Script::client(addresses.clone(), vec![Reply::Http(200)]);
    assert_eq!(
        fetch(&client, &ics, Access::Source)
            .await
            .unwrap_err()
            .message,
        BLOCKED
    );
    assert!(script.requests.lock().unwrap().is_empty());
}

const PAGE: &str = "https://klec.sogang.ac.kr/index.php?bbs_idx=1&pagekind=c&bbsid=00010";

fn school_addresses() -> Vec<SocketAddr> {
    vec!["8.8.8.8:443".parse().unwrap()]
}

async fn fetch(
    client: &Client,
    url: &str,
    validators: Option<(Option<String>, Option<String>)>,
    max_bytes: usize,
    access: Access,
) -> Result<Fetch, FetchError> {
    let url = Url::parse(url).unwrap();
    client
        .get(&url, validators, max_bytes, &AtomicBool::new(false), access)
        .await
}

#[tokio::test(start_paused = true)]
async fn a_body_over_the_callers_limit_is_refused_without_a_retry() {
    // Announced by its length: refused before any byte is read.
    let (client, script) = Script::client(school_addresses(), vec![Reply::Body(1025)]);
    let error = fetch(&client, PAGE, None, 1024, Access::Source)
        .await
        .unwrap_err();
    assert_eq!(error.message, TOO_LARGE);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
    // Exactly the limit arrives.
    let (client, _) = Script::client(school_addresses(), vec![Reply::Body(1024)]);
    match fetch(&client, PAGE, None, 1024, Access::Source)
        .await
        .unwrap()
    {
        Fetch::Body { bytes, .. } => assert_eq!(bytes.len(), 1024),
        _ => panic!("a body at the limit must arrive"),
    }
    // Without a length the limit applies while streaming.
    let trickle = Reply::Trickle {
        chunks: 3,
        every: Duration::from_millis(10),
    };
    let (client, script) = Script::client(school_addresses(), vec![trickle]);
    let error = fetch(&client, PAGE, None, 2 * CHUNK.len(), Access::Source)
        .await
        .unwrap_err();
    assert_eq!(error.message, TOO_LARGE);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_source_request_is_tried_three_times_and_resolves_before_each() {
    let replies = vec![Reply::Http(503), Reply::ConnectionFailure, Reply::Body(4)];
    let (client, script) = Script::client(school_addresses(), replies);
    let started = tokio::time::Instant::now();
    assert!(matches!(
        fetch(&client, PAGE, None, 1024, Access::Source).await,
        Ok(Fetch::Body { .. })
    ));
    // Pauses of 1 s and 2 s between the attempts.
    assert!(started.elapsed() >= Duration::from_secs(3));
    assert_eq!(script.requests.lock().unwrap().len(), 3);
    // Every attempt checks the DNS answers again before it connects.
    assert_eq!(
        script.lookups.lock().unwrap().as_slice(),
        vec![("klec.sogang.ac.kr".to_string(), 443); 3]
    );
    // The third failure is final, and a refusal is never retried.
    let replies = vec![Reply::Http(500), Reply::Http(500), Reply::Http(500)];
    let (client, script) = Script::client(school_addresses(), replies);
    let error = fetch(&client, PAGE, None, 1024, Access::Source)
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(script.requests.lock().unwrap().len(), 3);
    let (client, script) = Script::client(school_addresses(), vec![Reply::Http(404)]);
    let error = fetch(&client, PAGE, None, 1024, Access::Source)
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(404));
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_redirect_is_returned_only_when_its_target_passes_the_policy() {
    let target = "https://klec.sogang.ac.kr/index.php?bbs_idx=2&pagekind=c&bbsid=00010";
    for location in [target, "/index.php?bbs_idx=2&pagekind=c&bbsid=00010"] {
        let (client, script) = Script::client(school_addresses(), vec![Reply::Redirect(location)]);
        match fetch(&client, PAGE, None, 1024, Access::Source)
            .await
            .unwrap()
        {
            Fetch::Redirect(url) => assert_eq!(url.as_str(), target),
            _ => panic!("the client must hand the redirect to its caller"),
        }
        // The client itself never follows it.
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }
    for location in [
        "https://example.com/index.php?bbs_idx=2&pagekind=c&bbsid=00010",
        "http://klec.sogang.ac.kr/index.php?bbs_idx=2&pagekind=c&bbsid=00010",
        "https://klec.sogang.ac.kr/admin.php",
        "https://127.0.0.1/",
    ] {
        let (client, script) = Script::client(school_addresses(), vec![Reply::Redirect(location)]);
        let error = fetch(&client, PAGE, None, 1024, Access::Source)
            .await
            .unwrap_err();
        assert_eq!(error.message, BLOCKED, "{location}");
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn only_another_hosts_unavailable_robots_policy_means_no_rules_for_an_image() {
    let other = "https://images.example.com/robots.txt";
    for status in [401, 403, 404, 410] {
        let (client, _) = Script::client(school_addresses(), vec![Reply::Http(status)]);
        match fetch(&client, other, None, 1024, Access::EmbeddedImage)
            .await
            .unwrap()
        {
            Fetch::Body { bytes, mime, .. } => {
                assert!(bytes.is_empty());
                assert_eq!(mime.as_deref(), Some("text/plain"));
            }
            _ => panic!("an unavailable robots policy must be an empty body"),
        }
    }
    // A rate limit or a server error is not "no rules".
    for (status, replies) in [(429, 1), (500, 3)] {
        let replies = (0..replies).map(|_| Reply::Http(status)).collect();
        let (client, _) = Script::client(school_addresses(), replies);
        let error = fetch(&client, other, None, 1024, Access::EmbeddedImage)
            .await
            .unwrap_err();
        assert_eq!(error.status, Some(status));
    }
    // Nor is a school host's error, nor another host's missing image.
    for url in [
        "https://scec.sogang.ac.kr/robots.txt",
        "https://images.example.com/a.png",
    ] {
        let (client, _) = Script::client(school_addresses(), vec![Reply::Http(404)]);
        let error = fetch(&client, url, None, 1024, Access::EmbeddedImage)
            .await
            .unwrap_err();
        assert_eq!(error.status, Some(404), "{url}");
    }
    // Ordinary source access never reaches another host at all.
    let (client, script) = Script::client(school_addresses(), vec![]);
    let error = fetch(&client, other, None, 1024, Access::Source)
        .await
        .unwrap_err();
    assert_eq!(error.message, BLOCKED);
    assert!(script.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn not_modified_needs_the_validators_the_request_sent() {
    let validators = Some((Some("\"v1\"".to_string()), Some("yesterday".to_string())));
    let (client, script) = Script::client(school_addresses(), vec![Reply::Http(304)]);
    assert!(matches!(
        fetch(&client, PAGE, validators, 1024, Access::Source).await,
        Ok(Fetch::NotModified)
    ));
    {
        let requests = script.requests.lock().unwrap();
        assert_eq!(requests[0].headers()[header::IF_NONE_MATCH], "\"v1\"");
        assert_eq!(
            requests[0].headers()[header::IF_MODIFIED_SINCE],
            "yesterday"
        );
    }
    // A 304 to a request that sent none would leave the caller without a document.
    for validators in [None, Some((None, None))] {
        let (client, _) = Script::client(school_addresses(), vec![Reply::Http(304)]);
        assert!(fetch(&client, PAGE, validators, 1024, Access::Source)
            .await
            .is_err());
    }
    // A changed document carries its new validator.
    let (client, _) = Script::client(school_addresses(), vec![Reply::Body(4)]);
    match fetch(&client, PAGE, None, 1024, Access::Source)
        .await
        .unwrap()
    {
        Fetch::Body { etag, mime, .. } => {
            assert_eq!(etag.as_deref(), Some("\"v2\""));
            assert_eq!(mime.as_deref(), Some("text/html"));
        }
        _ => panic!("a 200 must be a body"),
    }
}
