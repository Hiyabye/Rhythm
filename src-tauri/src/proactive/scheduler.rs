use super::notification::{self, DeliveryItem};
use crate::{
    archive::{
        self,
        sync::{self, Controller},
    },
    storage::{
        proactive::{
            ProactiveClaim, ProactiveEventKey, ProactiveOutcome, ProactiveSettings, ProactiveTask,
            ProactiveTaskInput, SchedulerOutcome,
        },
        Settings, Store,
    },
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::async_runtime::JoinHandle;
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{oneshot, watch};

const WAKE_INTERVAL: Duration = Duration::from_secs(15);

pub struct Scheduler {
    store: sync::SharedStore,
    archive: Arc<Controller>,
    network: archive::Client,
    root: PathBuf,
    app: AppHandle,
    control: Mutex<Control>,
    pending_activation: Arc<Mutex<Option<notification::NotificationActivation>>>,
    accepting_callbacks: Arc<AtomicBool>,
    callback_gate: Arc<Mutex<()>>,
}
struct Control {
    sender: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    task_done: Option<oneshot::Receiver<()>>,
}

/// One tracked task waits for each delivered macOS alert's user response. Alerts
/// expire after 30 minutes while the rate limit allows three per ten minutes, so
/// this bound leaves headroom instead of stalling new deliveries.
const MAX_PENDING_RESPONSE_TASKS: usize = 12;

#[derive(Default)]
pub(super) struct ResponseTasks(tokio::task::JoinSet<()>);

impl ResponseTasks {
    pub(super) fn is_full(&self) -> bool {
        self.0.len() >= MAX_PENDING_RESPONSE_TASKS
    }

    pub(super) fn spawn(&mut self, task: impl Future<Output = ()> + Send + 'static) -> bool {
        if self.is_full() {
            return false;
        }
        self.0.spawn(task);
        true
    }

    pub(super) async fn drain(&mut self) {
        while self.0.join_next().await.is_some() {}
    }
}

struct NotificationState {
    #[cfg(target_os = "macos")]
    pending_activation: Arc<Mutex<Option<notification::NotificationActivation>>>,
    #[cfg(target_os = "macos")]
    accepting_callbacks: Arc<AtomicBool>,
    #[cfg(target_os = "macos")]
    callback_gate: Arc<Mutex<()>>,
    response_tasks: ResponseTasks,
}

impl Scheduler {
    pub fn new(
        store: sync::SharedStore,
        archive: Arc<Controller>,
        network: archive::Client,
        root: PathBuf,
        app: AppHandle,
    ) -> Result<Self, String> {
        let (sender, _) = watch::channel(false);
        let callback_gate = Arc::new(Mutex::new(()));
        let pending_activation = Arc::new(Mutex::new(None));
        let accepting_callbacks = Arc::new(AtomicBool::new(true));
        #[cfg(not(target_os = "macos"))]
        app.notification()
            .on_action({
                let app = app.clone();
                let store = store.clone();
                let accepting_callbacks = accepting_callbacks.clone();
                let callback_gate = callback_gate.clone();
                move |action| {
                    let Ok(_callback_guard) = callback_gate.lock() else {
                        return;
                    };
                    if !accepting_callbacks.load(Ordering::Acquire) {
                        return;
                    }
                    if let Some(activation) = notification::action_activation(action) {
                        let attempts = activation.event_attempts.to_vec();
                        let Ok(store_guard) = store.lock() else {
                            return;
                        };
                        if let Err(error) = store_guard.proactive_mark_delivered_batch(&attempts) {
                            eprintln!("Proactive notification activation: {error}");
                            return;
                        }
                        if let Ok(mut pending) = pending_activation.lock() {
                            *pending = Some(activation.clone());
                        }
                        let handle = app.clone();
                        if let Err(error) = app.run_on_main_thread(move || {
                            if let Err(error) = crate::core::lifecycle::show_main_window(&handle) {
                                eprintln!("Lifecycle: 알림에서 창을 표시하지 못했습니다: {error}");
                            }
                        }) {
                            eprintln!("Lifecycle: 알림 창 복원을 예약하지 못했습니다: {error}");
                        }
                        notification::emit_activation(&app, activation);
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            store,
            archive,
            network,
            root,
            app,
            control: Mutex::new(Control {
                sender,
                task: None,
                task_done: None,
            }),
            pending_activation,
            accepting_callbacks,
            callback_gate,
        })
    }

    pub fn start(&self) -> Result<(), String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "스케줄러를 시작할 수 없습니다.")?;
        if control
            .task_done
            .as_mut()
            .is_some_and(|done| matches!(done.try_recv(), Err(oneshot::error::TryRecvError::Empty)))
        {
            return Ok(());
        }
        let mut stop = control.sender.subscribe();
        let store = self.store.clone();
        let archive = self.archive.clone();
        let network = self.network.clone();
        let root = self.root.clone();
        let app = self.app.clone();
        let mut notification_state = NotificationState {
            #[cfg(target_os = "macos")]
            pending_activation: self.pending_activation.clone(),
            #[cfg(target_os = "macos")]
            accepting_callbacks: self.accepting_callbacks.clone(),
            #[cfg(target_os = "macos")]
            callback_gate: self.callback_gate.clone(),
            response_tasks: ResponseTasks::default(),
        };
        let (done_sender, done_receiver) = oneshot::channel();
        control.task = Some(tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(WAKE_INTERVAL);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(error) = cycle(
                            &store,
                            &archive,
                            &network,
                            &root,
                            &app,
                            &mut notification_state,
                            &mut stop,
                        ).await {
                            eprintln!("Proactive scheduler: {error}");
                        }
                    }
                    _ = notification_state.response_tasks.0.join_next(), if !notification_state.response_tasks.0.is_empty() => {},
                    changed = stop.changed() => if changed.is_err() || *stop.borrow() { break; },
                }
            }
            notification_state.response_tasks.drain().await;
            let _ = done_sender.send(());
        }));
        control.task_done = Some(done_receiver);
        Ok(())
    }

    pub async fn shutdown(&self) {
        let _callback_gate = self.callback_gate.lock();
        self.accepting_callbacks.store(false, Ordering::Release);
        let task = if let Ok(mut control) = self.control.lock() {
            let _ = control.sender.send(true);
            self.archive.cancel();
            control.task.take()
        } else {
            None
        };
        drop(_callback_gate);
        if let Some(task) = task {
            let _ = task.await;
        }
        self.archive.wait().await;
    }

    pub fn take_activation(&self) -> Result<Option<notification::NotificationActivation>, String> {
        self.pending_activation
            .lock()
            .map(|mut activation| activation.take())
            .map_err(|_| "알림 이동 정보를 확인할 수 없습니다.".into())
    }
}

