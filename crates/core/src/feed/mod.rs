//! Feed sync (docs/FEED.md "Applying the feed", #177): verifies the signed
//! manifest, follows `moved_to`, picks the cursor or snapshot path and applies
//! each listed file in its own store transaction. Runs at launch and every 30
//! minutes from the scheduler loop while `sogang_notices` is on, and on demand
//! (user decision P2).
use crate::job::{Busy, Slot};
use crate::storage::{
    feed::{FeedResult, FeedStatus, Unmirrored},
    with_store, SharedStore, Store,
};
use rhythm_feed::{record, FileRef, Manifest};
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

pub mod transport;
use transport::{Base, FetchError, Transport};

/// The feed signing key (#166 D10), `key_id` `7fdf378f071d58dc`. Rotating it
/// means shipping the new key here before the publisher switches.
const TRUSTED_KEYS: [[u8; 32]; 1] = [[
    0x05, 0x43, 0xce, 0x43, 0x7c, 0xd4, 0xd9, 0x2a, 0x9b, 0x22, 0x9e, 0x7f, 0xbf, 0xbc, 0xbf, 0x55,
    0x6b, 0xdd, 0xbd, 0xf2, 0xf2, 0x15, 0x45, 0xaf, 0xfa, 0xd2, 0xf3, 0x73, 0x63, 0x8f, 0xc2, 0x6d,
]];
/// `moved_to` hops followed in one sync.
const MAX_MOVES: usize = 3;
/// Between successful syncs (P2).
const INTERVAL: Duration = Duration::from_secs(30 * 60);
/// After a failed one, so a launch without a network catches up soon.
const RETRY: Duration = Duration::from_secs(5 * 60);

/// Why a sync stopped; `result` is what the status reports.
#[derive(Debug, PartialEq)]
pub(crate) struct Failure {
    result: FeedResult,
    message: String,
    /// The server asked for at least this pause (429).
    wait: Option<Duration>,
}

impl Failure {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            result: FeedResult::Failed,
            message: message.into(),
            wait: None,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::failed(message)
    }
}

impl From<FetchError> for Failure {
    fn from(error: FetchError) -> Self {
        Self {
            result: match error {
                FetchError::Offline => FeedResult::Offline,
                _ => FeedResult::Failed,
            },
            message: error.message(),
            wait: match error {
                FetchError::RateLimited(wait) => Some(wait),
                _ => None,
            },
        }
    }
}

fn feed_error(error: rhythm_feed::Error) -> Failure {
    match error {
        // Keep the mirror; only a newer app can read the feed.
        rhythm_feed::Error::UnsupportedFormat(_) => Failure {
            result: FeedResult::UpdateRequired,
            message: "공지 피드 형식이 바뀌었습니다. 앱을 업데이트하세요.".into(),
            wait: None,
        },
        rhythm_feed::Error::BadSignature => {
            Failure::failed("공지 피드의 서명을 확인하지 못해 반영하지 않았습니다.")
        }
        rhythm_feed::Error::Invalid(_) => {
            Failure::failed("공지 피드 파일이 올바르지 않아 반영하지 않았습니다.")
        }
    }
}

/// What one sync changed.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Summary {
    pub files: usize,
    pub revised: usize,
    pub removed: usize,
}

