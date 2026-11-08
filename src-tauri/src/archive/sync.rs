use super::{cybercampus, model::*, network, parser};
use crate::storage::Store;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::Notify;
use url::Url;

#[cfg(test)]
pub use super::network::{Access, Fetch};
pub type SharedStore = Arc<Mutex<Store>>;
const ARTICLE_LIMIT: usize = 4 * 1024 * 1024;
const FILE_LIMIT: usize = 20 * 1024 * 1024;
const IMPORT_RESOURCES: u64 = 256 * 1024 * 1024;

pub trait Transport: Send + Sync {
    fn fetch<'a>(
        &'a self,
        url: &'a Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &'a AtomicBool,
        access: network::Access,
    ) -> impl Future<Output = Result<network::Fetch, String>> + Send + 'a;
}
impl Transport for network::Client {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
        access: network::Access,
    ) -> Result<network::Fetch, String> {
        self.get(url, validators, limit, cancel, access).await
    }
}
pub fn with_store<T>(
    store: &SharedStore,
    operation: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, String> {
    let guard = store
        .lock()
        .map_err(|_| "로컬 저장소에 접근할 수 없습니다.")?;
    operation(&guard)
}
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
#[derive(Clone, Default)]
pub struct Budget {
    pub pages: Option<u32>,
    pub articles: Option<u32>,
}
pub struct Controller {
    busy: AtomicBool,
    cancel: AtomicBool,
    status: Mutex<SyncStatus>,
    next_task_id: AtomicU64,
    task_outcomes: Mutex<HashMap<u64, Option<TaskOutcome>>>,
    admission: Mutex<()>,
    active_task_id: AtomicU64,
    task_notifications: Mutex<HashMap<u64, Arc<Notify>>>,
    /// Boards of the running job, waiting or collecting.
    queue: Mutex<Queue>,
    /// robots.txt of other image hosts, loaded once per job.
    image_robots: RobotsCache,
    /// The eClass session and stored login of the `cybercampus` source.
    pub cybercampus: cybercampus::Connector,
}
type RobotsCache = Mutex<HashMap<String, Result<Arc<Robots>, String>>>;
/// A job's boards. Up to `network::MAX_HOSTS` boards collect at once, never
/// two on the same host, so each host keeps its own request pacing.
#[derive(Default)]
struct Queue {
    pending: VecDeque<SourceInfo>,
    /// Collecting boards by ID, with their host and own cancel flag.
    running: HashMap<String, (String, Arc<AtomicBool>)>,
    /// Extensions whose every subscribed board the job collects. Until its
    /// last board ends, their boards subscribed meanwhile join it.
    joinable: HashSet<Option<String>>,
    /// Every board the job took; its completion check covers them.
    taken: Vec<String>,
}
/// One board's run inside a job: the controller plus that board's own cancel
/// flag, so unsubscribing stops only this board. `Controller::cancel` sets
/// every board's flag.
pub(super) struct Board<'a> {
    controller: &'a Controller,
    cancel: &'a AtomicBool,
}
enum BoardError {
    /// The board failed; the job continues with its other boards.
    Failed(String),
    /// The job was cancelled or cannot continue.
    Stop(String),
}
fn host(source: &SourceInfo) -> String {
    Url::parse(&source.definition.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| source.definition.id.clone())
}
#[derive(Clone, Debug)]
pub struct TaskOutcome {
    pub result: Result<(), String>,
    pub cancelled: bool,
}
impl Controller {
    pub fn new() -> Arc<Self> {
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
            queue: Mutex::default(),
            image_robots: Mutex::new(HashMap::new()),
            cybercampus: cybercampus::Connector::native(),
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
    /// Reports progress: of one board when `source_id` is set, else of the job.
    fn update(&self, source_id: Option<String>, mode: &str, page: u32, message: &str) {
        if let Ok(mut status) = self.status.lock() {
            status.running = true;
            status.message = message.into();
            let Some(source_id) = source_id else {
                status.mode = mode.into();
                return;
            };
            let board = ActiveSource {
                source_id,
                mode: mode.into(),
                page,
                message: message.into(),
            };
            match status
                .active
                .iter_mut()
                .find(|active| active.source_id == board.source_id)
            {
                Some(active) => *active = board,
                None => status.active.push(board),
            }
        }
    }
    fn board_finished(&self, source_id: &str) {
        if let Ok(mut status) = self.status.lock() {
            status.active.retain(|active| active.source_id != source_id);
        }
    }
    pub fn cancel(&self) {
        // Under the queue lock, so a board that starts now reads the flag.
        let queue = self.queue.lock();
        self.cancel.store(true, Ordering::Release);
        if let Ok(queue) = queue {
            for (_, cancel) in queue.running.values() {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    /// Stops one board of the running job, leaving its other boards running.
    pub fn cancel_source(&self, id: &str) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.pending.retain(|source| source.definition.id != id);
            if let Some((_, cancel)) = queue.running.get(id) {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    /// Adds a board subscribed while a job of every subscribed board runs.
    /// Returns whether that job will collect it.
    pub fn join(&self, source: SourceInfo) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        if !queue.joinable.contains(&source.extension) || !source.definition.supported {
            return false;
        }
        let id = &source.definition.id;
        let collecting = queue
            .running
            .get(id)
            .is_some_and(|(_, cancel)| !cancel.load(Ordering::Acquire));
        if !collecting && !queue.pending.iter().any(|s| &s.definition.id == id) {
            queue.pending.push_back(source);
        }
        true
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
        self.reserve_task(0, HashSet::new())
    }
    fn reserve_task(&self, task_id: u64, joinable: HashSet<Option<String>>) -> Result<(), String> {
        let _admission = self
            .admission
            .lock()
            .map_err(|_| "동기화를 시작할 수 없습니다.")?;
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "다른 동기화가 진행 중입니다. 완료하거나 중단한 뒤 시도해 주세요.")?;
        self.cancel.store(false, Ordering::Release);
        self.active_task_id.store(task_id, Ordering::Release);
        if let Ok(mut queue) = self.queue.lock() {
            *queue = Queue {
                joinable,
                ..Queue::default()
            };
        }
        if let Ok(mut status) = self.status.lock() {
            status.active.clear();
        }
        if let Ok(mut robots) = self.image_robots.lock() {
            robots.clear();
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
    #[cfg(test)]
    pub fn start(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        client: network::Client,
        ids: Option<Vec<String>>,
        mode: String,
    ) -> Result<SyncStatus, String> {
        let (status, task_id) = self.start_tracked(store, root, client, ids, mode)?;
        if let Ok(mut outcomes) = self.task_outcomes.lock() {
            outcomes.remove(&task_id);
        }
        if let Ok(mut notifications) = self.task_notifications.lock() {
            notifications.remove(&task_id);
        }
        Ok(status)
    }
    #[cfg(test)]
    pub fn start_tracked(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        client: network::Client,
        ids: Option<Vec<String>>,
        mode: String,
    ) -> Result<(SyncStatus, u64), String> {
        self.start_tracked_with_transport(store, root, client, ids, mode)
    }
    pub fn start_tracked_with_transport<T: Transport + Clone + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        transport: T,
        ids: Option<Vec<String>>,
        mode: String,
    ) -> Result<(SyncStatus, u64), String> {
        self.start_tracked_with_lease(store, root, transport, ids, mode, None)
    }
    pub fn start_tracked_admitted(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        client: network::Client,
        ids: Option<Vec<String>>,
        mode: String,
        lease: crate::operation_gate::AdmissionLease,
    ) -> Result<(SyncStatus, u64), String> {
        self.start_tracked_with_lease(store, root, client, ids, mode, Some(lease))
    }
    fn start_tracked_with_lease<T: Transport + Clone + 'static>(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        transport: T,
        ids: Option<Vec<String>>,
        mode: String,
        lease: Option<crate::operation_gate::AdmissionLease>,
    ) -> Result<(SyncStatus, u64), String> {
        if ![
            "historical",
            "incremental",
            "refresh",
            "skipped",
            "resources",
        ]
        .contains(&mode.as_str())
        {
            return Err("동기화 방식을 확인해 주세요.".into());
        }
        // The same lock serializes extension disabling and job admission.
        let task_id = self.next_task_id.fetch_add(1, Ordering::AcqRel);
        let sources = with_store(&store, |s| {
            let mut eligible = Vec::new();
            for source in s.sources()? {
                if source.subscribed
                    && source.definition.supported
                    && s.source_extension_enabled(&source.definition.id)?
                {
                    eligible.push(source);
                }
            }
            let (sources, others): (Vec<_>, Vec<_>) = eligible.into_iter().partition(|source| {
                ids.as_ref()
                    .is_none_or(|ids| ids.contains(&source.definition.id))
            });
            if sources.is_empty() {
                return Err("동기화할 정보원을 구독해 주세요.".into());
            }
            if ids.as_ref().is_some_and(|ids| {
                ids.iter()
                    .any(|id| !sources.iter().any(|s| &s.definition.id == id))
            }) {
                return Err("지원되는 구독 정보원을 선택해 주세요.".into());
            }
            // A job the user started for every subscribed board of an extension
            // also collects that extension's boards subscribed while it runs.
            // Scheduled jobs (no lease) cover one board each and never do.
            let joinable = if lease.is_some() {
                sources
                    .iter()
                    .map(|source| source.extension.clone())
                    .filter(|extension| !others.iter().any(|o| &o.extension == extension))
                    .collect()
            } else {
                HashSet::new()
            };
            self.reserve_task(task_id, joinable)?;
            Ok(sources)
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
        tauri::async_runtime::spawn(async move {
            let _lease = lease;
            let mut outcome = controller
                .execute(&store, &root, &transport, sources, &mode, Budget::default())
                .await;
            if outcome.is_ok() && !matches!(mode.as_str(), "skipped" | "resources") {
                let taken = controller
                    .queue
                    .lock()
                    .map(|queue| queue.taken.clone())
                    .unwrap_or_default();
                // A board unsubscribed while it ran was stopped on purpose.
                outcome = match with_store(&store, |store| {
                    Ok(taken.iter().all(|id| {
                        store
                            .source(id)
                            .is_ok_and(|source| source.status == "completed" || !source.subscribed)
                    }))
                }) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("정보원 동기화가 안전한 제한으로 완료되지 않았습니다.".into()),
                    Err(error) => Err(error),
                };
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
    pub fn retry_admitted(
        self: &Arc<Self>,
        store: SharedStore,
        root: PathBuf,
        client: network::Client,
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
        tauri::async_runtime::spawn(async move {
            let _lease = lease;
            let result = async {
                if cybercampus::owns(&resource.url) {
                    let bytes = AtomicU64::new(0);
                    return cybercampus::download(
                        &controller.cybercampus,
                        &cybercampus::client::Reqwest,
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
                    &controller.image_robots,
                    &resource,
                    &bytes,
                )
                .await?
                .into_result()?;
                // A retried viewer PDF becomes the body of its metadata-only notice.
                let document = with_store(&store, |s| s.resource_document(resource.id))?;
                let board = Board {
                    controller: &controller,
                    cancel: &controller.cancel,
                };
                board
                    .fill_pdf_body::<network::Client>(&store, &root, None, document)
                    .await
                    .map(|_| ())
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
    pub async fn execute<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        sources: Vec<SourceInfo>,
        mode: &str,
        budget: Budget,
    ) -> Result<(), String> {
        {
            let mut queue = self
                .queue
                .lock()
                .map_err(|_| "동기화 대기열을 준비하지 못했습니다.")?;
            // Boards joined between admission and this point stay queued once.
            for source in sources.into_iter().rev() {
                let id = &source.definition.id;
                if !queue.pending.iter().any(|s| &s.definition.id == id) {
                    queue.pending.push_front(source);
                }
            }
        }
        let resource_bytes = AtomicU64::new(0);
        let work = || self.worker(store, root, transport, mode, &budget, &resource_bytes);
        let results = tokio::join!(work(), work(), work());
        const _: () = assert!(network::MAX_HOSTS == 3, "one worker per collecting host");
        if let Ok(mut queue) = self.queue.lock() {
            queue.pending.clear();
            queue.joinable.clear();
        }
        let mut failures = Vec::new();
        for result in [results.0, results.1, results.2] {
            for failure in result? {
                if !failures.contains(&failure) {
                    failures.push(failure);
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join(" "))
        }
    }
    /// Collects queued boards one at a time, skipping boards whose host another
    /// worker is collecting, until the queue is empty. Returns board failures.
    async fn worker<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        mode: &str,
        budget: &Budget,
        resource_bytes: &AtomicU64,
    ) -> Result<Vec<String>, String> {
        let mut failures = Vec::new();
        loop {
            active(&self.cancel)?;
            if mode == "resources" && resource_bytes.load(Ordering::Acquire) >= IMPORT_RESOURCES {
                failures.push(BUDGET_REACHED.to_owned());
                return Ok(failures);
            }
            let next = {
                let mut queue = self
                    .queue
                    .lock()
                    .map_err(|_| "동기화 대기열을 확인하지 못했습니다.")?;
                let free = queue.pending.iter().position(|source| {
                    let host = host(source);
                    !queue.running.values().any(|(busy, _)| *busy == host)
                });
                match free.and_then(|index| queue.pending.remove(index)) {
                    Some(source) => {
                        let id = source.definition.id.clone();
                        let cancel = Arc::new(AtomicBool::new(self.cancel.load(Ordering::Acquire)));
                        queue
                            .running
                            .insert(id.clone(), (host(&source), cancel.clone()));
                        queue.taken.push(id);
                        Some((source, cancel))
                    }
                    None if queue.pending.is_empty() => {
                        // The last board has ended: later subscriptions wait
                        // for the next job.
                        if queue.running.is_empty() {
                            queue.joinable.clear();
                        }
                        return Ok(failures);
                    }
                    None => None,
                }
            };
            let Some((source, cancel)) = next else {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                continue;
            };
            let id = source.definition.id.clone();
            let board = Board {
                controller: self,
                cancel: &cancel,
            };
            let result = board
                .run(store, root, transport, source, mode, budget, resource_bytes)
                .await;
            if let Ok(mut queue) = self.queue.lock() {
                queue.running.remove(&id);
            }
            self.board_finished(&id);
            match result {
                Ok(()) => {}
                Err(BoardError::Failed(failure)) => failures.push(failure),
                Err(BoardError::Stop(error)) => return Err(error),
            }
        }
    }
}

impl Board<'_> {
    #[allow(clippy::too_many_arguments)] // Shares the job's transport and resource budget.
    async fn run<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        source: SourceInfo,
        mode: &str,
        budget: &Budget,
        resource_bytes: &AtomicU64,
    ) -> Result<(), BoardError> {
        // Retrying skipped notices and downloading missing files are not syncs of
        // the board, so they leave its status, counters and last error as they are.
        let retry = matches!(mode, "skipped" | "resources");
        let mut current = source;
        if !retry {
            current.status = "running".into();
            // Progress counters describe this attempt, not distinct IDs accumulated
            // across failed pages and restarts. Checkpoints remain cumulative.
            current.failed = 0;
            current.discovered = 0;
            current.archived = 0;
            persist(store, &current, "running", None, false).map_err(BoardError::Stop)?;
        }
        // A manual refresh reports new notices first, then continues an
        // unfinished backlog in the same job. Either step's failure or
        // cancellation stops this board and keeps what the previous step saved.
        let outcome = if current.definition.family == cybercampus::FAMILY {
            self.cybercampus(store, root, &mut current, mode, resource_bytes)
                .await
        } else if mode == "refresh" {
            self.refresh(store, root, transport, &mut current, budget, resource_bytes)
                .await
        } else {
            self.source(
                store,
                root,
                transport,
                &mut current,
                mode,
                budget,
                resource_bytes,
            )
            .await
        };
        // Older notices whose body is a PDF viewer gain its text after any
        // run of their board, without re-requesting the notice.
        let outcome = match outcome {
            Ok(()) if current.definition.family != cybercampus::FAMILY => {
                let definition = current.definition.clone();
                self.backfill_pdf_bodies(store, root, transport, &definition, resource_bytes)
                    .await
            }
            outcome => outcome,
        };
        let Err(error) = outcome else {
            return Ok(());
        };
        let stopped = self.controller.cancel.load(Ordering::Acquire)
            || !with_store(store, |s| {
                s.source_extension_enabled(&current.definition.id)
            })
            .map_err(BoardError::Stop)?;
        // Only this board was stopped, by unsubscribing it.
        let unsubscribed = !stopped && self.cancel.load(Ordering::Acquire);
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
            )
            .map_err(BoardError::Stop)?;
        }
        if stopped {
            Err(BoardError::Stop(error))
        } else if unsubscribed {
            Ok(())
        } else {
            Err(BoardError::Failed(format!(
                "{}: {error}",
                current.definition.name
            )))
        }
    }
    /// The `cybercampus` source has no public listing; its connector reads the
    /// logged-in user's courses through the eClass session instead.
    async fn cybercampus(
        &self,
        store: &SharedStore,
        root: &Path,
        current: &mut SourceInfo,
        mode: &str,
        resource_bytes: &AtomicU64,
    ) -> Result<(), String> {
        let http = cybercampus::client::Reqwest;
        match mode {
            // Its skipped notices have no body to retry; the next sync lists them again.
            "skipped" => Ok(()),
            "resources" => {
                cybercampus::download_missing(
                    &self.controller.cybercampus,
                    &http,
                    store,
                    root,
                    self.cancel,
                    resource_bytes,
                )
                .await
            }
            _ => {
                let id = current.definition.id.clone();
                let report =
                    |message: &str| self.controller.update(Some(id.clone()), mode, 1, message);
                cybercampus::sync(
                    &self.controller.cybercampus,
                    &http,
                    store,
                    root,
                    self.cancel,
                    current,
                    cybercampus::Progress { report: &report },
                    resource_bytes,
                )
                .await?;
                persist(store, current, "completed", None, true)
            }
        }
    }
}

mod articles;
mod listing;
mod pdf;
mod resources;
mod robots;
use articles::*;
use pdf::*;
pub use resources::*;
use robots::*;

#[cfg(test)]
mod tests;