pub(super) fn profile_interval(profile: &str) -> u32 {
    match profile {
        "eco" => 60,
        "performance" => 15,
        _ => 30,
    }
}

pub(super) fn resolved_interval(requested: u32, profile: &str) -> u32 {
    if requested == 0 {
        profile_interval(profile)
    } else {
        requested
    }
}

fn with_store<T>(
    store: &sync::SharedStore,
    operation: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, String> {
    let guard = store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    operation(&guard)
}

fn settings(store: &sync::SharedStore) -> Result<(ProactiveSettings, Settings), String> {
    with_store(store, |store| {
        Ok((store.proactive_settings()?, store.settings()?))
    })
}

fn reconcile_tasks(
    store: &sync::SharedStore,
    prefs: &ProactiveSettings,
    energy: &Settings,
) -> Result<(), String> {
    let (sources, extension_enabled) = with_store(store, |store| {
        Ok((store.sources()?, store.enabled("sogang_notices")?))
    })?;
    let interval = resolved_interval(prefs.scheduler_interval_minutes, &energy.energy_profile);
    let active = sources
        .iter()
        .filter(|source| source.definition.supported && source.subscribed)
        .map(|source| source.definition.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let current = with_store(store, |store| store.proactive_tasks())?
        .into_iter()
        .map(|task| (task.task_id.clone(), task))
        .collect::<std::collections::HashMap<_, _>>();
    for source in sources
        .into_iter()
        .filter(|source| source.definition.supported)
    {
        let task_id = format!("source:{}", source.definition.id);
        let enabled =
            prefs.scheduler_enabled && extension_enabled && active.contains(&source.definition.id);
        if current
            .get(&task_id)
            .is_none_or(|task| task.enabled != enabled || task.interval_minutes != interval)
        {
            with_store(store, |store| {
                store
                    .proactive_save_task(&ProactiveTaskInput {
                        task_id,
                        interval_minutes: interval,
                        enabled,
                    })
                    .map(|_| ())
            })?;
        }
    }
    Ok(())
}

fn stop_requested(stop: &watch::Receiver<bool>) -> bool {
    *stop.borrow()
}

fn task_is_active(prefs: &ProactiveSettings, extension_enabled: bool, subscribed: bool) -> bool {
    prefs.scheduler_enabled && !prefs.scheduler_paused && extension_enabled && subscribed
}

pub(super) fn can_admit_task(
    stop: &watch::Receiver<bool>,
    prefs: &ProactiveSettings,
    extension_enabled: bool,
    subscribed: bool,
) -> bool {
    !stop_requested(stop) && task_is_active(prefs, extension_enabled, subscribed)
}
pub(super) async fn cancel_or_take_scheduled_outcome(
    archive: &Arc<Controller>,
    task_id: u64,
) -> Result<Option<sync::TaskOutcome>, String> {
    if archive.cancel_task(task_id)? {
        archive.task_completion_notified(task_id).await?;
    }
    archive.take_task_outcome(task_id)
}

async fn wait_scheduled_job(
    store: &sync::SharedStore,
    archive: &Arc<Controller>,
    task_id: u64,
    source_id: &str,
    stop: &mut watch::Receiver<bool>,
) -> Result<Option<sync::TaskOutcome>, String> {
    let mut recheck = tokio::time::interval(Duration::from_secs(1));
    loop {
        if let Some(outcome) = archive.take_task_outcome(task_id)? {
            return Ok(Some(outcome));
        }
        if stop_requested(stop) {
            return cancel_or_take_scheduled_outcome(archive, task_id).await;
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return cancel_or_take_scheduled_outcome(archive, task_id).await;
                }
            }
            _ = recheck.tick() => {
                let active = with_store(store, |store| {
                    let settings = store.proactive_settings()?;
                    let extension = store.enabled("sogang_notices")?;
                    let subscribed = store.sources()?.into_iter().any(|source| {
                        source.definition.id == source_id && source.subscribed
                    });
                    Ok(task_is_active(&settings, extension, subscribed))
                })?;
                if !active {
                    return cancel_or_take_scheduled_outcome(archive, task_id).await;
                }
            }
            _ = archive.task_completion_notified(task_id) => {}
        }
    }
}

