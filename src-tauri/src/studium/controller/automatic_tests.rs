//! Synthetic clocks, temporary profiles and transport replies only. These
//! tests exercise the automatic command/controller boundary without school I/O.
use super::*;
use crate::{
    archive::network::{Access, Fetch},
    commands::{change_extension, finish_studium_disable},
    extensions::{Registry, STUDIUM},
    operation_gate::AdmissionGate,
    studium::commands::{automatic_start_with_transport, start_with_transport},
};
use chrono::{Datelike, Timelike};
use std::sync::atomic::AtomicUsize;
use tokio::sync::Semaphore;
use url::Url;

const COMMON: &str = "/SEAT/domian5.asp";
const DETAIL: &str = "/SEAT/roomview5.asp?room_no=5";

#[derive(Default)]
struct Probe {
    requests: Mutex<Vec<String>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
    block_next_common: AtomicBool,
    block: Mutex<Option<Arc<Blocked>>>,
    failure: Mutex<Option<(String, String)>>,
}

struct Blocked {
    release: Semaphore,
    entered: AtomicBool,
    dropped: AtomicBool,
}

impl Blocked {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            release: Semaphore::new(0),
            entered: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
        })
    }
}

struct FetchGuard {
    probe: Arc<Probe>,
    blocked: Option<Arc<Blocked>>,
}

impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.probe.active.fetch_sub(1, Ordering::AcqRel);
        if let Some(block) = &self.blocked {
            block.dropped.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Default)]
struct Fake(Arc<Probe>);

impl Fake {
    fn count(&self) -> usize {
        self.0.requests.lock().unwrap().len()
    }

    fn block_next_common(&self) -> Arc<Blocked> {
        let block = Blocked::new();
        *self.0.block.lock().unwrap() = Some(block.clone());
        self.0.block_next_common.store(true, Ordering::Release);
        block
    }

    fn fail_at(&self, route: &str, error: &str) {
        *self.0.failure.lock().unwrap() = Some((route.into(), error.into()));
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
        let route = url[url::Position::BeforePath..url::Position::AfterQuery].to_string();
        assert_eq!(url.port_or_known_default(), Some(444));
        assert_eq!(
            limit,
            if route == "/robots.txt" {
                65_536
            } else {
                1_000_000
            }
        );
        self.0.requests.lock().unwrap().push(route.clone());
        let active = self.0.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.0.maximum.fetch_max(active, Ordering::AcqRel);
        let blocked = if route == COMMON && self.0.block_next_common.swap(false, Ordering::AcqRel) {
            self.0.block.lock().unwrap().clone()
        } else {
            None
        };
        let _guard = FetchGuard {
            probe: self.0.clone(),
            blocked: blocked.clone(),
        };
        if let Some(block) = blocked {
            block.entered.store(true, Ordering::Release);
            // Ignore cancellation to require the owner to drop and join us.
            block.release.acquire().await.unwrap().forget();
        }
        if let Some((at, error)) = self.0.failure.lock().unwrap().as_ref() {
            if at == &route {
                return Err(error.clone());
            }
        }
        let text = match route.as_str() {
            "/robots.txt" => "User-agent: *\nAllow: /\n".into(),
            COMMON => summary(),
            DETAIL => detail(),
            _ => panic!("unexpected route {route}"),
        };
        Ok(Fetch::Body {
            bytes: text.into_bytes(),
            etag: None,
            last_modified: None,
            mime: Some(
                if route == "/robots.txt" {
                    "text/plain"
                } else {
                    "text/html"
                }
                .into(),
            ),
        })
    }
}

fn clock() -> String {
    let now = chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap());
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

fn summary() -> String {
    let rows = rooms::all().into_iter().map(|room| format!(
        "<tr><td>1</td><td><a href=\"roomview5.asp?room_no={}\">{}</a></td><td>{}</td><td>{}</td><td>1</td></tr>",
        room.id, room.name, room.capacity, room.capacity - 1
    )).collect::<String>();
    format!("<table><tr><td>{}</td></tr><tr><th>순번</th><th>열람실명</th><th>전체 좌석수</th><th>사용 좌석수</th><th>잔여 좌석수</th></tr>{rows}</table>", clock())
}

