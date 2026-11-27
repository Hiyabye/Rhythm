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
    transport::{self as network, Access, Fetch, FetchError, Transport},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use url::Url;

/// Listing pages; notices use `parser::article_limit`.
const LISTING_LIMIT: usize = 4 * 1024 * 1024;
/// A history turn stopped by the run's deadline: normal, not a failure.
const HISTORY_DEADLINE: &str =
    "이번 실행의 이력 시간을 다 써서 멈췄습니다. 다음 실행에서 이어서 가져옵니다.";
/// Deletion-audit fetches per board and run (user decision).
const AUDIT_PER_RUN: usize = 3;
/// Skipped-notice retries per board and run (user decision).
const RETRIES_PER_RUN: usize = 3;
/// A notice whose fetch fails transiently on this many runs in a row is
/// skipped instead of holding its page (user decision 2026-11-23): from the
/// GitHub runner, a few large pages timed out on every run and stopped their
/// boards for good.
const TRANSIENT_RUNS: i64 = 3;
/// The incremental edit window: known notices are re-fetched for edits only
/// within the first three pages and the first this many regular rows (user
/// decision, #80). Ten-row boards keep three pages; the main site's 50-row
/// pages would otherwise re-fetch about 150 notices per board and run.
const EDIT_ROWS: usize = 30;

/// Limits for one bounded run; history continues from the checkpoint next run.
#[derive(Clone, Copy, Default)]
pub struct Budget {
    /// Listing pages per pass.
    pub pages: Option<u32>,
    /// Notices per pass; in the history phase, per board and turn.
    pub articles: Option<u32>,
    /// How long the history phase may run (`--history-minutes`), measured from
    /// when the incremental phase has finished, so a slow new-notice pass never
    /// eats the window. With none (and no `history_until`), each board gets one
    /// history turn per run.
    pub history_window: Option<Duration>,
    /// When the history phase stops starting work. `run_boards` sets it from
    /// `history_window` once the incremental phase is done; a caller may also
    /// give a fixed instant, which `history_window` replaces.
    pub history_until: Option<Instant>,
}

/// One run's outcomes and timings, for the run log.
pub struct Report {
    /// Each board's outcome in input order; after cancellation, boards not yet
    /// started have none.
    pub outcomes: Vec<(String, Result<(), String>)>,
    pub incremental: Duration,
    pub history: Duration,
    /// Boards that ended the run with history still owed.
    pub history_owed: usize,
    /// Each server group's incremental phase, in the order the groups finished.
    pub groups: Vec<GroupTime>,
}

/// How long one server group's incremental passes took, to find a slow server.
pub struct GroupTime {
    /// The group's host names, in board order.
    pub hosts: Vec<String>,
    pub boards: usize,
    /// When a worker took the group, from the start of the crawl.
    pub started: Duration,
    pub took: Duration,
    /// The group's slowest board and its time.
    pub slowest: (String, Duration),
}

/// The boards the collector crawls: every supported public board. Cybercampus
/// is personal data and stays in the app (#166).
pub fn public_boards() -> Vec<SourceDefinition> {
    catalog::definitions()
        .into_iter()
        .filter(|source| source.supported && source.family != catalog::CYBERCAMPUS_FAMILY)
        .collect()
}

/// Boards that have never finished a first pass start at most this many per
/// run (#80 step 3): a first pass reads three pages of notices, and starting
/// over a hundred boards on one paced server at once would outlast the run,
/// which then uploads nothing and repeats.
pub const NEW_BOARDS_PER_RUN: usize = 20;

/// The boards this run crawls: every board whose first pass is done, plus the
/// first `NEW_BOARDS_PER_RUN` others in catalog order. Returns them and the
/// number left for a later run.
pub fn admitted(
    state: &State,
    boards: Vec<SourceDefinition>,
) -> Result<(Vec<SourceDefinition>, usize), String> {
    let mut admitted = Vec::with_capacity(boards.len());
    let mut started = 0;
    let mut waiting = 0;
    for board in boards {
        if state.seed_complete(&board.id)? {
            admitted.push(board);
        } else if started < NEW_BOARDS_PER_RUN {
            started += 1;
            admitted.push(board);
        } else {
            waiting += 1;
        }
    }
    Ok((admitted, waiting))
}