pub(super) async fn run_scheduled_task<T: sync::Transport + Clone + 'static>(
    store: &sync::SharedStore,
    archive: &Arc<Controller>,
    root: &Path,
    transport: &T,
    task: &ProactiveTask,
    stop: &mut watch::Receiver<bool>,
) -> Result<ProactiveOutcome, String> {
    let source_id = task.task_id.strip_prefix("source:").unwrap_or("");
    let (prefs, enabled, subscribed) = with_store(store, |store| {
        let prefs = store.proactive_settings()?;
        let extension_enabled = store.enabled("sogang_notices")?;
        let subscribed = store
            .sources()?
            .into_iter()
            .any(|source| source.definition.id == source_id && source.subscribed);
        Ok((prefs, extension_enabled, subscribed))
    })?;
    let outcome = if stop_requested(stop) || !task_is_active(&prefs, enabled, subscribed) {
        ProactiveOutcome::Cancelled
    } else {
        match archive.start_tracked_with_transport(
            store.clone(),
            root.to_path_buf(),
            transport.clone(),
            Some(vec![source_id.to_string()]),
            "incremental".into(),
        ) {
            Ok((_, task_id)) => {
                match wait_scheduled_job(store, archive, task_id, source_id, stop).await? {
                    None => ProactiveOutcome::Cancelled,
                    Some(result) if result.cancelled => ProactiveOutcome::Cancelled,
                    Some(result) if result.result.is_ok() => ProactiveOutcome::Success,
                    Some(_) => ProactiveOutcome::Failure,
                }
            }
            Err(error) if error.contains("다른 동기화") => ProactiveOutcome::Skipped,
            Err(_) => {
                let active = with_store(store, |store| {
                    let prefs = store.proactive_settings()?;
                    let enabled = store.enabled("sogang_notices")?;
                    let subscribed = store
                        .sources()?
                        .into_iter()
                        .any(|source| source.definition.id == source_id && source.subscribed);
                    Ok(task_is_active(&prefs, enabled, subscribed))
                })?;
                if active {
                    ProactiveOutcome::Failure
                } else {
                    ProactiveOutcome::Cancelled
                }
            }
        }
    };
    let error = (outcome == ProactiveOutcome::Failure).then_some("정보원 동기화에 실패했습니다.");
    with_store(store, |store| {
        store
            .proactive_record_task_outcome(&task.task_id, outcome, error)
            .map(|_| ())
    })?;
    Ok(outcome)
}

