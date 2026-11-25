//! The sync against an in-memory feed: the store and the feed crate are real,
//! only the network boundary is fake, and the feed is signed with disposable
//! RFC 8032 test keys.
use super::*;
use crate::{archive::model::*, storage::Store};
use rhythm_feed::{
    manifest::{BoardHealth, EmbeddingInfo, Segment, Snapshot},
    record::{BodyState, Notice, Record, Removal},
    test_keys::{other_signer, signer},
    Signer,
};
use std::collections::HashMap;
use url::Url;

#[derive(Default)]
struct Fake {
    files: Mutex<HashMap<String, Result<Vec<u8>, FetchError>>>,
    requests: Mutex<Vec<String>>,
}
impl Fake {
    fn put(&self, url: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.into(), Ok(bytes));
    }
    fn fail(&self, url: &str, error: FetchError) {
        self.files.lock().unwrap().insert(url.into(), Err(error));
    }
    fn take_requests(&self) -> Vec<String> {
        std::mem::take(&mut self.requests.lock().unwrap())
    }
}
impl Transport for Fake {
    async fn get(&self, url: &Url, limit: u64, _: &AtomicBool) -> Result<Vec<u8>, FetchError> {
        self.requests.lock().unwrap().push(url.to_string());
        let bytes = self
            .files
            .lock()
            .unwrap()
            .get(url.as_str())
            .cloned()
            .unwrap_or_else(|| Err(FetchError::Failed("HTTP 404".into())))?;
        if bytes.len() as u64 > limit {
            return Err(FetchError::Failed("too large".into()));
        }
        Ok(bytes)
    }
}

/// A publisher writing to `base` of the fake.
struct Feed<'a> {
    fake: &'a Fake,
    base: String,
}
impl Feed<'_> {
    fn file(&self, path: &str, bytes: Vec<u8>) -> FileRef {
        let file = FileRef::new(path.into(), &bytes);
        self.fake.put(&format!("{}{path}", self.base), bytes);
        file
    }
    fn records(&self, name: &str, entries: &[(u64, Record)]) -> FileRef {
        self.file(name, record::encode(entries).unwrap())
    }
    fn publish(&self, manifest: &Manifest, signer: &Signer) {
        let bytes = serde_json::to_vec(manifest).unwrap();
        self.fake.put(
            &format!("{}manifest.json.sig", self.base),
            signer.sign(&bytes),
        );
        self.fake.put(&format!("{}manifest.json", self.base), bytes);
    }
}

fn notice(id: &str, title: &str) -> Record {
    Record::Notice(Box::new(Notice {
        source_id: "one".into(),
        external_id: id.into(),
        revision_hash: rhythm_feed::manifest::sha256_hex(title.as_bytes()),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        title: title.into(),
        author: None,
        category: None,
        published_at: "2026-11-02".into(),
        modified_at: None,
        body_state: BodyState::Normal,
        excerpt: format!("{title} 본문"),
        body_hash: None,
        resources: vec![],
        dates: vec![],
        dates_version: 1,
        historical: false,
        first_seen_at: "2026-11-02T01:00:00.000Z".into(),
        revised_at: "2026-11-02T01:00:00.000Z".into(),
    }))
}
fn removal(id: &str) -> Record {
    Record::Remove(Removal {
        source_id: "one".into(),
        external_id: id.into(),
        removed_at: "2026-11-03T00:00:00.000Z".into(),
    })
}
fn manifest(generated_at: &str, snapshot: Snapshot, segments: Vec<Segment>) -> Manifest {
    Manifest {
        format: rhythm_feed::FORMAT.into(),
        generated_at: generated_at.into(),
        moved_to: None,
        embedding: EmbeddingInfo {
            model: "model".into(),
            scope: "model#hash".into(),
            dimensions: 1024,
            canary: None,
        },
        calendars: Vec::new(),
        meals: Vec::new(),
        boards: vec![
            BoardHealth {
                source_id: "one".into(),
                last_success_at: Some("2026-11-18T00:00:00.000Z".into()),
                consecutive_failures: 4,
                history_complete: true,
            },
            BoardHealth {
                source_id: "two".into(),
                last_success_at: None,
                consecutive_failures: 9,
                history_complete: false,
            },
        ],
        snapshot,
        segments,
    }
}
fn segment(feed: &Feed, from: u64, to: u64, records: &[(u64, Record)]) -> Segment {
    Segment {
        from_seq: from,
        to_seq: to,
        records: feed.records(&format!("segments/run-{from}-{to}.jsonl.zst"), records),
        vectors: None,
    }
}

