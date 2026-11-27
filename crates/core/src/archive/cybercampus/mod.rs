//! Cybercampus (eClass) connector (#86). It reads the logged-in user's own
//! courses of the current term: course notices become archive documents of the
//! `cybercampus` source, assignments and other deadlines go to
//! `cybercampus_tasks`. Read-only: it never submits, posts or confirms anything,
//! although opening a notice counts as a view on eClass, as in a browser.
pub mod client;
pub mod credentials;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod parser;
#[cfg(test)]
mod tests;

use super::model::{ArticleInput, ListingEntry, ResourceRecord, SourceInfo};
use super::sync::{self, with_store, DownloadOutcome, SharedStore};
use crate::storage::cybercampus::{CourseInput, TaskInput};
use client::{Http, Session, PAGE_LIMIT};
use credentials::Vault;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use url::Url;

pub use rhythm_sources::catalog::{
    CYBERCAMPUS_FAMILY as FAMILY, CYBERCAMPUS_SOURCE_ID as SOURCE_ID,
};
/// eClass shows 20 notices per list page; a course rarely has more than a few
/// pages, so this bounds a malformed total, not a real board.
const NOTICE_PAGES: u32 = 25;
const NOTICE_DISPLAY: u32 = 20;
/// Rows asked of the TODO list; the response carries no total, so a page this
/// full may continue.
const TODO_DISPLAY: usize = 50;

const LOCKED: &str = "사이버캠퍼스 로그인 정보에 접근하지 못했습니다.";

