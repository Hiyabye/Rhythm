//! A session-only timer. Idle time owns no admission lease; every actual check
//! uses the same Store/admission/controller boundary as a manual request.
use super::*;
use crate::studium::model::AutomaticStatus;
use crate::{archive::sync::SharedStore, operation_gate::AdmissionGate};

pub(super) struct Run {
    pub(super) job: Arc<Job>,
    next_check_at: Option<String>,
    error: Option<String>,
}

pub(super) fn active(state: &State, id: u64) -> bool {
    state.automatic.as_ref().is_some_and(|run| {
        run.job.id == id
            && !run.job.cancel.load(Ordering::Acquire)
            && !run.job.finished.load(Ordering::Acquire)
    })
}

pub(super) fn snapshot(state: &State) -> AutomaticStatus {
    let run = state.automatic.as_ref();
    let is_active = run.is_some_and(|run| active(state, run.job.id));
    AutomaticStatus {
        active: is_active,
        // Command snapshots replace this with the validated persisted value.
        interval_minutes: 5,
        next_check_at: run
            .filter(|_| is_active)
            .and_then(|run| run.next_check_at.clone()),
        error: run.and_then(|run| run.error.clone()),
    }
}

pub(super) fn clear(state: &mut State) {
    if let Some(run) = state.automatic.as_mut() {
        run.next_check_at = None;
        run.error = None;
    }
}

