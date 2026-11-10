//! Exercises the exact command/lifecycle helpers with an isolated Store and a
//! transport blocked inside fetch. No Tauri window, OS sender or school request.
use super::*;
use crate::{
    archive::network::{Access, Fetch},
    commands::{change_extension, finish_studium_disable},
    extensions::{Registry, STUDIUM},
    studium::controller::Controller,
};
use std::{
    future::{pending, Future},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::Semaphore;
use url::Url;

#[derive(Clone)]
struct BlockedTransport(Arc<FetchState>);

struct FetchState {
    requests: AtomicUsize,
    dropped: AtomicUsize,
    entered: Semaphore,
}

struct FetchGuard(Arc<FetchState>);
impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::Release);
    }
}

impl BlockedTransport {
    fn new() -> Self {
        Self(Arc::new(FetchState {
            requests: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
            entered: Semaphore::new(0),
        }))
    }

    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.0.entered.acquire())
            .await
            .expect("command did not start its fetch")
            .unwrap()
            .forget();
    }

    fn requests(&self) -> usize {
        self.0.requests.load(Ordering::Acquire)
    }
}

impl Transport for BlockedTransport {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, String> {
        assert_eq!(access, Access::Studium);
        assert_eq!(url.as_str(), "https://libseat.sogang.ac.kr:444/robots.txt");
        assert!(validators.is_none());
        assert_eq!(limit, 64 * 1024);
        let _guard = FetchGuard(self.0.clone());
        self.0.requests.fetch_add(1, Ordering::Release);
        self.0.entered.add_permits(1);
        // Deliberately ignores cancellation. Command/lifecycle join must wait
        // until the owning controller has dropped the in-flight request.
        pending().await
    }
}

struct Harness {
    controller: Arc<Controller>,
    gate: Arc<AdmissionGate>,
    shutdown: AtomicBool,
    store: SharedStore,
    registry: Registry,
    path: PathBuf,
    _directory: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("commands.sqlite3");
        let store = Store::open(&path).unwrap();
        store.set_enabled(STUDIUM, true).unwrap();
        let controller = Controller::new();
        Self {
            registry: Registry::new(crate::archive::sync::Controller::new(), controller.clone()),
            controller,
            gate: Arc::new(AdmissionGate::default()),
            shutdown: AtomicBool::new(false),
            store: Arc::new(Mutex::new(store)),
            path,
            _directory: directory,
        }
    }

    async fn start(&self, transport: BlockedTransport) -> Result<Snapshot, String> {
        start_with_transport(
            &self.controller,
            &self.gate,
            &self.shutdown,
            &self.store,
            transport,
        )
        .await
    }

    fn snapshot(&self) -> Snapshot {
        let store = self.store.lock().unwrap();
        self.controller
            .snapshot(store.studium_preferences().unwrap())
            .unwrap()
    }

    async fn stop(&self) {
        let completion = cancel_at_boundary(&self.store, &self.controller).unwrap();
        if let Some(completion) = completion {
            completion.wait().await;
        }
    }
}

