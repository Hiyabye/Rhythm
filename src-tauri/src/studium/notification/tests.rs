//! No OS permission prompt, native handoff or school request is made here.
use super::*;
use crate::{
    archive::{
        network::{Access, Fetch},
        sync::Transport,
    },
    extensions::{PROACTIVE, STUDIUM},
    storage::Store,
    studium::{commands::start_with_transport, model::AlertRuleInput},
};
use chrono::{Datelike, Timelike};
use std::sync::atomic::AtomicU32;
use tokio::sync::Semaphore;
use url::Url;

struct PermissionBlock {
    entered: AtomicBool,
    dropped: AtomicBool,
    release: Semaphore,
}

struct PermissionGuard(Arc<PermissionBlock>);
impl Drop for PermissionGuard {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::Release);
    }
}

pub(super) struct Wire {
    authorization: Mutex<String>,
    block: Mutex<Option<Arc<PermissionBlock>>>,
    sent: Mutex<Vec<(AlertCandidate, String)>>,
    fail: AtomicBool,
}

impl Wire {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            authorization: Mutex::new("authorized".into()),
            block: Mutex::new(None),
            sent: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        })
    }

    pub(super) async fn authorization(&self) -> String {
        let blocked = self.block.lock().unwrap().take();
        if let Some(block) = blocked {
            let _guard = PermissionGuard(block.clone());
            block.entered.store(true, Ordering::Release);
            block.release.acquire().await.unwrap().forget();
        }
        self.authorization.lock().unwrap().clone()
    }

    pub(super) fn send(&self, candidate: &AlertCandidate, token: &str) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap()
            .push((candidate.clone(), token.into()));
        if self.fail.load(Ordering::Acquire) {
            Err("합성 전달 결과 불확실".into())
        } else {
            Ok(())
        }
    }

    fn block_permission(&self) -> Arc<PermissionBlock> {
        let block = Arc::new(PermissionBlock {
            entered: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
            release: Semaphore::new(0),
        });
        *self.block.lock().unwrap() = Some(block.clone());
        block
    }

    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

