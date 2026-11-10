use super::{
    cancelled,
    model::{Preferences, RoomState, SeatData, Snapshot},
    network::Session,
    parser, rooms,
};
use crate::{archive::sync::Transport, operation_gate::AdmissionLease, storage::Store};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

const RATE_LIMITED: &str =
    "학교 서버가 요청을 제한했습니다. 잠시 기다린 후 지금 확인을 다시 눌러 주세요.";

/// The shared transport's 429 text asks notice users to sync again; a seat
/// check is repeated only with the 지금 확인 button.
fn seat_error(error: &str) -> &str {
    if error == crate::archive::network::RATE_LIMITED {
        RATE_LIMITED
    } else {
        error
    }
}

struct Job {
    id: u64,
    cancel: AtomicBool,
    finished: AtomicBool,
    done: Notify,
    automatic_id: Option<u64>,
    stop_reason: Mutex<Option<StopReason>>,
}

enum StopReason {
    AccessDenied(String),
    RateLimited,
}

impl Job {
    fn new(id: u64, automatic_id: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            id,
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            done: Notify::new(),
            automatic_id,
            stop_reason: Mutex::new(None),
        })
    }
    fn note_access_error(&self, error: &str) {
        let reason = if error == crate::archive::network::RATE_LIMITED || error == RATE_LIMITED {
            Some(StopReason::RateLimited)
        } else if matches!(
            error,
            "공개 자료에 접근할 수 없습니다. (HTTP 401)"
                | "공개 자료에 접근할 수 없습니다. (HTTP 403)"
        ) {
            Some(StopReason::AccessDenied(error.into()))
        } else {
            None
        };
        if let Some(reason) = reason {
            if let Ok(mut saved) = self.stop_reason.lock() {
                *saved = Some(reason);
            }
        }
    }
    fn automatic_stop_error(&self) -> Option<String> {
        self.stop_reason.lock().ok().and_then(|reason| match reason.as_ref()? {
            StopReason::AccessDenied(error) => Some(format!("{error} 자동 확인을 정지했습니다. 연결을 확인한 뒤 다시 시작해 주세요.")),
            StopReason::RateLimited => Some("학교 서버가 요청을 제한하여 자동 확인을 정지했습니다. 잠시 기다린 후 다시 시작해 주세요.".into()),
        })
    }
}

