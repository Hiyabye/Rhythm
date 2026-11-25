use super::*;
use crate::archive::model::*;
use feed::{Applied, FeedResult};
use proactive::ProactiveEventKind;
use rhythm_feed::{
    record::{
        BodyState, Classification, DateKind, Embedding, Entry, Notice, NoticeDate, Record, Removal,
        Resource, ResourceKind,
    },
    vector, Revision, DIMENSIONS,
};
use rusqlite::OptionalExtension;

fn definition(id: &str, family: &str) -> SourceDefinition {
    SourceDefinition {
        id: id.into(),
        name: format!("공지 {id}"),
        organization: "서강대학교".into(),
        description: "공개 공지".into(),
        url: format!("https://www.sogang.ac.kr/{id}"),
        family: family.into(),
        supported: true,
        group: None,
    }
}
fn database() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("db")).unwrap();
    store
        .catalog(&[
            definition("one", "main"),
            definition("campus", "cybercampus"),
        ])
        .unwrap();
    (directory, store)
}
fn article(id: &str, body: &str) -> ArticleInput {
    ArticleInput {
        external_id: id.into(),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        title: format!("공지 {id}"),
        author: Some("학사지원팀".into()),
        category: Some("학사".into()),
        published_at: "2026-11-02".into(),
        modified_at: None,
        original: "{}".into(),
        unsupported_body: false,
        body_html: format!("<p>{body}</p>"),
        body_text: body.into(),
        resources: vec![],
        etag: None,
        last_modified: None,
    }
}
/// The feed's record of `article`, with the collector's revision hash.
fn notice(source: &str, article: &ArticleInput) -> Notice {
    Notice {
        source_id: source.into(),
        external_id: article.external_id.clone(),
        revision_hash: rhythm_feed::revision_hash(&Revision {
            canonical_url: &article.canonical_url,
            title: &article.title,
            author: article.author.as_deref(),
            category: article.category.as_deref(),
            published_at: &article.published_at,
            body_text: &article.body_text,
            resources: &[],
        }),
        canonical_url: article.canonical_url.clone(),
        title: article.title.clone(),
        author: article.author.clone(),
        category: article.category.clone(),
        published_at: article.published_at.clone(),
        modified_at: None,
        body_state: BodyState::Normal,
        excerpt: rhythm_feed::excerpt(&article.body_text),
        body_hash: Some(rhythm_feed::body_hash(&article.body_text)),
        resources: vec![Resource {
            url: "https://www.sogang.ac.kr/file/a.pdf".into(),
            filename: "a.pdf".into(),
            kind: ResourceKind::Attachment,
        }],
        dates: vec![NoticeDate {
            day: "2026-11-20".into(),
            time: None,
            end_day: None,
            end_time: None,
            label: "신청 마감".into(),
            kind: DateKind::Deadline,
        }],
        dates_version: 1,
        historical: false,
        first_seen_at: "2026-11-02T01:00:00.000Z".into(),
        revised_at: "2026-11-02T01:00:00.000Z".into(),
    }
}
fn feed_notice(id: &str, body: &str) -> Notice {
    let mut notice = notice("one", &article(id, body));
    // The feed lists the attachment; the hash covers it as the page did.
    notice.revision_hash = rhythm_feed::revision_hash(&Revision {
        canonical_url: &notice.canonical_url,
        title: &notice.title,
        author: notice.author.as_deref(),
        category: notice.category.as_deref(),
        published_at: &notice.published_at,
        body_text: body,
        resources: &[("https://www.sogang.ac.kr/file/a.pdf", "a.pdf", "attachment")],
    });
    notice
}
fn entry(seq: u64, record: Record) -> Entry {
    Entry {
        seq,
        record: Some(record),
    }
}
fn upsert(seq: u64, notice: &Notice) -> Entry {
    entry(seq, Record::Notice(Box::new(notice.clone())))
}
fn embedding(seq: u64, notice: &Notice, vector: u32) -> Entry {
    entry(
        seq,
        Record::Embedding(Embedding {
            source_id: notice.source_id.clone(),
            external_id: notice.external_id.clone(),
            revision_hash: notice.revision_hash.clone(),
            scope: FEED_SCOPE.into(),
            vector,
        }),
    )
}
fn classification(seq: u64, notice: &Notice, category: &str) -> Entry {
    entry(
        seq,
        Record::Classification(Classification {
            source_id: notice.source_id.clone(),
            external_id: notice.external_id.clone(),
            revision_hash: notice.revision_hash.clone(),
            scope: FEED_SCOPE.into(),
            category: category.into(),
            score: 0.75,
            margin: 0.125,
            low_confidence: false,
        }),
    )
}
const FEED_SCOPE: &str = "@cf/qwen/qwen3-embedding-0.6b#hash";
/// `entry` with its vector or category under another scope.
fn scoped(mut entry: Entry, scope: &str) -> Entry {
    match entry.record.as_mut() {
        Some(Record::Embedding(record)) => record.scope = scope.into(),
        Some(Record::Classification(record)) => record.scope = scope.into(),
        _ => unreachable!("only vectors and categories carry a scope"),
    }
    entry
}
fn removal(seq: u64, id: &str) -> Entry {
    entry(
        seq,
        Record::Remove(Removal {
            source_id: "one".into(),
            external_id: id.into(),
            removed_at: "2026-11-18T01:00:00.000Z".into(),
        }),
    )
}
fn vectors(count: u32) -> Vec<u8> {
    let quantized: Vec<_> = (0..count)
        .map(|seed| {
            let values: Vec<f32> = (0..DIMENSIONS as u32)
                .map(|i| ((i * 7 + seed * 13) % 101) as f32 - 50.0)
                .collect();
            vector::quantize(&values).unwrap()
        })
        .collect();
    vector::encode(&quantized)
}

