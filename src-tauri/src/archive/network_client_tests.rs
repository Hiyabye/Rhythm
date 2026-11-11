//! Exercise Client::get/attempt with DNS and wire responses replaced in memory.
//! No live school traffic; the injected connection failure is not native TLS
//! handshake evidence. R46/native compatibility still needs a platform live run.
use super::*;
use std::{collections::VecDeque, sync::Mutex as StdMutex};

enum Reply {
    Http(u16),
    ConnectionFailure,
    Pending,
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
        let mut client = Client::new().unwrap();
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
            Reply::ConnectionFailure => Err(Failure::Retry(CONNECTION_FAILED.into())),
            Reply::Pending => std::future::pending().await,
        }
    }

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

fn public_addresses() -> Vec<SocketAddr> {
    vec!["8.8.8.8:444".parse().unwrap()]
}

fn status_url() -> Url {
    Url::parse(crate::studium::rooms::STATUS_URL).unwrap()
}

async fn get(client: &Client, url: &Url) -> Result<Fetch, String> {
    client
        .get(url, None, 1024, &AtomicBool::new(false), Access::Studium)
        .await
}

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

#[tokio::test]
async fn studium_client_refuses_each_redirect_without_a_second_request() {
    for status in [301, 302, 303, 307, 308] {
        let (client, script) = Script::client(public_addresses(), vec![Reply::Http(status)]);
        assert!(get(&client, &status_url())
            .await
            .unwrap_err()
            .contains("이동 응답을 거부"));
        script.assert_one_request(&status_url());
    }
}

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
        if status == 429 {
            assert_eq!(error, RATE_LIMITED);
        } else {
            assert!(error.contains(&format!("HTTP {status}")), "{error}");
        }
        assert_eq!(
            is_access_denied(&error),
            matches!(status, 401 | 403),
            "{error}"
        );
        script.assert_one_request(&url);
    }
    // A 404 page is not a missing robots policy.
    let (client, _) = Script::client(public_addresses(), vec![Reply::Http(404)]);
    assert!(get(&client, &status_url()).await.is_err());
}

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

#[tokio::test(start_paused = true)]
async fn studium_client_deadline_ends_a_hung_attempt_without_retry() {
    let (client, script) = Script::client(public_addresses(), vec![Reply::Pending]);
    let started = tokio::time::Instant::now();
    assert!(get(&client, &status_url())
        .await
        .unwrap_err()
        .contains("응답 시간이 초과"));
    assert_eq!(started.elapsed(), Duration::from_secs(25));
    script.assert_one_request(&status_url());
}

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
    assert!(task.await.unwrap().unwrap_err().contains("중단"));
    script.assert_one_request(&status_url());
}