/// One file to apply after the manifest is accepted.
enum Step<'a> {
    Snapshot(&'a rhythm_feed::manifest::Snapshot),
    Segment(&'a rhythm_feed::manifest::Segment),
}

/// FEED.md step 4: the files from the cursor, or the snapshot and every
/// segment after it. `true` when the sync starts from the snapshot.
fn plan(manifest: &Manifest, cursor: Option<u64>) -> (Vec<Step<'_>>, bool) {
    let newest = manifest
        .segments
        .last()
        .map_or(manifest.snapshot.through_seq, |s| s.to_seq);
    if cursor == Some(newest) {
        return (Vec::new(), false);
    }
    if let Some(next) = cursor.map(|c| c + 1) {
        if let Some(index) = manifest
            .segments
            .iter()
            .position(|s| s.from_seq <= next && next <= s.to_seq)
        {
            let steps = manifest.segments[index..].iter().map(Step::Segment);
            return (steps.collect(), false);
        }
    }
    let through = manifest.snapshot.through_seq;
    let steps = std::iter::once(Step::Snapshot(&manifest.snapshot)).chain(
        manifest
            .segments
            .iter()
            .filter(|s| s.to_seq > through)
            .map(Step::Segment),
    );
    (steps.collect(), true)
}

async fn download(
    transport: &impl Transport,
    base: &Base,
    file: &FileRef,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, Failure> {
    if file.size > transport::FILE_LIMIT {
        return Err(Failure::failed(
            "공지 피드 파일이 허용된 크기를 넘었습니다.",
        ));
    }
    let bytes = transport
        .get(&base.file(&file.path)?, file.size, cancel)
        .await?;
    file.check(&bytes).map_err(feed_error)?;
    Ok(bytes)
}

/// Fetches and verifies the manifest at `base`, following signed moves.
async fn manifest(
    transport: &impl Transport,
    mut base: Base,
    trusted: &[[u8; 32]],
    cancel: &AtomicBool,
) -> Result<(Base, Manifest), Failure> {
    for _ in 0..=MAX_MOVES {
        let bytes = transport
            .get(
                &base.file(transport::MANIFEST)?,
                transport::MANIFEST_LIMIT,
                cancel,
            )
            .await?;
        let signature = transport
            .get(
                &base.file(transport::SIGNATURE)?,
                transport::SIGNATURE_LIMIT,
                cancel,
            )
            .await?;
        let manifest = rhythm_feed::verify(&bytes, &signature, trusted).map_err(feed_error)?;
        match manifest.moved_to.as_deref().map(Base::parse).transpose()? {
            Some(next) if next != base => base = next,
            _ => return Ok((base, manifest)),
        }
    }
    Err(Failure::failed(
        "공지 피드 주소가 너무 여러 번 바뀌어 따라가지 않았습니다.",
    ))
}

/// Runs one apply under the store lock unless the sync was cancelled since its
/// download: the commit boundary checks `cancel`, so a disable, maintenance or
/// exit that stopped the sync never sees a late write or a moved cursor.
fn apply<T>(
    store: &SharedStore,
    cancel: &AtomicBool,
    operation: impl FnOnce(&Store) -> Result<T, String>,
) -> Result<T, Failure> {
    with_store(store, |s| {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        operation(s).map(Some)
    })?
    .ok_or_else(|| FetchError::Cancelled.into())
}

/// Downloads one step's records and, if listed, vectors.
async fn fetch(
    transport: &impl Transport,
    base: &Base,
    step: &Step<'_>,
    cancel: &AtomicBool,
) -> Result<(Vec<u8>, Vec<u8>), Failure> {
    if cancel.load(Ordering::Acquire) {
        return Err(FetchError::Cancelled.into());
    }
    let (records, vectors) = match step {
        Step::Snapshot(s) => (&s.records, &s.vectors),
        Step::Segment(s) => (&s.records, &s.vectors),
    };
    let records = download(transport, base, records, cancel).await?;
    let vectors = match vectors {
        Some(file) => download(transport, base, file, cancel).await?,
        None => Vec::new(),
    };
    Ok((records, vectors))
}

/// Brings in `boards`, whose records earlier syncs skipped because the
/// catalog did not have them yet (#80), and the record `kinds` an older
/// version skipped (calendar events #319, menu weeks #318): their records from
/// the snapshot path (the snapshot and every segment after it), while the
/// cursor and the other boards stay as they are.
#[allow(clippy::too_many_arguments)]
async fn backfill(
    store: &SharedStore,
    data_root: &Path,
    transport: &impl Transport,
    base: &Base,
    manifest: &Manifest,
    boards: &HashSet<String>,
    kinds: Unmirrored,
    cancel: &AtomicBool,
) -> Result<Summary, Failure> {
    let mut summary = Summary::default();
    for step in plan(manifest, None).0 {
        let (records, vectors) = fetch(transport, base, &step, cancel).await?;
        let (entries, after) = match step {
            Step::Snapshot(s) => (record::decode_snapshot(&records, s.through_seq), 0),
            Step::Segment(s) => (
                record::decode_segment(&records, s.from_seq, s.to_seq),
                manifest.snapshot.through_seq,
            ),
        };
        let entries = entries.map_err(feed_error)?;
        let applied = apply(store, cancel, |s| {
            s.feed_backfill(boards, kinds, after, &entries, &vectors)
        })?;
        for path in &applied.orphaned {
            let _ = tokio::fs::remove_file(data_root.join(path)).await;
        }
        summary.files += 1;
        summary.revised += applied.revised;
        summary.removed += applied.removed;
    }
    Ok(summary)
}

/// One sync. Orphaned resource files are deleted under `data_root` after the
/// store lock is released, as 휴지통's permanent deletion does.
pub(crate) async fn sync(
    store: &SharedStore,
    data_root: &Path,
    transport: &impl Transport,
    trusted: &[[u8; 32]],
    cancel: &AtomicBool,
) -> Result<Summary, Failure> {
    let state = with_store(store, |s| s.feed_state())?;
    // A first sync takes the snapshot path, which covers every board, every
    // calendar event and every menu week.
    let (new_boards, unmirrored) = match state.cursor {
        Some(_) => with_store(store, |s| Ok((s.feed_new_boards()?, s.feed_unmirrored()?)))?,
        None => (HashSet::new(), Unmirrored::default()),
    };
    // A stored base that no longer parses falls back to the app's own.
    let base = state
        .base
        .as_deref()
        .and_then(|base| Base::parse(base).ok())
        .unwrap_or_else(Base::default_base);
    let (base, manifest) = manifest(transport, base, trusted, cancel).await?;
    with_store(store, |s| s.feed_accept(&manifest, base.as_str()))?;
    let (steps, quiet) = plan(&manifest, state.cursor);
    // The cursor path leaves the backfilled boards out: their records up to
    // the newest seq are applied already, and an older revision must not
    // follow a newer one. Backfilled events and menu weeks need no skip: the
    // cursor path replays their records after the cursor in order, which ends
    // at the newest state the backfill reached.
    let (mut summary, skip) = if quiet || (new_boards.is_empty() && !unmirrored.any()) {
        (Summary::default(), HashSet::new())
    } else {
        let summary = backfill(
            store,
            data_root,
            transport,
            &base,
            &manifest,
            &new_boards,
            unmirrored,
            cancel,
        )
        .await?;
        (summary, new_boards)
    };
    for step in steps {
        let (records, vectors) = fetch(transport, &base, &step, cancel).await?;
        let applied = match step {
            Step::Snapshot(snapshot) => {
                let entries =
                    record::decode_snapshot(&records, snapshot.through_seq).map_err(feed_error)?;
                apply(store, cancel, |s| {
                    s.feed_apply_snapshot(snapshot.through_seq, &entries, &vectors)
                })?
            }
            Step::Segment(segment) => {
                let entries = record::decode_segment(&records, segment.from_seq, segment.to_seq)
                    .map_err(feed_error)?;
                apply(store, cancel, |s| {
                    s.feed_apply_segment(
                        segment.from_seq,
                        segment.to_seq,
                        &entries,
                        &vectors,
                        quiet,
                        &skip,
                    )
                })?
            }
        };
        // The rows are gone, so a file left behind is only unused disk space.
        for path in &applied.orphaned {
            let _ = tokio::fs::remove_file(data_root.join(path)).await;
        }
        summary.files += 1;
        summary.revised += applied.revised;
        summary.removed += applied.removed;
    }
    // Only now: a sync that stopped early backfills the same boards (and the
    // events) again.
    with_store(store, |s| s.feed_remember_boards())?;
    Ok(summary)
}

/// Runs one sync at a time and remembers when the next is due.
#[derive(Default)]
pub struct Controller {
    active: Slot,
    /// When the scheduler may start the next sync; `None` at launch.
    next: Mutex<Option<Instant>>,
}

impl Controller {
    pub fn running(&self) -> bool {
        self.active.running()
    }

    pub fn cancel(&self) {
        self.active.cancel();
    }

    pub async fn wait(&self) {
        self.active.wait().await;
    }

    pub fn due(&self) -> bool {
        self.next
            .lock()
            .map(|next| next.is_none_or(|next| Instant::now() >= next))
            .unwrap_or(false)
    }

    pub fn status(&self, store: &SharedStore) -> Result<FeedStatus, String> {
        let mut status = with_store(store, |s| s.feed_status(chrono::Utc::now()))?;
        status.running = self.running();
        Ok(status)
    }

    /// Runs a sync unless one is already running (`Ok(false)`), records how
    /// it ended and schedules the next.
    pub async fn run(
        &self,
        store: &SharedStore,
        data_root: &Path,
        transport: &impl Transport,
    ) -> Result<bool, String> {
        self.run_with(store, data_root, transport, &TRUSTED_KEYS)
            .await
    }

    async fn run_with(
        &self,
        store: &SharedStore,
        data_root: &Path,
        transport: &impl Transport,
        trusted: &[[u8; 32]],
    ) -> Result<bool, String> {
        let running = match self.active.begin() {
            Ok(running) => running,
            Err(Busy::Running) => return Ok(false),
            Err(Busy::Unavailable) => return Err("공지 피드 상태를 확인하지 못했습니다.".into()),
        };
        let cancel = running.token();
        let outcome = sync(store, data_root, transport, trusted, cancel.flag()).await;
        let wait = match &outcome {
            Ok(_) => INTERVAL,
            Err(failure) => failure.wait.unwrap_or_default().max(RETRY),
        };
        if let Ok(mut next) = self.next.lock() {
            *next = Some(Instant::now() + wait);
        }
        match outcome {
            Ok(_) => with_store(store, |s| s.feed_record(FeedResult::Ok, None))?,
            // A cancelled sync (maintenance, exit, the extension turned off)
            // keeps the last result; the next one picks up where it stopped.
            Err(failure) if cancel.is_cancelled() => drop(failure),
            Err(failure) => with_store(store, |s| {
                s.feed_record(failure.result, Some(&failure.message))
            })?,
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