/// Crawls the boards in two phases (#80, user decisions 2026-11-22). First
/// every board's incremental pass and upkeep, so new notices never wait for a
/// backfill; then the history backlog until `budget.history_until`, boards
/// taking turns of at most `budget.articles` notices, least progress first.
///
/// Boards are grouped by the server their host resolves to: at most
/// `network::MAX_HOSTS` servers at once and one board per server, so the
/// transport's per-server pacing is the only limit a server sees. Each worker
/// takes every board of one server, the server with the most boards first.
pub async fn run_boards<T: Transport>(
    state: &State,
    transport: &T,
    boards: &[SourceDefinition],
    budget: Budget,
    cancel: &AtomicBool,
) -> Report {
    let start = Instant::now();
    let policies = &Policies::default();
    let groups = server_groups(transport, boards).await;
    let outcomes = Mutex::new(Vec::<(usize, Result<(), String>)>::new());
    let record = |index: usize, outcome: Result<(), String>| {
        if let Ok(mut outcomes) = outcomes.lock() {
            match outcomes.iter_mut().find(|(i, _)| *i == index) {
                Some(entry) => entry.1 = outcome,
                None => outcomes.push((index, outcome)),
            }
        }
    };
    let times = Mutex::new(Vec::new());
    in_workers(groups.clone(), |group| async {
        let started = start.elapsed();
        let mut hosts: Vec<String> = Vec::new();
        let mut slowest = (String::new(), Duration::ZERO);
        let size = group.len();
        for index in group {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let host = host(&boards[index]);
            if !hosts.contains(&host) {
                hosts.push(host);
            }
            let board_start = Instant::now();
            record(
                index,
                run_incremental(state, transport, &boards[index], budget, cancel, policies).await,
            );
            if board_start.elapsed() >= slowest.1 {
                slowest = (boards[index].id.clone(), board_start.elapsed());
            }
        }
        if let Ok(mut times) = times.lock() {
            times.push(GroupTime {
                hosts,
                boards: size,
                started,
                took: start.elapsed() - started,
                slowest,
            });
        }
    })
    .await;
    let incremental = start.elapsed();
    let budget = Budget {
        history_until: budget
            .history_window
            .map(|window| Instant::now() + window)
            .or(budget.history_until),
        ..budget
    };
    // History only for boards whose incremental pass succeeded.
    let ready: HashSet<usize> = outcomes
        .lock()
        .map(|outcomes| {
            outcomes
                .iter()
                .filter(|(_, outcome)| outcome.is_ok())
                .map(|(index, _)| *index)
                .collect()
        })
        .unwrap_or_default();
    let mut history_groups = Vec::new();
    for group in groups.into_iter().rev() {
        let mut turns = Vec::new();
        for index in group.into_iter().filter(|i| ready.contains(i)) {
            match state.board(&boards[index].id) {
                Ok(board) if !board.history_complete => turns.push(Turn {
                    page: board.next_page,
                    id: boards[index].id.clone(),
                    index,
                }),
                Ok(_) => {}
                Err(error) => record(index, Err(error)),
            }
        }
        if !turns.is_empty() {
            history_groups.push(turns);
        }
    }
    take_turns(
        history_groups,
        || stopped(cancel, budget),
        |index| async move {
            match run_history(state, transport, &boards[index], budget, cancel, policies).await {
                // Without a deadline each board gets one turn, as before.
                Ok(page) => page.filter(|_| budget.history_until.is_some()),
                Err(error) => {
                    record(index, Err(error));
                    None
                }
            }
        },
    )
    .await;
    let mut outcomes = outcomes.into_inner().unwrap_or_default();
    outcomes.sort_by_key(|(index, _)| *index);
    let history_owed = outcomes
        .iter()
        .filter(|(index, outcome)| {
            outcome.is_ok()
                && state
                    .board(&boards[*index].id)
                    .is_ok_and(|board| !board.history_complete)
        })
        .count();
    Report {
        outcomes: outcomes
            .into_iter()
            .map(|(index, outcome)| (boards[index].id.clone(), outcome))
            .collect(),
        incremental,
        history: start.elapsed() - incremental,
        history_owed,
        groups: times.into_inner().unwrap_or_default(),
    }
}

