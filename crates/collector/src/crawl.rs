//! The public-board crawl: listing discovery with the incremental frontier and
//! history checkpoints, and the per-notice commit. Ported from the app's
//! `archive/sync/{listing,articles}.rs` without its resource downloads, PDF
//! bodies, Cybercampus and job controller: the collector publishes resource
//! references only, and the app fetches files when a notice is opened.
use crate::state::{Board, State};
use rhythm_sources::model::{ListingEntry, SourceDefinition};
use rhythm_sources::{
    catalog, parser,
    robots::Robots,
    transport::{self as network, Access, Fetch, Transport},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use url::Url;

const ARTICLE_LIMIT: usize = 4 * 1024 * 1024;

/// Limits for one bounded run; history continues from the checkpoint next run.
#[derive(Clone, Copy, Default)]
pub struct Budget {
    pub pages: Option<u32>,
    pub articles: Option<u32>,
}

/// The boards the collector crawls: every supported public board. Cybercampus
/// is personal data and stays in the app (#166).
pub fn public_boards() -> Vec<SourceDefinition> {
    catalog::definitions()
        .into_iter()
        .filter(|source| source.supported && source.family != catalog::CYBERCAMPUS_FAMILY)
        .collect()
}

/// Crawls the boards with at most `network::MAX_HOSTS` hosts at once and one
/// board per host, so each host keeps its own pacing, as in the app's
/// Controller. Each worker takes every board of one host in turn, the host
/// with the most boards first. Returns each board's outcome in input order;
/// after cancellation, boards not yet started have none.
pub async fn run_boards<T: Transport>(
    state: &State,
    transport: &T,
    boards: &[SourceDefinition],
    budget: Budget,
    cancel: &AtomicBool,
) -> Vec<(String, Result<(), String>)> {
    let mut hosts: Vec<Vec<usize>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (index, board) in boards.iter().enumerate() {
        let host = Url::parse(&board.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| board.id.clone());
        match names.iter().position(|name| *name == host) {
            Some(group) => hosts[group].push(index),
            None => {
                names.push(host);
                hosts.push(vec![index]);
            }
        }
    }
    // Popped from the end: the longest groups go first.
    hosts.sort_by_key(Vec::len);
    let queue = Mutex::new(hosts);
    let outcomes = Mutex::new(Vec::new());
    let worker = || async {
        loop {
            let Some(group) = queue.lock().ok().and_then(|mut queue| queue.pop()) else {
                return;
            };
            for index in group {
                if cancel.load(Ordering::Acquire) {
                    return;
                }
                let board = &boards[index];
                let outcome = run_board(state, transport, board, budget, cancel).await;
                if let Ok(mut outcomes) = outcomes.lock() {
                    outcomes.push((index, board.id.clone(), outcome));
                }
            }
        }
    };
    tokio::join!(worker(), worker(), worker());
    const _: () = assert!(network::MAX_HOSTS == 3, "one worker per crawling host");
    let mut outcomes = outcomes.into_inner().unwrap_or_default();
    outcomes.sort_by_key(|(index, ..)| *index);
    outcomes
        .into_iter()
        .map(|(_, id, outcome)| (id, outcome))
        .collect()
}

/// Crawls one board: new notices and recent edits first, then the unfinished
/// history backlog. Each pass persists its own progress, so a failure or
/// cancellation between them keeps what the first pass saved.
pub async fn run_board<T: Transport>(
    state: &State,
    transport: &T,
    definition: &SourceDefinition,
    budget: Budget,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut board = state.board(&definition.id)?;
    // Counters describe this attempt; checkpoints stay cumulative.
    board.status = "running".into();
    board.failed = 0;
    board.discovered = 0;
    board.archived = 0;
    state.progress(&board, "running", None, false)?;
    let crawl = Crawl {
        state,
        transport,
        cancel,
        budget,
    };
    let mut outcome = crawl.pass(definition, &mut board, Pass::Incremental).await;
    if outcome.is_ok() && !board.history_complete {
        outcome = match active(cancel) {
            Ok(()) => crawl.pass(definition, &mut board, Pass::Historical).await,
            Err(error) => Err(error),
        };
    }
    let Err(error) = outcome else {
        return Ok(());
    };
    let status = if cancel.load(Ordering::Acquire) {
        "cancelled"
    } else {
        "failed"
    };
    state.progress(&board, status, Some(&error), false)?;
    Err(error)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Incremental,
    Historical,
}

struct Crawl<'a, T> {
    state: &'a State,
    transport: &'a T,
    cancel: &'a AtomicBool,
    budget: Budget,
}

