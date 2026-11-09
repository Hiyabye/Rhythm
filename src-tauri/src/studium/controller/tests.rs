//! Synthetic transport and temporary-profile checks; no school requests occur.
use super::*;
use crate::{
    archive::network::{Access, Fetch},
    operation_gate::AdmissionGate,
};
use chrono::{Datelike, Timelike};
use std::{collections::VecDeque, path::PathBuf};
use tokio::sync::Semaphore;
use url::Url;

const COMMON: &str = "/SEAT/domian5.asp";

enum Reply {
    Html(String),
    Error(String),
    Blocked(String, Arc<Block>),
}

struct Block {
    entered: Semaphore,
    release: Semaphore,
    dropped: AtomicBool,
}

impl Block {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            dropped: AtomicBool::new(false),
        })
    }

    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.entered.acquire())
            .await
            .expect("scripted fetch did not start")
            .unwrap()
            .forget();
    }
}

struct FetchGuard(Arc<Block>);
impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::Release);
    }
}

struct Script {
    steps: VecDeque<(String, Reply)>,
    requests: Vec<String>,
}

#[derive(Clone)]
struct Fake(Arc<Mutex<Script>>);

impl Fake {
    fn new(steps: Vec<(&str, Reply)>) -> Self {
        Self(Arc::new(Mutex::new(Script {
            steps: steps
                .into_iter()
                .map(|(route, reply)| (route.into(), reply))
                .collect(),
            requests: Vec::new(),
        })))
    }

    fn requests(&self) -> Vec<String> {
        self.0.lock().unwrap().requests.clone()
    }

    fn exhausted(&self) {
        assert!(self.0.lock().unwrap().steps.is_empty());
    }
}

impl Transport for Fake {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, String> {
        assert_eq!(access, Access::Studium);
        assert!(validators.is_none());
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://libseat.sogang.ac.kr:444"
        );
        let route = url[url::Position::BeforePath..url::Position::AfterQuery].to_string();
        assert_eq!(
            limit,
            if route == "/robots.txt" {
                64 * 1024
            } else {
                1_000_000
            }
        );
        let reply = {
            let mut script = self.0.lock().unwrap();
            script.requests.push(route.clone());
            if route == "/robots.txt" {
                return Ok(body("User-agent: *\nAllow: /\n".into(), "text/plain"));
            }
            let (expected, reply) = script.steps.pop_front().expect("unexpected extra request");
            assert_eq!(route, expected);
            reply
        };
        let html = match reply {
            Reply::Html(html) => html,
            Reply::Error(error) => return Err(error),
            Reply::Blocked(html, gate) => {
                let _guard = FetchGuard(gate.clone());
                gate.entered.add_permits(1);
                // Deliberately ignores cancel: the controller must drop this fetch.
                gate.release.acquire().await.unwrap().forget();
                html
            }
        };
        Ok(body(html, "text/html"))
    }
}

fn body(text: String, mime: &str) -> Fetch {
    Fetch::Body {
        bytes: text.into_bytes(),
        etag: None,
        last_modified: None,
        mime: Some(mime.into()),
    }
}

struct Harness {
    path: PathBuf,
    controller: Arc<Controller>,
    store: Arc<Mutex<Store>>,
    gate: Arc<AdmissionGate>,
    preferences: Preferences,
    _directory: tempfile::TempDir,
}

impl Harness {
    fn new(ids: &[&str]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.sqlite3");
        let store = Store::open(&path).unwrap();
        store.set_enabled(crate::extensions::STUDIUM, true).unwrap();
        let ids: Vec<String> = ids.iter().map(|id| (*id).into()).collect();
        let preferences = store.studium_save_preferences(&ids, &ids[0]).unwrap();
        Self {
            _directory: directory,
            path,
            controller: Controller::new(),
            store: Arc::new(Mutex::new(store)),
            gate: Arc::new(AdmissionGate::default()),
            preferences,
        }
    }

    async fn start(&self, transport: Fake) {
        let lease = self.gate.shared().await.unwrap();
        let saved = self.store.lock().unwrap();
        assert!(saved.enabled(crate::extensions::STUDIUM).unwrap());
        self.controller
            .start(
                self.preferences.clone(),
                transport,
                self.store.clone(),
                lease,
            )
            .unwrap();
    }