fn detail() -> String {
    let room = rooms::get("5").unwrap();
    let positions = (1..=room.capacity)
        .map(|seat| {
            format!(
        "<div id=\"Layer{seat}\"><table><tr><td bgcolor=\"{}\">{seat}</td></tr></table></div>",
        if seat == 1 { "gray" } else { "red" }
    )
        })
        .collect::<String>();
    format!("<table><tr><td>{}</td><td>총 좌석수 : {}</td><td>사용 좌석수 : {}</td><td>잔여 좌석수 : 1</td><td>{}</td></tr></table><table><tr><td bgcolor=\"red\">사용중</td><td bgcolor=\"gray\">미사용</td></tr></table>{positions}",
        room.name, room.capacity, room.capacity - 1, clock())
}

struct Harness {
    controller: Arc<Controller>,
    gate: Arc<AdmissionGate>,
    shutdown: Arc<AtomicBool>,
    store: Arc<Mutex<Store>>,
    _directory: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("automatic.sqlite3")).unwrap();
        store.set_enabled(STUDIUM, true).unwrap();
        Self {
            controller: Controller::new(),
            gate: Arc::new(AdmissionGate::default()),
            shutdown: Arc::new(AtomicBool::new(false)),
            store: Arc::new(Mutex::new(store)),
            _directory: directory,
        }
    }

    fn snapshot(&self) -> Snapshot {
        let store = self.store.lock().unwrap();
        self.controller
            .snapshot(store.studium_preferences().unwrap())
            .unwrap()
    }

    async fn start_auto(&self, transport: Fake) -> Result<Snapshot, String> {
        automatic_start_with_transport(
            &self.controller,
            &self.gate,
            &self.shutdown,
            &self.store,
            transport,
        )
        .await
    }

    async fn start_manual(&self, transport: Fake) -> Result<Snapshot, String> {
        start_with_transport(
            &self.controller,
            &self.gate,
            &self.shutdown,
            &self.store,
            transport,
        )
        .await
    }

    async fn pause(&self) {
        let completion = {
            let _store = self.store.lock().unwrap();
            self.controller.pause_automatic()
        };
        if let Some(completion) = completion {
            completion.wait().await;
        }
    }

    async fn stop(&self) {
        let completion = {
            let _store = self.store.lock().unwrap();
            self.controller.cancel();
            self.controller.completion()
        };
        if let Some(completion) = completion {
            completion.wait().await;
        }
    }
}

async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::advance(Duration::from_millis(50)).await;
        tokio::task::yield_now().await;
    }
    panic!("automatic controller did not reach the expected state");
}

async fn waiting(h: &Harness) {
    until(|| {
        let snapshot = h.snapshot();
        snapshot.automatic.active && !snapshot.running && snapshot.automatic.next_check_at.is_some()
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn automatic_waits_from_completion_and_never_overlaps_slow_work() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = fake.block_next_common();
    h.start_auto(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    assert!(h.start_auto(fake.clone()).await.is_err());
    assert!(h.start_manual(fake.clone()).await.is_err());
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(fake.count(), 2); // robots + the still-running common response
    assert_eq!(fake.0.maximum.load(Ordering::Acquire), 1);
    block.release.add_permits(1);
    waiting(&h).await;
    assert_eq!(fake.count(), 3);

    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), 3);
    tokio::time::advance(Duration::from_secs(2)).await;
    until(|| fake.count() == 6).await;
    waiting(&h).await;
    assert_eq!(fake.0.maximum.load(Ordering::Acquire), 1);
    h.pause().await;
}

#[tokio::test(start_paused = true)]
async fn automatic_does_not_replay_missed_ticks_after_a_large_clock_advance() {
    let h = Harness::new();
    let fake = Fake::default();
    h.start_auto(fake.clone()).await.unwrap();
    waiting(&h).await;
    assert_eq!(fake.count(), 3);
    tokio::time::advance(Duration::from_secs(24 * 3600)).await;
    until(|| fake.count() >= 6).await;
    waiting(&h).await;
    assert_eq!(fake.count(), 6);
    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), 6);
    h.pause().await;
}