/// Captured under the registration boundary so a delayed disable joins only
/// the generation it cancelled, even after a later enable/start.
pub(crate) struct CompletionHandle(Vec<Arc<Job>>);
impl CompletionHandle {
    pub(crate) async fn wait(self) {
        for job in self.0 {
            loop {
                let notified = job.done.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if job.finished.load(Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        }
    }
}

#[derive(Default)]
struct State {
    next: u64,
    job: Option<Arc<Job>>,
    results: Vec<RoomState>,
    next_automatic: u64,
    automatic: Option<automatic::Run>,
    last_completed: Option<tokio::time::Instant>,
}

#[derive(Default)]
pub(crate) struct Controller {
    state: Mutex<State>,
    changed: Notify,
    notifications: Mutex<std::sync::Weak<super::notification::Notifications>>,
}

impl Controller {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub(crate) fn attach_notifications(
        &self,
        notifications: &Arc<super::notification::Notifications>,
    ) {
        if let Ok(mut saved) = self.notifications.lock() {
            *saved = Arc::downgrade(notifications);
        }
    }
    fn notifications(&self) -> Option<Arc<super::notification::Notifications>> {
        self.notifications
            .lock()
            .ok()
            .and_then(|saved| saved.upgrade())
    }
    pub(crate) fn cancel_notifications(&self) -> Option<super::notification::Completion> {
        self.notifications()
            .and_then(|notifications| notifications.cancel())
    }
    pub(crate) fn notification_current(&self, candidate: &super::model::AlertCandidate) -> bool {
        self.state.lock().ok().is_some_and(|state| {
            state.job.as_ref().is_some_and(|job| {
                job.id == candidate.job_id && !job.cancel.load(Ordering::Acquire)
            }) && state.results.iter().any(|room| {
                room.room_id == candidate.room_id
                    && matches!(room.status.as_str(), "ready" | "count_only")
                    && room.result.as_ref() == Some(&candidate.data)
            })
        })
    }
    pub(crate) fn ensure_idle(&self) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if state
            .job
            .as_ref()
            .is_some_and(|j| !j.finished.load(Ordering::Acquire))
        {
            Err("좌석 확인 중입니다. 완료하거나 취소한 뒤 다시 시도해 주세요.".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn snapshot(&self, preferences: Preferences) -> Result<Snapshot, String> {
        let state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        let results = preferences
            .room_ids
            .iter()
            .map(|id| {
                state
                    .results
                    .iter()
                    .find(|r| &r.room_id == id)
                    .cloned()
                    .unwrap_or_else(|| empty(id, "not_checked"))
            })
            .collect();
        Ok(Snapshot {
            rooms: rooms::all(),
            preferences,
            results,
            job_id: state.job.as_ref().map(|j| j.id),
            running: state
                .job
                .as_ref()
                .is_some_and(|j| !j.finished.load(Ordering::Acquire)),
            automatic: automatic::snapshot(&state),
            favorites: std::collections::BTreeMap::new(),
            alert_rules: std::collections::BTreeMap::new(),
            alert_error: self
                .notifications()
                .and_then(|notifications| notifications.last_error()),
        })
    }
    /// Caller holds Store's mutex, the same boundary as extension disable and
    /// target editing. No await or network operation happens while registering.
    pub(crate) fn start<T: Transport + 'static>(
        self: &Arc<Self>,
        preferences: Preferences,
        transport: T,
        store: Arc<Mutex<Store>>,
        lease: AdmissionLease,
    ) -> Result<(), String> {
        self.start_for(preferences, transport, store, lease, None)
    }
    fn start_for<T: Transport + 'static>(
        self: &Arc<Self>,
        preferences: Preferences,
        transport: T,
        store: Arc<Mutex<Store>>,
        lease: AdmissionLease,
        automatic_id: Option<u64>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if let Some(id) = automatic_id {
            if !automatic::active(&state, id) {
                return Err("자동 확인이 정지되었습니다.".into());
            }
        }
        if state
            .job
            .as_ref()
            .is_some_and(|j| !j.finished.load(Ordering::Acquire))
        {
            return Err("이미 좌석을 확인하고 있습니다.".into());
        }
        state.next = state.next.checked_add(1).ok_or("좌석 작업 번호 오류")?;
        let job = Job::new(state.next, automatic_id);
        state.results = preferences
            .room_ids
            .iter()
            .map(|id| empty(id, "loading"))
            .collect();
        state.job = Some(job.clone());
        self.changed.notify_waiters();
        let owner = self.clone();
        // The completion guard releases the task-owned admission lease and
        // signals waiters even if the task unwinds or is dropped.
        let finish = Completion {
            owner: owner.clone(),
            job: job.clone(),
            lease: Some(lease),
        };
        tokio::spawn(async move {
            let _finish = finish;
            let outcome = tokio::select! {
                value = tokio::time::timeout(Duration::from_secs(240), owner.execute(&preferences, &transport, &store, &job)) =>
                    value.unwrap_or_else(|_| Err("좌석 확인 전체 제한 시간을 넘었습니다.".into())),
                _ = cancelled(&job.cancel) => Err("좌석 확인을 취소했습니다.".into()),
            };
            if let Err(error) = outcome {
                job.note_access_error(&error);
                owner.fail_pending(&job, seat_error(&error));
            }
        });
        Ok(())
    }
    pub(crate) fn cancel(&self) {
        if let Ok(state) = self.state.lock() {
            if let Some(run) = &state.automatic {
                run.job.cancel.store(true, Ordering::Release);
            }
            if let Some(job) = &state.job {
                job.cancel.store(true, Ordering::Release);
            }
        }
        self.changed.notify_waiters();
        self.cancel_notifications();
    }
    pub(crate) fn cancel_job(&self, id: u64) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if let Some(job) = &state.job {
            if job.id == id {
                job.cancel.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
    pub(crate) fn completion(&self) -> Option<CompletionHandle> {
        let state = self.state.lock().ok()?;
        let mut jobs = Vec::new();
        if let Some(run) = &state.automatic {
            jobs.push(run.job.clone());
        }
        if let Some(job) = &state.job {
            jobs.push(job.clone());
        }
        (!jobs.is_empty()).then_some(CompletionHandle(jobs))
    }
    pub(crate) async fn wait(&self) {
        if let Some(completion) = self.completion() {
            completion.wait().await;
        }
    }
    pub(crate) fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.results.clear();
            automatic::clear(&mut state);
        }
    }
    fn fail_pending(&self, job: &Job, error: &str) {
        if let Ok(mut state) = self.state.lock() {
            if state.job.as_ref().map(|j| j.id) != Some(job.id) {
                return;
            }
            for room in &mut state.results {
                if room.status == "loading" {
                    room.status = if job.cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    room.error = Some(error.into());
                    room.result = None;
                    room.checked_at = Some(parser::now_timestamp());
                }
            }
        }
    }
    fn publish(
        &self,
        job: &Job,
        store: &Arc<Mutex<Store>>,
        value: RoomState,
    ) -> Result<(), String> {
        let saved = store.lock().map_err(|_| "설정 잠금 오류")?;
        if !saved.enabled(crate::extensions::STUDIUM)? || job.cancel.load(Ordering::Acquire) {
            return Err("좌석 확인을 취소했습니다.".into());
        }
        let mut state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if state.job.as_ref().map(|j| j.id) != Some(job.id) || job.cancel.load(Ordering::Acquire) {
            return Err("이전 좌석 확인 결과를 무시했습니다.".into());
        }
        if let Some(target) = state
            .results
            .iter_mut()
            .find(|r| r.room_id == value.room_id)
        {
            if let Some(notifications) = self.notifications() {
                match saved.studium_alert_observe(job.id, &value) {
                    Ok(candidate) => notifications.observe(&value.room_id, candidate),
                    Err(error) => {
                        notifications.observe(&value.room_id, None);
                        notifications.record_error(error);
                    }
                }
            }
            *target = value;
        }
        Ok(())
    }
    async fn execute<T: Transport>(
        &self,
        preferences: &Preferences,
        transport: &T,
        store: &Arc<Mutex<Store>>,
        job: &Job,
    ) -> Result<(), String> {
        let mut session = Session::new(transport, &job.cancel).await?;
        let html = session.html(rooms::STATUS_URL, &job.cancel).await?;
        let mut summaries = parser::parse_status(&html, chrono::Utc::now().timestamp())?;
        for id in &preferences.room_ids {
            if job.cancel.load(Ordering::Acquire) {
                return Err("좌석 확인을 취소했습니다.".into());
            }
            let summary = summaries
                .remove(id)
                .unwrap_or_else(|| Err("열람실 현황 행을 찾지 못했습니다.".into()));
            let value = match summary {
                Err(error) => failed(id, error),
                Ok(mut summary) => {
                    let detail = session
                        .html(&rooms::get(id)?.url(), &job.cancel)
                        .await
                        .and_then(|html| {
                            parser::parse_seats(&html, id, chrono::Utc::now().timestamp())
                        });
                    if let Err(error) = &detail {
                        job.note_access_error(error);
                        if job.automatic_id.is_some() && job.automatic_stop_error().is_some() {
                            return Err(error.clone());
                        }
                    }
                    match detail {
                        Ok(detail)
                            if parser::timestamp_seconds(&detail.source_time)?
                                >= parser::timestamp_seconds(&summary.source_time)? =>
                        {
                            result(id, "ready", detail, None)
                        }
                        Ok(_) => {
                            summary.seat_numbers = None;
                            result(
                                id,
                                "count_only",
                                summary,
                                Some("좌석 배치가 공통 현황보다 오래되었습니다.".into()),
                            )
                        }
                        Err(error)
                            if error == crate::archive::network::RATE_LIMITED
                                || error.contains("HTTP 429") =>
                        {
                            let error = seat_error(&error).to_owned();
                            self.publish(job, store, failed(id, error.clone()))?;
                            return Err(error);
                        }
                        Err(error) if error.contains("HTTP 401") || error.contains("HTTP 403") => {
                            failed(id, error)
                        }
                        Err(error) => {
                            summary.seat_numbers = None;
                            result(id, "count_only", summary, Some(error))
                        }
                    }
                }
            };
            self.publish(job, store, value)?;
        }
        Ok(())
    }
}

fn empty(id: &str, status: &str) -> RoomState {
    RoomState {
        room_id: id.into(),
        status: status.into(),
        result: None,
        error: None,
        checked_at: None,
    }
}
fn failed(id: &str, error: String) -> RoomState {
    RoomState {
        error: Some(error),
        checked_at: Some(parser::now_timestamp()),
        ..empty(id, "failed")
    }
}
fn result(id: &str, status: &str, data: SeatData, error: Option<String>) -> RoomState {
    RoomState {
        result: Some(data),
        error,
        checked_at: Some(parser::now_timestamp()),
        ..empty(id, status)
    }
}
struct Completion {
    owner: Arc<Controller>,
    job: Arc<Job>,
    lease: Option<AdmissionLease>,
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.owner
            .fail_pending(&self.job, "좌석 확인이 중단되었습니다.");
        // A joined job has released its admission lease as well as published
        // terminal state, so maintenance can immediately drain the gate.
        drop(self.lease.take());
        if let Ok(mut state) = self.owner.state.lock() {
            if state.job.as_ref().map(|job| job.id) == Some(self.job.id) {
                state.last_completed = Some(tokio::time::Instant::now());
            }
        }
        self.job.finished.store(true, Ordering::Release);
        self.job.done.notify_waiters();
        self.owner.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests;

mod automatic;
#[cfg(test)]
mod automatic_tests;