/// `(body_text, body_pending, from_feed, removed_at, revisions)` of a notice.
type Row = (String, bool, bool, Option<String>, i64);
fn row(store: &Store, id: &str) -> Option<Row> {
    store
        .connection
        .query_row(
            "SELECT body_text,body_pending,from_feed,removed_at,
               (SELECT COUNT(*) FROM document_revisions r WHERE r.document_id=d.id)
             FROM documents d WHERE source_id='one' AND external_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .unwrap()
}
fn count(store: &Store, sql: &str) -> i64 {
    store.connection.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn document(store: &Store, id: &str) -> i64 {
    store.document_id("one", id).unwrap().unwrap()
}

#[test]
fn a_snapshot_then_segments_mirror_the_feed() {
    let (_directory, store) = database();
    assert_eq!(store.feed_cursor().unwrap(), None);
    // Without a snapshot there is no cursor to continue from.
    assert!(store
        .feed_apply_segment(1, 1, &[], &[], false, &Default::default())
        .is_err());

    let first = feed_notice("1", &"가".repeat(400));
    let second = feed_notice("2", "둘째 본문");
    let applied = store
        .feed_apply_snapshot(
            5,
            &[
                upsert(1, &first),
                upsert(2, &second),
                embedding(3, &first, 0),
                classification(4, &first, "장학"),
            ],
            &vectors(1),
        )
        .unwrap();
    assert_eq!((applied.revised, applied.skipped), (2, 0));
    assert_eq!(store.feed_cursor().unwrap(), Some(5));
    // The excerpt stands in for the body until the notice is opened.
    let (body, pending, from_feed, removed, revisions) = row(&store, "1").unwrap();
    assert_eq!(body.chars().count(), rhythm_feed::EXCERPT_CHARS);
    assert!(pending && from_feed && removed.is_none());
    assert_eq!(revisions, 1);
    let id = document(&store, "1");
    let listed = store.document(id, None).unwrap();
    assert_eq!(listed.resources.len(), 1);
    assert_eq!(listed.resources[0].status, "pending");
    let dates = store.notice_dates(id).unwrap();
    assert_eq!((dates.len(), dates[0].kind.as_str()), (1, "deadline"));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM feed_embeddings"), 1);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM feed_classifications WHERE category='장학'"
        ),
        1
    );
    // The excerpt is searchable like a body.
    let page = store
        .archive_query(&ArchiveQuery {
            query: "둘째".into(),
            ..ArchiveQuery::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 1);

    // A new revision of notice 2, a stale vector of its old revision and the
    // removal of notice 1, in a segment the cursor reaches into.
    let revised = feed_notice("2", "고친 본문");
    let applied = store
        .feed_apply_segment(
            5,
            8,
            &[
                upsert(5, &first),
                upsert(6, &revised),
                embedding(7, &second, 0),
                removal(8, "1"),
            ],
            &vectors(1),
            false,
            &Default::default(),
        )
        .unwrap();
    assert_eq!(
        applied,
        Applied {
            revised: 1,
            removed: 1,
            skipped: 1,
            orphaned: vec![]
        },
        "seq 5 is already applied; the old revision's vector is skipped"
    );
    assert_eq!(store.feed_cursor().unwrap(), Some(8));
    assert!(row(&store, "1").is_none());
    let (body, pending, _, _, revisions) = row(&store, "2").unwrap();
    assert_eq!((body.as_str(), pending, revisions), ("고친 본문", true, 2));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM feed_embeddings"),
        0,
        "notice 1's vector left with it"
    );
    // A segment that does not continue from the cursor is refused.
    assert!(store
        .feed_apply_segment(10, 11, &[], &[], false, &Default::default())
        .is_err());
    assert!(store
        .feed_apply_segment(3, 8, &[], &[], false, &Default::default())
        .is_err());
}

#[test]
fn a_snapshot_removes_only_notices_that_came_from_the_feed() {
    let (_directory, store) = database();
    // A v1 notice the feed never described, and two feed notices.
    store
        .upsert_article("one", &article("v1", "예전 본문"), false)
        .unwrap();
    let kept = feed_notice("kept", "본문");
    let saved = feed_notice("saved", "저장한 공지");
    let gone = feed_notice("gone", "사라질 공지");
    store
        .feed_apply_snapshot(
            4,
            &[
                upsert(1, &kept),
                upsert(2, &saved),
                upsert(3, &gone),
                classification(4, &kept, "학사"),
            ],
            &[],
        )
        .unwrap();
    store
        .set_document_state(document(&store, "saved"), None, Some(true))
        .unwrap();
    // A downloaded attachment of the notice that goes.
    store
        .connection
        .execute(
            "UPDATE resources SET status='downloaded',local_path='resources/ab/cd',bytes=1,sha256=?2
             WHERE document_id=?1",
            params![document(&store, "gone"), "0".repeat(64)],
        )
        .unwrap();

    // The next snapshot lists only `kept`, without its category.
    let applied = store
        .feed_apply_snapshot(9, &[upsert(9, &kept)], &[])
        .unwrap();
    assert_eq!(applied.removed, 2);
    assert_eq!(applied.orphaned, vec!["resources/ab/cd".to_string()]);
    assert!(row(&store, "gone").is_none());
    let (_, _, _, removed, _) = row(&store, "saved").unwrap();
    assert!(removed.is_some(), "a saved notice stays, marked removed");
    let (body, pending, from_feed, _, _) = row(&store, "v1").unwrap();
    assert_eq!(
        (body.as_str(), pending, from_feed),
        ("예전 본문", false, false)
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM feed_classifications"),
        0,
        "a snapshot carries every current category"
    );

    // Listed again, the saved notice is no longer marked removed.
    store
        .feed_apply_segment(
            9,
            10,
            &[upsert(10, &saved)],
            &[],
            false,
            &Default::default(),
        )
        .unwrap();
    assert_eq!(row(&store, "saved").unwrap().3, None);
}

#[test]
fn a_v1_notice_keeps_its_body_when_the_feed_describes_the_same_revision() {
    let (_directory, store) = database();
    let same = article("same", "같은 본문");
    let changed = article("changed", "예전 본문");
    store.upsert_article("one", &same, false).unwrap();
    store.upsert_article("one", &changed, false).unwrap();
    let mut newer = changed.clone();
    newer.body_text = "새 본문".into();
    let applied = store
        .feed_apply_snapshot(
            2,
            &[
                upsert(1, &notice("one", &same)),
                upsert(2, &notice("one", &newer)),
            ],
            &[],
        )
        .unwrap();
    assert_eq!(applied.revised, 1, "only the changed notice is revised");
    let (body, pending, from_feed, _, revisions) = row(&store, "same").unwrap();
    assert_eq!(
        (body.as_str(), pending, from_feed, revisions),
        ("같은 본문", false, true, 1)
    );
    let (body, pending, from_feed, _, revisions) = row(&store, "changed").unwrap();
    assert_eq!(
        (body.as_str(), pending, from_feed, revisions),
        ("새 본문", true, true, 2)
    );
    // The adopted notice keeps the dates extracted from its full body.
    let id = document(&store, "same");
    assert!(store.notice_dates(id).unwrap().is_empty());
}

