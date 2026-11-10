//! Session-owned seat alerts. Candidates and seat data stay in memory; Store
//! owns the small rule/latch, rate reservation and one-use activation ledger.
use super::{controller::Controller, model::AlertCandidate, rooms};
use crate::{archive::sync::SharedStore, operation_gate::AdmissionGate};
#[cfg(any(target_os = "macos", test))]
use std::time::Duration;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

#[cfg(any(windows, test))]
pub(crate) const TOKEN_PREFIX: &str = "rhythm-win://studium/";
const MAX_RESPONSES: usize = 12;

struct Run {
    cancel: AtomicBool,
    finished: AtomicBool,
    done: Notify,
}

pub(crate) struct Completion {
    run: Arc<Run>,
    callback_gate: Arc<Mutex<()>>,
}
impl Completion {
    pub(crate) async fn wait(self) {
        loop {
            let done = self.run.done.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if self.run.finished.load(Ordering::Acquire) {
                break;
            }
            done.await;
        }
        // External Windows activations are not worker tasks. Drain one that
        // was already inside its final, synchronous activation boundary too.
        drop(self.callback_gate.lock());
    }
}

#[derive(Default)]
struct Memory {
    pending: BTreeMap<String, AlertCandidate>,
    activation: Option<PendingActivation>,
    error: Option<String>,
}

struct PendingActivation {
    room_id: String,
    run: Weak<Run>,
}

pub(crate) struct Notifications {
    store: SharedStore,
    controller: Weak<Controller>,
    gate: Arc<AdmissionGate>,
    app: Option<AppHandle>,
    control: Mutex<Option<Arc<Run>>>,
    memory: Mutex<Memory>,
    wake: Notify,
    shutdown: AtomicBool,
    callback_gate: Arc<Mutex<()>>,
    #[cfg(test)]
    wire: Option<Arc<tests::Wire>>,
}

