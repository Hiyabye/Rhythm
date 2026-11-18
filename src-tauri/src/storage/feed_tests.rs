use super::*;
use crate::archive::model::*;
use feed::Applied;
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
            scope: "model#hash".into(),
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
            scope: "model#hash".into(),
            category: category.into(),
            score: 0.75,
            margin: 0.125,
            low_confidence: false,
        }),
    )
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
    assert!(store.feed_apply_segment(1, 1, &[], &[]).is_err());

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
    assert!(store.feed_apply_segment(10, 11, &[], &[]).is_err());
    assert!(store.feed_apply_segment(3, 8, &[], &[]).is_err());
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
        .feed_apply_segment(9, 10, &[upsert(10, &saved)], &[])
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
            .execute_batch(&format!("{UNDO_FEED}PRAGMA user_version=36;"))
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
            "{UNDO_FEED}CREATE TABLE feed_state(id INTEGER); PRAGMA user_version=36;"
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
