use super::{cybercampus, model::*, network, parser};
use crate::storage::Store;
use cybercampus::client::Http;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;
use url::Url;

const FILE_LIMIT: usize = 20 * 1024 * 1024;
const IMPORT_RESOURCES: u64 = 256 * 1024 * 1024;

pub use crate::storage::{with_store, SharedStore};
pub use network::Transport;
// Disabling a source extension sets the cancel flag under the store lock that
// job admission also takes, and admission resets it only for sources whose
// extension is on, so the flag alone stops a disabled extension's work.
pub(super) fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("동기화를 중단했습니다. 저장된 자료는 유지됩니다.".into())
    } else {
        Ok(())
    }
}
pub(super) fn ensure_active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("동기화를 중단했습니다.".into());
    }
    Ok(())
}
/// Sync jobs of the sources the app still collects itself: only Cybercampus
/// (#178). Public boards arrive through the feed (`crate::feed`).
pub struct Controller {
    busy: AtomicBool,
    cancel: AtomicBool,
    status: Mutex<SyncStatus>,
    next_task_id: AtomicU64,
    task_outcomes: Mutex<HashMap<u64, Option<TaskOutcome>>>,
    admission: Mutex<()>,
    active_task_id: AtomicU64,
    task_notifications: Mutex<HashMap<u64, Arc<Notify>>>,
    /// The collecting source and its own cancel flag, so unsubscribing it
    /// stops it without counting as a cancelled job. `cancel` sets it too.
    board: Mutex<Option<(String, Arc<AtomicBool>)>>,
    /// The eClass session and stored login of the `cybercampus` source.
    pub cybercampus: cybercampus::Connector,
    /// Bodies of opened feed notices, fetched outside sync jobs.
    pub bodies: body::Bodies,
}
type RobotsCache = Mutex<HashMap<String, Result<Arc<Robots>, String>>>;
#[derive(Clone, Debug)]
pub struct TaskOutcome {
    pub result: Result<(), String>,
    pub cancelled: bool,
}
impl Controller {
    pub fn new() -> Arc<Self> {
        Self::with_connector(cybercampus::Connector::native())
    }
    pub fn with_connector(cybercampus: cybercampus::Connector) -> Arc<Self> {
        Arc::new(Self {
            busy: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            status: Mutex::new(SyncStatus {
                running: false,
                mode: String::new(),
                message: "동기화 대기 중".into(),
                active: Vec::new(),
            }),
            next_task_id: AtomicU64::new(1),
            task_outcomes: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            active_task_id: AtomicU64::new(0),
            task_notifications: Mutex::new(HashMap::new()),
            board: Mutex::default(),
            cybercampus,
            bodies: body::Bodies::default(),
        })
    }
    pub fn status(&self) -> Result<SyncStatus, String> {
        self.status
            .lock()
            .map(|s| s.clone())
            .map_err(|_| "동기화 상태를 확인하지 못했습니다.".into())
    }
    pub fn take_task_outcome(&self, task_id: u64) -> Result<Option<TaskOutcome>, String> {
        let mut outcomes = self
            .task_outcomes
            .lock()
            .map_err(|_| "동기화 결과를 확인하지 못했습니다.")?;
        match outcomes.get(&task_id) {
            Some(Some(_)) => {
                let outcome = outcomes.remove(&task_id).flatten();
                if let Ok(mut notifications) = self.task_notifications.lock() {
                    notifications.remove(&task_id);
                }
                Ok(outcome)
            }
            Some(None) => Ok(None),
            None => Err("동기화 작업을 찾을 수 없습니다.".into()),
        }
    }
    /// Reports progress: of the collecting source when `source_id` is set,
    /// else of the job.
    fn update(&self, source_id: Option<String>, mode: &str, page: u32, message: &str) {
        if let Ok(mut status) = self.status.lock() {
            status.running = true;
            status.message = message.into();
            let Some(source_id) = source_id else {
                status.mode = mode.into();
                return;
            };
            status.active = vec![ActiveSource {
                source_id,
                mode: mode.into(),
                page,
                message: message.into(),
            }];
        }
    }
    pub fn cancel(&self) {
        // Under the board lock, so a source that starts now reads the flag.
        let board = self.board.lock();
        self.cancel.store(true, Ordering::Release);
        if let Ok(board) = board {
            if let Some((_, cancel)) = board.as_ref() {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    /// Stops source `id` if it is collecting, as unsubscribing it does.
    pub fn cancel_source(&self, id: &str) {
        if let Ok(board) = self.board.lock() {
            if let Some((_, cancel)) = board.as_ref().filter(|(board, _)| board == id) {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    pub fn cancel_task(&self, task_id: u64) -> Result<bool, String> {
        let _admission = self
            .admission
            .lock()
            .map_err(|_| "동기화를 중단하지 못했습니다.")?;
        if self.active_task_id.load(Ordering::Acquire) == task_id {
            self.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub async fn task_completion_notified(&self, task_id: u64) -> Result<(), String> {
        let notify = self
            .task_notifications
            .lock()
            .map_err(|_| "동기화 완료 상태를 확인할 수 없습니다.")?
            .get(&task_id)
            .cloned()
            .ok_or_else(|| "동기화 작업을 찾을 수 없습니다.".to_string())?;
        notify.notified().await;
        Ok(())
    }
    pub async fn wait(&self) {
        while self.busy.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
    fn reserve(&self) -> Result<(), String> {
        self.reserve_task(0)
    }
    fn reserve_task(&self, task_id: u64) -> Result<(), String> {
        let _admission = self
            .admission
            .lock()
            .map_err(|_| "동기화를 시작할 수 없습니다.")?;
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "다른 동기화가 진행 중입니다. 완료하거나 중단한 뒤 시도해 주세요.")?;
        self.cancel.store(false, Ordering::Release);
        self.active_task_id.store(task_id, Ordering::Release);
        if let Ok(mut status) = self.status.lock() {
            status.active.clear();
        }
        Ok(())
    }
    fn finish(&self, message: String) {
        if let Ok(mut status) = self.status.lock() {
            status.running = false;
            status.message = message;
            status.active.clear();
        }
        self.busy.store(false, Ordering::Release);
    }
    fn finish_task(&self, task_id: u64, outcome: TaskOutcome, message: String) {
        let Ok(_admission) = self.admission.lock() else {
            let _ = self.active_task_id.compare_exchange(
                task_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            self.finish(message);
            self.notify_task_completion(task_id);
            return;
        };
        let cancelled = outcome.result.is_err() && self.cancel.load(Ordering::Acquire);
        let outcome = TaskOutcome {
            result: outcome.result,
            cancelled,
        };
        if let Ok(mut outcomes) = self.task_outcomes.lock() {
            if let Some(result) = outcomes.get_mut(&task_id) {
                *result = Some(outcome);
            }
            if let Ok(mut status) = self.status.lock() {
                status.running = false;
                status.message = message;
                status.active.clear();
            }
            let _ = self.active_task_id.compare_exchange(
                task_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            self.notify_task_completion(task_id);
            self.busy.store(false, Ordering::Release);
        } else {
            self.finish(message);
            let _ = self.active_task_id.compare_exchange(
                task_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            self.notify_task_completion(task_id);
        }
    }
    fn notify_task_completion(&self, task_id: u64) {
        if let Ok(notifications) = self.task_notifications.lock() {
            if let Some(notify) = notifications.get(&task_id) {
                notify.notify_one();
            }
        }
    }
    pub fn start_tracked_with_transport<H: Http + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        http: Arc<H>,
        ids: Option<Vec<String>>,
        mode: String,
    ) -> Result<(SyncStatus, u64), String> {
        self.start_tracked_with_lease(store, root, http, ids, mode, None)
    }
    pub fn start_tracked_admitted<H: Http + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        http: Arc<H>,
        ids: Option<Vec<String>>,
        mode: String,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<(SyncStatus, u64), String> {
        self.start_tracked_with_lease(store, root, http, ids, mode, Some(lease))
    }
    fn start_tracked_with_lease<H: Http + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        http: Arc<H>,
        ids: Option<Vec<String>>,
        mode: String,
        lease: Option<crate::operation_gate::AdmissionLease>,
    ) -> Result<(SyncStatus, u64), String> {
        if !["incremental", "refresh", "skipped", "resources"].contains(&mode.as_str()) {
            return Err("동기화 방식을 확인해 주세요.".into());
        }
        // The same lock serializes extension disabling and job admission.
        let task_id = self.next_task_id.fetch_add(1, Ordering::AcqRel);
        let source = with_store(&store, |s| {
            let mut eligible = None;
            for source in s.sources()? {
                if source.subscribed
                    && super::app_collected(&source.definition.id)
                    && s.source_extension_enabled(&source.definition.id)?
                {
                    eligible = Some(source);
                }
            }
            let Some(source) = eligible.filter(|source| {
                ids.as_ref()
                    .is_none_or(|ids| ids.contains(&source.definition.id))
            }) else {
                return Err("동기화할 정보원을 구독해 주세요.".into());
            };
            if ids
                .as_ref()
                .is_some_and(|ids| ids.iter().any(|id| id != &source.definition.id))
            {
                return Err("지원되는 구독 정보원을 선택해 주세요.".into());
            }
            self.reserve_task(task_id)?;
            Ok(source)
        })?;
        // The command path (holding a lease) reports through status polling and
        // never takes its outcome, so only the scheduler path registers one;
        // otherwise the entries would stay for the life of the process.
        if lease.is_none() {
            self.task_outcomes
                .lock()
                .map_err(|_| "동기화 결과를 기록할 수 없습니다.")?
                .insert(task_id, None);
            self.task_notifications
                .lock()
                .map_err(|_| "동기화 완료 상태를 기록할 수 없습니다.")?
                .insert(task_id, Arc::new(Notify::new()));
        }
        self.update(None, &mode, 0, "동기화를 준비하고 있습니다.");
        let controller = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let id = source.definition.id.clone();
            let mut outcome = controller
                .execute(&store, &root, &*http, source, &mode)
                .await;
            if outcome.is_ok() && !matches!(mode.as_str(), "skipped" | "resources") {
                // A source unsubscribed while it ran was stopped on purpose.
                outcome = match with_store(&store, |store| {
                    store
                        .source(&id)
                        .map(|source| source.status == "completed" || !source.subscribed)
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("안전 한도에 걸려 동기화를 끝내지 못했습니다.".into()),
                    Err(error) => Err(error),
                };
            }
            // Before the job reports done, so the refresh it triggers already
            // leaves out the old notices this job archived.
            if let Err(error) = with_store(&store, |store| store.auto_trash_old()) {
                eprintln!("Archive: 오래된 공지를 휴지통으로 옮기지 못했습니다: {error}");
            }
            let task_outcome = TaskOutcome {
                result: outcome.clone(),
                cancelled: outcome.is_err() && controller.cancel.load(Ordering::Acquire),
            };
            let message = match &outcome {
                Ok(()) => "동기화 작업이 끝났습니다. 정보원별 결과를 확인해 주세요.".into(),
                Err(error) => error.clone(),
            };
            controller.finish_task(task_id, task_outcome, message);
        });
        Ok((self.status()?, task_id))
    }
    /// Downloads one resource again: an eClass file through `http` and the
    /// session, anything else through `client` under its source's robots.
    pub fn retry_admitted<H: Http + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        client: network::Client,
        http: Arc<H>,
        id: i64,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<SyncStatus, String> {
        let resource = with_store(&store, |s| {
            if !s.source_extension_enabled(&s.resource_source(id)?)? {
                return Err("이 자료의 확장 기능을 먼저 켜 주세요.".into());
            }
            let resource = s.resource(id)?;
            self.reserve()?;
            Ok(resource)
        })?;
        self.update(None, "resource", 0, "첨부 자료를 다시 내려받고 있습니다.");
        let controller = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = async {
                if cybercampus::owns(&resource.url) {
                    let bytes = AtomicU64::new(0);
                    return cybercampus::download(
                        &controller.cybercampus,
                        &*http,
                        &store,
                        &root,
                        &controller.cancel,
                        &resource,
                        &bytes,
                    )
                    .await?
                    .into_result();
                }
                let url =
                    Url::parse(&resource.url).map_err(|_| "첨부 자료 주소가 올바르지 않습니다.")?;
                let access = if resource.kind == "image" {
                    network::Access::EmbeddedImage
                } else {
                    network::Access::Source
                };
                let policy = Robots::load(&client, &url, &controller.cancel, access).await?;
                let bytes = AtomicU64::new(0);
                download_resource(
                    &store,
                    &root,
                    &client,
                    &controller.cancel,
                    &policy,
                    &RobotsCache::default(),
                    &resource,
                    &bytes,
                )
                .await?
                .into_result()?;
                // A retried viewer PDF becomes the body of its metadata-only notice.
                let document = with_store(&store, |s| s.resource_document(resource.id))?;
                fill_pdf_body(&store, &root, &controller.cancel, document).await
            }
            .await;
            controller.finish(
                result
                    .map(|_| "첨부 자료 다운로드가 끝났습니다. 저장 상태를 확인해 주세요.".into())
                    .unwrap_or_else(|e| e),
            );
        });
        self.status()
    }
    /// Collects `source` (Cybercampus) through its eClass session.
    pub async fn execute<H: Http>(
        &self,
        store: &SharedStore,
        root: &Path,
        http: &H,
        source: SourceInfo,
        mode: &str,
    ) -> Result<(), String> {
        let id = source.definition.id.clone();
        let cancel = {
            let mut board = self
                .board
                .lock()
                .map_err(|_| "동기화 대기열을 준비하지 못했습니다.")?;
            let cancel = Arc::new(AtomicBool::new(self.cancel.load(Ordering::Acquire)));
            *board = Some((id.clone(), cancel.clone()));
            cancel
        };
        let result = self.run(store, root, http, source, mode, &cancel).await;
        if let Ok(mut board) = self.board.lock() {
            *board = None;
        }
        if let Ok(mut status) = self.status.lock() {
            status.active.clear();
        }
        result
    }
    async fn run<H: Http>(
        &self,
        store: &SharedStore,
        root: &Path,
        http: &H,
        source: SourceInfo,
        mode: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        // Retrying skipped notices and downloading missing files are not syncs of
        // the source, so they leave its status, counters and last error as they are.
        let retry = matches!(mode, "skipped" | "resources");
        let mut current = source;
        if !retry {
            current.status = "running".into();
            // Progress counters describe this attempt; checkpoints stay cumulative.
            current.failed = 0;
            current.discovered = 0;
            current.archived = 0;
            persist(store, &current, "running", None, false)?;
        }
        let resource_bytes = AtomicU64::new(0);
        let outcome = match mode {
            // Its skipped notices have no body to retry; the next sync lists them again.
            "skipped" => Ok(()),
            "resources" => {
                cybercampus::download_missing(
                    &self.cybercampus,
                    http,
                    store,
                    root,
                    cancel,
                    &resource_bytes,
                )
                .await
            }
            _ => {
                let id = current.definition.id.clone();
                let report = |message: &str| self.update(Some(id.clone()), mode, 1, message);
                match cybercampus::sync(
                    &self.cybercampus,
                    http,
                    store,
                    root,
                    cancel,
                    &mut current,
                    cybercampus::Progress { report: &report },
                    &resource_bytes,
                )
                .await
                {
                    Ok(()) => persist(store, &current, "completed", None, true),
                    Err(error) => Err(error),
                }
            }
        };
        let Err(error) = outcome else {
            return Ok(());
        };
        let stopped = self.cancel.load(Ordering::Acquire)
            || !with_store(store, |s| {
                s.source_extension_enabled(&current.definition.id)
            })?;
        // Only this source was stopped, by unsubscribing it.
        let unsubscribed = !stopped && cancel.load(Ordering::Acquire);
        if !retry {
            persist(
                store,
                &current,
                if stopped || unsubscribed {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(&error),
                false,
            )?;
        }
        if stopped {
            Err(error)
        } else if unsubscribed {
            Ok(())
        } else {
            Err(format!("{}: {error}", current.definition.name))
        }
    }
}

mod articles;
mod body;
mod pdf;
mod resources;
use articles::*;
use pdf::*;
pub use resources::*;
pub use rhythm_sources::robots;
use robots::*;

#[cfg(test)]
mod tests;