#[tokio::test(start_paused = true)]
async fn pausing_automatic_joins_its_fetch_and_prevents_late_results_or_more_requests() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = fake.block_next_common();
    h.start_auto(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    h.pause().await;
    assert!(block.dropped.load(Ordering::Acquire));
    assert_eq!(fake.0.active.load(Ordering::Acquire), 0);
    assert!(!h.snapshot().automatic.active);
    assert!(!h.snapshot().running);
    assert!(h.snapshot().results[0].result.is_none());
    let count = fake.count();
    block.release.add_permits(1);
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), count);
    assert!(h.snapshot().results[0].result.is_none());
    assert!(h.gate.close().unwrap().acquire().await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn starting_then_pausing_automatic_preserves_an_existing_manual_job() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = fake.block_next_common();
    h.start_manual(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    let job = h.snapshot().job_id;
    h.start_auto(fake.clone()).await.unwrap();
    assert_eq!(fake.count(), 2);
    h.pause().await;
    assert!(h.snapshot().running);
    assert_eq!(h.snapshot().job_id, job);
    assert!(!h.snapshot().automatic.active);
    assert!(!block.dropped.load(Ordering::Acquire));
    block.release.add_permits(1);
    until(|| !h.snapshot().running).await;
    assert_eq!(fake.count(), 3);
    assert_eq!(h.snapshot().results[0].status, "ready");
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), 3);
}

#[tokio::test(start_paused = true)]
async fn manual_work_during_automatic_wait_resets_the_interval_after_manual_completion() {
    let h = Harness::new();
    let fake = Fake::default();
    h.start_auto(fake.clone()).await.unwrap();
    waiting(&h).await;
    tokio::time::advance(Duration::from_secs(200)).await;
    let block = fake.block_next_common();
    h.start_manual(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    tokio::time::advance(Duration::from_secs(50)).await;
    block.release.add_permits(1);
    waiting(&h).await;
    assert_eq!(fake.count(), 6);
    tokio::time::advance(Duration::from_secs(299)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), 6);
    tokio::time::advance(Duration::from_secs(2)).await;
    until(|| fake.count() >= 9).await;
    waiting(&h).await;
    assert_eq!(fake.count(), 9);
    h.pause().await;
}