fn definition(id: &str) -> SourceDefinition {
    SourceDefinition {
        id: id.into(),
        name: format!("게시판 {id}"),
        organization: "서강대학교".into(),
        description: "공개 공지".into(),
        url: format!("https://www.sogang.ac.kr/{id}"),
        family: "main".into(),
        supported: true,
        group: None,
    }
}
fn store() -> (tempfile::TempDir, SharedStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.sqlite3")).unwrap();
    store
        .catalog(&[definition("one"), definition("two")])
        .unwrap();
    (directory, Arc::new(Mutex::new(store)))
}
fn titles(store: &SharedStore) -> Vec<String> {
    let page = store
        .lock()
        .unwrap()
        .archive_query(&crate::archive::model::ArchiveQuery {
            ascending: true,
            ..Default::default()
        })
        .unwrap();
    let mut titles: Vec<String> = page.items.into_iter().map(|i| i.title).collect();
    titles.sort();
    titles
}
fn status(store: &SharedStore) -> FeedStatus {
    store
        .lock()
        .unwrap()
        .feed_status(chrono::Utc::now())
        .unwrap()
}
const BASE: &str = transport::DEFAULT_BASE;
const SNAPSHOT: &str = "snapshots/2.jsonl.zst";

/// A feed with a snapshot through seq 2 and one segment 3–4 after it.
fn first_feed(fake: &Fake) -> Manifest {
    let feed = Feed {
        fake,
        base: BASE.into(),
    };
    let snapshot = Snapshot {
        through_seq: 2,
        records: feed.records(
            SNAPSHOT,
            &[(1, notice("1", "첫 공지")), (2, notice("2", "둘째 공지"))],
        ),
        vectors: None,
    };
    let segments = vec![segment(
        &feed,
        3,
        4,
        &[(3, notice("3", "셋째 공지")), (4, removal("1"))],
    )];
    let manifest = manifest("2026-11-18T12:00:00.000Z", snapshot, segments);
    feed.publish(&manifest, &signer());
    manifest
}

async fn run(controller: &Controller, store: &SharedStore, root: &Path, fake: &Fake) -> bool {
    controller
        .run_with(store, root, fake, &[signer().public_key()])
        .await
        .unwrap()
}

#[tokio::test]
async fn a_first_sync_applies_the_snapshot_and_later_segments_without_notifying() {
    let (directory, store) = store();
    store.lock().unwrap().subscribe("one", true).unwrap();
    let fake = Fake::default();
    first_feed(&fake);
    let controller = Controller::default();
    assert!(controller.due());
    assert!(run(&controller, &store, directory.path(), &fake).await);
    assert!(!controller.due());

    assert_eq!(titles(&store), ["둘째 공지", "셋째 공지"]);
    let guard = store.lock().unwrap();
    assert_eq!(guard.feed_cursor().unwrap(), Some(4));
    // The segment after the snapshot belongs to the first sync too.
    assert!(guard.proactive_candidates(10).unwrap().is_empty());
    drop(guard);
    let status = status(&store);
    assert_eq!(status.result.as_deref(), Some("ok"));
    assert_eq!(status.error, None);
    assert!(status.checked_at.is_some());
    assert_eq!(
        status.generated_at.as_deref(),
        Some("2026-11-18T12:00:00.000Z")
    );
    // Only the subscribed board, and only at three or more failures.
    assert_eq!(
        status
            .failing_boards
            .iter()
            .map(|b| (b.source_id.as_str(), b.consecutive_failures))
            .collect::<Vec<_>>(),
        [("one", 4)]
    );
}