#[tokio::test]
async fn command_rechecks_maintenance_after_shared_admission_before_registration() {
    let h = Harness::new();
    let lease = h.gate.shared().await.unwrap();
    let reservation = h.gate.close().unwrap();
    let transport = BlockedTransport::new();
    let result = register(
        &h.controller,
        &h.gate,
        &h.shutdown,
        &h.store,
        transport.clone(),
        lease,
    );
    assert!(result.unwrap_err().contains("백업"));
    assert_eq!(transport.requests(), 0);
    assert!(!h.snapshot().running);
    // A rejected command must release its admitted lease.
    let guard = tokio::time::timeout(Duration::from_secs(2), reservation.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(guard);
    assert!(h.gate.shared().await.is_ok());
}

#[tokio::test]
async fn command_rechecks_shutdown_and_extension_under_registration_lock() {
    let h = Harness::new();
    for shutdown in [true, false] {
        let lease = h.gate.shared().await.unwrap();
        h.shutdown.store(shutdown, Ordering::Release);
        h.store
            .lock()
            .unwrap()
            .set_enabled(STUDIUM, shutdown)
            .unwrap();
        let transport = BlockedTransport::new();
        let error = register(
            &h.controller,
            &h.gate,
            &h.shutdown,
            &h.store,
            transport.clone(),
            lease,
        )
        .unwrap_err();
        assert!(error.contains(if shutdown { "종료" } else { "꺼져" }));
        assert_eq!(transport.requests(), 0);
        assert!(h.snapshot().job_id.is_none());
    }
}

#[tokio::test]
async fn command_duplicate_start_keeps_the_running_generation_and_its_lease() {
    let h = Harness::new();
    let first_transport = BlockedTransport::new();
    let first = h.start(first_transport.clone()).await.unwrap();
    first_transport.entered().await;
    let second_transport = BlockedTransport::new();
    assert!(h.start(second_transport.clone()).await.is_err());
    assert_eq!(h.snapshot().job_id, first.job_id);
    assert_eq!(second_transport.requests(), 0);
    assert_eq!(first_transport.requests(), 1);
    h.stop().await;
    assert_eq!(first_transport.0.dropped.load(Ordering::Acquire), 1);
    assert!(!h.snapshot().running);
    assert!(h.gate.close().unwrap().acquire().await.is_ok());
}

#[tokio::test]
async fn extension_disable_commits_joins_without_store_lock_and_clears_results() {
    let h = Harness::new();
    let transport = BlockedTransport::new();
    h.start(transport.clone()).await.unwrap();
    transport.entered().await;
    let completion =
        change_extension(&h.registry, &h.store, &h.controller, STUDIUM, false).unwrap();
    assert!(!h.store.lock().unwrap().enabled(STUDIUM).unwrap());
    assert!(h.start(BlockedTransport::new()).await.is_err());
    let mut finish = Box::pin(finish_studium_disable(&h.store, &h.controller, completion));
    // Poll the production join before the controller gets its next cancel tick.
    std::future::poll_fn(|cx| {
        assert!(matches!(finish.as_mut().poll(cx), Poll::Pending));
        Poll::Ready(())
    })
    .await;
    assert!(h.store.try_lock().is_ok());
    finish.await.unwrap();
    assert_eq!(transport.0.dropped.load(Ordering::Acquire), 1);
    assert!(!h.snapshot().running);
    assert!(h
        .snapshot()
        .results
        .iter()
        .all(|r| r.status == "not_checked"));
}

#[tokio::test]
async fn delayed_disable_completion_does_not_clear_reenabled_generation() {
    let h = Harness::new();
    let first_transport = BlockedTransport::new();
    let first = h.start(first_transport.clone()).await.unwrap();
    first_transport.entered().await;
    let old_completion =
        change_extension(&h.registry, &h.store, &h.controller, STUDIUM, false).unwrap();
    h.controller.wait().await;
    change_extension(&h.registry, &h.store, &h.controller, STUDIUM, true).unwrap();
    let second_transport = BlockedTransport::new();
    let second = h.start(second_transport.clone()).await.unwrap();
    second_transport.entered().await;
    assert_ne!(first.job_id, second.job_id);
    finish_studium_disable(&h.store, &h.controller, old_completion)
        .await
        .unwrap();
    let current = h.snapshot();
    assert!(current.running);
    assert_eq!(current.job_id, second.job_id);
    assert!(current.results.iter().all(|r| r.status == "loading"));
    h.stop().await;
}

#[tokio::test]
async fn failed_extension_write_preserves_enabled_flag_and_running_result() {
    let h = Harness::new();
    let transport = BlockedTransport::new();
    let before = h.start(transport.clone()).await.unwrap();
    transport.entered().await;
    let connection = rusqlite::Connection::open(&h.path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_studium_disable BEFORE UPDATE ON extension_preferences
             WHEN NEW.extension_id='studium' AND NEW.enabled=0
             BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;",
        )
        .unwrap();
    assert!(change_extension(&h.registry, &h.store, &h.controller, STUDIUM, false).is_err());
    assert!(h.store.lock().unwrap().enabled(STUDIUM).unwrap());
    let after = h.snapshot();
    assert!(after.running);
    assert_eq!(after.job_id, before.job_id);
    assert!(after.results.iter().all(|r| r.status == "loading"));
    assert_eq!(transport.0.dropped.load(Ordering::Acquire), 0);
    h.stop().await;
}

#[tokio::test]
async fn maintenance_closes_command_admission_and_joins_the_captured_job() {
    let h = Harness::new();
    let transport = BlockedTransport::new();
    h.start(transport.clone()).await.unwrap();
    transport.entered().await;
    let (reservation, completion) = reserve_maintenance(&h.gate, &h.store, &h.controller).unwrap();
    assert!(h.start(BlockedTransport::new()).await.is_err());
    let mut acquire = Box::pin(reservation.acquire());
    std::future::poll_fn(|cx| {
        assert!(matches!(acquire.as_mut().poll(cx), Poll::Pending));
        Poll::Ready(())
    })
    .await;
    completion.unwrap().wait().await;
    assert_eq!(transport.0.dropped.load(Ordering::Acquire), 1);
    let maintenance = acquire.await.unwrap();
    assert!(h.gate.shared().await.is_err());
    drop(maintenance);
    assert!(h.gate.shared().await.is_ok());
    assert_eq!(transport.requests(), 1); // Recovery starts no seat check.
}

#[tokio::test]
async fn maintenance_preparation_failure_reopens_admission() {
    let h = Harness::new();
    let store = h.store.clone();
    assert!(std::thread::spawn(move || {
        let _guard = store.lock().unwrap();
        panic!("synthetic Store lock failure");
    })
    .join()
    .is_err());
    assert!(reserve_maintenance(&h.gate, &h.store, &h.controller).is_err());
    assert!(h.gate.ensure_open().is_ok());
    assert!(h.gate.shared().await.is_ok());
}

#[tokio::test]
async fn exit_cancellation_waits_for_request_drop_and_rejects_late_registration() {
    let h = Harness::new();
    let transport = BlockedTransport::new();
    h.start(transport.clone()).await.unwrap();
    transport.entered().await;
    let late_lease = h.gate.shared().await.unwrap();
    h.shutdown.store(true, Ordering::Release);
    let completion = cancel_at_boundary(&h.store, &h.controller).unwrap();
    let late_transport = BlockedTransport::new();
    assert!(register(
        &h.controller,
        &h.gate,
        &h.shutdown,
        &h.store,
        late_transport.clone(),
        late_lease,
    )
    .is_err());
    completion.unwrap().wait().await;
    assert_eq!(transport.0.dropped.load(Ordering::Acquire), 1);
    assert_eq!(late_transport.requests(), 0);
    assert!(!h.snapshot().running);
    // The exit path has drained both the job's and rejected command's leases.
    assert!(h.gate.close().unwrap().acquire().await.is_ok());
}