#[test]
fn records_the_app_cannot_use_are_skipped() {
    let (_directory, store) = database();
    store
        .upsert_article("one", &article("deleted", "본문"), false)
        .unwrap();
    let id = document(&store, "deleted");
    store.trash_documents(&[id]).unwrap();
    store.purge_trashed(Some(&[id])).unwrap();

    let mut campus = feed_notice("c1", "본문");
    campus.source_id = "campus".into();
    let mut unknown = feed_notice("u1", "본문");
    unknown.source_id = "elsewhere".into();
    let mut invalid = feed_notice("bad", "본문");
    invalid.canonical_url = "http://www.sogang.ac.kr/ko/detail/bad".into();
    let deleted = notice("one", &article("deleted", "본문"));
    let fine = feed_notice("fine", "본문");
    let applied = store
        .feed_apply_snapshot(
            6,
            &[
                upsert(1, &campus),
                upsert(2, &unknown),
                upsert(3, &invalid),
                upsert(4, &deleted),
                Entry {
                    seq: 5,
                    record: None,
                },
                upsert(6, &fine),
                classification(6, &fine, "없는 분류"),
            ],
            &[],
        )
        .unwrap();
    // Four notices and the category are skipped; the unknown kind is ignored.
    assert_eq!((applied.revised, applied.skipped), (1, 5));
    assert!(
        row(&store, "deleted").is_none(),
        "a permanent deletion wins"
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM documents"), 1);
    // A vectors file that is not whole vectors fails the file.
    assert!(store.feed_apply_snapshot(7, &[], &[0; 10]).is_err());
    assert_eq!(store.feed_cursor().unwrap(), Some(6));
}

#[test]
fn menu_weeks_are_not_mirrored_yet() {
    // Format 1.3 menu weeks (#318) pass without error until the app mirrors them.
    let (_directory, store) = database();
    let week = rhythm_feed::record::MealWeek {
        week: "2026-11-23".into(),
        source_id: "dorm-menu".into(),
        external_id: "1".into(),
        image_url: "https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000047/a.jpg".into(),
        status: rhythm_feed::record::MealStatus::Unreadable,
        days: Vec::new(),
        breakfast_common: None,
        drinks: None,
    };
    let fine = feed_notice("fine", "본문");
    let applied = store
        .feed_apply_snapshot(
            2,
            &[upsert(1, &fine), entry(2, Record::MealWeek(Box::new(week)))],
            &[],
        )
        .unwrap();
    assert_eq!((applied.revised, applied.skipped), (1, 0));
    assert_eq!(store.feed_cursor().unwrap(), Some(2));
}

#[test]
fn a_snapshot_never_becomes_notifications_and_keeps_the_feed_dates() {
    let (_directory, store) = database();
    store
        .feed_apply_snapshot(1, &[upsert(1, &feed_notice("1", "본문"))], &[])
        .unwrap();
    let (baseline, latest): (i64, i64) = store
        .connection
        .query_row(
            "SELECT baseline_revision_id,(SELECT MAX(id) FROM document_revisions) FROM proactive_baseline",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(baseline, latest);
    // The date backfill leaves a notice that holds only its excerpt alone.
    assert_eq!(store.refresh_notice_dates(100).unwrap(), 0);
    assert_eq!(store.notice_dates(document(&store, "1")).unwrap().len(), 1);
}

#[test]
fn a_populated_profile_upgrades_and_a_failed_upgrade_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", "main")]).unwrap();
        store
            .upsert_article("one", &article("1", "본문"), false)
            .unwrap();
        store
            .set_document_state(document(&store, "1"), Some(true), Some(true))
            .unwrap();
        store
            .connection
            .execute_batch(&format!(
                "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}{UNDO_BODY_HASH}{UNDO_FEED_SYNC}{UNDO_FEED}PRAGMA user_version=36;"
            ))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    let (body, pending, from_feed, removed, _) = row(&store, "1").unwrap();
    assert_eq!(
        (body.as_str(), pending, from_feed, removed),
        ("본문", false, false, None)
    );
    let item = store.document(document(&store, "1"), None).unwrap();
    assert!(item.item.is_read && item.item.saved);
    assert_eq!(store.feed_cursor().unwrap(), None);
    drop(store);

    // A table left behind makes 037 fail; the open changes nothing.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}{UNDO_BODY_HASH}{UNDO_FEED_SYNC}{UNDO_FEED}CREATE TABLE feed_state(id INTEGER); PRAGMA user_version=36;"
        ))
        .unwrap();
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 36);
    assert!(connection
        .prepare("SELECT revision_hash FROM documents")
        .is_err());
}

fn manifest(generated_at: &str, boards: &[(&str, u32)]) -> rhythm_feed::Manifest {
    let file = rhythm_feed::FileRef::new("snapshots/1.jsonl.zst".into(), b"x");
    rhythm_feed::Manifest {
        format: rhythm_feed::FORMAT.into(),
        generated_at: generated_at.into(),
        moved_to: None,
        embedding: rhythm_feed::manifest::EmbeddingInfo {
            model: "model".into(),
            scope: FEED_SCOPE.into(),
            dimensions: 1024,
            canary: None,
        },
        calendars: Vec::new(),
        meals: Vec::new(),
        boards: boards
            .iter()
            .map(
                |&(source_id, failures)| rhythm_feed::manifest::BoardHealth {
                    source_id: source_id.into(),
                    last_success_at: None,
                    consecutive_failures: failures,
                    history_complete: true,
                },
            )
            .collect(),
        snapshot: rhythm_feed::manifest::Snapshot {
            through_seq: 1,
            records: file,
            vectors: None,
        },
        segments: vec![],
    }
}

#[test]
fn the_status_reports_feed_age_and_subscribed_failing_boards() {
    let (_directory, store) = database();
    store
        .catalog(&[
            definition("one", "main"),
            definition("two", "main"),
            definition("campus", "cybercampus"),
        ])
        .unwrap();
    store.subscribe("one", true).unwrap();
    store.subscribe("two", true).unwrap();
    let at = |text: &str| {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let empty = store.feed_status(at("2026-11-18T12:00:00Z")).unwrap();
    assert_eq!(
        (empty.generated_at, empty.result, empty.stale),
        (None, None, false)
    );

    store
        .feed_accept(
            &manifest(
                "2026-11-18T12:00:00.000Z",
                &[("one", 3), ("two", 2), ("gone", 9)],
            ),
            "https://feed.example.org/",
        )
        .unwrap();
    store.feed_record(FeedResult::Ok, None).unwrap();
    let fresh = store.feed_status(at("2026-11-18T15:00:00Z")).unwrap();
    assert!(!fresh.stale, "three hours is not yet stale");
    assert!(fresh.checked_at.is_some());
    // At three failures, on a subscribed board the app knows.
    assert_eq!(
        fresh
            .failing_boards
            .iter()
            .map(|b| b.source_id.as_str())
            .collect::<Vec<_>>(),
        ["one"]
    );
    assert!(store.feed_status(at("2026-11-18T15:00:01Z")).unwrap().stale);
    // Each board row carries the same health, whether subscribed or not.
    let health = |id: &str| store.source(id).unwrap().feed;
    let one = health("one").unwrap();
    assert_eq!(
        (one.consecutive_failures, one.failing, one.history_complete),
        (3, true, true)
    );
    let two = health("two").unwrap();
    assert_eq!((two.consecutive_failures, two.failing), (2, false));
    assert!(
        health("campus").is_none(),
        "the feed never describes Cybercampus"
    );
    store.subscribe("one", false).unwrap();
    assert!(health("one").unwrap().failing);
    assert_eq!(
        store
            .sources()
            .unwrap()
            .iter()
            .filter(|s| s.feed.is_some())
            .count(),
        2
    );
    store.subscribe("one", true).unwrap();

    // A failure keeps the last success time; an older manifest is refused.
    let checked = fresh.checked_at;
    store
        .feed_record(FeedResult::Offline, Some("연결 안 됨"))
        .unwrap();
    let offline = store.feed_status(at("2026-11-18T13:00:00Z")).unwrap();
    assert_eq!(
        (
            offline.result.as_deref(),
            offline.error.as_deref(),
            offline.checked_at
        ),
        (Some("offline"), Some("연결 안 됨"), checked)
    );
    assert!(store
        .feed_accept(
            &manifest("2026-11-18T11:59:59.000Z", &[]),
            "https://feed.example.org/"
        )
        .is_err());
    assert_eq!(
        store
            .feed_status(at("2026-11-18T13:00:00Z"))
            .unwrap()
            .failing_boards
            .len(),
        1
    );
}

#[test]
fn a_schema_37_profile_upgrades_and_a_failed_upgrade_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", "main")]).unwrap();
        store
            .feed_apply_snapshot(3, &[upsert(1, &feed_notice("1", "본문"))], &[])
            .unwrap();
        store
            .connection
            .execute_batch(&format!(
                "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}{UNDO_BODY_HASH}{UNDO_FEED_SYNC}PRAGMA user_version=37;"
            ))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(store.feed_cursor().unwrap(), Some(3));
    assert_eq!(store.feed_state().unwrap().base, None);
    assert_eq!(store.feed_status(chrono::Utc::now()).unwrap().result, None);
    drop(store);

    // A table left behind makes 038 fail; the open changes nothing.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}{UNDO_BODY_HASH}{UNDO_FEED_SYNC}CREATE TABLE feed_boards(id INTEGER); PRAGMA user_version=37;"
        ))
        .unwrap();
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 37);
    assert!(connection.prepare("SELECT base FROM feed_state").is_err());
}