#[tokio::test]
async fn later_syncs_resume_from_the_cursor_and_skip_an_unchanged_feed() {
    let (directory, store) = store();
    let fake = Fake::default();
    let mut manifest = first_feed(&fake);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    let baseline = store.lock().unwrap().proactive_baseline().unwrap();
    fake.take_requests();

    // Nothing new: the manifest and its signature only.
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(
        fake.take_requests(),
        [
            format!("{BASE}manifest.json"),
            format!("{BASE}manifest.json.sig")
        ]
    );

    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    manifest.generated_at = "2026-11-18T12:30:00.000Z".into();
    manifest
        .segments
        .push(segment(&feed, 5, 5, &[(5, notice("5", "다섯째 공지"))]));
    feed.publish(&manifest, &signer());
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(
        fake.take_requests(),
        [
            format!("{BASE}manifest.json"),
            format!("{BASE}manifest.json.sig"),
            format!("{BASE}segments/run-5-5.jsonl.zst"),
        ]
    );
    assert_eq!(titles(&store), ["다섯째 공지", "둘째 공지", "셋째 공지"]);
    let guard = store.lock().unwrap();
    assert_eq!(guard.feed_cursor().unwrap(), Some(5));
    // A notice after the first sync may notify (#177 PR 6 decides how).
    assert_eq!(guard.proactive_baseline().unwrap(), baseline);
    assert_eq!(
        guard
            .proactive_candidates(10)
            .unwrap()
            .iter()
            .map(|c| c.title.as_str())
            .collect::<Vec<_>>(),
        ["다섯째 공지"]
    );
}

#[tokio::test]
async fn a_cursor_outside_the_listed_segments_takes_the_snapshot_path() {
    let (directory, store) = store();
    let fake = Fake::default();
    let mut manifest = first_feed(&fake);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;

    // A restart (FEED.md): a new snapshot beyond every old seq, no segments.
    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    manifest.generated_at = "2026-11-18T13:00:00.000Z".into();
    manifest.snapshot = Snapshot {
        through_seq: 10,
        records: feed.records("snapshots/10.jsonl.zst", &[(10, notice("3", "셋째 공지"))]),
        vectors: None,
    };
    manifest.segments.clear();
    feed.publish(&manifest, &signer());
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(titles(&store), ["셋째 공지"]);
    assert_eq!(store.lock().unwrap().feed_cursor().unwrap(), Some(10));
}

#[tokio::test]
async fn an_unverified_or_older_manifest_changes_nothing() {
    let (directory, store) = store();
    let fake = Fake::default();
    let mut manifest = first_feed(&fake);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };

    // Another key signed it.
    manifest.generated_at = "2026-11-18T12:30:00.000Z".into();
    manifest
        .segments
        .push(segment(&feed, 5, 5, &[(5, notice("5", "다섯째 공지"))]));
    feed.publish(&manifest, &other_signer());
    run(&controller, &store, directory.path(), &fake).await;
    let failed = status(&store);
    assert_eq!(failed.result.as_deref(), Some("failed"));
    assert!(failed.error.unwrap().contains("서명"));
    assert_eq!(store.lock().unwrap().feed_cursor().unwrap(), Some(4));

    // Correctly signed but older than the accepted one.
    manifest.generated_at = "2026-11-18T11:00:00.000Z".into();
    feed.publish(&manifest, &signer());
    run(&controller, &store, directory.path(), &fake).await;
    assert!(status(&store).error.unwrap().contains("오래되었습니다"));
    assert_eq!(store.lock().unwrap().feed_cursor().unwrap(), Some(4));
    assert_eq!(titles(&store), ["둘째 공지", "셋째 공지"]);
}

#[tokio::test]
async fn a_newer_format_asks_for_an_update_and_keeps_the_mirror() {
    let (directory, store) = store();
    let fake = Fake::default();
    let mut manifest = first_feed(&fake);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    manifest.format = "2.0".into();
    manifest.generated_at = "2026-11-18T12:30:00.000Z".into();
    Feed {
        fake: &fake,
        base: BASE.into(),
    }
    .publish(&manifest, &signer());
    run(&controller, &store, directory.path(), &fake).await;
    let status = status(&store);
    assert_eq!(status.result.as_deref(), Some("update_required"));
    assert!(status.error.unwrap().contains("업데이트"));
    assert_eq!(titles(&store), ["둘째 공지", "셋째 공지"]);
}