async fn cycle(
    store: &sync::SharedStore,
    archive: &Arc<Controller>,
    network: &archive::Client,
    root: &Path,
    app: &AppHandle,
    notifications: &mut NotificationState,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let (prefs, energy) = settings(store)?;
    with_store(store, |store| {
        store.proactive_finalize_expired_claims().map(|_| ())
    })?;
    reconcile_tasks(store, &prefs, &energy)?;
    if prefs.scheduler_enabled && !prefs.scheduler_paused {
        let due = with_store(store, |store| store.proactive_due_tasks())?;
        let mut failures = 0;
        let mut successes = 0;
        let mut cancelled = 0;
        let mut notification_error = None;
        let mut skipped = 0;
        for task in &due {
            if stop_requested(stop) {
                break;
            }
            let source_id = task
                .task_id
                .strip_prefix("source:")
                .unwrap_or("")
                .to_string();
            let mut task_stop = stop.clone();
            let (outcome, discovery_error) = wait_for_scheduled_task_and_notifications(
                run_scheduled_task(store, archive, root, network, task, &mut task_stop),
                store,
                app,
                notifications,
                stop,
            )
            .await?;
            if notification_error.is_none() {
                notification_error = discovery_error;
            }
            match outcome {
                ProactiveOutcome::Failure => failures += 1,
                ProactiveOutcome::Success => successes += 1,
                ProactiveOutcome::Cancelled => cancelled += 1,
                ProactiveOutcome::Skipped => skipped += 1,
            }
            if stop_requested(stop) {
                break;
            }
            let (updated, _) = settings(store)?;
            let active = with_store(store, |store| {
                let extension = store.enabled("sogang_notices")?;
                let subscribed = store
                    .sources()?
                    .into_iter()
                    .any(|source| source.definition.id == source_id && source.subscribed);
                Ok(can_admit_task(stop, &updated, extension, subscribed))
            })?;
            if active {
                discover_and_deliver(store, &updated, app, notifications, stop).await?;
            }
        }
        if !due.is_empty() {
            let outcome = if failures == due.len() {
                SchedulerOutcome::Failure
            } else if failures > 0 || (successes > 0 && (cancelled > 0 || skipped > 0)) {
                SchedulerOutcome::Partial
            } else if successes > 0 {
                SchedulerOutcome::Success
            } else if cancelled == due.len() {
                SchedulerOutcome::Cancelled
            } else {
                SchedulerOutcome::Skipped
            };
            with_store(store, |store| {
                store
                    .proactive_record_scheduler_run(
                        outcome,
                        failures
                            .gt(&0)
                            .then_some("일부 정보원 동기화가 실패했습니다."),
                    )
                    .map(|_| ())
            })?;
        }
        if let Some(error) = notification_error {
            return Err(error);
        }
    }
    if !stop_requested(stop) {
        let (current, _) = settings(store)?;
        if should_discover_notifications(&current) {
            discover_and_deliver(store, &current, app, notifications, stop).await?;
        }
    }
    Ok(())
}