/// The page as the app fetches it when `notice` is opened.
fn fetched(id: &str, body: &str, resources: &[(&str, &str, &str)]) -> ArticleInput {
    ArticleInput {
        original: format!("<html>{body}</html>"),
        resources: resources
            .iter()
            .map(|(url, filename, kind)| ResourceInput {
                url: (*url).into(),
                filename: (*filename).into(),
                kind: (*kind).into(),
            })
            .collect(),
        ..article(id, body)
    }
}
const ATTACHMENT: (&str, &str, &str) =
    ("https://www.sogang.ac.kr/file/a.pdf", "a.pdf", "attachment");

#[test]
fn an_opened_notice_fills_its_feed_revision_in_place() {
    let (_directory, store) = database();
    let body = "2026년 11월 25일까지 신청서를 제출하세요.";
    let notice = feed_notice("1", body);
    store
        .feed_apply_snapshot(1, &[upsert(1, &notice)], &[])
        .unwrap();
    let id = document(&store, "1");
    assert!(store.document(id, None).unwrap().summary_only);
    let (_, entry) = store.pending_body(id).unwrap().unwrap();
    assert_eq!(entry.url, notice.canonical_url);

    let filled = store
        .fill_feed_body(id, &fetched("1", body, &[ATTACHMENT]), true)
        .unwrap();
    assert_eq!(
        filled,
        feed::Filled::default(),
        "the feed described this text"
    );
    let (text, pending, _, _, revisions) = row(&store, "1").unwrap();
    assert_eq!((text.as_str(), pending, revisions), (body, false, 1));
    let detail = store.document(id, None).unwrap();
    assert!(!detail.summary_only);
    assert_eq!(detail.body_html, format!("<p>{body}</p>"));
    assert_eq!(detail.resources.len(), 1);
    // Still the feed's revision: nothing above the notification baseline.
    assert!(
        count(&store, "SELECT MAX(id) FROM document_revisions")
            <= store.proactive_baseline().unwrap()
    );
    // Dates now come from the body.
    let dates = store.notice_dates(id).unwrap();
    assert_eq!(dates[0].day, "2026-11-25");
    assert!(store.pending_body(id).unwrap().is_none());

    // The feed's next record of the same revision keeps the body, and a second
    // fill (another open finishing later) changes nothing.
    store
        .feed_apply_segment(2, 2, &[upsert(2, &notice)], &[], false, &Default::default())
        .unwrap();
    assert_eq!(row(&store, "1").unwrap().0, body);
    let filled = store
        .fill_feed_body(id, &fetched("1", "늦게 받은 본문", &[ATTACHMENT]), true)
        .unwrap();
    assert_eq!(filled, feed::Filled::default());
    assert_eq!(row(&store, "1").unwrap().0, body);
}

#[test]
fn a_revised_school_text_takes_its_own_hash() {
    let (_directory, store) = database();
    let notice = feed_notice("2", "옛 본문");
    store
        .feed_apply_snapshot(
            1,
            &[upsert(1, &notice), embedding(2, &notice, 0)],
            &vectors(1),
        )
        .unwrap();
    let id = document(&store, "2");
    // The feed's attachment was downloaded; the school dropped it since.
    store
        .connection
        .execute(
            "UPDATE resources SET status='downloaded',local_path='resources/ab/cd',bytes=1,sha256=?2
             WHERE document_id=?1",
            params![id, "0".repeat(64)],
        )
        .unwrap();
    let image = ("https://www.sogang.ac.kr/file/b.png", "b.png", "image");
    let page = fetched("2", "고친 본문", &[image]);
    let filled = store.fill_feed_body(id, &page, true).unwrap();
    assert_eq!(
        filled,
        feed::Filled {
            revised: true,
            orphaned: vec!["resources/ab/cd".into()]
        }
    );
    let detail = store.document(id, None).unwrap();
    assert_eq!(detail.body_text, "고친 본문");
    assert_eq!(
        detail.revisions.len(),
        1,
        "filled in place, not a new revision"
    );
    let urls: Vec<_> = detail.resources.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(urls, [image.0]);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM feed_embeddings"),
        1,
        "the feed's vector stays until it describes the new revision"
    );

    // When the feed catches up with that revision, the body stays.
    let mut later = feed_notice("2", "고친 본문");
    later.revision_hash = feed::article_revision_hash(&page);
    later.resources = vec![Resource {
        url: image.0.into(),
        filename: image.1.into(),
        kind: ResourceKind::Image,
    }];
    store
        .feed_apply_segment(2, 2, &[upsert(2, &later)], &[], false, &Default::default())
        .unwrap();
    let (text, pending, _, _, revisions) = row(&store, "2").unwrap();
    assert_eq!((text.as_str(), pending, revisions), ("고친 본문", false, 1));
}