#[tokio::test]
async fn a_signed_move_is_followed_and_remembered() {
    let (directory, store) = store();
    let fake = Fake::default();
    let mut manifest = first_feed(&fake);
    let moved = "https://feed.example.org/v2/";
    let old = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    let mut pointer = manifest.clone();
    pointer.moved_to = Some(moved.into());
    old.publish(&pointer, &signer());
    let new = Feed {
        fake: &fake,
        base: moved.into(),
    };
    manifest.snapshot.records = new.records(
        SNAPSHOT,
        &[(1, notice("1", "첫 공지")), (2, notice("2", "둘째 공지"))],
    );
    manifest.segments = vec![segment(
        &new,
        3,
        4,
        &[(3, notice("3", "셋째 공지")), (4, removal("1"))],
    )];
    new.publish(&manifest, &signer());

    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(titles(&store), ["둘째 공지", "셋째 공지"]);
    assert_eq!(
        store.lock().unwrap().feed_state().unwrap().base.as_deref(),
        Some(moved)
    );
    fake.take_requests();
    run(&controller, &store, directory.path(), &fake).await;
    assert!(fake
        .take_requests()
        .iter()
        .all(|url| url.starts_with(moved)));
}

#[tokio::test]
async fn a_file_that_does_not_match_its_manifest_stops_before_it() {
    let (directory, store) = store();
    let fake = Fake::default();
    first_feed(&fake);
    fake.put(
        &format!("{BASE}segments/run-3-4.jsonl.zst"),
        b"tampered".to_vec(),
    );
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    // The snapshot is in; the segment is not.
    assert_eq!(store.lock().unwrap().feed_cursor().unwrap(), Some(2));
    assert_eq!(titles(&store), ["둘째 공지", "첫 공지"]);
    assert_eq!(status(&store).result.as_deref(), Some("failed"));
}

#[tokio::test]
async fn an_offline_feed_is_recorded_quietly_and_retried_soon() {
    let (directory, store) = store();
    let fake = Fake::default();
    fake.fail(&format!("{BASE}manifest.json"), FetchError::Offline);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    let status = status(&store);
    assert_eq!(status.result.as_deref(), Some("offline"));
    assert_eq!(status.checked_at, None);
    let next = controller.next.lock().unwrap().unwrap();
    assert!(next <= Instant::now() + RETRY);

    fake.fail(
        &format!("{BASE}manifest.json"),
        FetchError::RateLimited(Duration::from_secs(20 * 60)),
    );
    run(&controller, &store, directory.path(), &fake).await;
    let next = controller.next.lock().unwrap().unwrap();
    assert!(next > Instant::now() + Duration::from_secs(19 * 60));
}

#[test]
fn the_compiled_key_is_the_production_feed_key() {
    assert_eq!(
        rhythm_feed::manifest::key_id(&TRUSTED_KEYS[0]),
        "7fdf378f071d58dc"
    );
}

fn notice_on(board: &str, id: &str, title: &str) -> Record {
    let mut record = notice(id, title);
    if let Record::Notice(notice) = &mut record {
        notice.source_id = board.into();
    }
    record
}
fn unread_titles(store: &SharedStore) -> Vec<String> {
    let page = store
        .lock()
        .unwrap()
        .archive_query(&crate::archive::model::ArchiveQuery {
            unread_only: true,
            ..Default::default()
        })
        .unwrap();
    let mut titles: Vec<String> = page.items.into_iter().map(|i| i.title).collect();
    titles.sort();
    titles
}