async fn wait_for_scheduled_task_and_notifications<F>(
    task: F,
    store: &sync::SharedStore,
    app: &AppHandle,
    notifications: &mut NotificationState,
    stop: &mut watch::Receiver<bool>,
) -> Result<(ProactiveOutcome, Option<String>), String>
where
    F: Future<Output = Result<ProactiveOutcome, String>>,
{
    tokio::pin!(task);
    let mut notification_error = None;
    let mut notification_tick =
        tokio::time::interval_at(tokio::time::Instant::now() + WAKE_INTERVAL, WAKE_INTERVAL);
    loop {
        tokio::select! {
            result = &mut task => {
                return result.map(|outcome| (outcome, notification_error));
            }
            _ = notification_tick.tick() => {
                match settings(store) {
                    Ok((prefs, _)) if should_discover_notifications(&prefs) => {
                        if let Err(error) =
                            discover_and_deliver(store, &prefs, app, notifications, stop).await
                        {
                            notification_error.get_or_insert(error);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        notification_error.get_or_insert(error);
                    }
                }
            }
        }
    }
}

pub(super) fn quiet_end_delay_minutes(
    quiet: &crate::storage::proactive::QuietHours,
    local_minutes: u16,
) -> Option<u32> {
    if !quiet.contains(local_minutes) {
        return None;
    }
    let (_, end) = quiet.window()?;
    let remaining = (u32::from(end) + 24 * 60 - u32::from(local_minutes)) % (24 * 60);
    Some(remaining.max(1))
}

pub(super) fn should_discover_notifications(prefs: &ProactiveSettings) -> bool {
    prefs.notifications_enabled
}

pub(super) fn available_notification_slots(recent_attempts: u32) -> usize {
    notification::RATE_LIMIT.saturating_sub(recent_attempts) as usize
}

pub(super) fn revalidate_delivery_batch(
    store: &sync::SharedStore,
    items: Vec<DeliveryItem>,
) -> Result<Vec<DeliveryItem>, String> {
    let store = store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    let prefs = store.proactive_settings()?;
    let local_minutes = store.proactive_local_minutes()?;
    let quiet_delay = quiet_end_delay_minutes(&prefs.quiet_hours, local_minutes);
    if !prefs.notifications_enabled || quiet_delay.is_some() {
        let minutes = quiet_delay.unwrap_or(1);
        for item in items {
            store.proactive_defer_event(
                item.event_id,
                item.attempt_count,
                minutes,
                Some(if prefs.notifications_enabled {
                    "조용한 시간에는 알림을 보내지 않습니다."
                } else {
                    "알림이 꺼져 있어 나중에 다시 확인합니다."
                }),
            )?;
        }
        return Ok(Vec::new());
    }
    let mut eligible = Vec::with_capacity(items.len());
    for mut item in items {
        let candidate = store.proactive_event_candidate(item.event_id)?;
        let score = store.personalization_notification_score(item.document_id)?;
        if !candidate.source_active
            || score
                .as_ref()
                .is_none_or(|score| !notification::eligible(prefs.notification_sensitivity, score))
        {
            let reason = if !candidate.source_active {
                "정보원 구독 또는 확장 기능이 꺼져 있습니다.".to_string()
            } else {
                score
                    .as_ref()
                    .map(|score| score.reasons.join(", "))
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "현재 알림 기준에 맞지 않습니다.".into())
            };
            store.proactive_suppress_event(
                item.event_id,
                Some(item.attempt_count),
                Some(&reason),
            )?;
            continue;
        }
        item.source_id = candidate.source_id;
        item.source_name = candidate.source_name;
        item.title = candidate.title;
        eligible.push(item);
    }
    if eligible.is_empty() {
        return Ok(eligible);
    }
    let attempts = eligible
        .iter()
        .map(|item| (item.event_id, item.attempt_count))
        .collect::<Vec<_>>();
    if !store.proactive_reserve_delivery_attempts(&attempts)? {
        for item in &eligible {
            store.proactive_defer_event(
                item.event_id,
                item.attempt_count,
                1,
                Some("알림 전송 한도에 도달해 나중에 다시 확인합니다."),
            )?;
        }
        return Ok(Vec::new());
    }
    Ok(eligible)
}