pub struct Connector {
    pub session: Session,
    pub vault: Vault,
    /// How many times the login was forgotten. A save compares it before
    /// storing, so a forget that happened during its network check wins; the
    /// lock also keeps a forget from slipping between that check and the store.
    forgotten: std::sync::Mutex<u64>,
}
impl Connector {
    pub fn native() -> Self {
        Self {
            session: Session::default(),
            vault: Vault::native(),
            forgotten: Default::default(),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn memory() -> Self {
        Self {
            session: Session::unpaced(),
            vault: Vault::memory(),
            forgotten: Default::default(),
        }
    }
    /// The user's explicit save: logs in first (this attempt ignores the
    /// account-lock guard; success clears it) and stores the login only when
    /// eClass accepts it, the extension is still on and the login was not
    /// forgotten meanwhile.
    pub async fn save_login<H: Http>(
        &self,
        http: &H,
        store: &SharedStore,
        credentials: &credentials::Credentials,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let seen = *self.forgotten.lock().map_err(|_| LOCKED)?;
        self.session.login(http, store, credentials, cancel).await?;
        let outcome = {
            let forgotten = self.forgotten.lock().map_err(|_| LOCKED)?;
            if !with_store(store, |s| s.enabled(crate::extensions::CYBERCAMPUS))? {
                Err("사이버캠퍼스 확장 기능을 먼저 켜 주세요.".to_string())
            } else if *forgotten != seen {
                Err("저장하는 동안 로그인 정보를 삭제해서 저장하지 않았습니다.".to_string())
            } else {
                self.vault.save(credentials)
            }
        };
        if outcome.is_err() {
            // The login that just succeeded must not leave a session behind.
            self.session.reset().await;
        }
        outcome
    }
    /// Deletes the stored login, the in-memory session and the account-lock
    /// guard.
    pub async fn forget_login(&self, store: &SharedStore) -> Result<(), String> {
        self.forget_stored()?;
        self.session.reset().await;
        with_store(store, |s| s.cybercampus_login_clear())
    }
    /// Deletes the stored login and makes a pending save give up.
    fn forget_stored(&self) -> Result<(), String> {
        let mut forgotten = self.forgotten.lock().map_err(|_| LOCKED)?;
        *forgotten += 1;
        self.vault.clear()
    }
}

/// True for resources this connector downloads with the session.
pub fn owns(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| client::validate(&url).is_ok())
}

fn canonical_url() -> String {
    format!("{}/ilos/main/main_form.acl", parser::ORIGIN)
}
/// The list row's title and preview, stored in `documents.modified_at`. eClass
/// gives no edit time, so an unchanged row is not fetched (and not counted as
/// another view) again.
fn fingerprint(row: &parser::NoticeRow) -> String {
    let digest = Sha256::digest(format!("{}\n{}", row.title, row.preview));
    format!("list:{:x}", digest)[..37].to_owned()
}
/// Carries the TODO list's contribution of the last successful sync into a
/// fresh task list: the `pending` flag of rows both have and the stored rows
/// (`pending` ones are the TODO-only quizzes and lectures) the fresh list lacks.
fn keep_todo_rows(stored: &HashMap<(String, String), TaskInput>, tasks: &mut Vec<TaskInput>) {
    for task in tasks.iter_mut() {
        let key = (task.category.clone(), task.item_id.clone());
        task.pending |= stored.get(&key).is_some_and(|known| known.pending);
    }
    let mut kept = stored
        .values()
        .filter(|known| {
            known.pending
                && !tasks
                    .iter()
                    .any(|task| task.category == known.category && task.item_id == known.item_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    kept.sort_by(|a, b| (&a.category, &a.item_id).cmp(&(&b.category, &b.item_id)));
    tasks.extend(kept);
}
fn text(response: &client::Response) -> Result<String, String> {
    String::from_utf8(response.bytes.clone())
        .map_err(|_| "사이버캠퍼스 응답의 문자 인코딩을 지원하지 않습니다.".into())
}

pub struct Progress<'a> {
    pub report: &'a (dyn Fn(&str) + Sync),
}

/// One sync of the source: current-term courses, their notices and deadlines.
/// Each course is independent; a failed course keeps its stored rows.
#[allow(clippy::too_many_arguments)]
pub async fn sync<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    current: &mut SourceInfo,
    progress: Progress<'_>,
    resource_bytes: &AtomicU64,
) -> Result<(), String> {
    let session = &connector.session;
    let vault = &connector.vault;
    let request = |path: &'static str, form: Option<Vec<(&'static str, String)>>| async move {
        session
            .request(http, vault, store, path, form, PAGE_LIMIT, cancel)
            .await
    };
    (progress.report)("사이버캠퍼스에 로그인하고 있습니다.");
    let terms = parser::terms(&text(
        &request("/ilos/main/rg/regular_register_list_form.acl", None).await?,
    )?);
    // The newest term with courses; a term opened early for registration can
    // be listed before it has any.
    let mut selected = None;
    for term in terms.iter().take(3) {
        let page = request(
            "/ilos/main/rg/regular_register_list.acl",
            Some(vec![
                ("YEAR", term.year.clone()),
                ("TERM", term.term.clone()),
                ("SCH_VALUE", String::new()),
            ]),
        )
        .await?;
        let courses = parser::courses(&text(&page)?);
        if !courses.is_empty() {
            selected = Some((term.clone(), courses));
            break;
        }
    }
    let Some((term, courses)) = selected else {
        return Err("이번 학기 사이버캠퍼스 강좌를 찾지 못했습니다.".into());
    };
    let year: i32 = term
        .year
        .parse()
        .map_err(|_| "학기 정보를 확인하지 못했습니다.")?;
    with_store(store, |s| {
        sync::ensure_active(cancel)?;
        s.cybercampus_set_courses(
            year,
            &term.term,
            &courses
                .iter()
                .map(|course| CourseInput {
                    key: course.key.clone(),
                    name: course.name.clone(),
                })
                .collect::<Vec<_>>(),
        )
    })?;
    // The first sync archives what is already there as history, so it does not
    // announce every existing notice as new.
    let historical = !with_store(store, |s| s.seed_complete(SOURCE_ID))?;
    let mut failures = Vec::new();
    let mut tasks: HashMap<String, Vec<TaskInput>> = HashMap::new();
    for (index, course) in courses.iter().enumerate() {
        sync::active(cancel)?;
        (progress.report)(&format!(
            "{} 강좌를 확인하고 있습니다. ({}/{})",
            course.name,
            index + 1,
            courses.len()
        ));
        let outcome = async {
            parser::entered(&text(
                &request(
                    "/ilos/cls/st/co/eclass_room2.acl",
                    Some(vec![
                        ("KJKEY", course.key.clone()),
                        ("returnURI", "/ilos/cls/st/submain/submain_form.acl".into()),
                    ]),
                )
                .await?,
            )?)?;
            // The deadlines do not depend on the notices: a notice step that
            // failed still lets them refresh, and its error is the one reported.
            let noticed = notices(
                connector,
                http,
                store,
                root,
                cancel,
                current,
                course,
                year,
                historical,
                resource_bytes,
            )
            .await;
            if cancel.load(Ordering::Acquire) {
                return noticed;
            }
            let assigned = assignments(connector, http, store, cancel, course).await;
            match (noticed, assigned) {
                (noticed, Ok(assigned)) => {
                    tasks.insert(course.key.clone(), assigned);
                    noticed
                }
                (Err(error), Err(_)) => Err(error),
                (Ok(()), Err(error)) => Err(error),
            }
        }
        .await;
        if let Err(error) = outcome {
            if cancel.load(Ordering::Acquire) {
                return Err(error);
            }
            failures.push(format!("{}: {error}", course.name));
        }
    }
    sync::active(cancel)?;
    // The TODO list covers every course in one request and adds the other
    // deadline kinds (quizzes, lectures) that are not assignments.
    let keys = courses
        .iter()
        .map(|course| course.key.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let todo = request(
        "/ilos/mp/todo_list.acl",
        Some(vec![
            ("todoKjList", keys),
            ("chk_cate", "ALL".into()),
            ("start", "0".into()),
            ("display", "50".into()),
        ]),
    )
    .await
    .and_then(|page| text(&page))
    .map(|page| parser::todo(&page));
    // Whether `tasks` holds everything the TODO list would have said: a failed
    // request says nothing, and a full page may have more rows behind it.
    let mut todo_complete = false;
    match todo {
        Ok(items) => {
            todo_complete = items.len() < TODO_DISPLAY;
            for item in items {
                let Some(course_tasks) = tasks.get_mut(&item.course_key) else {
                    continue;
                };
                if let Some(task) = course_tasks
                    .iter_mut()
                    .find(|task| task.category == item.category && task.item_id == item.item_id)
                {
                    task.pending = true;
                    continue;
                }
                course_tasks.push(TaskInput {
                    category: item.category,
                    item_id: item.item_id,
                    label: item.label,
                    title: item.title,
                    due_at: parser::month_day(&item.due, year),
                    pending: true,
                    ..TaskInput::default()
                });
            }
        }
        Err(error) if cancel.load(Ordering::Acquire) => return Err(error),
        Err(error) => failures.push(format!("할 일 목록: {error}")),
    }
    for (course_key, course_tasks) in &mut tasks {
        with_store(store, |s| {
            sync::ensure_active(cancel)?;
            if !todo_complete {
                keep_todo_rows(&s.cybercampus_stored_tasks(course_key)?, course_tasks);
            }
            s.cybercampus_replace_tasks(course_key, course_tasks)
        })?;
    }
    if historical && failures.is_empty() {
        with_store(store, |s| s.mark_seed_complete(SOURCE_ID))?;
    }
    current.history_complete = true;
    current.next_page = 1;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join(" "))
    }
}