    async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(15), self.controller.wait())
            .await
            .expect("seat job did not finish");
    }

    fn snapshot(&self) -> Snapshot {
        self.controller.snapshot(self.preferences.clone()).unwrap()
    }
}

fn clock(offset: i64) -> String {
    let now = chrono::Utc::now() + chrono::Duration::seconds(offset);
    let now = now.with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap());
    let hour = now.hour() % 12;
    format!(
        "{:04}-{:02}-{:02} {} {}:{:02}:{:02} 현재",
        now.year(),
        now.month(),
        now.day(),
        if now.hour() < 12 { "오전" } else { "오후" },
        if hour == 0 { 12 } else { hour },
        now.minute(),
        now.second()
    )
}

fn summary(clock: &str) -> String {
    let rows = rooms::all().into_iter().map(|room| format!(
        "<tr><td>1</td><td><a href=\"roomview5.asp?room_no={}\">{}</a></td><td>{}</td><td>{}</td><td>2</td></tr>",
        room.id, room.name, room.capacity, room.capacity - 2)).collect::<String>();
    format!("<table><tr><td>{clock}</td></tr><tr><th>순번</th><th>열람실명</th><th>전체 좌석수</th><th>사용 좌석수</th><th>잔여 좌석수</th></tr>{rows}</table>")
}

fn map(id: &str, available: &[u32], clock: &str) -> String {
    let room = rooms::get(id).unwrap();
    let positions = (1..=room.capacity)
        .map(|number| {
            format!(
        "<div id=\"Layer{number}\"><table><tr><td bgcolor=\"{}\">{number}</td></tr></table></div>",
        if available.contains(&number) { "gray" } else { "red" })
        })
        .collect::<String>();
    format!("<table><tr><td>{}</td><td>총 좌석수 : {}</td><td>사용 좌석수 : {}</td><td>잔여 좌석수 : {}</td><td>{clock}</td></tr></table><table><tr><td bgcolor=\"red\">사용중</td><td bgcolor=\"gray\">미사용</td></tr></table>{positions}",
        room.name, room.capacity, room.capacity - available.len() as u32, available.len())
}

#[tokio::test]
async fn common_is_fetched_once_and_each_selected_room_has_one_detail_request() {
    let h = Harness::new(&["5", "6", "11"]);
    let time = clock(0);
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html(map("5", &[3], &time)),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[7, 9, 10], &time)),
        ),
        (
            "/SEAT/roomview5.asp?room_no=11",
            Reply::Html(map("11", &[], &time)),
        ),
    ]);
    h.start(fake.clone()).await;
    h.wait().await;
    fake.exhausted();
    assert_eq!(
        fake.requests(),
        [
            "/robots.txt",
            COMMON,
            "/SEAT/roomview5.asp?room_no=5",
            "/SEAT/roomview5.asp?room_no=6",
            "/SEAT/roomview5.asp?room_no=11"
        ]
    );
    let snapshot = h.snapshot();
    assert!(!snapshot.running);
    assert!(snapshot
        .results
        .iter()
        .all(|room| room.status == "ready" && room.error.is_none()));
    assert_eq!(
        snapshot.results[0].result.as_ref().unwrap().seat_numbers,
        Some(vec![3])
    );
    let second = snapshot.results[1].result.as_ref().unwrap();
    assert_eq!(second.remaining, 3); // A valid detail replaces different common counts.
    assert_eq!(second.seat_numbers, Some(vec![7, 9, 10]));
    assert_eq!(
        snapshot.results[2].result.as_ref().unwrap().seat_numbers,
        Some(vec![])
    );
}

#[tokio::test]
async fn malformed_common_room_fails_only_that_room_and_skips_its_detail() {
    let h = Harness::new(&["5", "6"]);
    let time = clock(0);
    let bad = summary(&time).replace("<td>102</td>", "<td>103</td>");
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(bad)),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[8], &time)),
        ),
    ]);
    h.start(fake.clone()).await;
    h.wait().await;
    fake.exhausted();
    let snapshot = h.snapshot();
    assert_eq!(snapshot.results[0].status, "failed");
    assert!(snapshot.results[0].result.is_none());
    assert!(snapshot.results[0].error.is_some());
    assert_eq!(snapshot.results[1].status, "ready");
    assert_eq!(fake.requests().len(), 3);
}