async fn discover_and_deliver(
    store: &sync::SharedStore,
    prefs: &ProactiveSettings,
    app: &AppHandle,
    notifications: &mut NotificationState,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    if !should_discover_notifications(prefs) || notifications.response_tasks.is_full() {
        return Ok(());
    }
    let (local_minutes, quiet) = with_store(store, |store| {
        Ok((
            store.proactive_local_minutes()?,
            notification::local_quiet(store, prefs)?,
        ))
    })?;
    let quiet_delay = if quiet {
        quiet_end_delay_minutes(&prefs.quiet_hours, local_minutes).or(Some(1))
    } else {
        None
    };
    let authorization = crate::notification_permission::notification_authorization_status().await;
    if !quiet && matches!(authorization.as_str(), "denied" | "not_determined") {
        return Ok(());
    }
    let recent = with_store(store, |store| {
        store.proactive_recent_delivery_attempts(notification::RATE_WINDOW_SECS)
    })?;
    let slots = if quiet {
        notification::CANDIDATE_LIMIT
    } else {
        available_notification_slots(recent)
    };
    if slots == 0 {
        return Ok(());
    }
    let mut deliver = Vec::new();
    for event in with_store(store, notification::due_events)?
        .into_iter()
        .take(slots)
    {
        let item = {
            let store = store
                .lock()
                .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
            let candidate = store.proactive_event_candidate(event.id)?;
            let score = store.personalization_notification_score(event.document_id)?;
            let rejection_reason = if !candidate.source_active {
                Some("정보원 구독 또는 확장 기능이 꺼져 있습니다.".to_string())
            } else if score
                .as_ref()
                .is_none_or(|score| !notification::eligible(prefs.notification_sensitivity, score))
            {
                Some(
                    score
                        .as_ref()
                        .map(|score| score.reasons.join(", "))
                        .filter(|reason| !reason.is_empty())
                        .unwrap_or_else(|| "현재 알림 기준에 맞지 않습니다.".into()),
                )
            } else {
                None
            };
            if let Some(reason) = rejection_reason {
                store.proactive_suppress_event(
                    event.id,
                    (event.attempt_count > 0).then_some(event.attempt_count),
                    Some(&reason),
                )?;
                None
            } else if let Some(minutes) = quiet_delay {
                if event.state == crate::storage::proactive::ProactiveEventState::Claimed {
                    store.proactive_defer_event(
                        event.id,
                        event.attempt_count,
                        minutes,
                        Some("조용한 시간에는 알림을 보내지 않습니다."),
                    )?;
                } else {
                    store.proactive_defer_pending_event(
                        event.id,
                        event.attempt_count,
                        minutes,
                        Some("조용한 시간에는 알림을 보내지 않습니다."),
                    )?;
                }
                None
            } else {
                let claimed = store.proactive_claim_due_event(event.id)?;
                Some(DeliveryItem {
                    event_id: claimed.id,
                    document_id: claimed.document_id,
                    revision_id: claimed.revision_id,
                    source_id: candidate.source_id,
                    source_name: candidate.source_name,
                    kind: claimed.event_type,
                    title: candidate.title,
                    attempt_count: claimed.attempt_count,
                })
            }
        };
        if let Some(item) = item {
            deliver.push(item);
        }
    }
    if deliver.len() < slots {
        let store = store
            .lock()
            .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
        let fresh = notification::consider_candidates(&store, prefs)?;
        for candidate in fresh.into_iter().take(slots - deliver.len()) {
            let claim = store.proactive_claim_event(&ProactiveEventKey {
                document_id: candidate.document_id,
                revision_id: candidate.revision_id,
                event_type: candidate.kind,
            })?;
            if let ProactiveClaim::Claimed {
                event_id,
                attempt_count: 0,
            } = claim
            {
                if let Some(minutes) = quiet_delay {
                    store.proactive_defer_pending_event(
                        event_id,
                        0,
                        minutes,
                        Some("조용한 시간에는 알림을 보내지 않습니다."),
                    )?;
                } else {
                    let claimed = store.proactive_claim_due_event(event_id)?;
                    let mut item = candidate;
                    item.event_id = event_id;
                    item.attempt_count = claimed.attempt_count;
                    deliver.push(item);
                }
            }
        }
    }
    if deliver.is_empty() {
        return Ok(());
    }
    // Re-read the complete delivery policy and current source/document state under
    // one lock immediately before handing the digest to the operating system.
    let deliver = revalidate_delivery_batch(store, deliver)?;
    if deliver.is_empty() {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    match notification::show_digest(app, &deliver) {
        Ok(()) => {
            let attempts = deliver
                .iter()
                .map(|item| (item.event_id, item.attempt_count))
                .collect::<Vec<_>>();
            with_store(store, |store| {
                store.proactive_mark_attempted_batch(&attempts).map(|_| ())
            })?;
        }
        Err(error) => {
            for item in deliver {
                with_store(store, |store| {
                    store
                        .proactive_fail_delivery(item.event_id, item.attempt_count, &error)
                        .map(|_| ())
                })?;
            }
        }
    }
    #[cfg(target_os = "macos")]
    match notification::show_digest(app, &deliver).await {
        Ok(Some(handle)) => {
            let attempts = deliver
                .iter()
                .map(|item| (item.event_id, item.attempt_count))
                .collect::<Vec<_>>();
            with_store(store, |store| {
                store.proactive_mark_attempted_batch(&attempts).map(|_| ())
            })?;
            let store = store.clone();
            let app = app.clone();
            let pending_activation = notifications.pending_activation.clone();
            let accepting_callbacks = notifications.accepting_callbacks.clone();
            let callback_gate = notifications.callback_gate.clone();
            let mut stop = stop.clone();
            notifications.response_tasks.spawn(async move {
                let response = tokio::select! {
                    _ = stop.changed() => return,
                    response = handle.response() => response,
                };
                let Ok(_callback_guard) = callback_gate.lock() else {
                    return;
                };
                if !accepting_callbacks.load(Ordering::Acquire) {
                    return;
                }
                match response {
                    Ok(response) => {
                        use mac_usernotifications::response::CloseReason;
                        if response.is_default_action()
                            && response.close_reason != Some(CloseReason::Dismissed)
                            && response.close_reason != Some(CloseReason::Expired)
                        {
                            if let Some(activation) = notification::activation_for_items(&deliver) {
                                if !accepting_callbacks.load(Ordering::Acquire) {
                                    return;
                                }
                                let attempts = activation.event_attempts.to_vec();
                                if let Err(error) = with_store(&store, |store| {
                                    store.proactive_mark_delivered_batch(&attempts).map(|_| ())
                                }) {
                                    eprintln!("Proactive notification activation: {error}");
                                    return;
                                }
                                if !accepting_callbacks.load(Ordering::Acquire) {
                                    return;
                                }
                                if let Ok(mut pending) = pending_activation.lock() {
                                    *pending = Some(activation.clone());
                                }
                                let handle = app.clone();
                                let main_thread_gate = callback_gate.clone();
                                let main_thread_accepting = accepting_callbacks.clone();
                                if let Err(error) = app.run_on_main_thread(move || {
                                    let Ok(_main_thread_guard) = main_thread_gate.lock() else {
                                        return;
                                    };
                                    if !main_thread_accepting.load(Ordering::Acquire) {
                                        return;
                                    }
                                    if let Err(error) =
                                        crate::core::lifecycle::show_main_window(&handle)
                                    {
                                        eprintln!("Lifecycle: 알림 창 복원 실패: {error}");
                                    }
                                }) {
                                    eprintln!("Lifecycle: 알림 창 복원 예약 실패: {error}");
                                    return;
                                }
                                notification::emit_activation(&app, activation);
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("Proactive notification response error: {error}");
                    }
                }
            });
        }
        Ok(None) => {}
        Err(error) => {
            for item in deliver {
                with_store(store, |store| {
                    store
                        .proactive_fail_delivery(item.event_id, item.attempt_count, &error)
                        .map(|_| ())
                })?;
            }
        }
    }
    Ok(())
}
