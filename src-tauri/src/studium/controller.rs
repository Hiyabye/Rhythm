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

struct Job {
    id: u64,
    cancel: AtomicBool,
    finished: AtomicBool,
    done: Notify,
}

/// Captured under the registration boundary so a delayed disable joins only
/// the generation it cancelled, even after a later enable/start.
pub(crate) struct CompletionHandle(Arc<Job>);
impl CompletionHandle {
    pub(crate) async fn wait(self) {
        loop {
            let notified = self.0.done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.finished.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    }
}

#[derive(Default)]
struct State {
    next: u64,
    job: Option<Arc<Job>>,
    results: Vec<RoomState>,
}

#[derive(Default)]
pub(crate) struct Controller {
    state: Mutex<State>,
}

impl Controller {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
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
        let mut state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if state
            .job
            .as_ref()
            .is_some_and(|j| !j.finished.load(Ordering::Acquire))
        {
            return Err("이미 좌석을 확인하고 있습니다.".into());
        }
        state.next = state.next.checked_add(1).ok_or("좌석 작업 번호 오류")?;
        let job = Arc::new(Job {
            id: state.next,
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            done: Notify::new(),
        });
        state.results = preferences
            .room_ids
            .iter()
            .map(|id| empty(id, "loading"))
            .collect();
        state.job = Some(job.clone());
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
                owner.fail_pending(&job, &error);
            }
        });
        Ok(())
    }
    pub(crate) fn cancel(&self) {
        if let Ok(state) = self.state.lock() {
            if let Some(job) = &state.job {
                job.cancel.store(true, Ordering::Release);
            }
        }
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
        self.state
            .lock()
            .ok()
            .and_then(|s| s.job.clone())
            .map(CompletionHandle)
    }
    pub(crate) async fn wait(&self) {
        if let Some(completion) = self.completion() {
            completion.wait().await;
        }
    }
    pub(crate) fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.results.clear();
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
        self.job.finished.store(true, Ordering::Release);
        self.job.done.notify_waiters();
    }
}

#[cfg(test)]
mod tests;