#[tokio::test]
async fn malformed_detail_and_older_detail_keep_only_verified_common_counts() {
    let h = Harness::new(&["5", "6"]);
    let time = clock(0);
    let old = clock(-30);
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html("<html>changed layout</html>".into()),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[99], &old)),
        ),
    ]);
    h.start(fake.clone()).await;
    h.wait().await;
    fake.exhausted();
    for room in h.snapshot().results {
        assert_eq!(room.status, "count_only");
        let result = room.result.unwrap();
        assert_eq!(result.remaining, 2);
        assert_eq!(result.seat_numbers, None);
        assert_eq!(result.source_url, rooms::STATUS_URL);
        assert!(room.error.is_some());
    }
}

#[tokio::test]
async fn detail_network_failure_is_count_only_and_next_room_still_completes() {
    let h = Harness::new(&["5", "6"]);
    let time = clock(0);
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Error("synthetic timeout".into()),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[1], &time)),
        ),
    ]);
    h.start(fake.clone()).await;
    h.wait().await;
    fake.exhausted();
    let snapshot = h.snapshot();
    assert_eq!(snapshot.results[0].status, "count_only");
    assert_eq!(
        snapshot.results[0].error.as_deref(),
        Some("synthetic timeout")
    );
    assert_eq!(snapshot.results[1].status, "ready");
}

#[tokio::test]
async fn access_denials_fail_the_room_without_fallback_or_retry() {
    for code in [401, 403] {
        let h = Harness::new(&["5", "6"]);
        let time = clock(0);
        let fake = Fake::new(vec![
            (COMMON, Reply::Html(summary(&time))),
            (
                "/SEAT/roomview5.asp?room_no=5",
                Reply::Error(format!("HTTP {code}")),
            ),
            (
                "/SEAT/roomview5.asp?room_no=6",
                Reply::Html(map("6", &[1], &time)),
            ),
        ]);
        h.start(fake.clone()).await;
        h.wait().await;
        fake.exhausted();
        let snapshot = h.snapshot();
        assert_eq!(snapshot.results[0].status, "failed");
        assert!(snapshot.results[0].result.is_none());
        assert_eq!(snapshot.results[1].status, "ready");
        assert_eq!(fake.requests().len(), 4);
    }
}

#[tokio::test]
async fn rate_limit_fails_pending_rooms_and_stops_without_retrying() {
    for error in [crate::archive::network::RATE_LIMITED, "HTTP 429"] {
        let h = Harness::new(&["5", "6"]);
        let time = clock(0);
        let fake = Fake::new(vec![
            (COMMON, Reply::Html(summary(&time))),
            ("/SEAT/roomview5.asp?room_no=5", Reply::Error(error.into())),
        ]);
        h.start(fake.clone()).await;
        h.wait().await;
        fake.exhausted();
        assert!(h
            .snapshot()
            .results
            .iter()
            .all(|room| room.status == "failed" && room.result.is_none()));
        assert_eq!(
            fake.requests(),
            ["/robots.txt", COMMON, "/SEAT/roomview5.asp?room_no=5"]
        );
    }
}

#[tokio::test]
async fn new_cycle_clears_old_numbers_and_global_failure_clears_all_targets() {
    let h = Harness::new(&["5", "6"]);
    let time = clock(0);
    h.start(Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html(map("5", &[4], &time)),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[8], &time)),
        ),
    ]))
    .await;
    h.wait().await;
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.result.is_some()));
    let block = Block::new();
    let fake = Fake::new(vec![(
        COMMON,
        Reply::Blocked("invalid common page".into(), block.clone()),
    )]);
    h.start(fake.clone()).await;
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.status == "loading" && room.result.is_none()));
    block.entered().await;
    block.release.add_permits(1);
    h.wait().await;
    fake.exhausted();
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.status == "failed"
            && room.result.is_none()
            && room.error.is_some()
            && room.checked_at.is_some()));
    assert_eq!(fake.requests(), ["/robots.txt", COMMON]);
}

#[tokio::test]
async fn common_access_failure_has_no_detail_requests() {
    let h = Harness::new(&["5", "6"]);
    let fake = Fake::new(vec![(COMMON, Reply::Error("HTTP 403".into()))]);
    h.start(fake.clone()).await;
    h.wait().await;
    fake.exhausted();
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.status == "failed" && room.result.is_none()));
    assert_eq!(fake.requests(), ["/robots.txt", COMMON]);
}