#[allow(clippy::too_many_arguments)]
async fn notices<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    current: &mut SourceInfo,
    course: &parser::Course,
    year: i32,
    historical: bool,
    resource_bytes: &AtomicU64,
) -> Result<(), String> {
    let request = |path: &'static str, form: Vec<(&'static str, String)>| {
        connector.session.request(
            http,
            &connector.vault,
            store,
            path,
            Some(form),
            PAGE_LIMIT,
            cancel,
        )
    };
    let mut rows = Vec::new();
    for page in 0..NOTICE_PAGES {
        // `start` is the 1-based row offset, not a page number.
        let list = parser::notice_list(&text(
            &request(
                "/ilos/cls/st/notice/notice_list.acl",
                vec![
                    ("start", (page * NOTICE_DISPLAY + 1).to_string()),
                    ("display", NOTICE_DISPLAY.to_string()),
                    ("SCH_VALUE", String::new()),
                    ("ODR", String::new()),
                ],
            )
            .await?,
        )?)?;
        let received = list.rows.len();
        rows.extend(list.rows);
        if received == 0 || rows.len() as u32 >= list.total {
            break;
        }
    }
    let mut failure = None;
    for row in rows {
        sync::active(cancel)?;
        let external_id = format!("{}-{}", course.key, row.seq);
        let marker = fingerprint(&row);
        let existing = with_store(store, |s| s.document_id(SOURCE_ID, &external_id))?;
        // Fetching would count as another eClass view of a notice the user deleted.
        if existing.is_none() && with_store(store, |s| s.notice_deleted(SOURCE_ID, &external_id))? {
            continue;
        }
        current.discovered += 1;
        if let Some(id) = existing {
            if with_store(store, |s| {
                s.listing_unchanged(SOURCE_ID, &external_id, &marker)
            })? {
                pending(connector, http, store, root, cancel, id, resource_bytes).await?;
                continue;
            }
        }
        // A notice that could not be read is not fetched again until its row changes.
        if existing.is_none()
            && with_store(store, |s| {
                s.skipped_unchanged(SOURCE_ID, &external_id, &skipped_marker(&marker))
            })?
        {
            continue;
        }
        match notice(
            connector,
            http,
            store,
            cancel,
            course,
            &row,
            &external_id,
            &marker,
            year,
        )
        .await
        {
            Ok(Fetched::Article(article)) => {
                let (id, _) = with_store(store, |s| {
                    sync::ensure_active(cancel)?;
                    s.upsert_article(SOURCE_ID, &article, historical)
                })?;
                with_store(store, |s| s.clear_skipped(SOURCE_ID, &external_id))?;
                current.archived += 1;
                pending(connector, http, store, root, cancel, id, resource_bytes).await?;
            }
            Ok(Fetched::Unusable(reason)) => {
                // Neither body nor attachments, or a page that never parses:
                // listed as skipped instead of silently dropped, and not a
                // failure of the course.
                let entry = ListingEntry {
                    external_id: external_id.clone(),
                    url: canonical_url(),
                    pinned: false,
                    modified_at: None,
                    title: None,
                };
                let title = row.title.chars().take(80).collect::<String>();
                with_store(store, |s| {
                    s.record_skipped(
                        SOURCE_ID,
                        &entry,
                        historical,
                        &format!("{title}: {reason}"),
                        &skipped_marker(&marker),
                    )
                })?;
            }
            Err(error) => {
                if cancel.load(Ordering::Acquire) {
                    return Err(error);
                }
                current.failed += 1;
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

/// What a notice's pages came to.
enum Fetched {
    Article(Box<ArticleInput>),
    /// Fetched fine but nothing to archive; the reason is shown to the user.
    Unusable(String),
}
/// `skipped_articles` keeps a 64-character hash of the list fingerprint.
fn skipped_marker(marker: &str) -> String {
    format!("{:x}", Sha256::digest(marker))
}

#[allow(clippy::too_many_arguments)]
async fn notice<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    cancel: &AtomicBool,
    course: &parser::Course,
    row: &parser::NoticeRow,
    external_id: &str,
    marker: &str,
    year: i32,
) -> Result<Fetched, String> {
    let page = connector
        .session
        .request(
            http,
            &connector.vault,
            store,
            "/ilos/cls/st/notice/notice_view_pop.acl",
            Some(vec![("ARTL_NUM", row.seq.clone())]),
            PAGE_LIMIT,
            cancel,
        )
        .await?;
    let body = text(&page)?;
    // Transport and session errors above fail the course. A page that is
    // fetched but cannot be read comes out the same on every sync, so it
    // is reported as unusable instead.
    let view = match parser::notice_view(&body) {
        Ok(view) => view,
        Err(error) => return Ok(Fetched::Unusable(error.to_string())),
    };
    let mut original = body;
    let mut resources = Vec::new();
    if let Some(files) = &view.files {
        let fragment = text(
            &connector
                .session
                .request(
                    http,
                    &connector.vault,
                    store,
                    if files.path == "/ilos/co/efile_list.acl" {
                        "/ilos/co/efile_list.acl"
                    } else {
                        "/ilos/cls/st/co/efile_list.acl"
                    },
                    Some(
                        files
                            .fields
                            .iter()
                            .map(|(name, value)| (*name, value.clone()))
                            .collect(),
                    ),
                    PAGE_LIMIT,
                    cancel,
                )
                .await?,
        )?;
        resources = parser::attachments(&fragment);
        // The attachment list is part of what was fetched for this notice.
        original.push_str("\n<!-- efile_list.acl -->\n");
        original.push_str(&fragment);
    }
    let (body_html, body_text, inline) =
        match super::parser::sanitize(&view.body_html, &format!("{}/ilos/", parser::ORIGIN)) {
            Ok(parts) => parts,
            Err(error) => return Ok(Fetched::Unusable(error)),
        };
    for image in inline {
        if !resources.iter().any(|resource| resource.url == image.url) {
            resources.push(image);
        }
    }
    if body_text.trim().is_empty() && body_html.trim().is_empty() && resources.is_empty() {
        return Ok(Fetched::Unusable(
            "본문과 첨부 파일이 없는 공지입니다.".into(),
        ));
    }
    let Some(published_at) =
        parser::month_day(&view.posted, year).and_then(|value| value.get(..10).map(str::to_owned))
    else {
        return Ok(Fetched::Unusable(
            "공지 게시일을 확인하지 못했습니다.".into(),
        ));
    };
    Ok(Fetched::Article(Box::new(ArticleInput {
        external_id: external_id.into(),
        canonical_url: canonical_url(),
        title: if view.title.is_empty() {
            row.title.clone()
        } else {
            view.title
        },
        author: view.author,
        category: Some(course.name.clone()),
        published_at,
        modified_at: Some(marker.into()),
        original,
        unsupported_body: false,
        body_html,
        body_text,
        resources,
        etag: None,
        last_modified: None,
    })))
}

/// Assignments of the entered course. A closed assignment already stored is
/// not fetched again; open and new ones are, because only the detail page
/// tells the deadline and whether it was submitted.
async fn assignments<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    cancel: &AtomicBool,
    course: &parser::Course,
) -> Result<Vec<TaskInput>, String> {
    let list = connector
        .session
        .request(
            http,
            &connector.vault,
            store,
            "/ilos/cls/st/activity/activity_list.acl",
            Some(vec![
                ("type", "report".into()),
                ("start", "1".into()),
                ("display", "50".into()),
            ]),
            PAGE_LIMIT,
            cancel,
        )
        .await?;
    let rows = parser::assignments(&text(&list)?);
    let stored = with_store(store, |s| s.cybercampus_stored_tasks(&course.key))?;
    let mut result = Vec::new();
    for row in rows {
        sync::active(cancel)?;
        let known = stored.get(&("report".to_owned(), row.seq.clone()));
        if let Some(known) = known.filter(|known| !row.open && known.due_at.is_some()) {
            result.push(TaskInput {
                title: row.title,
                week: row.week,
                pending: false,
                ..known.clone()
            });
            continue;
        }
        let page = connector
            .session
            .request(
                http,
                &connector.vault,
                store,
                &format!(
                    "/ilos/cls/st/report/report_view_form.acl?RT_SEQ={}",
                    row.seq
                ),
                None,
                PAGE_LIMIT,
                cancel,
            )
            .await?;
        let detail = parser::assignment(&text(&page)?)?;
        result.push(TaskInput {
            category: "report".into(),
            item_id: row.seq,
            label: "과제".into(),
            title: row.title,
            week: row.week,
            opens_at: detail.opens_at,
            due_at: detail.due_at,
            late_due_at: detail.late_due_at,
            status: detail.status,
            submitted_at: detail.submitted_at,
            pending: false,
        });
    }
    Ok(result)
}

/// Downloads a document's pending files with the session.
async fn pending<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    document_id: i64,
    total: &AtomicU64,
) -> Result<(), String> {
    for resource in with_store(store, |s| s.pending_resources(document_id))? {
        download(connector, http, store, root, cancel, &resource, total).await?;
    }
    Ok(())
}
/// Downloads one resource. A file failure is recorded on the resource and is
/// not a sync failure; cancellation and storage errors still propagate.
pub async fn download<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    resource: &ResourceRecord,
    total: &AtomicU64,
) -> Result<DownloadOutcome, String> {
    sync::active(cancel)?;
    let mut budget_bound = false;
    let outcome = async {
        let url = Url::parse(&resource.url).map_err(|_| "첨부 자료 주소를 해석하지 못했습니다.")?;
        // Images on other hosts are kept as references only.
        client::validate(&url).map_err(|_| {
            "사이버캠퍼스 밖의 자료는 자동으로 내려받지 않아 지원하지 않습니다.".to_string()
        })?;
        let limit =
            sync::resource_limit(resource, total.load(Ordering::Acquire), &mut budget_bound)?;
        let path = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let response = connector
            .session
            .request(http, &connector.vault, store, &path, None, limit, cancel)
            .await?;
        sync::install_resource(
            store,
            root,
            cancel,
            resource,
            response.bytes,
            response.mime,
            limit,
            total,
        )
        .await
    }
    .await;
    match outcome {
        Ok(()) => Ok(DownloadOutcome::Downloaded),
        Err(error) if error == client::CANCELLED => Err(error),
        Err(error) => {
            sync::record_failure(store, root, cancel, resource, error, budget_bound).await
        }
    }
}
/// The `resources` mode for this source: missing files of archived notices.
pub async fn download_missing<H: Http>(
    connector: &Connector,
    http: &H,
    store: &SharedStore,
    root: &Path,
    cancel: &AtomicBool,
    total: &AtomicU64,
) -> Result<(), String> {
    for id in with_store(store, |s| s.documents_missing_resources(SOURCE_ID))? {
        sync::active(cancel)?;
        pending(connector, http, store, root, cancel, id, total).await?;
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    /// The saved login's ID; the password is never returned.
    pub id: Option<String>,
    /// eClass rejected the saved login; automatic syncs stop until it is saved again.
    pub rejected: bool,
    pub courses: Vec<crate::storage::cybercampus::CybercampusCourse>,
}
impl Status {
    /// The saved login's ID, the session's rejection flag and the synced courses.
    pub fn read(connector: &Connector, store: &SharedStore) -> Result<Self, String> {
        Ok(Self {
            id: connector.vault.load()?.map(|credentials| credentials.id),
            rejected: with_store(store, |s| s.cybercampus_login_stopped())?,
            courses: with_store(store, |s| s.cybercampus_courses())?,
        })
    }
}
