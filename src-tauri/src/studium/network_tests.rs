//! In-memory transport responses only; never contacts the seat server.
use super::*;
use crate::archive::network::FetchError;
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Mutex},
};

struct Stub {
    replies: Mutex<VecDeque<Result<Fetch, FetchError>>>,
    requests: Mutex<Vec<String>>,
}

impl Stub {
    fn new(replies: Vec<Result<Fetch, FetchError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn routes(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for Stub {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        assert_eq!(access, Access::Studium);
        assert!(validators.is_none());
        let route = url[url::Position::BeforePath..url::Position::AfterQuery].to_string();
        assert_eq!(
            limit,
            if route == "/robots.txt" {
                64 * 1024
            } else {
                RESPONSE_LIMIT
            }
        );
        self.requests.lock().unwrap().push(route);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra fetch")
    }
}

fn response(bytes: &[u8], mime: Option<&str>) -> Result<Fetch, FetchError> {
    Ok(Fetch::Body {
        bytes: bytes.to_vec(),
        mime: mime.map(str::to_string),
        etag: None,
        last_modified: None,
    })
}

fn allow() -> Result<Fetch, FetchError> {
    response(b"User-agent: *\nAllow: /\n", Some("text/plain"))
}

#[tokio::test]
async fn robots_denial_stops_before_requesting_the_common_page() {
    let fake = Stub::new(vec![response(
        b"User-agent: *\nDisallow: /SEAT/\n",
        Some("text/plain"),
    )]);
    let cancel = AtomicBool::new(false);
    let mut session = Session::new(&fake, &cancel).await.unwrap();
    assert!(session.html(rooms::STATUS_URL, &cancel).await.is_err());
    assert_eq!(fake.routes(), ["/robots.txt"]);
}

#[tokio::test]
async fn unreadable_robots_errors_redirects_and_unhonorable_delay_fail_closed() {
    for reply in [
        response(b"<html>error</html>", Some("text/html")),
        response(&[0xff], Some("text/plain")),
        response(b"User-agent: *\nCrawl-delay: 61\n", Some("text/plain")),
        Ok(Fetch::Redirect(
            Url::parse("https://example.com/robots.txt").unwrap(),
        )),
        Err("HTTP 403".into()),
    ] {
        let fake = Stub::new(vec![reply]);
        let cancel = AtomicBool::new(false);
        assert!(Session::new(&fake, &cancel).await.is_err());
        assert_eq!(fake.routes(), ["/robots.txt"]);
    }
}

#[tokio::test]
async fn html_response_limits_mime_redirect_and_encoding_are_enforced_without_retry() {
    for reply in [
        response(&vec![b'a'; RESPONSE_LIMIT + 1], Some("text/html")),
        response(b"{}", Some("application/json")),
        response(&[0xff], Some("text/html")),
        Ok(Fetch::Redirect(
            Url::parse("https://example.com/challenge").unwrap(),
        )),
        Ok(Fetch::NotModified),
    ] {
        let fake = Stub::new(vec![allow(), reply]);
        let cancel = AtomicBool::new(false);
        let mut session = Session::new(&fake, &cancel).await.unwrap();
        assert!(session.html(rooms::STATUS_URL, &cancel).await.is_err());
        assert_eq!(fake.routes(), ["/robots.txt", "/SEAT/domian5.asp"]);
        assert!(fake.replies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn valid_html_decodes_cp949_and_does_not_persist_http_validators() {
    let text = "<html><body>빈 좌석</body></html>";
    let (bytes, _, errors) = encoding_rs::EUC_KR.encode(text);
    assert!(!errors);
    let first = Ok(Fetch::Body {
        bytes: bytes.into_owned(),
        mime: Some("text/html".into()),
        etag: Some("\"synthetic-etag\"".into()),
        last_modified: Some("Sat, 17 Oct 2026 00:26:13 GMT".into()),
    });
    let fake = Stub::new(vec![allow(), first, response(text.as_bytes(), None)]);
    let cancel = AtomicBool::new(false);
    let mut session = Session::new(&fake, &cancel).await.unwrap();
    assert_eq!(
        session.html(rooms::STATUS_URL, &cancel).await.unwrap(),
        text
    );
    assert_eq!(
        session
            .html(&rooms::get("5").unwrap().url(), &cancel)
            .await
            .unwrap(),
        text
    );
    assert_eq!(
        fake.routes(),
        [
            "/robots.txt",
            "/SEAT/domian5.asp",
            "/SEAT/roomview5.asp?room_no=5"
        ]
    );
}

#[tokio::test]
async fn cancellation_during_policy_pacing_prevents_the_next_fetch() {
    let fake = Stub::new(vec![allow()]);
    let cancel = AtomicBool::new(false);
    let mut session = Session::new(&fake, &cancel).await.unwrap();
    cancel.store(true, Ordering::Release);
    tokio::time::timeout(Duration::from_millis(100), async {
        assert!(session.html(rooms::STATUS_URL, &cancel).await.is_err());
    })
    .await
    .expect("cancel waited through policy pacing");
    assert_eq!(fake.routes(), ["/robots.txt"]);
}
