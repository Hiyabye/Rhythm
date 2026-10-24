use super::*;
use crate::archive::model::*;

fn definition(id: &str, supported: bool) -> SourceDefinition {
    SourceDefinition {
        id: id.into(),
        name: format!("학사 공지 {id}"),
        organization: "서강대학교".into(),
        description: "공개 공지".into(),
        url: format!("https://www.sogang.ac.kr/{id}"),
        family: "main".into(),
        supported,
    }
}
fn article(external_id: &str, title: &str, text: &str, date: &str) -> ArticleInput {
    ArticleInput {
        external_id: external_id.into(),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{external_id}"),
        title: title.into(),
        author: Some("학사지원팀".into()),
        category: Some("학사".into()),
        published_at: date.into(),
        modified_at: None,
        original: format!("{{\"body\":{text:?},\"views\":1}}"),
        body_html: format!("<p>{text}</p>"),
        body_text: text.into(),
        resources: vec![],
        etag: None,
        last_modified: None,
    }
}
fn database() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("db")).unwrap();
    store
        .catalog(&[
            definition("one", true),
            definition("two", true),
            definition("unsupported", false),
        ])
        .unwrap();
    (directory, store)
}
#[test]
fn schema_one_upgrade_preserves_every_preference() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../../migrations/001_foundation.sql"))
            .unwrap();
        connection.execute("UPDATE settings SET theme='dark',energy_profile='eco',interests='컴퓨터 보안 · 장학금' WHERE id=1",[]).unwrap();
        connection
            .execute(
                "INSERT INTO extension_preferences VALUES('sogang_notices',1)",
                [],
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, 2);
    assert_eq!(store.settings().unwrap().theme, "dark");
    assert_eq!(store.settings().unwrap().energy_profile, "eco");
    assert_eq!(store.settings().unwrap().interests, "컴퓨터 보안 · 장학금");
    assert!(store.enabled("sogang_notices").unwrap());
    assert_eq!(store.archive_summary().unwrap().documents, 0);
    drop(store);
    assert_eq!(
        Store::open(&path).unwrap().info().unwrap().schema_version,
        2
    );
}
#[test]
fn migration_failure_rolls_back_all_schema_two_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../../migrations/001_foundation.sql"))
        .unwrap();
    // A collision late in migration 2, after sources/documents have been created.
    connection.execute_batch("CREATE TABLE document_state(sentinel TEXT); INSERT INTO document_state VALUES('preserved');").unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('sources','documents','document_revisions')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM document_state", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
}
#[test]
fn future_schema_is_not_changed_even_to_enable_wal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA user_version=99;CREATE TABLE sentinel(value TEXT);INSERT INTO sentinel VALUES('keep');").unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "delete"
    );
    assert_eq!(
        connection
            .query_row("SELECT value FROM sentinel", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}
#[test]
fn catalog_refresh_preserves_subscriptions_progress_and_interruption_checkpoint() {
    let (directory, store) = database();
    assert!(store.subscribe("unsupported", true).is_err());
    assert!(store.subscribe("missing", true).is_err());
    store.subscribe("one", true).unwrap();
    store
        .source_progress("one", "running", 7, false, 130, 120, 10, None, false)
        .unwrap();
    let attempt = store.source("one").unwrap().last_attempt;
    assert!(attempt.is_some());
    store
        .source_progress("one", "completed", 7, false, 130, 120, 10, None, true)
        .unwrap();
    let success = store.source("one").unwrap().last_success;
    store
        .source_progress(
            "one",
            "failed",
            7,
            false,
            130,
            120,
            10,
            Some("일시적인 오류"),
            false,
        )
        .unwrap();
    assert_eq!(store.source("one").unwrap().last_attempt, attempt);
    assert_eq!(store.source("one").unwrap().last_success, success);
    let mut refreshed = definition("one", true);
    refreshed.name = "바뀐 표시 이름".into();
    store.catalog(&[refreshed]).unwrap();
    assert!(store.source("one").unwrap().subscribed);
    assert_eq!(store.source("one").unwrap().next_page, 7);
    store
        .source_progress("one", "running", 7, false, 130, 120, 10, None, false)
        .unwrap();
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    let source = reopened.source("one").unwrap();
    assert_eq!(source.status, "interrupted");
    assert_eq!(source.next_page, 7);
    assert!(source.subscribed);
    assert_eq!(source.last_success, success);
}
#[test]
fn revisions_deduplicate_content_but_preserve_changes_and_user_state() {
    let (directory, store) = database();
    let mut original = article(
        "123",
        "장학금 신청",
        "장학금 신청은 2026-10-30까지 가능합니다.",
        "2026-10-20",
    );
    let (id, changed) = store.upsert_article("one", &original, true).unwrap();
    assert!(changed);
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    let first_revision = store.document(id, None).unwrap().revision_id;
    original.original = "{\"views\":9999}".into();
    original.etag = Some("new-validator".into());
    original.modified_at = Some("2026-10-24".into());
    assert_eq!(
        store.upsert_article("one", &original, false).unwrap(),
        (id, false)
    );
    assert_eq!(store.document(id, None).unwrap().revisions.len(), 1);
    assert_eq!(
        store.validators("one", "123").unwrap().unwrap().0,
        Some("new-validator".into())
    );
    original.body_text = "장학금 신청 마감이 2026-11-01로 변경되었습니다.".into();
    original.body_html = format!("<p>{}</p>", original.body_text);
    assert_eq!(
        store.upsert_article("one", &original, false).unwrap(),
        (id, true)
    );
    let current = store.document(id, None).unwrap();
    assert_eq!(current.revisions.len(), 2);
    assert!(current.item.is_read && current.item.saved);
    assert!(current.item.historical);
    assert!(current.body_text.contains("2026-11-01"));
    assert!(store
        .document(id, Some(first_revision))
        .unwrap()
        .body_text
        .contains("2026-10-30"));
    let other = store.upsert_article("two", &original, false).unwrap().0;
    assert_ne!(id, other);
    let other_revision = store.document(other, None).unwrap().revision_id;
    assert!(store.document(id, Some(other_revision)).is_err());
    assert_eq!(store.archive_summary().unwrap().documents, 2);
    assert_eq!(store.archive_summary().unwrap().new_since_sync, 1);
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    let current = reopened.document(id, None).unwrap();
    assert!(current.item.is_read && current.item.saved);
    assert_eq!(current.revisions.len(), 2);
    reopened.set_enabled("sogang_notices", false).unwrap();
    assert!(reopened.document(id, None).unwrap().item.saved);
}
#[test]
fn korean_literal_substrings_mixed_queries_and_pagination_are_searchable() {
    let (_directory, store) = database();
    let entries = [
        (
            "a",
            "장학금 신청 안내",
            "2026학년도 컴퓨터공학 AI 연구 인턴 모집 C++ 100% _test_",
            "2026-10-01",
        ),
        (
            "b",
            "학부 수강신청",
            "Computer Science 대학원 수강 2026-10-02",
            "2026-10-02",
        ),
        ("c", "취업 안내", "Rust internship 채용 공고", "2026-10-03"),
    ];
    for (id, title, body, date) in entries {
        store
            .upsert_article("one", &article(id, title, body, date), true)
            .unwrap();
    }
    for (query, expected) in [
        ("장", 1),
        ("학금", 1),
        ("장학금", 1),
        ("컴퓨터공학", 1),
        ("퓨터공", 1),
        ("AI 연구", 1),
        ("computer science", 1),
        ("2026-10", 1),
        ("C++", 1),
        ("100%", 1),
        ("_test_", 1),
        ("%", 1),
        ("_", 1),
        ("\"", 0),
        ("AND OR", 0),
        ("없는검색어", 0),
        ("", 3),
        ("  ", 3),
    ] {
        let page = store
            .archive_query(&ArchiveQuery {
                query: query.into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, expected, "query={query:?}");
    }
    let page = store
        .archive_query(&ArchiveQuery {
            limit: Some(1),
            offset: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].title, "학부 수강신청");
    let oldest = store
        .archive_query(&ArchiveQuery {
            ascending: true,
            limit: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(oldest.items[0].title, "장학금 신청 안내");
    assert!(store
        .archive_query(&ArchiveQuery {
            source_id: Some("two".into()),
            ..Default::default()
        })
        .unwrap()
        .items
        .is_empty());
    assert!(store
        .archive_query(&ArchiveQuery {
            category: Some("장학".into()),
            ..Default::default()
        })
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                category: Some("학사".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
        3
    );
    let id = oldest.items[0].id;
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                saved_only: true,
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                unread_only: true,
                ..Default::default()
            })
            .unwrap()
            .total,
        2
    );
}
#[test]
fn fts_tracks_updates_and_deletes_with_foreign_key_integrity() {
    let (_directory, store) = database();
    let mut input = article("a", "이전제목", "원래본문", "2026-10-01");
    let id = store.upsert_article("one", &input, true).unwrap().0;
    input.title = "변경제목".into();
    input.body_text = "수정본문".into();
    input.body_html = "<p>수정본문</p>".into();
    store.upsert_article("one", &input, true).unwrap();
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "원래본문".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "수정본문".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    assert!(store
        .connection
        .execute("DELETE FROM sources WHERE id='one'", [])
        .is_err());
    store
        .connection
        .execute("DELETE FROM documents WHERE id=?1", [id])
        .unwrap();
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "수정본문".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM document_revisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM document_state", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store
        .connection
        .execute_batch("INSERT INTO documents_fts(documents_fts,rank) VALUES('integrity-check',1)")
        .unwrap();
    assert!(store
        .connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}
#[test]
fn resource_states_revisions_and_storage_accounting_survive_offline_reopen() {
    let (directory, store) = database();
    let mut input = article(
        "image",
        "포스터 안내",
        "아래 포스터를 확인하세요.",
        "2026-10-01",
    );
    input.resources.push(ResourceInput {
        url: "https://www.sogang.ac.kr/file/image.png".into(),
        filename: "../../unsafe.png".into(),
        kind: "image".into(),
    });
    let id = store.upsert_article("one", &input, true).unwrap().0;
    let resource = store.pending_resources(id).unwrap().remove(0);
    assert_eq!(
        store.document(id, None).unwrap().item.content_status,
        "missing_resources"
    );
    assert!(store
        .resource_result(
            resource.id,
            "downloaded",
            Some("image/png"),
            Some(128),
            Some("../escape"),
            Some("hash"),
            None
        )
        .is_err());
    store
        .resource_result(
            resource.id,
            "downloaded",
            Some("image/png"),
            Some(128),
            Some("resources/abc.png"),
            Some("abc"),
            None,
        )
        .unwrap();
    assert_eq!(
        store.resource(resource.id).unwrap().filename,
        "../../unsafe.png"
    );
    assert_eq!(
        store.document(id, None).unwrap().item.content_status,
        "archived"
    );
    input.title = "변경된 포스터 안내".into();
    store.upsert_article("one", &input, true).unwrap();
    assert!(store.pending_resources(id).unwrap().is_empty());
    assert_eq!(store.archive_summary().unwrap().resource_bytes, 128);
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    let detail = reopened.document(id, None).unwrap();
    assert_eq!(
        detail.resources[0].local_path.as_deref(),
        Some("resources/abc.png")
    );
    assert_eq!(detail.revisions.len(), 2);
    assert_eq!(
        reopened
            .archive_query(&ArchiveQuery {
                query: "포스터".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
}
#[test]
fn http_resource_metadata_preserves_text_without_authorizing_downloads() {
    let (_directory, store) = database();
    let mut input = article("mixed", "옛 공지", "장학금 신청 안내", "2019-10-01");
    input.resources.push(ResourceInput {
        url: "http://www.sogang.ac.kr/file/old.png".into(),
        filename: "old.png".into(),
        kind: "image".into(),
    });
    let (id, _) = store.upsert_article("one", &input, true).unwrap();
    let document = store.document(id, None).unwrap();
    assert_eq!(document.body_text, "장학금 신청 안내");
    assert_eq!(
        document.resources[0].url,
        "http://www.sogang.ac.kr/file/old.png"
    );
    assert_eq!(document.resources[0].status, "pending");
    input.resources[0].url = "http://user:password@www.sogang.ac.kr/old.png".into();
    assert!(store.upsert_article("one", &input, true).is_err());
    assert_eq!(store.document(id, None).unwrap().revisions.len(), 1);
}
#[test]
fn invalid_content_does_not_partially_create_documents() {
    let (_directory, store) = database();
    let mut input = article("a", "공지", "본문", "2026-10-01");
    input.resources.push(ResourceInput {
        url: "javascript:alert(1)".into(),
        filename: "bad".into(),
        kind: "attachment".into(),
    });
    assert!(store.upsert_article("one", &input, true).is_err());
    assert_eq!(store.archive_summary().unwrap().documents, 0);
    assert!(store
        .source_progress("one", "running", 0, false, 0, 0, 0, None, false)
        .is_err());
}