impl<T: Transport> Crawl<'_, T> {
    async fn pass(
        &self,
        definition: &SourceDefinition,
        board: &mut Board,
        pass: Pass,
    ) -> Result<(), String> {
        let state = self.state;
        let historical = pass == Pass::Historical;
        // The durable flag, not the current notice count, decides whether the
        // initial bootstrap is still owed. A cancelled or failed seed leaves it
        // unset, so later incremental runs keep marking historical pages as
        // historical instead of reporting them as newly published notices.
        let seed = !historical && !state.seed_complete(&definition.id)?;
        if historical && board.history_complete {
            state.progress(board, "completed", None, true)?;
            return Ok(());
        }
        let first_page = if historical { board.next_page } else { 1 };
        let initial = parser::list_url(definition, first_page)?;
        let policy = Robots::load(self.transport, &initial, self.cancel, Access::Source).await?;
        let mut seen = HashSet::new();
        let mut page_hashes = HashSet::new();
        let mut page = first_page;
        let mut processed = 0;
        loop {
            active(self.cancel)?;
            if page > 100_000 {
                state.progress(
                    board,
                    "limited",
                    Some(
                        "안전한 페이지 수 제한에 도달했습니다. 전체 이력 완료로 표시하지 않습니다.",
                    ),
                    false,
                )?;
                return Ok(());
            }
            let url = parser::list_url(definition, page)?;
            let response = fetch(
                self.transport,
                &url,
                None,
                ARTICLE_LIMIT,
                self.cancel,
                &policy,
            )
            .await?;
            let listing = match response {
                Fetch::Body { bytes, mime, .. } => {
                    check_document_mime(definition, mime.as_deref(), true)?;
                    parser::listing(
                        definition,
                        page,
                        std::str::from_utf8(&bytes)
                            .map_err(|_| "공지 목록 문자 인코딩을 지원하지 않습니다.")?,
                    )?
                }
                Fetch::NotModified | Fetch::Redirect(_) => {
                    return Err(
                        "보관되지 않은 목록의 변경 없음 또는 이동 응답을 받았습니다.".into(),
                    )
                }
            };
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
                active(self.cancel)?;
                let existing_id = state.notice_id(&definition.id, &entry.external_id)?;
                let exists = existing_id.is_some();
                if !entry.pinned {
                    known_regular_frontier = true;
                    all_regular_known &= exists;
                }
                if !seen.insert(entry.external_id.clone()) || (historical && exists) {
                    continue;
                }
                // A sitemap-backed listing carries the site's own change time.
                // Skip the request while the stored notice already matches it.
                if let (true, Some(modified_at)) = (exists, entry.modified_at.as_deref()) {
                    if state.listing_unchanged(&definition.id, &entry.external_id, modified_at)? {
                        continue;
                    }
                }
                if self.budget.articles.is_some_and(|max| processed >= max) {
                    if !errors.is_empty() {
                        return Err(page_failure(&errors));
                    }
                    state.progress(board, "limited", Some("실행 제한으로 일부만 가져왔습니다. 전체 이력은 아직 완료되지 않았습니다."), false)?;
                    return Ok(());
                }
                if !exists {
                    board.discovered += 1;
                }
                let result = self
                    .article(definition, &entry, existing_id, historical || seed, &policy)
                    .await;
                processed += 1;
                match result {
                    Ok(inserted) => board.archived += u64::from(inserted),
                    Err(ArticleError::Permanent {
                        reason,
                        response_sha256,
                    }) => {
                        // Retrying the same response cannot succeed, so this notice
                        // must not hold the page checkpoint; it stays listed instead.
                        ensure_active(self.cancel)?;
                        state.record_skipped(
                            &definition.id,
                            &entry,
                            historical || seed,
                            &reason,
                            &response_sha256,
                        )?;
                    }
                    Err(ArticleError::Transient(error)) => {
                        active(self.cancel)?;
                        board.failed += 1;
                        // The server asked to slow down; the rest of the page
                        // would only repeat the refusal.
                        if error == network::RATE_LIMITED {
                            return Err(error);
                        }
                        errors.push(error);
                    }
                }
                state.progress(board, "running", None, false)?;
            }
            // Never checkpoint beyond a page containing transient article failures.
            if !errors.is_empty() {
                return Err(page_failure(&errors));
            }
            if historical {
                board.next_page = page + 1;
            }
            if !listing.has_next {
                if historical {
                    board.history_complete = true;
                }
                // Reaching the last page safely also established the initial
                // frontier, whether through the bounded seed or the history import.
                if historical || seed {
                    state.mark_seed_complete(&definition.id)?;
                }
                state.progress(board, "completed", None, true)?;
                return Ok(());
            }
            // The three-page edit window must not truncate discovery of a larger
            // burst. Only regular records (never repeated pins) establish a frontier.
            if !historical && page >= 3 && (seed || (known_regular_frontier && all_regular_known)) {
                if seed {
                    state.mark_seed_complete(&definition.id)?;
                }
                state.progress(board, "completed", None, true)?;
                return Ok(());
            }
            if self
                .budget
                .pages
                .is_some_and(|max| page - first_page + 1 >= max)
            {
                state.progress(
                    board,
                    "limited",
                    Some("실행 페이지 제한으로 중단했습니다. 이력은 이어서 가져올 수 있습니다."),
                    false,
                )?;
                return Ok(());
            }
            state.progress(board, "running", None, false)?;
            page += 1;
        }
    }

    /// Fetches and stores one notice. Returns whether a new notice was created.
    async fn article(
        &self,
        definition: &SourceDefinition,
        entry: &ListingEntry,
        existing_id: Option<i64>,
        historical: bool,
        policy: &Robots,
    ) -> Result<bool, ArticleError> {
        let state = self.state;
        let validators = state.validators(&definition.id, &entry.external_id)?;
        let url = parser::detail_url(definition, entry)?;
        let response = fetch(
            self.transport,
            &url,
            validators,
            ARTICLE_LIMIT,
            self.cancel,
            policy,
        )
        .await?;
        let inserted = match response {
            Fetch::NotModified => {
                existing_id.ok_or("보관되지 않은 원문의 변경 없음 응답을 받았습니다.")?;
                ensure_active(self.cancel)?;
                state.checked(&definition.id, &entry.external_id)?;
                false
            }
            Fetch::Redirect(_) => return Err("학교 서버 이동 응답을 처리하지 못했습니다.".into()),
            Fetch::Body {
                bytes,
                etag,
                last_modified,
                mime,
            } => {
                // A different content type is more likely a block or error page than
                // the notice itself, so it stays a transient failure.
                check_document_mime(definition, mime.as_deref(), false)?;
                let permanent = |reason: &str| ArticleError::Permanent {
                    reason: reason.into(),
                    response_sha256: format!("{:x}", Sha256::digest(&bytes)),
                };
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| permanent("공지 본문 문자 인코딩을 지원하지 않습니다."))?;
                let mut article =
                    parser::article(definition, entry, text).map_err(|e| permanent(&e))?;
                // An unsupported body (main-board iframe) never replaces readable
                // content (approved Sogang exception). The collector never fills a
                // body from a viewer PDF, so there is no newer PDF to wait for.
                let keep = match existing_id {
                    Some(id) if article.unsupported_body => state.has_readable_content(id)?,
                    _ => false,
                };
                ensure_active(self.cancel)?;
                if keep {
                    // The notice was checked, so this is neither a failure nor a skip.
                    state.checked(&definition.id, &entry.external_id)?;
                    false
                } else {
                    article.etag = etag;
                    article.last_modified = last_modified;
                    state.upsert(&definition.id, &article, historical)?;
                    existing_id.is_none()
                }
            }
        };
        state.clear_skipped(&definition.id, &entry.external_id)?;
        Ok(inserted)
    }
}