#[tokio::test]
async fn a_board_the_catalog_gains_is_backfilled_from_the_snapshot_path() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.sqlite3")).unwrap();
    store.catalog(&[definition("one")]).unwrap();
    let store: SharedStore = Arc::new(Mutex::new(store));
    let fake = Fake::default();
    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    let snapshot = Snapshot {
        through_seq: 2,
        records: feed.records(
            SNAPSHOT,
            &[
                (1, notice_on("one", "1", "첫 공지")),
                (2, notice_on("two", "a", "새 게시판 원본")),
            ],
        ),
        vectors: None,
    };
    let mut manifest = manifest(
        "2026-11-18T12:00:00.000Z",
        snapshot,
        vec![segment(
            &feed,
            3,
            4,
            &[
                (3, notice_on("two", "a", "새 게시판 수정")),
                (4, notice_on("two", "b", "새 게시판 둘째")),
            ],
        )],
    );
    feed.publish(&manifest, &signer());
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    // Board "two" is not in this catalog yet, so its records are skipped.
    assert_eq!(titles(&store), ["첫 공지"]);
    assert!(store.lock().unwrap().feed_new_boards().unwrap().is_empty());

    // An update adds the board; the next segment brings news on both boards.
    store
        .lock()
        .unwrap()
        .catalog(&[definition("one"), definition("two")])
        .unwrap();
    assert_eq!(
        store.lock().unwrap().feed_new_boards().unwrap(),
        HashSet::from(["two".to_string()])
    );
    manifest.generated_at = "2026-11-18T12:30:00.000Z".into();
    manifest.segments.push(segment(
        &feed,
        5,
        8,
        &[
            (5, notice_on("one", "5", "기존 게시판 새 공지")),
            (6, notice_on("two", "c", "새 게시판 셋째")),
            (7, notice_on("two", "a", "새 게시판 재수정")),
            (8, notice_on("two", "a", "새 게시판 최종")),
        ],
    ));
    feed.publish(&manifest, &signer());
    fake.take_requests();
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(
        fake.take_requests(),
        [
            format!("{BASE}manifest.json"),
            format!("{BASE}manifest.json.sig"),
            format!("{BASE}{SNAPSHOT}"),
            format!("{BASE}segments/run-3-4.jsonl.zst"),
            format!("{BASE}segments/run-5-8.jsonl.zst"),
            format!("{BASE}segments/run-5-8.jsonl.zst"),
        ]
    );
    // The board's latest state, its revised notice once, and nothing older after it.
    assert_eq!(
        titles(&store),
        [
            "기존 게시판 새 공지",
            "새 게시판 둘째",
            "새 게시판 셋째",
            "새 게시판 최종",
            "첫 공지"
        ]
    );
    {
        let guard = store.lock().unwrap();
        assert_eq!(guard.feed_cursor().unwrap(), Some(8));
        // Its four revisions in order, none replayed by the cursor path.
        let revised = guard.document_id("two", "a").unwrap().unwrap();
        assert_eq!(guard.document(revised, None).unwrap().revisions.len(), 4);
        // The backfill is history: read and silent. The existing board still notifies.
        assert_eq!(
            guard
                .proactive_candidates(10)
                .unwrap()
                .iter()
                .map(|c| c.title.as_str())
                .collect::<Vec<_>>(),
            ["기존 게시판 새 공지"]
        );
        assert!(guard.feed_new_boards().unwrap().is_empty());
    }
    assert_eq!(unread_titles(&store), ["기존 게시판 새 공지"]);

    // Once known, the board follows the cursor like the others.
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(
        fake.take_requests(),
        [
            format!("{BASE}manifest.json"),
            format!("{BASE}manifest.json.sig")
        ]
    );
}

#[tokio::test]
async fn a_backfill_that_stops_early_runs_again() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.sqlite3")).unwrap();
    store.catalog(&[definition("one")]).unwrap();
    let store: SharedStore = Arc::new(Mutex::new(store));
    let fake = Fake::default();
    let manifest = first_feed(&fake);
    let controller = Controller::default();
    run(&controller, &store, directory.path(), &fake).await;
    store
        .lock()
        .unwrap()
        .catalog(&[definition("one"), definition("two")])
        .unwrap();
    // The snapshot is unreachable: the sync fails and the board stays new.
    fake.fail(&format!("{BASE}{SNAPSHOT}"), FetchError::Offline);
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(status(&store).result.as_deref(), Some("offline"));
    assert_eq!(
        store.lock().unwrap().feed_new_boards().unwrap(),
        HashSet::from(["two".to_string()])
    );
    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    feed.records(
        SNAPSHOT,
        &[(1, notice("1", "첫 공지")), (2, notice("2", "둘째 공지"))],
    );
    feed.publish(&manifest, &signer());
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(status(&store).result.as_deref(), Some("ok"));
    assert!(store.lock().unwrap().feed_new_boards().unwrap().is_empty());
}