#[tokio::test]
async fn view_changes_and_profile_reopen_do_not_fetch_or_restore_live_results() {
    let h = Harness::new(&["5", "6"]);
    let time = clock(0);
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html(map("5", &[4], &time)),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Html(map("6", &[8], &time)),
        ),
    ]);
    h.start(fake.clone()).await;
    h.wait().await;
    let before = fake.requests();
    let preferences = h.store.lock().unwrap().studium_set_view_room("6").unwrap();
    let changed = h.controller.snapshot(preferences.clone()).unwrap();
    assert_eq!(changed.preferences.view_room, "6");
    assert_eq!(
        changed.results[0].result.as_ref().unwrap().seat_numbers,
        Some(vec![4])
    );
    assert_eq!(
        changed.results[1].result.as_ref().unwrap().seat_numbers,
        Some(vec![8])
    );
    let reopened = Store::open(&h.path).unwrap();
    assert_eq!(reopened.studium_preferences().unwrap(), preferences);
    let fresh = Controller::new()
        .snapshot(reopened.studium_preferences().unwrap())
        .unwrap();
    assert!(!fresh.running);
    assert!(fresh.job_id.is_none());
    assert!(fresh.results.iter().all(|room| room.status == "not_checked"
        && room.result.is_none()
        && room.checked_at.is_none()));
    assert_eq!(fake.requests(), before);
}

#[tokio::test]
async fn cancellation_drops_blocked_fetch_and_releases_maintenance_lease_before_return() {
    let h = Harness::new(&["5", "6"]);
    let block = Block::new();
    let fake = Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), block.clone()),
    )]);
    h.start(fake.clone()).await;
    block.entered().await;
    let reservation = h.gate.close().unwrap();
    assert!(h.gate.shared().await.is_err());
    let mut maintenance = Box::pin(reservation.acquire());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut maintenance)
            .await
            .is_err()
    );
    h.controller.cancel();
    h.wait().await;
    assert!(block.dropped.load(Ordering::Acquire));
    assert_eq!(block.release.available_permits(), 0); // Never released the fake network call.
    assert!(!h.snapshot().running);
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.status == "cancelled" && room.result.is_none()));
    // A zero-timeout poll must acquire immediately after wait() returns.
    let guard = tokio::time::timeout(Duration::ZERO, maintenance)
        .await
        .unwrap()
        .unwrap();
    assert!(h.gate.shared().await.is_err());
    drop(guard);
    assert!(h.gate.shared().await.is_ok());
    assert_eq!(fake.requests(), ["/robots.txt", COMMON]);
}

#[tokio::test]
async fn cancellation_preserves_completed_room_and_prevents_later_room_fetches() {
    let h = Harness::new(&["5", "6", "11"]);
    let time = clock(0);
    let block = Block::new();
    let fake = Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html(map("5", &[4], &time)),
        ),
        (
            "/SEAT/roomview5.asp?room_no=6",
            Reply::Blocked(map("6", &[8], &time), block.clone()),
        ),
    ]);
    h.start(fake.clone()).await;
    block.entered().await;
    assert_eq!(h.snapshot().results[0].status, "ready");
    h.controller.cancel();
    h.wait().await;
    let snapshot = h.snapshot();
    assert_eq!(snapshot.results[0].status, "ready");
    assert_eq!(
        snapshot.results[0].result.as_ref().unwrap().seat_numbers,
        Some(vec![4])
    );
    assert!(snapshot.results[1..]
        .iter()
        .all(|room| room.status == "cancelled" && room.result.is_none()));
    assert_eq!(fake.requests().len(), 4);
    fake.exhausted();
}

#[tokio::test]
async fn duplicate_start_is_rejected_and_old_cancel_cannot_cancel_a_new_job() {
    let h = Harness::new(&["5"]);
    let first_block = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), first_block.clone()),
    )]))
    .await;
    first_block.entered().await;
    let old = h.snapshot().job_id.unwrap();
    assert!(h.controller.ensure_idle().is_err());
    let lease = h.gate.shared().await.unwrap();
    let saved = h.store.lock().unwrap();
    assert!(h
        .controller
        .start(
            h.preferences.clone(),
            Fake::new(vec![]),
            h.store.clone(),
            lease
        )
        .is_err());
    drop(saved);
    assert_eq!(h.snapshot().job_id, Some(old));
    h.controller.cancel_job(old).unwrap();
    h.wait().await;
    assert!(h.controller.ensure_idle().is_ok());
    let time = clock(0);
    let second_block = Block::new();
    let fake = Fake::new(vec![
        (COMMON, Reply::Blocked(summary(&time), second_block.clone())),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Html(map("5", &[2], &time)),
        ),
    ]);
    h.start(fake.clone()).await;
    second_block.entered().await;
    let current = h.snapshot().job_id.unwrap();
    assert!(current > old);
    h.controller.cancel_job(old).unwrap();
    second_block.release.add_permits(1);
    h.wait().await;
    fake.exhausted();
    assert_eq!(h.snapshot().results[0].status, "ready");
}