#[derive(Clone)]
struct Fake {
    remaining: Arc<AtomicU32>,
    requests: Arc<AtomicU32>,
    failed: Arc<AtomicBool>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            remaining: Arc::new(AtomicU32::new(1)),
            requests: Arc::new(AtomicU32::new(0)),
            failed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Transport for Fake {
    async fn fetch(
        &self,
        url: &Url,
        _validators: Option<(Option<String>, Option<String>)>,
        _limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, String> {
        assert_eq!(access, Access::Studium);
        self.requests.fetch_add(1, Ordering::AcqRel);
        let (text, mime) = match url.path() {
            "/robots.txt" => ("User-agent: *\nAllow: /\n".into(), "text/plain"),
            "/SEAT/domian5.asp" if !self.failed.load(Ordering::Acquire) => {
                (summary(self.remaining.load(Ordering::Acquire)), "text/html")
            }
            "/SEAT/domian5.asp" => return Err("합성 조회 실패".into()),
            "/SEAT/roomview5.asp" => return Err("합성 상세 응답 없음: 검증된 수만 표시".into()),
            path => panic!("unexpected route {path}"),
        };
        Ok(Fetch::Body {
            bytes: text.into_bytes(),
            etag: None,
            last_modified: None,
            mime: Some(mime.into()),
        })
    }
}

fn summary(remaining: u32) -> String {
    let now = chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap());
    let hour = now.hour() % 12;
    let clock = format!(
        "{:04}-{:02}-{:02} {} {}:{:02}:{:02} 현재",
        now.year(),
        now.month(),
        now.day(),
        if now.hour() < 12 { "오전" } else { "오후" },
        if hour == 0 { 12 } else { hour },
        now.minute(),
        now.second()
    );
    let rows = rooms::all().into_iter().map(|room| format!(
        "<tr><td>1</td><td><a href=\"roomview5.asp?room_no={}\">{}</a></td><td>{}</td><td>{}</td><td>{remaining}</td></tr>",
        room.id, room.name, room.capacity, room.capacity - remaining)).collect::<String>();
    format!("<table><tr><td>{clock}</td></tr><tr><th>순번</th><th>열람실명</th><th>전체 좌석수</th><th>사용 좌석수</th><th>잔여 좌석수</th></tr>{rows}</table>")
}

struct Harness {
    store: SharedStore,
    controller: Arc<Controller>,
    gate: Arc<AdmissionGate>,
    notifications: Arc<Notifications>,
    wire: Arc<Wire>,
    _directory: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("alerts.sqlite3")).unwrap();
        store.set_enabled(STUDIUM, true).unwrap();
        store.set_enabled(PROACTIVE, true).unwrap();
        let mut settings = store.proactive_settings().unwrap();
        settings.notifications_enabled = true;
        store.proactive_save_settings(&settings).unwrap();
        store
            .studium_save_alert_rule(
                "5",
                &AlertRuleInput {
                    enabled: true,
                    kind: "remaining".into(),
                    minimum: 1,
                },
            )
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let controller = Controller::new();
        let gate = Arc::new(AdmissionGate::default());
        let wire = Wire::new();
        let notifications = Arc::new(Notifications {
            store: store.clone(),
            controller: Arc::downgrade(&controller),
            gate: gate.clone(),
            app: None,
            control: Mutex::new(None),
            memory: Mutex::new(Memory::default()),
            wake: Notify::new(),
            shutdown: AtomicBool::new(false),
            callback_gate: Arc::new(Mutex::new(())),
            wire: Some(wire.clone()),
        });
        controller.attach_notifications(&notifications);
        notifications.start().unwrap();
        Self {
            store,
            controller,
            gate,
            notifications,
            wire,
            _directory: directory,
        }
    }

    async fn check(&self, fake: &Fake) {
        start_with_transport(
            &self.controller,
            &self.gate,
            &AtomicBool::new(false),
            &self.store,
            fake.clone(),
        )
        .await
        .unwrap();
        until(|| {
            !self
                .controller
                .snapshot(self.store.lock().unwrap().studium_preferences().unwrap())
                .unwrap()
                .running
        })
        .await;
        // Let the receiver consume a published candidate or skip a miss.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    fn run(&self) -> Arc<Run> {
        self.notifications.control.lock().unwrap().clone().unwrap()
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
    panic!("seat notification test did not reach expected state");
}

