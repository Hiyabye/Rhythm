use super::{model::*, network, parser};
use crate::storage::Store;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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
pub use super::network::Fetch;
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
    ) -> impl Future<Output = Result<network::Fetch, String>> + Send + 'a;
}
impl Transport for network::Client {
    async fn fetch(
        &self,
        url: &Url,
        validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<network::Fetch, String> {
        self.get(url, validators, limit, cancel).await
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
fn active(store: &SharedStore, cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) || !with_store(store, |s| s.enabled("sogang_notices"))? {
        Err("동기화를 중단했습니다. 저장된 자료는 유지됩니다.".into())
    } else {
        Ok(())
    }
}
fn ensure_active(store: &Store, cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) || !store.enabled("sogang_notices")? {
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
                source_id: None,
                mode: String::new(),
                page: 0,
                message: "동기화 대기 중".into(),
            }),
            next_task_id: AtomicU64::new(1),
            task_outcomes: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            active_task_id: AtomicU64::new(0),
            task_notifications: Mutex::new(HashMap::new()),
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
    fn update(&self, source_id: Option<String>, mode: &str, page: u32, message: &str) {
        if let Ok(mut status) = self.status.lock() {
            *status = SyncStatus {
                running: true,
                source_id,
                mode: mode.into(),
                page,
                message: message.into(),
            };
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn cancel_task(&self, task_id: u64) -> Result<bool, String> {
        let _admission = self
            .admission
            .lock()
            .map_err(|_| "동기화를 중단하지 못했습니다.")?;
        if self.active_task_id.load(Ordering::Acquire) == task_id {
            self.cancel.store(true, Ordering::Release);
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
        Ok(())
    }
    fn finish(&self, message: String) {
        if let Ok(mut status) = self.status.lock() {
            status.running = false;
            status.message = message;
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
        if !["historical", "incremental"].contains(&mode.as_str()) {
            return Err("동기화 방식을 확인해 주세요.".into());
        }
        // The same lock serializes extension disabling and job admission.
        let task_id = self.next_task_id.fetch_add(1, Ordering::AcqRel);
        let sources = with_store(&store, |s| {
            ensure_active(s, &AtomicBool::new(false))?;
            let sources: Vec<_> = s
                .sources()?
                .into_iter()
                .filter(|source| {
                    source.subscribed
                        && source.definition.supported
                        && ids
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&source.definition.id))
                })
                .collect();
            if sources.is_empty() {
                return Err("동기화할 정보원을 구독해 주세요.".into());
            }
            if ids.as_ref().is_some_and(|ids| {
                ids.iter()
                    .any(|id| !sources.iter().any(|s| &s.definition.id == id))
            }) {
                return Err("지원되는 구독 정보원을 선택해 주세요.".into());
            }
            self.reserve_task(task_id)?;
            Ok(sources)
        })?;
        self.task_outcomes
            .lock()
            .map_err(|_| "동기화 결과를 기록할 수 없습니다.")?
            .insert(task_id, None);
        self.task_notifications
            .lock()
            .map_err(|_| "동기화 완료 상태를 기록할 수 없습니다.")?
            .insert(task_id, Arc::new(Notify::new()));
        let source_ids = sources
            .iter()
            .map(|source| source.definition.id.clone())
            .collect::<Vec<_>>();
        self.update(None, &mode, 0, "동기화를 준비하고 있습니다.");
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            let _lease = lease;
            let mut outcome = controller
                .execute(&store, &root, &transport, sources, &mode, Budget::default())
                .await;
            if outcome.is_ok() {
                outcome = match with_store(&store, |store| {
                    Ok(source_ids.iter().all(|id| {
                        store
                            .source(id)
                            .is_ok_and(|source| source.status == "completed")
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
            ensure_active(s, &AtomicBool::new(false))?;
            let resource = s.resource(id)?;
            self.reserve()?;
            Ok(resource)
        })?;
        self.update(None, "resource", 0, "첨부 자료를 다시 내려받고 있습니다.");
        let controller = self.clone();
        tauri::async_runtime::spawn(async move {
            let _lease = lease;
            let result = async {
                let url =
                    Url::parse(&resource.url).map_err(|_| "첨부 자료 주소가 올바르지 않습니다.")?;
                let policy = Robots::load(&client, &url, &controller.cancel).await?;
                let mut bytes = 0;
                download_resource(
                    &store,
                    &root,
                    &client,
                    &controller.cancel,
                    &policy,
                    &resource,
                    &mut bytes,
                )
                .await?
                .into_result()
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
        let mut resource_bytes = 0;
        let mut failures = Vec::new();
        for source in sources {
            active(store, &self.cancel)?;
            let mut current = source.clone();
            current.status = "running".into();
            // Progress counters describe this attempt, not distinct IDs accumulated
            // across failed pages and restarts. Checkpoints remain cumulative.
            current.failed = 0;
            current.discovered = 0;
            current.archived = 0;
            persist(store, &current, "running", None, false)?;
            let outcome = self
                .source(
                    store,
                    root,
                    transport,
                    &mut current,
                    mode,
                    &budget,
                    &mut resource_bytes,
                )
                .await;
            if let Err(error) = outcome {
                let cancelled = self.cancel.load(Ordering::Acquire)
                    || !with_store(store, |s| s.enabled("sogang_notices"))?;
                persist(
                    store,
                    &current,
                    if cancelled { "cancelled" } else { "failed" },
                    Some(&error),
                    false,
                )?;
                if cancelled {
                    return Err(error);
                }
                failures.push(format!("{}: {error}", current.definition.name));
                continue;
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join(" "))
        }
    }
    #[allow(clippy::too_many_arguments)] // One source run shares a bounded transport and resource budget.
    async fn source<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        current: &mut SourceInfo,
        mode: &str,
        budget: &Budget,
        resource_bytes: &mut u64,
    ) -> Result<(), String> {
        let historical = mode == "historical";
        // The durable flag, not the current document count, decides whether the
        // initial bootstrap is still owed. A cancelled or failed seed leaves it
        // unset, so later incremental runs keep marking historical pages as
        // historical instead of reporting them as newly published notices.
        let seed = !historical && !with_store(store, |s| s.seed_complete(&current.definition.id))?;
        if historical && current.history_complete {
            persist(store, current, "completed", None, true)?;
            return Ok(());
        }
        let first_page = if historical { current.next_page } else { 1 };
        let definition = &current.definition.clone();
        let initial = parser::list_url(definition, first_page)?;
        let policy = Robots::load(transport, &initial, &self.cancel).await?;
        let mut seen = HashSet::new();
        let mut page_hashes = HashSet::new();
        let mut page = first_page;
        let mut processed = 0;
        loop {
            active(store, &self.cancel)?;
            if page > 100_000 {
                persist(
                    store,
                    current,
                    "limited",
                    Some(
                        "안전한 페이지 수 제한에 도달했습니다. 전체 이력 완료로 표시하지 않습니다.",
                    ),
                    false,
                )?;
                return Ok(());
            }
            self.update(
                Some(definition.id.clone()),
                mode,
                page,
                "이번 동기화의 목록과 원문을 보관하고 있습니다.",
            );
            let url = parser::list_url(definition, page)?;
            let response =
                fetch(transport, &url, None, ARTICLE_LIMIT, &self.cancel, &policy).await?;
            let listing = match response {
                network::Fetch::Body { bytes, mime, .. } => {
                    check_document_mime(definition, mime.as_deref())?;
                    parser::listing(
                        definition,
                        page,
                        std::str::from_utf8(&bytes)
                            .map_err(|_| "공지 목록 문자 인코딩을 지원하지 않습니다.")?,
                    )?
                }
                network::Fetch::NotModified | network::Fetch::Redirect(_) => {
                    return Err(
                        "보관되지 않은 목록의 변경 없음 또는 이동 응답을 받았습니다.".into(),
                    )
                }
            };
            let pinned = listing.entries.iter().filter(|entry| entry.pinned).count();
            self.update(
                Some(definition.id.clone()),
                mode,
                page,
                &format!(
                    "{} / {} 페이지 · 고정 공지 {}건 포함 · 원문 보관 중",
                    page, listing.total_pages, pinned
                ),
            );
            let fingerprint = listing
                .entries
                .iter()
                .map(|e| e.external_id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            if listing.has_next && (!page_hashes.insert(fingerprint) || listing.entries.is_empty())
            {
                return Err("학교 목록이 반복되어 가져오기를 중단했습니다. 전체 이력을 완료하지 않았습니다.".into());
            }
            let mut errors = Vec::new();
            let mut known_regular_frontier = false;
            let mut all_regular_known = true;
            for entry in listing.entries {
                active(store, &self.cancel)?;
                let existing_id =
                    with_store(store, |s| s.document_id(&definition.id, &entry.external_id))?;
                let exists = existing_id.is_some();
                if !entry.pinned {
                    known_regular_frontier = true;
                    all_regular_known &= exists;
                }
                if !seen.insert(entry.external_id.clone()) {
                    continue;
                }
                if historical {
                    if let Some(id) = existing_id {
                        self.pending_resources(store, root, transport, &policy, id, resource_bytes)
                            .await?;
                        continue;
                    }
                }
                if budget.articles.is_some_and(|max| processed >= max) {
                    if !errors.is_empty() {
                        return Err(format!(
                            "원문 {}건을 보관하지 못했습니다. 같은 페이지에서 이어집니다. {}",
                            errors.len(),
                            errors[0]
                        ));
                    }
                    persist(store,current,"limited",Some("개발 검증용 제한으로 일부만 가져왔습니다. 전체 이력은 아직 완료되지 않았습니다."),false)?;
                    return Ok(());
                }
                if !exists {
                    current.discovered += 1;
                }
                let result = async {
                    let validators =
                        with_store(store, |s| s.validators(&definition.id, &entry.external_id))?;
                    let url = parser::detail_url(definition, &entry)?;
                    let response = fetch(
                        transport,
                        &url,
                        validators,
                        ARTICLE_LIMIT,
                        &self.cancel,
                        &policy,
                    )
                    .await?;
                    match response {
                        network::Fetch::NotModified => {
                            let id = existing_id
                                .ok_or("보관되지 않은 원문의 변경 없음 응답을 받았습니다.")?;
                            with_store(store, |s| {
                                ensure_active(s, &self.cancel)?;
                                s.checked(&definition.id, &entry.external_id)
                            })?;
                            self.pending_resources(
                                store,
                                root,
                                transport,
                                &policy,
                                id,
                                resource_bytes,
                            )
                            .await?;
                        }
                        network::Fetch::Redirect(_) => {
                            return Err("학교 서버 이동 응답을 처리하지 못했습니다.".into())
                        }
                        network::Fetch::Body {
                            bytes,
                            etag,
                            last_modified,
                            mime,
                        } => {
                            check_document_mime(definition, mime.as_deref())?;
                            let mut article = parser::article(
                                definition,
                                &entry,
                                std::str::from_utf8(&bytes)
                                    .map_err(|_| "공지 본문 문자 인코딩을 지원하지 않습니다.")?,
                            )?;
                            let replaces_readable = match existing_id {
                                Some(id) => with_store(store, |s| s.has_readable_content(id))?,
                                None => false,
                            };
                            if replaces_readable && article.unsupported_body {
                                return Err(
                                    "기존 원문을 읽을 수 없는 iframe 본문으로 대체하지 않았습니다."
                                        .into(),
                                );
                            }
                            article.etag = etag;
                            article.last_modified = last_modified;
                            let (id, _) = with_store(store, |s| {
                                ensure_active(s, &self.cancel)?;
                                s.upsert_article(&definition.id, &article, historical || seed)
                            })?;
                            if !exists {
                                current.archived += 1;
                            }
                            self.pending_resources(
                                store,
                                root,
                                transport,
                                &policy,
                                id,
                                resource_bytes,
                            )
                            .await?;
                        }
                    }
                    Ok::<_, String>(())
                }
                .await;
                processed += 1;
                if let Err(error) = result {
                    active(store, &self.cancel)?;
                    current.failed += 1;
                    errors.push(error);
                }
                persist(store, current, "running", None, false)?;
            }
            // Never checkpoint beyond a page containing failed article retrievals.
            if !errors.is_empty() {
                return Err(format!(
                    "원문 {}건을 보관하지 못했습니다. 같은 페이지에서 이어집니다. {}",
                    errors.len(),
                    errors[0]
                ));
            }
            if historical {
                current.next_page = page + 1;
            }
            if !listing.has_next {
                if historical {
                    current.history_complete = true;
                }
                // Reaching the last page safely also established the initial
                // frontier, whether through the bounded seed or a manual import.
                if historical || seed {
                    mark_seed_complete(store, &definition.id)?;
                }
                persist(store, current, "completed", None, true)?;
                return Ok(());
            }
            // The three-page edit window must not truncate discovery of a larger
            // burst. Only regular records (never repeated pins) establish a frontier.
            if !historical && page >= 3 && (seed || (known_regular_frontier && all_regular_known)) {
                if seed {
                    mark_seed_complete(store, &definition.id)?;
                }
                persist(store, current, "completed", None, true)?;
                return Ok(());
            }
            if budget.pages.is_some_and(|max| page - first_page + 1 >= max) {
                persist(store,current,"limited",Some("개발 검증용 페이지 제한으로 중단했습니다. 이력은 이어서 가져올 수 있습니다."),false)?;
                return Ok(());
            }
            persist(store, current, "running", None, false)?;
            page += 1;
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn pending_resources<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        policy: &Robots,
        id: i64,
        total: &mut u64,
    ) -> Result<(), String> {
        for resource in with_store(store, |s| s.pending_resources(id))? {
            // A persisted resource failure is nonfatal to article/page completion.
            // Cancellation and database failures still propagate.
            download_resource(
                store,
                root,
                transport,
                &self.cancel,
                policy,
                &resource,
                total,
            )
            .await?;
        }
        Ok(())
    }
}
fn mark_seed_complete(store: &SharedStore, source: &str) -> Result<(), String> {
    with_store(store, |s| s.mark_seed_complete(source))
}
fn persist(
    store: &SharedStore,
    source: &SourceInfo,
    status: &str,
    error: Option<&str>,
    success: bool,
) -> Result<(), String> {
    with_store(store, |s| {
        s.source_progress(
            &source.definition.id,
            status,
            source.next_page,
            source.history_complete,
            source.discovered,
            source.archived,
            source.failed,
            error,
            success,
        )
    })
}
fn check_document_mime(source: &SourceDefinition, mime: Option<&str>) -> Result<(), String> {
    if if source.id == "sogang-academic" {
        mime != Some("application/json")
    } else {
        !matches!(mime, Some("text/html" | "application/xhtml+xml"))
    } {
        return Err("학교 서버가 예상한 공지 형식 대신 다른 자료를 반환했습니다.".into());
    }
    Ok(())
}
async fn fetch<T: Transport>(
    transport: &T,
    url: &Url,
    validators: Option<(Option<String>, Option<String>)>,
    limit: usize,
    cancel: &AtomicBool,
    policy: &Robots,
) -> Result<network::Fetch, String> {
    let mut target = url.clone();
    for redirects in 0..=3 {
        // Apply robots to every hop before any request, not only the original URL.
        // Origin equality also prevents sending validators to another source host.
        if !policy.permits(&target) {
            return Err("학교의 robots.txt 정책에 따라 수집을 중단했습니다.".into());
        }
        if policy.delay > 0.65 {
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs_f64(policy.delay))=>{},_=wait_cancel(cancel)=>return Err("동기화를 중단했습니다.".into())}
        }
        match transport
            .fetch(&target, validators.clone(), limit, cancel)
            .await?
        {
            network::Fetch::Redirect(next) => {
                if redirects == 3 {
                    return Err("학교 서버의 이동 경로가 너무 많습니다.".into());
                }
                network::validate_url(&next)?;
                target = next;
            }
            response => return Ok(response),
        }
    }
    unreachable!()
}
async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
type RobotGroup = (Vec<String>, Vec<(String, bool)>, f64);
struct Robots {
    origin: String,
    rules: Vec<(String, bool)>,
    delay: f64,
}
impl Robots {
    async fn load<T: Transport>(
        transport: &T,
        url: &Url,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let robots = url
            .join("/robots.txt")
            .map_err(|_| "정보원 주소를 확인해 주세요.")?;
        match transport.fetch(&robots, None, 64 * 1024, cancel).await? {
            network::Fetch::Body { bytes, mime, .. } => {
                if matches!(mime.as_deref(), Some("text/html" | "application/xhtml+xml")) {
                    return Err("학교의 수집 정책 대신 HTML 오류 페이지를 받았습니다.".into());
                }
                Self::parse(
                    url,
                    std::str::from_utf8(&bytes)
                        .map_err(|_| "학교의 수집 정책을 해석하지 못했습니다.")?,
                )
            }
            _ => Err("학교의 수집 정책을 확인하지 못했습니다.".into()),
        }
    }
    fn parse(url: &Url, body: &str) -> Result<Self, String> {
        // An HTTP-200 error page is not an empty allow-all robots policy.
        let start = body.trim_start_matches('\u{feff}').trim_start();
        if start.starts_with('<') || start.contains('\0') {
            return Err("학교의 수집 정책 대신 해석할 수 없는 자료를 받았습니다.".into());
        }
        let mut groups: Vec<RobotGroup> = Vec::new();
        let mut agents = Vec::new();
        let mut rules = Vec::new();
        let mut delay = 0.0;
        let mut directives = false;
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "user-agent" => {
                    if directives {
                        groups.push((
                            std::mem::take(&mut agents),
                            std::mem::take(&mut rules),
                            delay,
                        ));
                        delay = 0.0;
                        directives = false;
                    }
                    agents.push(value.to_ascii_lowercase());
                }
                "allow" | "disallow" if !agents.is_empty() => {
                    directives = true;
                    if !value.is_empty() {
                        rules.push((value.into(), name.trim().eq_ignore_ascii_case("allow")));
                    }
                }
                "crawl-delay" if !agents.is_empty() => {
                    directives = true;
                    delay = value
                        .parse::<f64>()
                        .map_err(|_| "학교의 요청 간격 정책을 해석하지 못했습니다.")?;
                    // Validate before max(): floating-point max can discard NaN.
                    if !delay.is_finite() || !(0.0..=60.0).contains(&delay) {
                        return Err("학교의 수집 간격 정책은 자동 처리할 수 없습니다.".into());
                    }
                }
                _ => {}
            }
        }
        groups.push((agents, rules, delay));
        let specific = groups
            .iter()
            .any(|(a, _, _)| a.iter().any(|a| a == "rhythm"));
        let mut policy = Self {
            origin: url.origin().ascii_serialization(),
            rules: Vec::new(),
            delay: 0.65,
        };
        for (agents, rules, delay) in groups {
            if agents
                .iter()
                .any(|a| a == if specific { "rhythm" } else { "*" })
            {
                policy.rules.extend(rules);
                policy.delay = policy.delay.max(delay);
            }
        }
        if !policy.delay.is_finite() || policy.delay > 60.0 || policy.delay < 0.0 {
            return Err("학교의 수집 간격 정책은 자동 처리할 수 없습니다.".into());
        }
        Ok(policy)
    }
    fn permits(&self, url: &Url) -> bool {
        // Automatic cross-origin resource fetching is not needed for verified boards.
        if url.origin().ascii_serialization() != self.origin {
            return false;
        }
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let mut decision = (0, true);
        for (rule, allow) in &self.rules {
            let pattern = rule.strip_suffix('$').unwrap_or(rule);
            let matched = wildcard_prefix(pattern.as_bytes(), path.as_bytes(), rule.ends_with('$'));
            let weight = pattern.bytes().filter(|byte| *byte != b'*').count();
            if matched && (weight > decision.0 || (weight == decision.0 && *allow)) {
                decision = (weight, *allow);
            }
        }
        decision.1
    }
}
// Iterative glob matching with a single remembered wildcard; no recursion or
// regex backtracking. Unanchored robots rules match any suffix of the URL path.
fn wildcard_prefix(pattern: &[u8], text: &[u8], anchored: bool) -> bool {
    let (mut p, mut t) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while t < text.len() {
        if p == pattern.len() && !anchored {
            return true;
        }
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = t;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some(index) = star {
            retry += 1;
            t = retry;
            p = index + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[derive(Debug)]
enum DownloadOutcome {
    Downloaded,
    Failed(String),
}
impl DownloadOutcome {
    fn into_result(self) -> Result<(), String> {
        match self {
            Self::Downloaded => Ok(()),
            Self::Failed(error) => Err(error),
        }
    }
}
fn raster(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
async fn download_resource<T: Transport>(
    store: &SharedStore,
    root: &Path,
    transport: &T,
    cancel: &AtomicBool,
    policy: &Robots,
    resource: &ResourceRecord,
    total: &mut u64,
) -> Result<DownloadOutcome, String> {
    active(store, cancel)?;
    let outcome: Result<(), String> = async {
        let url=Url::parse(&resource.url).map_err(|_|"첨부 자료 주소를 해석하지 못했습니다.")?;
        network::validate_url(&url)?;
        if *total>=IMPORT_RESOURCES {return Err("이번 가져오기의 첨부 자료 한도(256 MB)에 도달했습니다. 필요한 자료를 개별적으로 다시 내려받아 주세요.".into());}
        let limit=(if resource.kind=="image" {10*1024*1024}else{FILE_LIMIT}).min((IMPORT_RESOURCES-*total) as usize);
        match fetch(transport,&url,None,limit,cancel,policy).await? {
            network::Fetch::Body{bytes,mime,..}=>{
                // Charge received bytes even when format validation or disk writes fail.
                // The transport independently caps the next response to remaining budget.
                *total=total.saturating_add(bytes.len() as u64);
                if bytes.len()>limit || *total>IMPORT_RESOURCES {return Err("이번 가져오기의 첨부 자료 한도(256 MB)를 초과했습니다.".into());}
                let media=if resource.kind=="image" {Some(raster(&bytes).ok_or("안전한 이미지 형식(PNG, JPEG, GIF, WebP)이 아니어서 보관하지 않았습니다.")?.to_string())} else {
                    let mime=mime.filter(|m|!matches!(m.as_str(),"text/html"|"application/xhtml+xml"|"application/javascript"|"text/javascript"|"image/svg+xml"));
                    if mime.is_none() || bytes.is_empty(){return Err("첨부 파일 대신 오류 페이지 또는 지원하지 않는 자료를 받았습니다.".into());}
                    let lower=resource.filename.to_ascii_lowercase();
                    if ![".pdf",".hwp",".hwpx",".doc",".docx",".xls",".xlsx",".ppt",".pptx",".zip",".txt",".png",".jpg",".jpeg",".gif",".webp"].iter().any(|ext|lower.ends_with(ext)){return Err("자동 다운로드를 지원하지 않는 첨부 파일 형식입니다.".into());}
                    if lower.ends_with(".pdf") && !bytes.starts_with(b"%PDF-"){return Err("PDF 파일 형식 검증에 실패했습니다.".into());}
                    mime
                };
                active(store,cancel)?;
                let hash=format!("{:x}",Sha256::digest(&bytes));
                let extension = if resource.kind=="image" {match media.as_deref(){Some("image/png")=>"png",Some("image/jpeg")=>"jpg",Some("image/gif")=>"gif",_=>"webp"}} else {resource.filename.rsplit('.').next().unwrap_or("bin")};
                let extension=extension.to_ascii_lowercase();
                let relative=format!("resources/{hash}.{extension}");
                tokio::fs::create_dir_all(root.join("resources")).await.map_err(|_|"첨부 저장 폴더를 만들지 못했습니다. 저장 공간과 권한을 확인해 주세요.")?;
                let temporary=root.join(format!("resources/{hash}.part"));
                tokio::fs::write(&temporary,&bytes).await.map_err(|_|"첨부 파일을 저장하지 못했습니다. 남은 저장 공간과 권한을 확인해 주세요.")?;
                let destination=root.join(&relative);
                // Generated hash paths never contain a remote filename. A completed file
                // is installed before its reference is committed to SQLite.
                if tokio::fs::try_exists(&destination).await.map_err(|_|"첨부 파일 경로를 확인하지 못했습니다.")? {
                    // A retry must repair corrupt bytes, not discard its verified download.
                    let size=tokio::fs::metadata(&destination).await.map_err(|_|"기존 첨부 파일을 확인하지 못했습니다.")?.len();
                    let matches = if size<=FILE_LIMIT as u64 {
                        let existing=tokio::fs::read(&destination).await.map_err(|_|"기존 첨부 파일을 확인하지 못했습니다.")?;
                        Sha256::digest(&existing)==Sha256::digest(&bytes)
                    } else {false};
                    if matches {
                        tokio::fs::remove_file(&temporary).await.map_err(|_|"임시 첨부 파일을 정리하지 못했습니다.")?;
                    } else {
                        // Windows does not replace an existing destination on rename.
                        tokio::fs::remove_file(&destination).await.map_err(|_|"손상된 첨부 파일을 교체하지 못했습니다.")?;
                        tokio::fs::rename(&temporary,&destination).await.map_err(|_|"첨부 파일 복구를 완료하지 못했습니다.")?;
                    }
                } else {tokio::fs::rename(&temporary,&destination).await.map_err(|_|"첨부 파일 저장을 완료하지 못했습니다.")?;}
                with_store(store,|s|{ensure_active(s,cancel)?;s.resource_result(resource.id,"downloaded",media.as_deref(),Some(bytes.len() as u64),Some(&relative),Some(&hash),None)})?;
                Ok(())
            },_=>Err("첨부 파일에 잘못된 변경 없음 응답이 도착했습니다.".into())
        }
    }.await;
    if let Err(error) = outcome {
        active(store, cancel)?;
        if resource.status == "downloaded" && local_resource(store, root, resource.id).await.is_ok()
        {
            return Ok(DownloadOutcome::Failed(format!(
                "다시 내려받지 못했습니다. 기존 로컬 자료는 안전하게 유지됩니다. {error}"
            )));
        }
        let status = if error.contains("크기") || error.contains("한도") {
            "too_large"
        } else if error.contains("지원")
            || error.contains("검증된")
            || error.contains("안전한 이미지")
        {
            "unsupported"
        } else {
            "failed"
        };
        with_store(store, |s| {
            s.resource_result(resource.id, status, None, None, None, None, Some(&error))
        })?;
        return Ok(DownloadOutcome::Failed(error));
    }
    Ok(DownloadOutcome::Downloaded)
}
pub async fn local_resource(
    store: &SharedStore,
    root: &Path,
    id: i64,
) -> Result<(ResourceRecord, Vec<u8>), String> {
    let resource = with_store(store, |s| s.resource(id))?;
    if resource.status != "downloaded" {
        return Err("이 자료는 아직 로컬에 저장되지 않았습니다.".into());
    }
    let path = root.join(
        resource
            .local_path
            .as_ref()
            .ok_or("로컬 자료 경로가 없습니다.")?,
    );
    let canonical_root = tokio::fs::canonicalize(root.join("resources"))
        .await
        .map_err(|_| "첨부 저장 폴더를 찾지 못했습니다.")?;
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| "로컬 첨부 파일을 찾지 못했습니다. 다시 내려받아 주세요.")?;
    if !canonical.starts_with(canonical_root) {
        return Err("안전하지 않은 로컬 자료 경로입니다.".into());
    }
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| "로컬 첨부 파일을 확인하지 못했습니다.")?;
    if metadata.len() > FILE_LIMIT as u64 {
        return Err("로컬 첨부 파일의 크기가 허용 범위를 초과했습니다.".into());
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|_| "로컬 첨부 파일을 읽지 못했습니다.")?;
    if Some(format!("{:x}", Sha256::digest(&bytes))) != resource.sha256 {
        return Err("로컬 자료의 무결성 검증에 실패했습니다. 다시 내려받아 주세요.".into());
    }
    Ok((resource, bytes))
}
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    raster(bytes)
}
#[cfg(test)]
mod tests;