#[tokio::test(start_paused = true)]
async fn automatic_stops_for_401_403_and_429_from_every_request_stage() {
    for stage in ["/robots.txt", COMMON, DETAIL] {
        for error in [
            "공개 자료에 접근할 수 없습니다. (HTTP 401)",
            "공개 자료에 접근할 수 없습니다. (HTTP 403)",
            crate::archive::network::RATE_LIMITED,
        ] {
            let h = Harness::new();
            let fake = Fake::default();
            fake.fail_at(stage, error);
            h.start_auto(fake.clone()).await.unwrap();
            until(|| !h.snapshot().automatic.active && !h.snapshot().running).await;
            assert!(h.snapshot().automatic.error.is_some(), "{stage}: {error}");
            let count = fake.count();
            assert_eq!(
                count,
                match stage {
                    "/robots.txt" => 1,
                    COMMON => 2,
                    _ => 3,
                }
            );
            tokio::time::advance(Duration::from_secs(3600)).await;
            tokio::task::yield_now().await;
            assert_eq!(fake.count(), count, "{stage}: {error}");
            h.stop().await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn automatic_start_rejects_closed_admission_disabled_extension_and_shutdown() {
    let h = Harness::new();
    let fake = Fake::default();
    let reservation = h.gate.close().unwrap();
    assert!(h.start_auto(fake.clone()).await.is_err());
    drop(reservation);
    h.store.lock().unwrap().set_enabled(STUDIUM, false).unwrap();
    assert!(h.start_auto(fake.clone()).await.is_err());
    h.store.lock().unwrap().set_enabled(STUDIUM, true).unwrap();
    h.shutdown.store(true, Ordering::Release);
    assert!(h.start_auto(fake.clone()).await.is_err());
    assert_eq!(fake.count(), 0);
    assert!(!h.snapshot().automatic.active);
    assert!(!h.snapshot().running);
}

#[tokio::test(start_paused = true)]
async fn maintenance_joins_an_idle_automatic_owner_and_recovery_starts_no_requests() {
    let h = Harness::new();
    let fake = Fake::default();
    h.start_auto(fake.clone()).await.unwrap();
    waiting(&h).await;
    let count = fake.count();
    let (reservation, completion) =
        crate::studium::commands::reserve_maintenance(&h.gate, &h.store, &h.controller).unwrap();
    if let Some(completion) = completion {
        completion.wait().await;
    }
    let guard = reservation.acquire().await.unwrap();
    assert!(!h.snapshot().automatic.active);
    assert!(!h.snapshot().running);
    assert_eq!(
        h.snapshot().automatic.error.as_deref(),
        Some(automatic::MAINTENANCE_STOPPED)
    );
    assert!(h.start_auto(fake.clone()).await.is_err());
    drop(guard); // The same reopening path is used after preparation failure.
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), count);
    assert!(!h.snapshot().automatic.active);
}

#[tokio::test(start_paused = true)]
async fn maintenance_reports_the_same_stop_when_the_timer_wakes_into_the_closed_gate() {
    let h = Harness::new();
    let fake = Fake::default();
    h.start_auto(fake.clone()).await.unwrap();
    waiting(&h).await;
    let count = fake.count();
    // The timer fails on the closed gate before the reservation cancels it.
    let reservation = h.gate.close().unwrap();
    tokio::time::advance(Duration::from_secs(3600)).await;
    until(|| !h.snapshot().automatic.active).await;
    let completion = {
        let _store = h.store.lock().unwrap();
        h.controller.cancel_for_maintenance();
        h.controller.completion()
    };
    if let Some(completion) = completion {
        completion.wait().await;
    }
    assert_eq!(
        h.snapshot().automatic.error.as_deref(),
        Some(automatic::MAINTENANCE_STOPPED)
    );
    assert_eq!(fake.count(), count);
    drop(reservation);
}

#[tokio::test(start_paused = true)]
async fn paused_automatic_keeps_no_error_after_maintenance() {
    let h = Harness::new();
    let fake = Fake::default();
    h.start_auto(fake.clone()).await.unwrap();
    waiting(&h).await;
    let completion = {
        let _store = h.store.lock().unwrap();
        h.controller.pause_automatic()
    };
    if let Some(completion) = completion {
        completion.wait().await;
    }
    let (reservation, completion) =
        crate::studium::commands::reserve_maintenance(&h.gate, &h.store, &h.controller).unwrap();
    if let Some(completion) = completion {
        completion.wait().await;
    }
    assert_eq!(h.snapshot().automatic.error, None);
    drop(reservation);
}

#[tokio::test(start_paused = true)]
async fn disabling_automatic_joins_its_fetch_clears_results_and_enable_does_not_restart() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = fake.block_next_common();
    h.start_auto(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    let registry = Registry::new(
        crate::archive::sync::Controller::new(),
        h.controller.clone(),
    );
    let completion = change_extension(&registry, &h.store, &h.controller, STUDIUM, false).unwrap();
    finish_studium_disable(&h.store, &h.controller, completion)
        .await
        .unwrap();
    assert!(block.dropped.load(Ordering::Acquire));
    assert!(!h.snapshot().automatic.active);
    assert!(!h.snapshot().running);
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.status == "not_checked"));
    let count = fake.count();
    block.release.add_permits(1);
    h.store.lock().unwrap().set_enabled(STUDIUM, true).unwrap();
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), count);
    assert!(!h.snapshot().automatic.active);
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|room| room.result.is_none()));
}

#[tokio::test(start_paused = true)]
async fn exit_joins_automatic_owner_and_manual_work_without_allowing_late_admission() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = fake.block_next_common();
    h.start_manual(fake.clone()).await.unwrap();
    until(|| block.entered.load(Ordering::Acquire)).await;
    h.start_auto(fake.clone()).await.unwrap();
    h.shutdown.store(true, Ordering::Release);
    h.stop().await;
    assert!(block.dropped.load(Ordering::Acquire));
    assert_eq!(fake.0.active.load(Ordering::Acquire), 0);
    assert!(!h.snapshot().running);
    assert!(!h.snapshot().automatic.active);
    let count = fake.count();
    block.release.add_permits(1);
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.count(), count);
    assert!(h.start_auto(fake.clone()).await.is_err());
    assert!(h.start_manual(fake).await.is_err());
    assert!(h.gate.close().unwrap().acquire().await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn timer_counts_wall_clock_time_that_the_monotonic_clock_missed_during_sleep() {
    // After a system sleep the monotonic deadline is still far away while the
    // shown wall-clock time has passed: the check is due at the next step.
    let started = tokio::time::Instant::now();
    automatic::due(
        started + Duration::from_secs(3600),
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .await;
    assert!(started.elapsed() <= Duration::from_secs(30));

    // A wall clock moved backward cannot postpone the monotonic deadline.
    let started = tokio::time::Instant::now();
    automatic::due(
        started + Duration::from_secs(300),
        chrono::Utc::now() + chrono::Duration::days(1),
    )
    .await;
    assert_eq!(started.elapsed(), Duration::from_secs(300));
}