#[test]
fn only_pending_feed_notices_are_fetched_and_a_pdf_body_keeps_the_feed_hash() {
    let (_directory, store) = database();
    store
        .upsert_article("one", &article("v1", "예전 본문"), false)
        .unwrap();
    let saved = feed_notice("saved", "저장한 공지");
    let pdf = feed_notice("pdf", "");
    store
        .feed_apply_snapshot(2, &[upsert(1, &saved), upsert(2, &pdf)], &[])
        .unwrap();
    let saved_id = document(&store, "saved");
    store
        .set_document_state(saved_id, None, Some(true))
        .unwrap();
    store
        .feed_apply_segment(
            3,
            3,
            &[removal(3, "saved")],
            &[],
            false,
            &Default::default(),
        )
        .unwrap();
    // A v1 body is here, and the school deleted the saved notice.
    assert!(store
        .pending_body(document(&store, "v1"))
        .unwrap()
        .is_none());
    assert!(store.pending_body(saved_id).unwrap().is_none());
    assert!(store.document(saved_id, None).unwrap().removed);
    assert_eq!(
        store
            .fill_feed_body(saved_id, &fetched("saved", "본문", &[]), true)
            .unwrap(),
        feed::Filled::default()
    );
    assert!(row(&store, "saved").unwrap().1, "still the excerpt");

    // A body made from a PDF's text is not compared with the feed's hash.
    let id = document(&store, "pdf");
    let hash = notice_hash(&store, id);
    let filled = store
        .fill_feed_body(
            id,
            &fetched("pdf", "PDF에서 읽은 본문", &[ATTACHMENT]),
            false,
        )
        .unwrap();
    assert!(!filled.revised);
    assert_eq!(notice_hash(&store, id), hash);
    assert_eq!(row(&store, "pdf").unwrap().0, "PDF에서 읽은 본문");
}
fn notice_hash(store: &Store, id: i64) -> String {
    store
        .connection
        .query_row(
            "SELECT revision_hash FROM documents WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_backfill_starts_read_and_later_arrivals_stay_unread() {
    let (_directory, store) = database();
    // A v1 notice the user has not read, then the first sync.
    store
        .upsert_article("one", &article("v1", "예전 본문"), false)
        .unwrap();
    let read = |store: &Store, id: &str| -> bool {
        store
            .connection
            .query_row(
                "SELECT is_read FROM document_state WHERE document_id=?1",
                [document(store, id)],
                |r| r.get(0),
            )
            .unwrap()
    };
    store
        .feed_apply_snapshot(1, &[upsert(1, &feed_notice("old", "본문"))], &[])
        .unwrap();
    store
        .feed_apply_segment(
            2,
            2,
            &[upsert(2, &feed_notice("caught-up", "본문"))],
            &[],
            true,
            &Default::default(),
        )
        .unwrap();
    // Later syncs: a new notice, and one the collector's history import found.
    let mut backlog = feed_notice("backlog", "본문");
    backlog.historical = true;
    store
        .feed_apply_segment(
            3,
            5,
            &[
                upsert(3, &feed_notice("new", "본문")),
                upsert(4, &backlog),
                upsert(5, &feed_notice("v1", "예전 본문")),
            ],
            &[],
            false,
            &Default::default(),
        )
        .unwrap();
    assert!(read(&store, "old") && read(&store, "caught-up") && read(&store, "backlog"));
    assert!(
        !read(&store, "new"),
        "a notice arriving after the first sync is news"
    );
    assert!(
        !read(&store, "v1"),
        "a notice already here keeps its read state"
    );
}

#[test]
fn feed_vectors_and_categories_back_ai_and_local_rows_cover_other_notices() {
    let (_directory, store) = database();
    store.set_enabled(crate::extensions::AI, true).unwrap();
    let local = "a".repeat(64);
    let config = crate::intelligence::classification::config_hash();
    // Vectors 0 and 1 point the same way; 2 is orthogonal to them.
    let base: Vec<f32> = (0..DIMENSIONS)
        .map(|i| if i < 512 { 1.0 } else { 0.0 })
        .collect();
    let other: Vec<f32> = (0..DIMENSIONS)
        .map(|i| if i < 512 { 0.0 } else { 1.0 })
        .collect();
    let file = vector::encode(&[
        vector::quantize(&base).unwrap(),
        vector::quantize(&base).unwrap(),
        vector::quantize(&other).unwrap(),
    ]);
    let scholarship = feed_notice("scholarship", "장학 안내");
    let twin = feed_notice("twin", "장학 설명");
    let foreign = feed_notice("foreign", "다른 모델");
    // A later config hash of the same model still counts; another model does not.
    store
        .feed_apply_snapshot(
            9,
            &[
                upsert(1, &scholarship),
                upsert(2, &twin),
                upsert(3, &foreign),
                embedding(4, &scholarship, 0),
                scoped(
                    embedding(5, &twin, 1),
                    "@cf/qwen/qwen3-embedding-0.6b#newer",
                ),
                scoped(embedding(6, &foreign, 1), "@cf/other/model#hash"),
                classification(7, &scholarship, "장학"),
                scoped(classification(8, &foreign, "학사"), "@cf/other/model#hash"),
            ],
            &file,
        )
        .unwrap();
    let (scholarship, twin, foreign) = (
        document(&store, "scholarship"),
        document(&store, "twin"),
        document(&store, "foreign"),
    );
    let revision = |id: i64| store.document(id, None).unwrap().revision_id;
    // A v1 notice the feed never described keeps the local model's rows, and
    // a local row on a feed notice is ignored.
    store
        .upsert_article("one", &article("v1", "예전 공지"), false)
        .unwrap();
    let v1 = document(&store, "v1");
    for (id, category) in [(v1, "연구"), (foreign, "행사·활동")] {
        store
            .ai_save_classification(&super::intelligence::ClassificationWrite {
                document_id: id,
                revision_id: revision(id),
                model_sha256: &local,
                config_hash: config,
                category,
                score: 0.8,
                margin: 0.1,
                low_confidence: false,
            })
            .unwrap();
    }
    let unit = vector::decode(&vector::encode(&[vector::quantize(&base).unwrap()]))
        .unwrap()
        .remove(0);
    store
        .ai_save_embedding(v1, revision(v1), &local, &unit)
        .unwrap();
    store
        .ai_save_embedding(foreign, revision(foreign), &local, &unit)
        .unwrap();

    let categories = |local_model: Option<&str>| {
        store
            .ai_classifications_for_documents(
                &[scholarship, twin, foreign, v1],
                local_model.map(|model| (model, config)),
            )
            .unwrap()
            .into_iter()
            .map(|row| (row.document_id, row.category))
            .collect::<Vec<_>>()
    };
    // Without a local model only the feed's category is in scope.
    assert_eq!(categories(None), vec![(scholarship, "장학".to_string())]);
    assert_eq!(
        categories(Some(&local)),
        vec![(scholarship, "장학".to_string()), (v1, "연구".to_string())]
    );
    assert_eq!(
        store
            .ai_classification(scholarship, revision(scholarship), None)
            .unwrap()
            .map(|row| row.category),
        Some("장학".into())
    );
    assert!(store
        .ai_classification(foreign, revision(foreign), Some((&local, config)))
        .unwrap()
        .is_none());
    // The archive filter and counts need no local model for feed notices.
    let page = store
        .archive_query(&ArchiveQuery {
            ai_category: Some("장학".into()),
            ..ArchiveQuery::default()
        })
        .unwrap();
    assert_eq!(
        page.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        vec![scholarship]
    );
    let counts = page.ai_categories.unwrap();
    assert_eq!(counts.unclassified, 3);

    // Similar notices compare feed vectors with each other and, with a local
    // model, with local vectors of other notices.
    let similar = |local_model: Option<&str>| {
        let mut ids = store
            .ai_similar_notices(&[scholarship], local_model, 5)
            .unwrap()
            .remove(0)
            .1
            .into_iter()
            .map(|notice| notice.document_id)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };
    assert_eq!(similar(None), vec![twin]);
    assert_eq!(similar(Some(&local)), vec![twin, v1]);
    // Semantic search reads the same vectors.
    let ranked = store
        .archive_ranked_query(
            &ArchiveQuery {
                query: "없는말".into(),
                ..ArchiveQuery::default()
            },
            Some((&local, &unit)),
        )
        .unwrap();
    let mut hits = ranked.meaning_only.clone();
    hits.sort();
    assert_eq!(hits, vec![scholarship, twin, v1]);
    // The local classifier embeds only Cybercampus notices: feed notices
    // take the feed's vectors and a v1 public notice keeps its stored one.
    assert!(store
        .ai_embedding_candidates(&"b".repeat(64), config, 10)
        .unwrap()
        .is_empty());
}

/// Notification candidates above the baseline as (external id, kind), then
/// the baseline moves past them so the next step starts clean.
fn take_candidates(store: &Store) -> Vec<(String, ProactiveEventKind)> {
    let found = store
        .proactive_candidates(100)
        .unwrap()
        .into_iter()
        .map(|c| {
            let id: String = store
                .connection
                .query_row(
                    "SELECT external_id FROM documents WHERE id=?1",
                    [c.document_id],
                    |r| r.get(0),
                )
                .unwrap();
            (id, c.event_type)
        })
        .collect();
    store.proactive_establish_baseline().unwrap();
    found
}
/// `notice` as a later revision whose attachment was renamed: the body and
/// title stay, so it is not news.
fn renamed_attachment(mut notice: Notice) -> Notice {
    notice.resources[0].filename = "b.pdf".into();
    notice.revision_hash = rhythm_feed::body_hash("renamed");
    notice
}
/// `notice` as a 1.0 publisher sends it: no `body_hash`.
fn without_body_hash(mut notice: Notice) -> Notice {
    notice.body_hash = None;
    notice
}
fn seen_at(id: &str, first_seen_at: &str) -> Notice {
    Notice {
        first_seen_at: first_seen_at.into(),
        ..feed_notice(id, &format!("본문 {id}"))
    }
}
fn accept(store: &Store, generated_at: &str) {
    store
        .feed_accept(&manifest(generated_at, &[]), "https://feed.example.org/")
        .unwrap();
}
fn new_since_sync(store: &Store) -> u64 {
    store.archive_summary().unwrap().new_since_sync
}

#[test]
fn new_since_sync_counts_what_the_latest_manifest_brought() {
    let (_directory, store) = database();
    store
        .catalog(&[
            definition("one", "main"),
            definition("cybercampus", "cybercampus"),
        ])
        .unwrap();
    // Only subscribed sources count, like the 받은 소식 list.
    store.subscribe("one", true).unwrap();
    store.subscribe("cybercampus", true).unwrap();
    // A notice the app crawled in v1 never counts, however recent.
    store
        .upsert_article("one", &article("v1", "예전 본문"), false)
        .unwrap();
    // The first sync has no previous manifest to compare with.
    accept(&store, "2026-11-18T12:00:00.000Z");
    store
        .feed_apply_snapshot(
            1,
            &[upsert(1, &seen_at("1", "2026-11-18T11:40:00.000Z"))],
            &[],
        )
        .unwrap();
    assert_eq!(new_since_sync(&store), 0);

    accept(&store, "2026-11-18T12:30:00.000Z");
    segment(
        &store,
        2,
        &[
            seen_at("2", "2026-11-18T12:10:00.000Z"),
            seen_at("3", "2026-11-18T12:20:00.000Z"),
        ],
    );
    assert_eq!(new_since_sync(&store), 2);
    store.subscribe("one", false).unwrap();
    assert_eq!(new_since_sync(&store), 0);
    store.subscribe("one", true).unwrap();
    // The same manifest again (a sync with nothing new) keeps the count.
    accept(&store, "2026-11-18T12:30:00.000Z");
    store.feed_record(FeedResult::Ok, None).unwrap();
    assert_eq!(new_since_sync(&store), 2);
    // A historical backfill notice does not count either.
    accept(&store, "2026-11-18T13:00:00.000Z");
    segment(
        &store,
        4,
        &[
            seen_at("4", "2026-11-18T12:40:00.000Z"),
            Notice {
                historical: true,
                ..seen_at("5", "2026-11-18T12:45:00.000Z")
            },
        ],
    );
    assert_eq!(new_since_sync(&store), 1);
    // A newer manifest that brought nothing resets it.
    accept(&store, "2026-11-18T13:30:00.000Z");
    assert_eq!(new_since_sync(&store), 0);

    // Cybercampus keeps its own run-based rule.
    store
        .upsert_article("cybercampus", &article("A-1", "강의 공지"), false)
        .unwrap();
    assert_eq!(new_since_sync(&store), 1);
}

#[test]
fn a_schema_40_profile_starts_counting_at_the_next_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", "main")]).unwrap();
        store.subscribe("one", true).unwrap();
        accept(&store, "2026-11-18T12:00:00.000Z");
        store
            .feed_apply_snapshot(
                1,
                &[upsert(1, &seen_at("1", "2026-11-18T11:40:00.000Z"))],
                &[],
            )
            .unwrap();
        accept(&store, "2026-11-18T12:30:00.000Z");
        segment(&store, 2, &[seen_at("2", "2026-11-18T12:10:00.000Z")]);
        store
            .connection
            .execute_batch(&format!(
                "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}PRAGMA user_version=40;"
            ))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(new_since_sync(&store), 0);
    accept(&store, "2026-11-18T13:00:00.000Z");
    segment(&store, 3, &[seen_at("3", "2026-11-18T12:40:00.000Z")]);
    assert_eq!(new_since_sync(&store), 1);
    drop(store);

    // The column left in place makes 041 fail; the open changes nothing.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=40;")
        .unwrap();
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let (version, previous): (i64, Option<String>) = connection
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version),previous_generated_at FROM feed_state",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (version, previous.as_deref()),
        (40, Some("2026-11-18T12:30:00.000Z"))
    );
}
fn segment(store: &Store, from: u64, notices: &[Notice]) {
    let entries: Vec<Entry> = notices
        .iter()
        .enumerate()
        .map(|(i, n)| upsert(from + i as u64, n))
        .collect();
    let to = from + notices.len() as u64 - 1;
    store
        .feed_apply_segment(from, to, &entries, &[], false, &Default::default())
        .unwrap();
}