/// Board indices grouped by server, largest group last (workers pop from the
/// end). A host the transport cannot place keeps a group of its own, keyed
/// by its name.
async fn server_groups<T: Transport>(
    transport: &T,
    boards: &[SourceDefinition],
) -> Vec<Vec<usize>> {
    #[derive(Clone, PartialEq)]
    enum Key {
        Server(IpAddr),
        Host(String),
    }
    let mut hosts: Vec<(String, Key)> = Vec::new();
    let mut groups: Vec<(Key, Vec<usize>)> = Vec::new();
    for (index, board) in boards.iter().enumerate() {
        let host = host(board);
        let key = match hosts.iter().find(|(name, _)| *name == host) {
            Some((_, key)) => key.clone(),
            None => {
                let key = match transport.server(&host).await {
                    Some(address) => Key::Server(address),
                    None => Key::Host(host.clone()),
                };
                hosts.push((host, key.clone()));
                key
            }
        };
        match groups.iter_mut().find(|(server, _)| *server == key) {
            Some((_, group)) => group.push(index),
            None => groups.push((key, vec![index])),
        }
    }
    let mut groups: Vec<Vec<usize>> = groups.into_iter().map(|(_, group)| group).collect();
    groups.sort_by_key(Vec::len);
    groups
}

/// A board's host name, or its ID when its URL has none.
fn host(board: &SourceDefinition) -> String {
    Url::parse(&board.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| board.id.clone())
}

/// Runs `work` on each group with `network::MAX_HOSTS` workers, taking groups
/// from the end of the list.
async fn in_workers<F, Fut>(groups: Vec<Vec<usize>>, work: F)
where
    F: Fn(Vec<usize>) -> Fut,
    Fut: Future<Output = ()>,
{
    let queue = Mutex::new(groups);
    let worker = || async {
        while let Some(group) = queue.lock().ok().and_then(|mut queue| queue.pop()) {
            work(group).await;
        }
    };
    tokio::join!(worker(), worker(), worker());
    const _: () = assert!(network::MAX_HOSTS == 3, "one worker per crawling server");
}

/// A board waiting for a history turn: its next history page, for ordering.
struct Turn {
    page: u32,
    id: String,
    index: usize,
}

/// The history phase's scheduler. Each server group, in the given order, gets
/// one round (one turn per board, least progress first, then source ID) and
/// goes back to the end of the queue while `turn` still returns a next page,
/// so every server gets turns before `stop` and one server never holds a
/// worker for the whole window. One group is with at most one worker, so a
/// server still sees one board at a time.
async fn take_turns<S, F, Fut>(groups: Vec<Vec<Turn>>, stop: S, turn: F)
where
    S: Fn() -> bool,
    F: Fn(usize) -> Fut,
    Fut: Future<Output = Option<u32>>,
{
    let queue = Mutex::new(VecDeque::from(groups));
    let worker = || async {
        while !stop() {
            let Some(mut group) = queue.lock().ok().and_then(|mut queue| queue.pop_front()) else {
                return;
            };
            group.sort_by(|a, b| (a.page, &a.id).cmp(&(b.page, &b.id)));
            let mut next = Vec::new();
            for board in group {
                if stop() {
                    return;
                }
                if let Some(page) = turn(board.index).await {
                    next.push(Turn { page, ..board });
                }
            }
            if !next.is_empty() {
                if let Ok(mut queue) = queue.lock() {
                    queue.push_back(next);
                }
            }
        }
    };
    tokio::join!(worker(), worker(), worker());
}