#[tokio::test(start_paused = true)]
async fn fresh_manual_results_notify_once_and_only_a_successful_miss_rearms() {
    let h = Harness::new();
    let fake = Fake::default();
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    h.check(&fake).await;
    assert_eq!(h.wire.count(), 1);
    fake.failed.store(true, Ordering::Release);
    h.check(&fake).await;
    fake.failed.store(false, Ordering::Release);
    h.check(&fake).await;
    assert_eq!(h.wire.count(), 1);
    fake.remaining.store(0, Ordering::Release);
    h.check(&fake).await;
    fake.remaining.store(1, Ordering::Release);
    h.check(&fake).await;
    until(|| h.wire.count() == 2).await;
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn denied_permission_waits_for_a_new_observation_without_consuming_the_latch() {
    let h = Harness::new();
    let fake = Fake::default();
    *h.wire.authorization.lock().unwrap() = "denied".into();
    h.check(&fake).await;
    until(|| h.notifications.last_error().is_some()).await;
    assert_eq!(h.wire.count(), 0);
    *h.wire.authorization.lock().unwrap() = "authorized".into();
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(h.wire.count(), 0);
    assert_eq!(fake.requests.load(Ordering::Acquire), 3);
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_newer_failed_or_nonmatching_generation_prevents_a_delayed_handoff() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = h.wire.block_permission();
    h.check(&fake).await;
    until(|| block.entered.load(Ordering::Acquire)).await;
    fake.remaining.store(0, Ordering::Release);
    h.check(&fake).await;
    block.release.add_permits(1);
    until(|| block.dropped.load(Ordering::Acquire)).await;
    assert_eq!(h.wire.count(), 0);
    fake.remaining.store(1, Ordering::Release);
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn a_rule_edit_or_manual_cancellation_prevents_a_delayed_handoff() {
    for cancel in [false, true] {
        let h = Harness::new();
        let fake = Fake::default();
        let block = h.wire.block_permission();
        h.check(&fake).await;
        until(|| block.entered.load(Ordering::Acquire)).await;
        {
            let store = h.store.lock().unwrap();
            if cancel {
                let id = h
                    .controller
                    .snapshot(store.studium_preferences().unwrap())
                    .unwrap()
                    .job_id
                    .unwrap();
                h.controller.cancel_job(id).unwrap();
            } else {
                store
                    .studium_save_alert_rule(
                        "5",
                        &AlertRuleInput {
                            enabled: true,
                            kind: "remaining".into(),
                            minimum: 2,
                        },
                    )
                    .unwrap();
            }
        }
        block.release.add_permits(1);
        until(|| block.dropped.load(Ordering::Acquire)).await;
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert_eq!(h.wire.count(), 0);
        h.notifications.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn disabling_joins_a_pending_handoff_and_a_captured_handle_cannot_cancel_reenable() {
    let h = Harness::new();
    let fake = Fake::default();
    let block = h.wire.block_permission();
    h.check(&fake).await;
    until(|| block.entered.load(Ordering::Acquire)).await;
    let completion = {
        let store = h.store.lock().unwrap();
        store.set_enabled(STUDIUM, false).unwrap();
        h.controller.cancel();
        h.notifications.cancel().unwrap()
    };
    completion.wait().await;
    assert!(block.dropped.load(Ordering::Acquire));
    assert_eq!(h.wire.count(), 0);
    let old = h.notifications.cancel().unwrap();
    h.store.lock().unwrap().set_enabled(STUDIUM, true).unwrap();
    h.notifications.resume_after_quiesce().await;
    old.wait().await;
    let count = fake.requests.load(Ordering::Acquire);
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.requests.load(Ordering::Acquire), count);
    assert_eq!(h.wire.count(), 0);
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn ambiguous_handoff_is_latched_without_automatic_or_next_success_replay() {
    let h = Harness::new();
    let fake = Fake::default();
    h.wire.fail.store(true, Ordering::Release);
    h.check(&fake).await;
    until(|| h.wire.count() == 1 && h.notifications.last_error().is_some()).await;
    h.wire.fail.store(false, Ordering::Release);
    h.check(&fake).await;
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(h.wire.count(), 1);
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn activation_validates_the_token_rechecks_admission_and_only_changes_the_view_room() {
    let h = Harness::new();
    let fake = Fake::default();
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    let token = h.wire.sent.lock().unwrap()[0].1.clone();
    {
        let store = h.store.lock().unwrap();
        store
            .studium_save_preferences(&["5".into(), "6".into()], "6")
            .unwrap();
    }
    let requests = fake.requests.load(Ordering::Acquire);
    let reservation = h.gate.close().unwrap();
    assert!(h.notifications.activate(&h.run(), &token).await.is_err());
    drop(reservation);
    assert!(!h
        .notifications
        .activate(&h.run(), &"a1".repeat(32))
        .await
        .unwrap());
    assert!(h.notifications.activate(&h.run(), &token).await.unwrap());
    assert_eq!(
        h.store
            .lock()
            .unwrap()
            .studium_preferences()
            .unwrap()
            .view_room,
        "5"
    );
    assert_eq!(
        h.notifications.take_activation().unwrap().as_deref(),
        Some("5")
    );
    assert!(h.notifications.take_activation().unwrap().is_none());
    assert!(!h.notifications.activate(&h.run(), &token).await.unwrap());
    assert_eq!(fake.requests.load(Ordering::Acquire), requests);
    h.notifications.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn maintenance_and_shutdown_drain_callbacks_and_recovery_does_not_replay() {
    let h = Harness::new();
    let fake = Fake::default();
    h.check(&fake).await;
    until(|| h.wire.count() == 1).await;
    let token = h.wire.sent.lock().unwrap()[0].1.clone();
    let old_run = h.run();
    let reservation = h.gate.close().unwrap();
    h.notifications.quiesce().await;
    let guard = reservation.acquire().await.unwrap();
    assert!(h.notifications.take_activation().is_err());
    drop(guard);
    assert!(h.notifications.take_activation().unwrap().is_none());
    h.notifications.resume_after_quiesce().await;
    assert!(!h.notifications.activate(&old_run, &token).await.unwrap());
    let requests = fake.requests.load(Ordering::Acquire);
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert_eq!(fake.requests.load(Ordering::Acquire), requests);
    assert_eq!(h.wire.count(), 1);
    h.notifications.shutdown().await;
    h.notifications.start().unwrap();
    assert!(!h.notifications.activate(&h.run(), &token).await.unwrap());
}

#[test]
fn seat_activation_uris_are_a_separate_exact_capability_namespace() {
    let token = "ab".repeat(32);
    let uri = format!("{TOKEN_PREFIX}{token}");
    assert_eq!(token_from_uri(&uri), Some(token.as_str()));
    for invalid in [
        format!("rhythm-win://notification/{token}"),
        format!("{uri}?x=1"),
        format!("{uri}#fragment"),
        format!("{TOKEN_PREFIX}{}", token.to_uppercase()),
        format!("{TOKEN_PREFIX}../{token}"),
    ] {
        assert!(token_from_uri(&invalid).is_none());
    }
    #[cfg(windows)]
    assert!(crate::proactive::windows_notification::token_from_uri(&uri).is_none());
}

#[tokio::test(start_paused = true)]
async fn fallback_activation_rechecks_current_selection_global_policy_and_shutdown() {
    for change in ["selection", "notifications", "extension", "shutdown"] {
        let h = Harness::new();
        let fake = Fake::default();
        h.check(&fake).await;
        until(|| h.wire.count() == 1).await;
        let token = h.wire.sent.lock().unwrap()[0].1.clone();
        assert!(h.notifications.activate(&h.run(), &token).await.unwrap());
        {
            let store = h.store.lock().unwrap();
            match change {
                "selection" => {
                    store.studium_save_preferences(&["6".into()], "6").unwrap();
                }
                "notifications" => {
                    let mut settings = store.proactive_settings().unwrap();
                    settings.notifications_enabled = false;
                    store.proactive_save_settings(&settings).unwrap();
                }
                "extension" => store.set_enabled(STUDIUM, false).unwrap(),
                _ => h.notifications.shutdown.store(true, Ordering::Release),
            }
        }
        assert!(
            h.notifications.take_activation().unwrap().is_none(),
            "{change}"
        );
        assert_eq!(fake.requests.load(Ordering::Acquire), 3);
        h.notifications.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_clears_errors_and_old_generation_fallback_cannot_reappear_after_resume() {
    let h = Harness::new();
    let old_run = h.run();
    h.notifications.record_error("이전 알림 권한 오류".into());
    h.notifications.cancel().unwrap().wait().await;
    assert!(h.notifications.last_error().is_none());
    h.notifications.resume_after_quiesce().await;
    // Simulate an old callback that passed its initial cancellation check but
    // reached memory after cancel had already cleared it.
    h.notifications.memory.lock().unwrap().activation = Some(PendingActivation {
        room_id: "5".into(),
        run: Arc::downgrade(&old_run),
    });
    assert!(h.notifications.take_activation().unwrap().is_none());
    assert!(h.notifications.last_error().is_none());
    h.notifications.shutdown().await;
}