impl Controller {
    /// Caller holds Store. Starting the session never replaces a running job.
    pub(crate) fn start_automatic<T: Transport + Clone + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        gate: Arc<AdmissionGate>,
        shutdown: Arc<AtomicBool>,
        transport: T,
    ) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
        if state
            .automatic
            .as_ref()
            .is_some_and(|run| !run.job.finished.load(Ordering::Acquire))
        {
            return Err("자동 확인이 실행 중이거나 정리되고 있습니다.".into());
        }
        state.next_automatic = state
            .next_automatic
            .checked_add(1)
            .ok_or("자동 확인 번호 오류")?;
        let job = Job::new(state.next_automatic, None);
        let initial = state
            .job
            .as_ref()
            .filter(|job| !job.finished.load(Ordering::Acquire))
            .cloned();
        state.automatic = Some(Run {
            job: job.clone(),
            next_check_at: None,
            error: None,
        });
        let owner = self.clone();
        let finish = Finish {
            owner: owner.clone(),
            job: job.clone(),
        };
        tokio::spawn(async move {
            let _finish = finish;
            if let Err(error) = owner
                .automatic_loop(&job, initial, &store, &gate, &shutdown, transport)
                .await
            {
                if !job.cancel.load(Ordering::Acquire) {
                    if let Ok(mut state) = owner.state.lock() {
                        if let Some(run) =
                            state.automatic.as_mut().filter(|run| run.job.id == job.id)
                        {
                            run.error = Some(error);
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// Caller holds Store. A manual check continues when the timer is paused.
    pub(crate) fn pause_automatic(&self) -> Option<CompletionHandle> {
        let state = self.state.lock().ok()?;
        let run = state.automatic.as_ref()?;
        run.job.cancel.store(true, Ordering::Release);
        let mut jobs = vec![run.job.clone()];
        if let Some(job) = state
            .job
            .as_ref()
            .filter(|job| job.automatic_id == Some(run.job.id))
        {
            job.cancel.store(true, Ordering::Release);
            jobs.push(job.clone());
        }
        self.changed.notify_waiters();
        Some(CompletionHandle(jobs))
    }

    async fn automatic_loop<T: Transport + Clone + 'static>(
        self: &Arc<Self>,
        run: &Arc<Job>,
        initial: Option<Arc<Job>>,
        store: &SharedStore,
        gate: &Arc<AdmissionGate>,
        shutdown: &Arc<AtomicBool>,
        transport: T,
    ) -> Result<(), String> {
        let mut immediate = initial.is_none();
        if let Some(initial) = initial {
            tokio::select! {
                _ = CompletionHandle(vec![initial.clone()]).wait() => {},
                _ = cancelled(&run.cancel) => return Ok(()),
            }
            if let Some(error) = initial.automatic_stop_error() {
                return Err(error);
            }
        }
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let lease = gate.shared().await?;
            let (running, deadline, observed_completion) = {
                let saved = store.lock().map_err(|_| "설정 잠금 오류")?;
                gate.ensure_open()?;
                if shutdown.load(Ordering::Acquire) || !saved.enabled(crate::extensions::STUDIUM)? {
                    return Ok(());
                }
                let interval = Duration::from_secs(u64::from(saved.studium_auto_interval()?) * 60);
                let mut state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
                if !active(&state, run.id) {
                    return Ok(());
                }
                let running = state
                    .job
                    .as_ref()
                    .filter(|job| !job.finished.load(Ordering::Acquire))
                    .cloned();
                if !immediate {
                    if let Some(error) = state
                        .job
                        .as_ref()
                        .and_then(|job| job.automatic_stop_error())
                    {
                        return Err(error);
                    }
                }
                let deadline = if immediate {
                    tokio::time::Instant::now()
                } else {
                    state
                        .last_completed
                        .unwrap_or_else(tokio::time::Instant::now)
                        + interval
                };
                let delay = deadline.saturating_duration_since(tokio::time::Instant::now());
                if let Some(current) = state.automatic.as_mut() {
                    current.next_check_at = if running.is_some() {
                        None
                    } else {
                        Some(
                            (chrono::Utc::now()
                                + chrono::Duration::from_std(delay)
                                    .map_err(|_| "자동 확인 간격 오류")?)
                            .to_rfc3339(),
                        )
                    };
                }
                (
                    running,
                    deadline,
                    (state.last_completed, state.job.as_ref().map(|job| job.id)),
                )
            };
            drop(lease);
            if let Some(job) = running {
                tokio::select! {
                    _ = CompletionHandle(vec![job.clone()]).wait() => {},
                    _ = cancelled(&run.cancel) => return Ok(()),
                }
                if let Some(error) = job.automatic_stop_error() {
                    return Err(error);
                }
                immediate = false;
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {},
                _ = &mut changed => continue,
                _ = cancelled(&run.cancel) => return Ok(()),
            }
            // Recheck the session, gate, extension and shutdown while holding
            // Store, then atomically register through the existing controller.
            let lease = gate.shared().await?;
            let cycle = {
                let saved = store.lock().map_err(|_| "설정 잠금 오류")?;
                gate.ensure_open()?;
                if shutdown.load(Ordering::Acquire) || !saved.enabled(crate::extensions::STUDIUM)? {
                    return Ok(());
                }
                let state = self.state.lock().map_err(|_| "좌석 작업 잠금 오류")?;
                if !active(&state, run.id) {
                    return Ok(());
                }
                if state
                    .job
                    .as_ref()
                    .is_some_and(|job| !job.finished.load(Ordering::Acquire))
                {
                    drop(state);
                    drop(saved);
                    drop(lease);
                    immediate = false;
                    continue;
                }
                // A manual cycle may have completed between the timer wake and
                // acquiring Store. Its completion must reset this deadline.
                if (state.last_completed, state.job.as_ref().map(|job| job.id))
                    != observed_completion
                {
                    drop(state);
                    drop(saved);
                    drop(lease);
                    immediate = false;
                    continue;
                }
                drop(state);
                self.start_for(
                    saved.studium_preferences()?,
                    transport.clone(),
                    store.clone(),
                    lease,
                    Some(run.id),
                )?;
                self.state
                    .lock()
                    .map_err(|_| "좌석 작업 잠금 오류")?
                    .job
                    .clone()
                    .expect("registered automatic cycle")
            };
            immediate = false;
            tokio::select! {
                _ = CompletionHandle(vec![cycle.clone()]).wait() => {},
                _ = cancelled(&run.cancel) => {
                    cycle.cancel.store(true, Ordering::Release);
                    CompletionHandle(vec![cycle.clone()]).wait().await;
                    return Ok(());
                },
            }
            if let Some(error) = cycle.automatic_stop_error() {
                return Err(error);
            }
        }
    }
}

struct Finish {
    owner: Arc<Controller>,
    job: Arc<Job>,
}
impl Drop for Finish {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            if let Some(job) = state.job.as_ref().filter(|job| {
                job.automatic_id == Some(self.job.id) && !job.finished.load(Ordering::Acquire)
            }) {
                job.cancel.store(true, Ordering::Release);
            }
            if let Some(run) = state
                .automatic
                .as_mut()
                .filter(|run| run.job.id == self.job.id)
            {
                run.next_check_at = None;
            }
        }
        self.job.finished.store(true, Ordering::Release);
        self.job.done.notify_waiters();
        self.owner.changed.notify_waiters();
    }
}