fn stopped(cancel: &AtomicBool, budget: Budget) -> bool {
    cancel.load(Ordering::Acquire) || past(budget.history_until)
}
fn past(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

/// Crawls one board completely: the incremental pass and upkeep, then one
/// history turn.
pub async fn run_board<T: Transport>(
    state: &State,
    transport: &T,
    definition: &SourceDefinition,
    budget: Budget,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let policies = &Policies::default();
    run_incremental(state, transport, definition, budget, cancel, policies).await?;
    run_history(state, transport, definition, budget, cancel, policies)
        .await
        .map(drop)
}

/// The first phase for one board: new notices and recent edits, then the
/// deletion audit and skipped retries.
async fn run_incremental<T: Transport>(
    state: &State,
    transport: &T,
    definition: &SourceDefinition,
    budget: Budget,
    cancel: &AtomicBool,
    policies: &Policies,
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
        policies,
    };
    let mut outcome = crawl.pass(definition, &mut board, Pass::Incremental).await;
    // The mechanical sitemap lists every current notice, so the audit there is
    // the set difference; other boards audit their recent notices.
    if let Ok(listed) = outcome.as_mut() {
        if !parser::sitemap_listing(definition) {
            *listed = None;
        }
    }
    let outcome = match outcome {
        Ok(listed) => match active(cancel) {
            Ok(()) => crawl.upkeep(definition, listed.as_ref()).await,
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    fail(state, &board, outcome, cancel)
}

/// One history turn for one board. Returns the next history page when the
/// turn stopped at a budget after making progress, so another turn can follow.
async fn run_history<T: Transport>(
    state: &State,
    transport: &T,
    definition: &SourceDefinition,
    budget: Budget,
    cancel: &AtomicBool,
    policies: &Policies,
) -> Result<Option<u32>, String> {
    let mut board = state.board(&definition.id)?;
    if board.history_complete {
        return Ok(None);
    }
    let before = (board.next_page, board.discovered);
    if let Err(error) = active(cancel) {
        return fail(state, &board, Err(error), cancel).map(|()| None);
    }
    let crawl = Crawl {
        state,
        transport,
        cancel,
        budget,
        policies,
    };
    let outcome = crawl
        .pass(definition, &mut board, Pass::Historical)
        .await
        .map(drop);
    fail(state, &board, outcome, cancel)?;
    let after = state.board(&definition.id)?;
    let progressed = (after.next_page, after.discovered) != before;
    Ok(
        (!after.history_complete && after.status == "limited" && progressed)
            .then_some(after.next_page),
    )
}

/// Records a failed or cancelled pass on the board and passes the outcome on.
fn fail(
    state: &State,
    board: &Board,
    outcome: Result<(), String>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let Err(error) = outcome else {
        return Ok(());
    };
    let status = if cancel.load(Ordering::Acquire) {
        "cancelled"
    } else {
        "failed"
    };
    state.progress(board, status, Some(&error), false)?;
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
    policies: &'a Policies,
}

/// robots.txt policies by origin, loaded once per run: every board of an
/// origin shares one file, and loading it for each pass cost about 90 s a run
/// on new-cms (118 loads). A failed load is not kept, so the next board on
/// that origin tries again.
#[derive(Default)]
struct Policies(Mutex<HashMap<String, Arc<Robots>>>);

impl Policies {
    async fn get<T: Transport>(
        &self,
        transport: &T,
        url: &Url,
        cancel: &AtomicBool,
    ) -> Result<Arc<Robots>, FetchError> {
        let origin = url.origin().ascii_serialization();
        if let Some(policy) = self.lookup(&origin) {
            return Ok(policy);
        }
        let policy = Arc::new(Robots::load(transport, url, cancel, Access::Source).await?);
        if let Ok(mut policies) = self.0.lock() {
            policies.insert(origin, Arc::clone(&policy));
        }
        Ok(policy)
    }

    fn lookup(&self, origin: &str) -> Option<Arc<Robots>> {
        self.0.lock().ok()?.get(origin).cloned()
    }
}

impl<T: Transport> Crawl<'_, T> {
    /// One pass over the listing. Returns every listed ID when the pass read
    /// the board's complete current listing in one go.
    async fn pass(
        &self,
        definition: &SourceDefinition,
        board: &mut Board,
        pass: Pass,
    ) -> Result<Option<HashSet<String>>, String> {
        let state = self.state;
        let historical = pass == Pass::Historical;
        // The durable flag, not the current notice count, decides whether the
        // initial bootstrap is still owed. A cancelled or failed seed leaves it
        // unset, so later incremental runs keep marking historical pages as
        // historical instead of reporting them as newly published notices.
        let seed = !historical && !state.seed_complete(&definition.id)?;
        if historical && board.history_complete {
            state.progress(board, "completed", None, true)?;
            return Ok(None);
        }
        let mut first_page = if historical { board.next_page } else { 1 };
        let initial = parser::list_url(definition, first_page)?;
        let policy = self
            .policies
            .get(self.transport, &initial, self.cancel)
            .await?;
        let mut seen = HashSet::new();
        let mut page_hashes = HashSet::new();
        // A sitemap listing skips unchanged entries by their own change time.
        let windowed = !historical && !seed && !parser::sitemap_listing(definition);
        let mut regular_rows = 0;
        let mut page = first_page;
        let mut processed = 0;
        loop {
            active(self.cancel)?;
            if historical && past(self.budget.history_until) {
                state.progress(board, "limited", Some(HISTORY_DEADLINE), false)?;
                return Ok(None);
            }
            if page > 100_000 {
                state.progress(
                    board,
                    "limited",
                    Some(
                        "안전한 페이지 수 제한에 도달했습니다. 전체 이력 완료로 표시하지 않습니다.",
                    ),
                    false,
                )?;
                return Ok(None);
            }
            let url = parser::list_url(definition, page)?;
            let response = fetch(
                self.transport,
                &url,
                None,
                LISTING_LIMIT,
                self.cancel,
                &policy,
            )
            .await?;
            let listing = match response {
                Fetch::Body { bytes, mime, .. } => {
                    parser::check_document_mime(definition, mime.as_deref(), true)?;
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|_| "공지 목록 문자 인코딩을 지원하지 않습니다.")?;
                    match parser::listing(definition, page, text) {
                        Ok(listing) => listing,
                        // The saved cursor can lie past the end once the board
                        // shrank, and every parser rejects such a page. Only a
                        // valid page 1 that declares fewer pages proves that;
                        // any other failure stays a failure.
                        Err(error) if historical && page == first_page && first_page > 1 => {
                            let last = self
                                .last_page_below(definition, first_page, &policy)
                                .await
                                .ok_or(error)?;
                            first_page = last;
                            page = last;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
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
            // A page of pins only repeats legitimately (sociology-news: the
            // newest notices are all pinned, so pages 1 and 2 list the same
            // pins); paging stays bounded by the declared total.
            let pins_only = listing.entries.iter().all(|entry| entry.pinned);
            if listing.has_next
                && ((!pins_only && !page_hashes.insert(fingerprint)) || listing.entries.is_empty())
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
                let skipped_by = state.skipped_by(&definition.id, &entry.external_id)?;
                let mut past_window = false;
                if !entry.pinned {
                    known_regular_frontier = true;
                    // A skipped notice was fetched already. Counting it as
                    // unknown would page an incremental pass through every
                    // older page and fetch unimported history as new notices.
                    all_regular_known &= exists || skipped_by.is_some();
                    past_window = windowed && regular_rows >= EDIT_ROWS;
                    regular_rows += 1;
                }
                if !seen.insert(entry.external_id.clone())
                    || (exists && (historical || past_window))
                {
                    continue;
                }
                // A skipped notice is retried by `upkeep` once a day, not on
                // every pass that lists it. The history pass also retries one an
                // older collector version skipped, so a parser fix (with a
                // `VERSION` bump) reaches the skipped notices at or past the
                // history checkpoint within its budget instead of a few a day.
                if skipped_by
                    .as_deref()
                    .is_some_and(|version| !historical || version == crate::VERSION)
                {
                    continue;
                }
                // A sitemap-backed listing carries the site's own change time.
                // Skip the request while the stored notice already matches it.
                if let (true, Some(modified_at)) = (exists, entry.modified_at.as_deref()) {
                    if state.listing_unchanged(&definition.id, &entry.external_id, modified_at)? {
                        continue;
                    }
                }
                // A listing without change times (k2web) shows each row's title:
                // a known notice is re-fetched only when that title changed, and
                // body-only edits arrive through the deletion audit (user
                // decision, #80 step 3). On one shared server, re-reading the edit
                // window of every board would outlast the run.
                if let (true, Some(title)) = (exists, entry.title.as_deref()) {
                    if state.listed_title_unchanged(&definition.id, &entry.external_id, title)? {
                        continue;
                    }
                }
                let deadline = historical && past(self.budget.history_until);
                if deadline || self.budget.articles.is_some_and(|max| processed >= max) {
                    if !errors.is_empty() {
                        return Err(page_failure(&errors));
                    }
                    let reason = if deadline {
                        HISTORY_DEADLINE
                    } else {
                        "실행 제한으로 일부만 가져왔습니다. 전체 이력은 아직 완료되지 않았습니다."
                    };
                    state.progress(board, "limited", Some(reason), false)?;
                    return Ok(None);
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
                    // Gone between listing and fetch: a known notice is reported
                    // missing; neither case is a failure or a skip.
                    Err(ArticleError::Missing) => {
                        ensure_active(self.cancel)?;
                        if exists {
                            state.mark_missing(&definition.id, &entry.external_id)?;
                        }
                        // Ends a run of transient failures (passes never reach
                        // a skipped notice).
                        state.clear_skipped(&definition.id, &entry.external_id)?;
                    }
                    Err(ArticleError::Transient(error)) => {
                        active(self.cancel)?;
                        // The server asked to slow down; the rest of the page
                        // would only repeat the refusal.
                        if error == network::RATE_LIMITED {
                            board.failed += 1;
                            return Err(error);
                        }
                        ensure_active(self.cancel)?;
                        if state.record_transient(&definition.id, &entry.external_id)?
                            >= TRANSIENT_RUNS
                        {
                            state.record_skipped(
                                &definition.id,
                                &entry,
                                historical || seed,
                                &skip_reason(
                                    &format!(
                                        "{TRANSIENT_RUNS}번의 실행에서 연이어 내려받지 못했습니다."
                                    ),
                                    &error,
                                ),
                                &crate::sha256_hex(b""),
                            )?;
                        } else {
                            board.failed += 1;
                            errors.push(error);
                        }
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
                return Ok((first_page == 1).then_some(seen));
            }
            // The edit window must not truncate discovery of a larger
            // burst. Only regular records (never repeated pins) establish a frontier.
            let window_read = page >= 3 || (windowed && regular_rows >= EDIT_ROWS);
            if !historical && window_read && (seed || (known_regular_frontier && all_regular_known))
            {
                if seed {
                    state.mark_seed_complete(&definition.id)?;
                }
                state.progress(board, "completed", None, true)?;
                return Ok(None);
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
                return Ok(None);
            }
            state.progress(board, "running", None, false)?;
            page += 1;
        }
    }

    /// Re-reads page 1 of a board whose resumed history cursor `cursor` was
    /// rejected. Returns the board's current last page only when page 1 parses
    /// and declares fewer than `cursor` pages, so the cursor provably lies past
    /// the end. Any fetch or parse failure returns `None` (a transient failure
    /// of the original pass, never "history is complete").
    async fn last_page_below(
        &self,
        definition: &SourceDefinition,
        cursor: u32,
        policy: &Robots,
    ) -> Option<u32> {
        let url = parser::list_url(definition, 1).ok()?;
        let Fetch::Body { bytes, mime, .. } = fetch(
            self.transport,
            &url,
            None,
            LISTING_LIMIT,
            self.cancel,
            policy,
        )
        .await
        .ok()?
        else {
            return None;
        };
        parser::check_document_mime(definition, mime.as_deref(), true).ok()?;
        let listing = parser::listing(definition, 1, std::str::from_utf8(&bytes).ok()?).ok()?;
        (1..cursor)
            .contains(&listing.total_pages)
            .then_some(listing.total_pages)
    }

    /// Deletion audit and skipped retries, after the passes. Neither changes the
    /// board's status, counters or checkpoints; only a rate limit, cancellation
    /// or a state error fails the board.
    async fn upkeep(
        &self,
        definition: &SourceDefinition,
        listed: Option<&HashSet<String>>,
    ) -> Result<(), String> {
        let state = self.state;
        let audits = state.audit_candidates(&definition.id, listed, AUDIT_PER_RUN)?;
        let retries = state.skipped_due(&definition.id, RETRIES_PER_RUN)?;
        if audits.is_empty() && retries.is_empty() {
            return Ok(());
        }
        let initial = parser::list_url(definition, 1)?;
        let policy = self
            .policies
            .get(self.transport, &initial, self.cancel)
            .await?;
        let work = audits.into_iter().map(|entry| (entry, false, false)).chain(
            retries
                .into_iter()
                .map(|(entry, historical)| (entry, historical, true)),
        );
        for (entry, historical, retry) in work {
            active(self.cancel)?;
            let existing_id = state.notice_id(&definition.id, &entry.external_id)?;
            match self
                .article(definition, &entry, existing_id, historical, &policy)
                .await
            {
                Ok(_) => {}
                Err(ArticleError::Missing) => {
                    ensure_active(self.cancel)?;
                    if existing_id.is_some() {
                        state.mark_missing(&definition.id, &entry.external_id)?;
                    }
                    // Nothing is left to archive.
                    state.clear_skipped(&definition.id, &entry.external_id)?;
                }
                Err(ArticleError::Permanent {
                    reason,
                    response_sha256,
                }) => {
                    ensure_active(self.cancel)?;
                    state.record_skipped(
                        &definition.id,
                        &entry,
                        historical,
                        &reason,
                        &response_sha256,
                    )?;
                }
                Err(ArticleError::Transient(error)) => {
                    active(self.cancel)?;
                    if error == network::RATE_LIMITED {
                        return Err(error);
                    }
                    // A failed retry uses one of its attempts and waits a day;
                    // an audit is tried again in a later run.
                    if retry {
                        ensure_active(self.cancel)?;
                        state.record_skipped(
                            &definition.id,
                            &entry,
                            historical,
                            &skip_reason("다시 시도했지만 내려받지 못했습니다.", &error),
                            &crate::sha256_hex(b""),
                        )?;
                    }
                }
            }
        }
        Ok(())
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
        let response = match fetch(
            self.transport,
            &url,
            validators,
            parser::article_limit(definition),
            self.cancel,
            policy,
        )
        .await
        {
            Err(error)
                if error
                    .status
                    .is_some_and(|status| parser::missing(definition, status, "")) =>
            {
                return Err(ArticleError::Missing)
            }
            // Fetching it again returns the same page, so a notice over the
            // limit is skipped (with the bounded retries) instead of holding
            // the board's history (user decision, #80 step 3).
            Err(error) if error.message == network::TOO_LARGE => {
                return Err(ArticleError::Permanent {
                    reason: "공지 원문이 너무 커서 보관하지 않았습니다.".into(),
                    // No response bytes are kept.
                    response_sha256: crate::sha256_hex(b""),
                });
            }
            response => response.map_err(String::from)?,
        };
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
                parser::check_document_mime(definition, mime.as_deref(), false)?;
                let permanent = |reason: &str| ArticleError::Permanent {
                    reason: reason.into(),
                    response_sha256: crate::sha256_hex(&bytes),
                };
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| permanent("공지 본문 문자 인코딩을 지원하지 않습니다."))?;
                if parser::missing(definition, 200, text) {
                    return Err(ArticleError::Missing);
                }
                let mut article =
                    parser::article(definition, entry, text).map_err(|e| permanent(&e))?;
                // An unsupported body (main-board iframe, k2web poster) never replaces readable
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
    /// The source answered that the notice no longer exists (`parser::missing`).
    Missing,
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

/// A skipped reason within the state's 1,024-byte limit.
fn skip_reason(summary: &str, error: &str) -> String {
    let mut reason = format!("{summary} {error}");
    let mut end = reason.len().min(1024);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason.truncate(end);
    reason
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
/// Requests a listing, notice or menu image, following at most three
/// same-policy redirects.
pub(crate) async fn fetch<T: Transport>(
    transport: &T,
    url: &Url,
    validators: Option<(Option<String>, Option<String>)>,
    limit: usize,
    cancel: &AtomicBool,
    policy: &Robots,
) -> Result<Fetch, FetchError> {
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
