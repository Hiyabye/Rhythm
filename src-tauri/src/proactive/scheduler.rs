use super::notification::{self, DeliveryItem};
use crate::{
    archive::{
        self,
        sync::{self, Controller},
    },
    operation_gate::AdmissionGate,
    storage::{
        proactive::{
            ProactiveClaim, ProactiveEventKey, ProactiveOutcome, ProactiveSettings, ProactiveTask,
            ProactiveTaskInput, SchedulerOutcome,
        },
        Settings, Store,
    },
};
use std::future::Future;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::async_runtime::JoinHandle;
use tauri::AppHandle;
use tokio::sync::{oneshot, watch};

const WAKE_INTERVAL: Duration = Duration::from_secs(15);

pub struct Scheduler {
    store: sync::SharedStore,
    archive: Arc<Controller>,
    network: archive::Client,
    root: PathBuf,
    app: AppHandle,
    operation_gate: Arc<AdmissionGate>,
    control: Mutex<Control>,
    pending_activation: Arc<Mutex<Option<notification::NotificationActivation>>>,
}
struct Control {
    sender: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    task_done: Option<oneshot::Receiver<()>>,
}

impl Scheduler {
    pub fn new(
        store: sync::SharedStore,
        archive: Arc<Controller>,
        network: archive::Client,
        root: PathBuf,
        app: AppHandle,
        operation_gate: Arc<AdmissionGate>,
    ) -> Result<Self, String> {
        let (sender, _) = watch::channel(false);
        let pending_activation = Arc::new(Mutex::new(None));
        Ok(Self {
            store,
            archive,
            network,
            root,
            app,
            operation_gate,
            control: Mutex::new(Control {
                sender,
                task: None,
                task_done: None,
            }),
            pending_activation,
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
        let _ = control.sender.send(false);
        let mut stop = control.sender.subscribe();
        let store = self.store.clone();
        let archive = self.archive.clone();
        let network = self.network.clone();
        let root = self.root.clone();
        let operation_gate = self.operation_gate.clone();
        let (done_sender, done_receiver) = oneshot::channel();
        control.task = Some(tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(WAKE_INTERVAL);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Ok(_lease) = operation_gate.shared().await {
                            if let Err(error) = cycle(
                                &store,
                                &archive,
                                &network,
                                &root,
                                &mut stop,
                            ).await {
                                eprintln!("Proactive scheduler: {error}");
                            }
                        }
                    }
                    changed = stop.changed() => if changed.is_err() || *stop.borrow() { break; },
                }
            }
            let _ = done_sender.send(());
        }));
        control.task_done = Some(done_receiver);
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.quiesce().await;
    }

    pub async fn quiesce(&self) {
        let task = if let Ok(mut control) = self.control.lock() {
            let _ = control.sender.send(true);
            self.archive.cancel();
            control.task.take()
        } else {
            None
        };
        if let Some(task) = task {
            let _ = task.await;
        }
        self.archive.wait().await;
    }

    pub async fn resume_after_quiesce(&self) {
        loop {
            let stopped = match self.control.lock() {
                Ok(mut control) => match control.task_done.as_mut() {
                    Some(done) => {
                        !matches!(done.try_recv(), Err(oneshot::error::TryRecvError::Empty))
                    }
                    None => true,
                },
                Err(_) => {
                    eprintln!("Proactive: 유지보수 취소 후 스케줄러 상태를 복구하지 못했습니다.");
                    return;
                }
            };
            if stopped {
                self.archive.wait().await;
                if let Err(error) = self.start() {
                    eprintln!(
                        "Proactive: 유지보수 취소 후 스케줄러를 다시 시작하지 못했습니다: {error}"
                    );
                }
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub fn take_activation(&self) -> Result<Option<notification::NotificationActivation>, String> {
        self.pending_activation
            .lock()
            .map(|mut activation| activation.take())
            .map_err(|_| "알림 이동 정보를 확인할 수 없습니다.".into())
    }

    /// Consume a Windows protocol activation after the store is initialized.
    /// Both startup argv and second-instance argv must call this same method.
    #[cfg(windows)]
    pub async fn handle_windows_activation(&self, uri: &str) -> Result<bool, String> {
        let Some(token) = super::windows_notification::token_from_uri(uri) else {
            return Ok(false);
        };
        let _lease = self.operation_gate.shared().await?;
        let batch = with_store(&self.store, |store| store.notification_token_consume(token))?;
        let Some(batch) = batch else {
            return Ok(false);
        };
        let (event_id, attempt_count) = batch.attempts[0];
        let activation = notification::NotificationActivation {
            event_id,
            document_id: batch.document_id,
            revision_id: batch.revision_id,
            attempt_count,
            event_ids: batch.attempts.iter().map(|(id, _)| *id).collect(),
            event_attempts: batch.attempts,
        };
        *self
            .pending_activation
            .lock()
            .map_err(|_| "알림 이동 정보를 저장할 수 없습니다.")? = Some(activation.clone());
        let app = self.app.clone();
        self.app
            .run_on_main_thread(move || {
                if let Err(error) = crate::core::lifecycle::show_main_window(&app) {
                    eprintln!("Lifecycle: 알림 창 복원 실패: {error}");
                }
            })
            .map_err(|e| e.to_string())?;
        notification::emit_activation(&self.app, activation);
        Ok(true)
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
                discover_and_deliver(store, &updated).await?;
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
            discover_and_deliver(store, &current).await?;
        }
    }
    Ok(())
}

async fn wait_for_scheduled_task_and_notifications<F>(
    task: F,
    store: &sync::SharedStore,
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
                            discover_and_deliver(store, &prefs).await
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

pub(super) fn notification_authorization_permits(authorization: &str) -> bool {
    authorization == "authorized"
}

pub(super) fn available_notification_slots(recent_attempts: u32) -> usize {
    notification::RATE_LIMIT.saturating_sub(recent_attempts) as usize
}

pub(super) fn revalidate_delivery_batch(
    store: &Store,
    items: Vec<DeliveryItem>,
) -> Result<Vec<DeliveryItem>, String> {
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
    Ok(eligible)
}

/// The final policy check, durable reservation and synchronous handoff share
/// one store lock. Settings or subscriptions cannot change between them.
#[cfg(any(windows, test))]
pub(super) fn handoff_validated<P>(
    shared: &sync::SharedStore,
    items: Vec<DeliveryItem>,
    token: &str,
    prepare: impl FnOnce(&Store, &[DeliveryItem]) -> Result<P, String>,
    send: impl FnOnce(P) -> Result<(), String>,
) -> Result<(), String> {
    let store = shared
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    let items = revalidate_delivery_batch(&store, items)?;
    if items.is_empty() {
        return Ok(());
    }
    let attempts = items
        .iter()
        .map(|item| (item.event_id, item.attempt_count))
        .collect::<Vec<_>>();
    let defer = |reason: &str| -> Result<(), String> {
        for (id, attempt) in &attempts {
            store.proactive_defer_event(*id, *attempt, 1, Some(reason))?;
        }
        Ok(())
    };
    let prepared = match prepare(&store, &items) {
        Ok(prepared) => prepared,
        Err(error) => {
            defer("알림을 준비할 수 없어 나중에 다시 확인합니다.")?;
            return Err(error);
        }
    };
    if let Err(error) = store.notification_token_create(token, &attempts) {
        defer("알림을 준비할 수 없어 나중에 다시 확인합니다.")?;
        return Err(error);
    }
    match store.proactive_reserve_delivery_attempts(&attempts) {
        Ok(true) => {}
        Ok(false) => {
            store.notification_token_revoke(token)?;
            defer("알림 전송 한도에 도달해 나중에 다시 확인합니다.")?;
            return Ok(());
        }
        Err(error) => {
            store.notification_token_revoke(token)?;
            defer("알림을 준비할 수 없어 나중에 다시 확인합니다.")?;
            return Err(error);
        }
    }
    match send(prepared) {
        Ok(()) => {
            store.proactive_mark_attempted_batch(&attempts)?;
            Ok(())
        }
        Err(error) => {
            store.notification_token_revoke(token)?;
            for (id, attempt) in &attempts {
                store.proactive_fail_delivery(*id, *attempt, &error)?;
            }
            Err(error)
        }
    }
}

async fn discover_and_deliver(
    store: &sync::SharedStore,
    prefs: &ProactiveSettings,
) -> Result<(), String> {
    if !should_discover_notifications(prefs) {
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
    // No Windows sender may claim or reserve events when native display is
    // blocked or unavailable, even during quiet hours.
    if !notification_authorization_permits(&authorization) {
        return Ok(());
    }
    #[cfg(windows)]
    let windows_token = super::windows_notification::new_token()?;
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
    #[cfg(windows)]
    {
        handoff_validated(
            store,
            deliver,
            &windows_token,
            |_, deliver| {
                let (title, body) =
                    notification::digest_text(deliver).ok_or("알림 내용을 만들지 못했습니다.")?;
                super::windows_notification::prepare(&title, &body, &windows_token)
            },
            super::windows_notification::show,
        )?;
    }
    Ok(())
}