#[test]
fn feed_arrivals_notify_only_when_the_title_or_body_changed() {
    use ProactiveEventKind::{New, Revision};
    let (_directory, store) = database();
    let long = format!("{}끝 A", "가".repeat(400));
    store
        .feed_apply_snapshot(
            2,
            &[
                upsert(1, &feed_notice("seen", "본문")),
                upsert(2, &feed_notice("long", &long)),
            ],
            &[],
        )
        .unwrap();
    assert_eq!(take_candidates(&store), [], "a first sync is not news");

    let mut backlog = feed_notice("backlog", "본문");
    backlog.historical = true;
    segment(
        &store,
        3,
        &[
            feed_notice("new", "본문"),
            feed_notice("two", "본문"),
            backlog,
        ],
    );
    assert_eq!(
        take_candidates(&store),
        [("new".to_string(), New), ("two".to_string(), New)]
    );

    // A body edit beyond the excerpt is news; a renamed attachment is not.
    let edited = feed_notice("long", &format!("{}끝 B", "가".repeat(400)));
    assert_eq!(edited.excerpt, feed_notice("long", &long).excerpt);
    segment(
        &store,
        6,
        &[edited, renamed_attachment(feed_notice("seen", "본문"))],
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM document_revisions"), 7);
    assert_eq!(take_candidates(&store), [("long".to_string(), Revision)]);

    // Once opened, a revision holds the full page. A renamed attachment is
    // still not news, whether or not the publisher sent a body hash.
    for id in ["new", "two"] {
        store
            .fill_feed_body(
                document(&store, id),
                &fetched(id, "본문", &[ATTACHMENT]),
                true,
            )
            .unwrap();
    }
    segment(
        &store,
        8,
        &[
            renamed_attachment(feed_notice("new", "본문")),
            without_body_hash(renamed_attachment(feed_notice("two", "본문"))),
        ],
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM document_revisions"), 9);
    assert_eq!(take_candidates(&store), []);
    // A changed body is news either way.
    segment(
        &store,
        10,
        &[
            feed_notice("new", "고친 본문"),
            without_body_hash(feed_notice("two", "고친 본문")),
        ],
    );
    assert_eq!(
        take_candidates(&store),
        [("new".to_string(), Revision), ("two".to_string(), Revision)]
    );
}

#[test]
fn a_feed_revision_of_a_crawled_notice_compares_with_the_crawled_body() {
    use ProactiveEventKind::Revision;
    let (_directory, store) = database();
    let body = format!("{}끝 A", "가".repeat(400));
    for id in ["v1", "old"] {
        store
            .upsert_article("one", &article(id, &body), false)
            .unwrap();
    }
    // `old` was crawled before revisions carried a body hash.
    store
        .connection
        .execute(
            "UPDATE document_revisions SET body_hash=NULL WHERE document_id=?1",
            [document(&store, "old")],
        )
        .unwrap();
    store
        .feed_apply_snapshot(1, &[upsert(1, &feed_notice("other", "본문"))], &[])
        .unwrap();
    assert_eq!(take_candidates(&store), []);

    // Same body, other metadata: a revision, but not news.
    segment(
        &store,
        2,
        &[
            renamed_attachment(feed_notice("v1", &body)),
            renamed_attachment(feed_notice("old", &body)),
        ],
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM document_revisions"), 5);
    assert_eq!(take_candidates(&store), []);

    // The crawled body hash shows an edit beyond the excerpt; without one
    // only the excerpt can be compared, so that edit goes unnoticed.
    let edited = format!("{}끝 B", "가".repeat(400));
    store
        .connection
        .execute(
            "UPDATE document_revisions SET body_hash=NULL WHERE document_id=?1",
            [document(&store, "old")],
        )
        .unwrap();
    segment(
        &store,
        4,
        &[feed_notice("v1", &edited), feed_notice("old", &edited)],
    );
    assert_eq!(take_candidates(&store), [("v1".to_string(), Revision)]);
}

#[test]
fn opportunities_list_open_deadlines_first_within_their_categories() {
    let (_directory, store) = database();
    let day = |offset: i64| {
        (chrono::Local::now().date_naive() + chrono::Duration::days(offset))
            .format("%Y-%m-%d")
            .to_string()
    };
    let deadline = |offset: i64, time: Option<&str>| NoticeDate {
        day: day(offset),
        time: time.map(Into::into),
        end_day: None,
        end_time: None,
        label: "신청 마감".into(),
        kind: DateKind::Deadline,
    };
    let opportunity = |id: &str, published: &str, dates: Vec<NoticeDate>| {
        let mut notice = feed_notice(id, "모집 안내");
        notice.published_at = published.into();
        notice.dates = dates;
        notice
    };
    let notices = [
        opportunity("soon", "2026-11-01", vec![deadline(3, None)]),
        // The nearest of several deadlines counts.
        opportunity(
            "sooner",
            "2026-11-01",
            vec![deadline(10, None), deadline(1, Some("18:00"))],
        ),
        opportunity("today", "2026-11-01", vec![deadline(0, None)]),
        opportunity("undated", "2026-11-02", vec![]),
        opportunity("closed", "2026-11-03", vec![deadline(-1, None)]),
        opportunity("scholarship", "2026-11-01", vec![deadline(2, None)]),
        opportunity("unclassified", "2026-11-01", vec![deadline(2, None)]),
    ];
    let categories = [
        ("soon", "공모전·대회"),
        ("sooner", "인턴·채용"),
        ("today", "교육·프로그램"),
        ("undated", "행사·활동"),
        ("closed", "공모전·대회"),
        ("scholarship", "장학"),
    ];
    let mut entries: Vec<Entry> = notices
        .iter()
        .enumerate()
        .map(|(index, notice)| upsert(index as u64 + 1, notice))
        .collect();
    for (index, (id, category)) in categories.iter().enumerate() {
        let notice = notices.iter().find(|n| n.external_id == *id).unwrap();
        entries.push(classification(20 + index as u64, notice, category));
    }
    store.feed_apply_snapshot(40, &entries, &[]).unwrap();
    let query = |include_closed: bool, ai_category: Option<&str>| ArchiveQuery {
        opportunities: true,
        include_closed,
        ai_category: ai_category.map(Into::into),
        ..ArchiveQuery::default()
    };

    // The page shows AI output, so it needs the AI extension.
    assert!(store.archive_query(&query(false, None)).is_err());
    let due_today = || store.archive_summary().unwrap().due_today;
    assert_eq!(due_today(), 0);
    store.set_enabled(crate::extensions::AI, true).unwrap();
    // The 기회 badge counts 오늘 마감 like the page: subscribed sources only.
    assert_eq!(due_today(), 0);
    store.subscribe("one", true).unwrap();
    assert_eq!(due_today(), 1);

    let ids = |page: &ArchivePage| {
        page.items
            .iter()
            .map(|item| item.title.trim_start_matches("공지 ").to_owned())
            .collect::<Vec<_>>()
    };
    let page = store.archive_query(&query(false, None)).unwrap();
    assert_eq!(ids(&page), ["today", "sooner", "soon", "undated"]);
    assert_eq!(page.total, 4);
    let id = |external: &str| document(&store, external);
    assert_eq!(
        page.deadlines,
        vec![
            ItemDeadline {
                document_id: id("today"),
                day: day(0),
                time: None,
                closed: false
            },
            ItemDeadline {
                document_id: id("sooner"),
                day: day(1),
                time: Some("18:00".into()),
                closed: false
            },
            ItemDeadline {
                document_id: id("soon"),
                day: day(3),
                time: None,
                closed: false
            },
        ]
    );
    let counts = page.ai_categories.unwrap();
    let count = |category: &str| {
        counts
            .counts
            .iter()
            .find(|row| row.category == category)
            .unwrap()
            .count
    };
    assert_eq!(
        (count("공모전·대회"), count("장학"), counts.unclassified),
        (1, 0, 0)
    );

    // Closed notices follow the open ones, newest first among the undated.
    let page = store.archive_query(&query(true, None)).unwrap();
    assert_eq!(ids(&page), ["today", "sooner", "soon", "closed", "undated"]);
    assert!(page.deadlines.contains(&ItemDeadline {
        document_id: id("closed"),
        day: day(-1),
        time: None,
        closed: true
    }));

    let page = store
        .archive_query(&query(false, Some("인턴·채용")))
        .unwrap();
    assert_eq!(ids(&page), ["sooner"]);
    assert!(store.archive_query(&query(false, Some("장학"))).is_err());

    // The ordinary list keeps its order and carries no deadlines.
    let page = store.archive_query(&ArchiveQuery::default()).unwrap();
    assert_eq!(page.total, 7);
    assert!(page.deadlines.is_empty());
}

#[test]
fn a_schema_41_profile_knows_the_boards_of_its_old_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", "main")]).unwrap();
        store
            .feed_apply_snapshot(3, &[upsert(1, &feed_notice("1", "본문"))], &[])
            .unwrap();
        store
            .connection
            .execute_batch(&format!(
                "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}PRAGMA user_version=41;"
            ))
            .unwrap();
    }
    // The migration records the catalog it finds; the new one arrives after it.
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert!(store.feed_new_boards().unwrap().is_empty());
    store
        .catalog(&[definition("one", "main"), definition("two", "main")])
        .unwrap();
    assert_eq!(
        store.feed_new_boards().unwrap(),
        std::collections::HashSet::from(["two".to_string()])
    );
    store.feed_remember_boards().unwrap();
    assert!(store.feed_new_boards().unwrap().is_empty());
    drop(store);

    // A table left behind makes 042 fail; the open changes nothing.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version=41;")
        .unwrap();
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let (version, cursor, known): (i64, Option<i64>, i64) = connection
        .query_row(
            "SELECT (SELECT user_version FROM pragma_user_version),
                    (SELECT cursor FROM feed_state WHERE id=1),
                    (SELECT COUNT(*) FROM feed_known_boards)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((version, cursor, known), (41, Some(3), 2));
}

fn academic(uid: &str, title: &str, start: &str, end: &str) -> Record {
    Record::Event(rhythm_feed::record::Event {
        calendar_id: "sogang-academic-calendar".into(),
        uid: uid.into(),
        title: title.into(),
        start_day: start.into(),
        end_day: end.into(),
    })
}

fn shown(store: &Store, from: &str, to: &str) -> Vec<(String, String, String)> {
    store
        .calendar_notice_dates(from, to)
        .unwrap()
        .events
        .into_iter()
        .map(|e| (e.title, e.start_day, e.end_day))
        .collect()
}

#[test]
fn calendar_events_are_mirrored_and_shown_in_their_range_while_the_layer_is_on() {
    let (directory, store) = database();
    store
        .feed_apply_snapshot(
            2,
            &[
                entry(1, academic("a", "개강", "2026-09-01", "2026-09-01")),
                entry(2, academic("b", "중간시험", "2026-10-20", "2026-10-27")),
            ],
            &[],
        )
        .unwrap();
    let applied = store
        .feed_apply_segment(
            3,
            6,
            &[
                entry(3, academic("a", "개강미사", "2026-08-31", "2026-09-02")),
                // Ends before it starts: skipped and counted, never stored.
                entry(4, academic("x", "역순", "2026-09-10", "2026-09-09")),
                entry(5, academic("c", "수강신청", "2026-09-10", "2026-09-10")),
                entry(
                    6,
                    Record::EventRemove(rhythm_feed::record::EventRemoval {
                        calendar_id: "sogang-academic-calendar".into(),
                        uid: "b".into(),
                        removed_at: "2026-10-01T00:00:00.000Z".into(),
                    }),
                ),
            ],
            &[],
            false,
            &Default::default(),
        )
        .unwrap();
    assert_eq!((applied.revised, applied.skipped), (0, 1));
    // A span that starts before the range still shows; the removal is gone.
    assert_eq!(
        shown(&store, "2026-09-01", "2026-10-31"),
        [
            ("개강미사".into(), "2026-08-31".into(), "2026-09-02".into()),
            ("수강신청".into(), "2026-09-10".into(), "2026-09-10".into()),
        ]
    );
    assert!(shown(&store, "2026-09-03", "2026-09-09").is_empty());
    assert!(
        store
            .calendar_notice_dates("2026-09-01", "2026-09-30")
            .unwrap()
            .academic
    );

    // Hidden: no events, and the choice survives a reopen.
    store.set_calendar_academic(false).unwrap();
    let hidden = store
        .calendar_notice_dates("2026-09-01", "2026-09-30")
        .unwrap();
    assert!(!hidden.academic && hidden.events.is_empty());
    drop(store);
    let store = Store::open(&directory.path().join("db")).unwrap();
    assert!(!store.calendar_academic().unwrap());
    store.set_calendar_academic(true).unwrap();

    // A snapshot that no longer lists an event removes it.
    store
        .feed_apply_snapshot(
            9,
            &[entry(
                9,
                academic("c", "수강신청", "2026-09-10", "2026-09-10"),
            )],
            &[],
        )
        .unwrap();
    assert_eq!(
        shown(&store, "2026-09-01", "2026-10-31"),
        [("수강신청".into(), "2026-09-10".into(), "2026-09-10".into())]
    );
}

#[test]
fn a_schema_42_profile_shows_the_layer_and_backfills_events_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", "main")]).unwrap();
        store
            .feed_apply_snapshot(3, &[upsert(1, &feed_notice("1", "본문"))], &[])
            .unwrap();
        store
            .connection
            .execute_batch(&format!("{UNDO_CALENDAR_EVENTS}PRAGMA user_version=42;"))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert!(store.calendar_academic().unwrap(), "on by default");
    assert!(
        !store.feed_events_known().unwrap(),
        "the next sync backfills"
    );
    assert_eq!(store.feed_cursor().unwrap(), Some(3));
    store.feed_remember_boards().unwrap();
    assert!(store.feed_events_known().unwrap());
    drop(store);

    // A table left behind makes 043 fail; the open changes nothing.
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(&format!(
            "{UNDO_CALENDAR_EVENTS}PRAGMA user_version=42; CREATE TABLE calendar_events(sentinel TEXT);"
        ))
        .unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 42);
    assert!(connection
        .query_row("SELECT calendar_academic FROM settings", [], |r| r
            .get::<_, i64>(0))
        .is_err());
    assert!(connection
        .query_row("SELECT events_known FROM feed_state", [], |r| r
            .get::<_, i64>(0))
        .is_err());
}