#[tokio::test]
async fn disabled_extension_rejects_late_publication_even_without_cancel_signal() {
    let h = Harness::new(&["5"]);
    let time = clock(0);
    let block = Block::new();
    h.start(Fake::new(vec![
        (COMMON, Reply::Html(summary(&time))),
        (
            "/SEAT/roomview5.asp?room_no=5",
            Reply::Blocked(map("5", &[4], &time), block.clone()),
        ),
    ]))
    .await;
    block.entered().await;
    h.store
        .lock()
        .unwrap()
        .set_enabled(crate::extensions::STUDIUM, false)
        .unwrap();
    block.release.add_permits(1);
    h.wait().await;
    assert!(h.snapshot().results[0].result.is_none());
    assert_eq!(h.snapshot().results[0].status, "failed");
    h.controller.clear();
    assert_eq!(h.snapshot().results[0].status, "not_checked");
    assert_eq!(
        h.store.lock().unwrap().studium_preferences().unwrap(),
        h.preferences
    );
}

#[tokio::test]
async fn disable_cancel_clear_cannot_be_undone_by_job_completion() {
    let h = Harness::new(&["5"]);
    let block = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), block.clone()),
    )]))
    .await;
    block.entered().await;
    {
        let store = h.store.lock().unwrap();
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        h.controller.cancel();
        h.controller.clear();
    }
    h.wait().await;
    let snapshot = h.snapshot();
    assert!(!snapshot.running);
    assert_eq!(snapshot.results[0].status, "not_checked");
    assert!(snapshot.results[0].result.is_none());
    assert!(block.dropped.load(Ordering::Acquire));
}

#[tokio::test]
async fn an_old_job_cannot_publish_into_the_next_jobs_results() {
    let h = Harness::new(&["5"]);
    let first = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), first.clone()),
    )]))
    .await;
    first.entered().await;
    let old = h.controller.state.lock().unwrap().job.clone().unwrap();
    h.controller.cancel();
    h.wait().await;
    let second = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), second.clone()),
    )]))
    .await;
    second.entered().await;
    // Clear the old flag to exercise generation rejection independently of cancellation.
    old.cancel.store(false, Ordering::Release);
    assert!(h
        .controller
        .publish(&old, &h.store, failed("5", "stale task".into()))
        .is_err());
    assert_eq!(h.snapshot().results[0].status, "loading");
    assert!(h.snapshot().results[0].error.is_none());
    h.controller.cancel();
    h.wait().await;
}

#[tokio::test]
async fn a_captured_completion_joins_its_generation_after_a_new_job_starts() {
    let h = Harness::new(&["5"]);
    let first = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), first.clone()),
    )]))
    .await;
    first.entered().await;
    let old_id = h.snapshot().job_id.unwrap();
    let completion = {
        let store = h.store.lock().unwrap();
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        h.controller.cancel();
        h.controller.completion().unwrap()
    };
    h.wait().await;
    h.store
        .lock()
        .unwrap()
        .set_enabled(crate::extensions::STUDIUM, true)
        .unwrap();
    let second = Block::new();
    h.start(Fake::new(vec![(
        COMMON,
        Reply::Blocked(summary(&clock(0)), second.clone()),
    )]))
    .await;
    second.entered().await;
    let current_id = h.snapshot().job_id.unwrap();
    assert!(current_id > old_id);
    tokio::time::timeout(Duration::ZERO, completion.wait())
        .await
        .expect("old completion joined a newer running generation");
    let snapshot = h.snapshot();
    assert_eq!(snapshot.job_id, Some(current_id));
    assert!(snapshot.running);
    assert_eq!(snapshot.results[0].status, "loading");
    assert!(!second.dropped.load(Ordering::Acquire));
    h.controller.cancel();
    h.wait().await;
}