impl Notifications {
    pub(crate) fn new(
        store: SharedStore,
        controller: Weak<Controller>,
        gate: Arc<AdmissionGate>,
        app: AppHandle,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            controller,
            gate,
            app: Some(app),
            control: Mutex::new(None),
            memory: Mutex::new(Memory::default()),
            wake: Notify::new(),
            shutdown: AtomicBool::new(false),
            callback_gate: Arc::new(Mutex::new(())),
            #[cfg(test)]
            wire: None,
        })
    }

    /// Starts an idle receiver only; startup/recovery never evaluates old data.
    pub(crate) fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut control = self.control.lock().map_err(|_| "좌석 알림 잠금 오류")?;
        if let Some(run) = control
            .as_ref()
            .filter(|run| !run.finished.load(Ordering::Acquire))
        {
            return if run.cancel.load(Ordering::Acquire) {
                Err("좌석 알림을 정리하고 있습니다.".into())
            } else {
                Ok(())
            };
        }
        let run = Arc::new(Run {
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            done: Notify::new(),
        });
        *control = Some(run.clone());
        let owner = self.clone();
        let task = async move {
            let _finish = Finish(run.clone());
            owner.run(run).await;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(task);
            }
            Err(_) => {
                tauri::async_runtime::spawn(task);
            }
        }
        Ok(())
    }

    /// Called under Store for extension disable. The captured handle cannot
    /// stop or join a later re-enabled generation.
    pub(crate) fn cancel(&self) -> Option<Completion> {
        let run = self.control.lock().ok()?.as_ref()?.clone();
        run.cancel.store(true, Ordering::Release);
        if let Ok(mut memory) = self.memory.lock() {
            memory.pending.clear();
            memory.activation = None;
            memory.error = None;
        }
        self.wake.notify_waiters();
        Some(Completion {
            run,
            callback_gate: self.callback_gate.clone(),
        })
    }

    pub(crate) async fn quiesce(&self) {
        let completion = self.cancel();
        if let Ok(store) = self.store.lock() {
            if let Err(error) = store.studium_alert_invalidate_tokens() {
                self.record_error(error);
            }
        }
        if let Some(completion) = completion {
            completion.wait().await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.quiesce().await;
    }

    pub(crate) async fn resume_after_quiesce(self: &Arc<Self>) {
        let run = self.control.lock().ok().and_then(|control| control.clone());
        if let Some(run) = run.filter(|run| run.cancel.load(Ordering::Acquire)) {
            Completion {
                run,
                callback_gate: self.callback_gate.clone(),
            }
            .wait()
            .await;
        }
        if let Err(error) = self.start() {
            self.record_error(error);
        }
    }

    pub(crate) fn take_activation(&self) -> Result<Option<String>, String> {
        // Store is always acquired before runtime memory, as in publication
        // and OS activation. A click can precede the frontend subscription by
        // enough time for the room, settings or worker generation to change.
        let store = self.store.lock().map_err(|_| "좌석 알림 설정 잠금 오류")?;
        self.gate.ensure_open()?;
        let current = self
            .control
            .lock()
            .map_err(|_| "좌석 알림 잠금 오류")?
            .clone();
        let mut memory = self.memory.lock().map_err(|_| "좌석 알림 이동 잠금 오류")?;
        let Some(activation) = memory.activation.take() else {
            return Ok(None);
        };
        let Some(run) = activation.run.upgrade() else {
            return Ok(None);
        };
        if self.cancelled(&run)
            || run.finished.load(Ordering::Acquire)
            || !current.is_some_and(|current| Arc::ptr_eq(&current, &run))
            || rooms::get(&activation.room_id).is_err()
            || !store.enabled(crate::extensions::STUDIUM)?
            || !store.proactive_effective_settings()?.notifications_enabled
            || !store
                .studium_preferences()?
                .room_ids
                .contains(&activation.room_id)
        {
            return Ok(None);
        }
        Ok(Some(activation.room_id))
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.memory
            .lock()
            .ok()
            .and_then(|memory| memory.error.clone())
    }

    pub(crate) fn record_error(&self, error: String) {
        if let Ok(mut memory) = self.memory.lock() {
            memory.error = Some(error);
        }
    }

    /// Every publish replaces the room's candidate, including a failed result.
    pub(crate) fn observe(&self, room: &str, candidate: Option<AlertCandidate>) {
        if rooms::get(room).is_err() {
            return;
        }
        let active = self
            .control
            .lock()
            .ok()
            .and_then(|control| control.clone())
            .is_some_and(|run| {
                !run.cancel.load(Ordering::Acquire) && !run.finished.load(Ordering::Acquire)
            });
        if let Ok(mut memory) = self.memory.lock() {
            memory.pending.remove(room);
            if active && !self.shutdown.load(Ordering::Acquire) {
                if let Some(candidate) = candidate {
                    memory.pending.insert(room.into(), candidate);
                }
            }
        }
        self.wake.notify_one();
    }

    fn pop(&self) -> Option<AlertCandidate> {
        let mut memory = self.memory.lock().ok()?;
        let first = memory.pending.keys().next()?.clone();
        memory.pending.remove(&first)
    }

    fn cancelled(&self, run: &Run) -> bool {
        self.shutdown.load(Ordering::Acquire) || run.cancel.load(Ordering::Acquire)
    }

    async fn run(self: Arc<Self>, run: Arc<Run>) {
        let mut responses = tokio::task::JoinSet::new();
        loop {
            if self.cancelled(&run) {
                break;
            }
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(candidate) = self.pop() {
                if responses.len() >= MAX_RESPONSES {
                    continue;
                }
                let result = tokio::select! {
                    biased;
                    _ = super::cancelled(&run.cancel) => break,
                    result = self.deliver(&run, candidate, &mut responses) => result,
                };
                match result {
                    Err(error) if !self.cancelled(&run) => self.record_error(error),
                    Err(_) => {}
                    Ok(()) => {
                        if let Ok(mut memory) = self.memory.lock() {
                            memory.error = None;
                        }
                    }
                }
                continue;
            }
            tokio::select! {
                _ = &mut notified => {},
                _ = responses.join_next(), if !responses.is_empty() => {},
                _ = super::cancelled(&run.cancel) => break,
            }
        }
        responses.abort_all();
        while responses.join_next().await.is_some() {}
    }

    async fn deliver(
        self: &Arc<Self>,
        run: &Arc<Run>,
        candidate: AlertCandidate,
        responses: &mut tokio::task::JoinSet<()>,
    ) -> Result<(), String> {
        let _lease = self.gate.shared().await?;
        #[cfg(test)]
        let authorization = match &self.wire {
            Some(wire) => wire.authorization().await,
            None => crate::notification_permission::notification_authorization_status().await,
        };
        #[cfg(not(test))]
        let authorization =
            crate::notification_permission::notification_authorization_status().await;
        if !matches!(
            authorization.as_str(),
            "authorized" | "provisional" | "ephemeral"
        ) {
            return Err(
                "운영체제 알림 권한을 확인해 주세요. 다음 좌석 확인 결과부터 다시 판단합니다."
                    .into(),
            );
        }
        if self.cancelled(run) {
            return Ok(());
        }
        let token = token()?;
        #[cfg(test)]
        if let Some(wire) = &self.wire {
            let store = self.store.lock().map_err(|_| "좌석 알림 설정 잠금 오류")?;
            if !self.current(run, &candidate)?
                || !store.studium_alert_reserve(&candidate, &token)?
            {
                return Ok(());
            }
            return wire.send(&candidate, &token);
        }
        #[cfg(windows)]
        {
            let _ = responses;
            let store = self.store.lock().map_err(|_| "좌석 알림 설정 잠금 오류")?;
            if !self.current(run, &candidate)? {
                return Ok(());
            }
            let (title, body) = text(&candidate)?;
            // Preparation may fail without consuming the latch or rate slot.
            let prepared =
                crate::proactive::windows_notification::prepare_studium(&title, &body, &token)?;
            if !self.current(run, &candidate)?
                || !store.studium_alert_reserve(&candidate, &token)?
            {
                return Ok(());
            }
            // This synchronous handoff shares Store with disable/settings. An
            // error after Show begins is ambiguous: retain the latch/attempt.
            crate::proactive::windows_notification::show(prepared).map_err(|_| {
                "좌석 알림 전달 결과를 확인하지 못했습니다. 자동 재전송하지 않습니다.".into()
            })
        }
        #[cfg(target_os = "macos")]
        {
            use std::{
                future::Future,
                task::{Context, Poll, Waker},
            };
            let (title, body) = text(&candidate)?;
            let mut sending = Box::pin(
                mac_usernotifications::Notification::new()
                    .title(title)
                    .message(body)
                    .timeout(Duration::from_secs(1_800))
                    .send(),
            );
            let first = {
                let store = self.store.lock().map_err(|_| "좌석 알림 설정 잠금 오류")?;
                if !self.current(run, &candidate)?
                    || !store.studium_alert_reserve(&candidate, &token)?
                {
                    return Ok(());
                }
                // mac-usernotifications 0.3.1 dispatches its OS worker on the
                // first poll, before awaiting acceptance. Begin that handoff
                // under the same Store boundary as disable, then release the
                // lock before awaiting anything. Re-poll installs our real
                // task waker if the OS has not responded synchronously.
                sending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
            };
            // The owning session keeps admission through this async handoff;
            // disable/maintenance cancel and join it before returning. A
            // reserved but interrupted send remains latched without replay.
            let sent = match first {
                Poll::Ready(result) => Ok(result),
                Poll::Pending => tokio::select! {
                    biased;
                    _ = super::cancelled(&run.cancel) => return Ok(()),
                    result = tokio::time::timeout(Duration::from_secs(20), sending) => result,
                },
            };
            let handle = sent
                .map_err(|_| {
                    "좌석 알림 전달 결과를 확인하지 못했습니다. 자동 재전송하지 않습니다."
                })?
                .map_err(|_| {
                    "좌석 알림 전달 결과를 확인하지 못했습니다. 자동 재전송하지 않습니다."
                })?;
            let owner = self.clone();
            let run = run.clone();
            responses.spawn(async move {
                let response = tokio::select! {
                    biased;
                    _ = super::cancelled(&run.cancel) => return,
                    response = tokio::time::timeout(Duration::from_secs(1_805), handle.response()) => response,
                };
                if let Ok(Ok(response)) = response {
                    use mac_usernotifications::response::CloseReason;
                    if response.is_default_action()
                        && response.close_reason != Some(CloseReason::Dismissed)
                        && response.close_reason != Some(CloseReason::Expired)
                    {
                        if let Err(error) = owner.activate(&run, &token).await {
                            if !owner.cancelled(&run) { owner.record_error(error); }
                        }
                    }
                }
            });
            Ok(())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (candidate, responses, token);
            Err("이 운영체제에서는 좌석 알림을 사용할 수 없습니다.".into())
        }
    }

    /// Caller holds Store before locking the controller, matching publication.
    fn current(&self, run: &Run, candidate: &AlertCandidate) -> Result<bool, String> {
        self.gate.ensure_open()?;
        Ok(!self.cancelled(run)
            && self
                .controller
                .upgrade()
                .is_some_and(|controller| controller.notification_current(candidate)))
    }

    #[cfg(windows)]
    pub(crate) async fn handle_windows_activation(&self, uri: &str) -> Result<bool, String> {
        let Some(token) = token_from_uri(uri) else {
            return Ok(false);
        };
        let run = self
            .control
            .lock()
            .map_err(|_| "좌석 알림 잠금 오류")?
            .clone();
        let Some(run) = run else {
            return Ok(false);
        };
        self.activate(&run, token).await
    }

    async fn activate(&self, run: &Arc<Run>, token: &str) -> Result<bool, String> {
        let _lease = self.gate.shared().await?;
        let _callback = self
            .callback_gate
            .lock()
            .map_err(|_| "좌석 알림 이동 잠금 오류")?;
        if self.cancelled(run) {
            return Ok(false);
        }
        let room = {
            let store = self.store.lock().map_err(|_| "좌석 알림 설정 잠금 오류")?;
            self.gate.ensure_open()?;
            if self.cancelled(run) {
                return Ok(false);
            }
            let Some(room) = store.studium_alert_consume(token)? else {
                return Ok(false);
            };
            store.studium_set_view_room(&room)?;
            room
        };
        if let Some(controller) = self.controller.upgrade() {
            controller.refresh_views();
        }
        if self.cancelled(run) {
            return Ok(false);
        }
        self.memory
            .lock()
            .map_err(|_| "좌석 알림 이동 잠금 오류")?
            .activation = Some(PendingActivation {
            room_id: room.clone(),
            run: Arc::downgrade(run),
        });
        if let Some(app) = &self.app {
            let handle = app.clone();
            let run = run.clone();
            app.run_on_main_thread(move || {
                if run.cancel.load(Ordering::Acquire) {
                    return;
                }
                if let Err(error) = crate::core::lifecycle::show_main_window(&handle) {
                    eprintln!("Studium: 알림 창 복원 실패: {error}");
                }
            })
            .map_err(|_| "좌석 알림 창을 열지 못했습니다.")?;
            let _ = app.emit("studium://notification-clicked", room);
        }
        Ok(true)
    }
}

struct Finish(Arc<Run>);
impl Drop for Finish {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.done.notify_waiters();
    }
}

fn token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "좌석 알림 보안 토큰을 만들지 못했습니다.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(any(windows, test))]
pub(crate) fn token_from_uri(uri: &str) -> Option<&str> {
    let token = uri.strip_prefix(TOKEN_PREFIX)?;
    crate::storage::notification_activation::valid_token(token).then_some(token)
}

fn text(candidate: &AlertCandidate) -> Result<(String, String), String> {
    let room = rooms::get(&candidate.room_id)?;
    Ok((
        "원하는 열람실 좌석이 확인됐습니다".into(),
        format!(
            "{} · 빈 좌석 {}석\n조회 당시 정보입니다. 이동할 때는 현황을 다시 확인해 주세요.",
            room.label, candidate.data.remaining
        ),
    ))
}

#[cfg(test)]
mod tests;