/// Transient failures hold the page checkpoint so the next run retries them.
/// A permanent failure belongs to the fetched response and is recorded instead.
enum ArticleError {
    Transient(String),
    Permanent {
        reason: String,
        response_sha256: String,
    },
}
impl From<String> for ArticleError {
    fn from(error: String) -> Self {
        Self::Transient(error)
    }
}
impl From<&str> for ArticleError {
    fn from(error: &str) -> Self {
        Self::Transient(error.into())
    }
}

fn page_failure(errors: &[String]) -> String {
    format!(
        "원문 {}건을 보관하지 못했습니다. 같은 페이지에서 이어집니다. {}",
        errors.len(),
        errors[0]
    )
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("동기화를 중단했습니다. 저장된 자료는 유지됩니다.".into())
    } else {
        Ok(())
    }
}
fn ensure_active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("동기화를 중단했습니다.".into());
    }
    Ok(())
}
fn check_document_mime(
    source: &SourceDefinition,
    mime: Option<&str>,
    listing: bool,
) -> Result<(), String> {
    // The mechanical sitemap is XML; article pages must still be HTML so a wrong
    // content type stays a transient block-page reading.
    let expected = if source.id == "sogang-academic" {
        mime == Some("application/json")
    } else if source.family == "mechanical" && listing {
        matches!(
            mime,
            Some("text/html" | "application/xhtml+xml" | "application/xml" | "text/xml")
        )
    } else {
        matches!(mime, Some("text/html" | "application/xhtml+xml"))
    };
    if !expected {
        return Err("학교 서버가 예상한 공지 형식 대신 다른 자료를 반환했습니다.".into());
    }
    Ok(())
}
/// Requests a listing or notice, following at most three same-policy redirects.
async fn fetch<T: Transport>(
    transport: &T,
    url: &Url,
    validators: Option<(Option<String>, Option<String>)>,
    limit: usize,
    cancel: &AtomicBool,
    policy: &Robots,
) -> Result<Fetch, String> {
    let mut target = url.clone();
    for redirects in 0..=3 {
        // Apply robots to every hop before any request, not only the original URL.
        // Origin equality also prevents sending validators to another source host.
        if !policy.permits(&target) {
            return Err("학교의 robots.txt 정책에 따라 수집을 중단했습니다.".into());
        }
        if policy.delay > 0.65 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs_f64(policy.delay)) => {},
                _ = wait_cancel(cancel) => return Err("동기화를 중단했습니다.".into()),
            }
        }
        match transport
            .fetch(&target, validators.clone(), limit, cancel, Access::Source)
            .await?
        {
            Fetch::Redirect(next) => {
                if redirects == 3 {
                    return Err("학교 서버의 이동 경로가 너무 많습니다.".into());
                }
                network::validate(&next, Access::Source)?;
                target = next;
            }
            response => return Ok(response),
        }
    }
    unreachable!()
}
async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests;