fn academic(uid: &str, title: &str, day: &str) -> Record {
    Record::Event(rhythm_feed::record::Event {
        calendar_id: "sogang-academic-calendar".into(),
        uid: uid.into(),
        title: title.into(),
        start_day: day.into(),
        end_day: day.into(),
    })
}

fn academic_titles(store: &SharedStore) -> Vec<String> {
    store
        .lock()
        .unwrap()
        .calendar_notice_dates("2026-09-01", "2026-10-31")
        .unwrap()
        .events
        .into_iter()
        .map(|e| e.title)
        .collect()
}

#[tokio::test]
async fn events_an_older_version_skipped_are_backfilled_once() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.sqlite3")).unwrap();
    store.catalog(&[definition("one")]).unwrap();
    let store: SharedStore = Arc::new(Mutex::new(store));
    let fake = Fake::default();
    let feed = Feed {
        fake: &fake,
        base: BASE.into(),
    };
    let snapshot = Snapshot {
        through_seq: 2,
        records: feed.records(
            SNAPSHOT,
            &[
                (1, notice_on("one", "1", "첫 공지")),
                (2, academic("a", "개강", "2026-09-01")),
            ],
        ),
        vectors: None,
    };
    let mut manifest = manifest(
        "2026-11-18T12:00:00.000Z",
        snapshot,
        vec![segment(
            &feed,
            3,
            5,
            &[
                (3, academic("a", "개강미사", "2026-09-01")),
                (4, academic("b", "중간시험", "2026-10-20")),
                (
                    5,
                    Record::EventRemove(rhythm_feed::record::EventRemoval {
                        calendar_id: "sogang-academic-calendar".into(),
                        uid: "b".into(),
                        removed_at: "2026-11-18T11:00:00.000Z".into(),
                    }),
                ),
            ],
        )],
    );
    feed.publish(&manifest, &signer());
    let controller = Controller::default();
    // A first sync takes the snapshot path: events arrive and are known.
    run(&controller, &store, directory.path(), &fake).await;
    assert_eq!(academic_titles(&store), ["개강미사"]);
    assert!(store.lock().unwrap().feed_events_known().unwrap());

    // As an older version left it: the cursor passed the events unseen.
    store.lock().unwrap().feed_forget_events().unwrap();
    // Nothing new: the backfill still runs, from the snapshot and segments.
    manifest.generated_at = "2026-11-18T12:30:00.000Z".into();
    feed.publish(&manifest, &signer());
    assert!(run(&controller, &store, directory.path(), &fake).await);
    assert_eq!(academic_titles(&store), ["개강미사"]);
    assert!(store.lock().unwrap().feed_events_known().unwrap());

    // A later segment on the cursor path: new and changed events apply.
    manifest.generated_at = "2026-11-18T13:00:00.000Z".into();
    manifest.segments.push(segment(
        &feed,
        6,
        7,
        &[
            (6, academic("c", "수강신청", "2026-09-10")),
            (7, academic("a", "개강미사(오전 10시)", "2026-09-01")),
        ],
    ));
    feed.publish(&manifest, &signer());
    assert!(run(&controller, &store, directory.path(), &fake).await);
    assert_eq!(
        academic_titles(&store),
        ["개강미사(오전 10시)", "수강신청"],
        "the newest state of each event"
    );
    // A backfill together with newer segments ends at the same state.
    store.lock().unwrap().feed_forget_events().unwrap();
    manifest.generated_at = "2026-11-18T13:30:00.000Z".into();
    feed.publish(&manifest, &signer());
    assert!(run(&controller, &store, directory.path(), &fake).await);
    assert_eq!(academic_titles(&store), ["개강미사(오전 10시)", "수강신청"]);
    // Events never touch notices or unread counts.
    assert_eq!(titles(&store), ["첫 공지"]);
}
