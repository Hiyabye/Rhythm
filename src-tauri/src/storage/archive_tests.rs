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
        group: None,
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
        unsupported_body: false,
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
    assert_eq!(store.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(store.settings().unwrap().theme, "dark");
    assert_eq!(store.settings().unwrap().energy_profile, "eco");
    assert_eq!(store.settings().unwrap().interests, "컴퓨터 보안 · 장학금");
    assert!(store.enabled("sogang_notices").unwrap());
    assert_eq!(store.archive_summary().unwrap().documents, 0);
    drop(store);
    assert_eq!(
        Store::open(&path).unwrap().info().unwrap().schema_version,
        SCHEMA_VERSION
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
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT last_revision_at FROM documents WHERE id=?1",
                [id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
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
        store
            .connection
            .query_row(
                "SELECT last_revision_at FROM documents WHERE id=?1",
                [id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
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
    let last_revision: Option<String> = store
        .connection
        .query_row(
            "SELECT last_revision_at FROM documents WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    let latest_collected: String = store
        .connection
        .query_row(
            "SELECT collected_at FROM document_revisions WHERE id=?1",
            [current.revision_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last_revision.as_deref(), Some(latest_collected.as_str()));
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
    // Each board counts only its own unread notices: "one" holds the read one.
    let unread = |id: &str| {
        store
            .sources()
            .unwrap()
            .into_iter()
            .find(|source| source.definition.id == id)
            .unwrap()
            .unread
    };
    assert_eq!((unread("one"), unread("two")), (0, 1));
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
fn embedded_nul_search_is_rejected_without_false_matches_or_fts_errors() {
    let (_directory, store) = database();
    store
        .upsert_article(
            "one",
            &article("a", "공지", "AI 연구 안내", "2026-10-01"),
            true,
        )
        .unwrap();
    for query in ["\0", "AI\0foo"] {
        let error = store
            .archive_query(&ArchiveQuery {
                query: query.into(),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error, "검색어에 사용할 수 없는 문자가 포함되어 있습니다.");
    }
    assert_eq!(store.archive_summary().unwrap().documents, 1);
}

#[test]
fn selected_revision_snippet_uses_its_own_body() {
    let (_directory, store) = database();
    let mut input = article("a", "원래 제목", "이전 버전 본문", "2026-10-01");
    let id = store.upsert_article("one", &input, true).unwrap().0;
    let first = store.document(id, None).unwrap().revision_id;
    input.title = "바뀐 제목".into();
    input.body_text = "현재 버전 본문".into();
    input.body_html = format!("<p>{}</p>", input.body_text);
    store.upsert_article("one", &input, false).unwrap();
    let old = store.document(id, Some(first)).unwrap();
    assert_eq!(old.item.title, "원래 제목");
    assert_eq!(old.body_text, "이전 버전 본문");
    assert_eq!(old.item.snippet, "이전 버전 본문");
}

#[test]
fn list_preview_marks_only_bodies_longer_than_the_excerpt() {
    let (_directory, store) = database();
    let exactly: String = "가".repeat(240);
    let longer: String = "나".repeat(241);
    store
        .upsert_article("one", &article("a", "경계", &exactly, "2026-10-03"), true)
        .unwrap();
    store
        .upsert_article("one", &article("b", "초과", &longer, "2026-10-02"), true)
        .unwrap();
    store
        .upsert_article(
            "one",
            &article("c", "짧음", "짧은 본문", "2026-10-01"),
            true,
        )
        .unwrap();
    let page = store
        .archive_query(&ArchiveQuery {
            query: String::new(),
            source_id: None,
            category: None,
            saved_only: false,
            unread_only: false,
            trashed: false,
            ascending: false,
            offset: 0,
            limit: None,
        })
        .unwrap();
    let snippets: Vec<_> = page.items.iter().map(|i| i.snippet.as_str()).collect();
    assert_eq!(snippets[0], exactly);
    assert_eq!(snippets[1], format!("{}…", "나".repeat(240)));
    assert_eq!(snippets[2], "짧은 본문");
}

fn assert_archive_integrity(store: &Store) {
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(store
        .connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    store
        .connection
        .execute_batch("INSERT INTO documents_fts(documents_fts,rank) VALUES('integrity-check',1)")
        .unwrap();
}

fn row_count(store: &Store, table: &str) -> i64 {
    // All callers supply test-owned static table names, never external input.
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn released_schema_two_reopen_preserves_archive_preferences_and_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("released-v020.db");
    // Construct the released schema directly, rather than relying on whichever
    // fresh-database migration sequence the current binary later implements.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../../migrations/001_foundation.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../../migrations/002_archive.sql"))
        .unwrap();
    drop(connection);
    let store = Store::open(&path).unwrap();
    store
        .catalog(&[definition("one", true), definition("two", true)])
        .unwrap();
    store
        .save(&Settings {
            theme: "dark".into(),
            energy_profile: "eco".into(),
            interests: "보안과 장학금".into(),
            ..Settings::default()
        })
        .unwrap();
    store.set_enabled("sogang_notices", true).unwrap();
    store.subscribe("one", true).unwrap();
    store
        .source_progress("one", "running", 8, false, 80, 79, 1, None, false)
        .unwrap();
    store
        .source_progress("one", "completed", 8, false, 80, 79, 1, None, true)
        .unwrap();
    store
        .source_progress("two", "running", 3, false, 20, 19, 1, None, false)
        .unwrap();
    let mut input = article("42", "처음 공지", "보존할 첫 본문", "2026-10-01");
    input.resources.push(ResourceInput {
        url: "https://www.sogang.ac.kr/file/a.png".into(),
        filename: "첨부.png".into(),
        kind: "image".into(),
    });
    let id = store.upsert_article("one", &input, true).unwrap().0;
    let first = store.document(id, None).unwrap();
    store
        .resource_result(
            first.resources[0].id,
            "downloaded",
            Some("image/png"),
            Some(128),
            Some("resources/test.png"),
            Some("test-hash"),
            None,
        )
        .unwrap();
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    input.title = "수정 공지".into();
    input.body_text = "장학금 신청으로 수정한 본문".into();
    input.body_html = format!("<p>{}</p>", input.body_text);
    store.upsert_article("one", &input, false).unwrap();
    store.set_document_state(id, Some(false), None).unwrap();
    let detail_before = serde_json::to_value(store.document(id, None).unwrap()).unwrap();
    let old_before =
        serde_json::to_value(store.document(id, Some(first.revision_id)).unwrap()).unwrap();
    let source_before = serde_json::to_value(store.source("one").unwrap()).unwrap();
    assert_archive_integrity(&store);
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.info().unwrap().schema_version, SCHEMA_VERSION);
    assert_eq!(
        reopened
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(reopened.settings().unwrap().theme, "dark");
    assert_eq!(reopened.settings().unwrap().energy_profile, "eco");
    assert_eq!(reopened.settings().unwrap().interests, "보안과 장학금");
    assert!(reopened.enabled("sogang_notices").unwrap());
    assert_eq!(
        serde_json::to_value(reopened.document(id, None).unwrap()).unwrap(),
        detail_before
    );
    assert_eq!(
        serde_json::to_value(reopened.document(id, Some(first.revision_id)).unwrap()).unwrap(),
        old_before
    );
    assert_eq!(
        serde_json::to_value(reopened.source("one").unwrap()).unwrap(),
        source_before
    );
    let interrupted = reopened.source("two").unwrap();
    assert_eq!(interrupted.status, "interrupted");
    assert_eq!(interrupted.next_page, 3);
    assert!(!interrupted.history_complete);
    assert_eq!(
        (
            interrupted.discovered,
            interrupted.archived,
            interrupted.failed
        ),
        (20, 19, 1)
    );
    let current = reopened.document(id, None).unwrap();
    assert!(!current.item.is_read && current.item.saved);
    reopened
        .set_document_state(id, Some(true), Some(false))
        .unwrap();
    assert!(reopened.document(id, None).unwrap().item.is_read);
    assert!(!reopened.document(id, None).unwrap().item.saved);
    assert_eq!(
        reopened
            .archive_query(&ArchiveQuery {
                query: "장학금".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    assert_archive_integrity(&reopened);
}

#[test]
fn late_revision_failure_rolls_back_current_document_fts_resources_and_state() {
    let (_directory, store) = database();
    let mut input = article("a", "기존제목", "원래본문", "2026-10-01");
    input.resources.push(ResourceInput {
        url: "https://www.sogang.ac.kr/file/a.png".into(),
        filename: "a.png".into(),
        kind: "image".into(),
    });
    let id = store.upsert_article("one", &input, true).unwrap().0;
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    let resource_id = store.document(id, None).unwrap().resources[0].id;
    store
        .resource_result(
            resource_id,
            "downloaded",
            Some("image/png"),
            Some(128),
            Some("resources/a.png"),
            Some("hash-a"),
            None,
        )
        .unwrap();
    let before = serde_json::to_value(store.document(id, None).unwrap()).unwrap();
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_revision BEFORE INSERT ON document_revisions BEGIN SELECT RAISE(ABORT,'test late revision failure'); END;").unwrap();
    input.title = "새로운제목".into();
    input.body_text = "수정된본문".into();
    input.body_html = "<p>수정된본문</p>".into();
    let error = store.upsert_article("one", &input, false).unwrap_err();
    assert!(error.contains("test late revision failure"), "{error}");
    assert_eq!(
        serde_json::to_value(store.document(id, None).unwrap()).unwrap(),
        before
    );
    assert_eq!(row_count(&store, "documents"), 1);
    assert_eq!(row_count(&store, "document_revisions"), 1);
    assert_eq!(row_count(&store, "resources"), 1);
    for (query, total) in [("원래본문", 1), ("수정된본문", 0), ("새로운제목", 0)] {
        assert_eq!(
            store
                .archive_query(&ArchiveQuery {
                    query: query.into(),
                    ..Default::default()
                })
                .unwrap()
                .total,
            total
        );
    }
    // The same late failure must also roll back a brand-new document and its
    // insert-triggered FTS entry, not just an update to an existing document.
    input.external_id = "new-document".into();
    assert!(store.upsert_article("one", &input, false).is_err());
    assert!(store.document_id("one", "new-document").unwrap().is_none());
    assert_eq!(row_count(&store, "documents"), 1);
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "수정된본문".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    input.external_id = "a".into();
    assert_archive_integrity(&store);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_revision;")
        .unwrap();
    assert_eq!(
        store.upsert_article("one", &input, false).unwrap(),
        (id, true)
    );
    assert_eq!(row_count(&store, "document_revisions"), 2);
    let current = store.document(id, None).unwrap();
    assert!(current.item.is_read && current.item.saved);
    assert_eq!(
        current.resources[0].local_path.as_deref(),
        Some("resources/a.png")
    );
    assert_archive_integrity(&store);
}

#[test]
fn sqlite_full_during_ingestion_preserves_committed_archive_and_index() {
    let (directory, store) = database();
    let mut input = article("a", "보존할공지", "장학금 원래본문", "2026-10-01");
    let id = store.upsert_article("one", &input, true).unwrap().0;
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    let before = serde_json::to_value(store.document(id, None).unwrap()).unwrap();
    let pages: i64 = store
        .connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    store
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    input.title = "실패할수정".into();
    input.body_text = "대용량테스트 ".repeat(100_000);
    input.body_html = format!("<p>{}</p>", input.body_text);
    let error = store.upsert_article("one", &input, false).unwrap_err();
    assert!(
        error.contains("database or disk is full"),
        "unexpected failure: {error}"
    );
    assert_eq!(
        serde_json::to_value(store.document(id, None).unwrap()).unwrap(),
        before
    );
    assert_eq!(row_count(&store, "documents"), 1);
    assert_eq!(row_count(&store, "document_revisions"), 1);
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "장학금".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "대용량테스트".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_archive_integrity(&store);
    drop(store);
    let reopened = Store::open(&directory.path().join("db")).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.document(id, None).unwrap()).unwrap(),
        before
    );
    assert_archive_integrity(&reopened);
}

#[test]
fn metadata_changes_make_revisions_but_volatile_fields_do_not() {
    let (_directory, store) = database();
    let mut input = article("a", "원래제목", "같은 본문", "2026-10-01");
    let id = store.upsert_article("one", &input, true).unwrap().0;
    store
        .set_document_state(id, Some(true), Some(true))
        .unwrap();
    let mut expected = 1;
    for field in [
        "title",
        "author",
        "category",
        "published_at",
        "canonical_url",
    ] {
        match field {
            "title" => input.title = "변경제목".into(),
            "author" => input.author = Some("장학지원팀".into()),
            "category" => input.category = Some("장학".into()),
            "published_at" => input.published_at = "2026-10-02".into(),
            "canonical_url" => {
                input.canonical_url = "https://www.sogang.ac.kr/ko/detail/new".into()
            }
            _ => unreachable!(),
        }
        assert_eq!(
            store.upsert_article("one", &input, false).unwrap(),
            (id, true),
            "{field}"
        );
        expected += 1;
        assert_eq!(store.document(id, None).unwrap().revisions.len(), expected);
        assert_eq!(
            store.upsert_article("one", &input, false).unwrap(),
            (id, false)
        );
    }
    input.original = "{\"viewCount\":999999,\"unrelated\":true}".into();
    input.modified_at = Some("2026-10-24T12:00:00Z".into());
    input.etag = Some("latest-etag".into());
    input.last_modified = Some("Sat, 24 Oct 2026 12:00:00 GMT".into());
    assert_eq!(
        store.upsert_article("one", &input, false).unwrap(),
        (id, false)
    );
    assert_eq!(store.document(id, None).unwrap().revisions.len(), expected);
    assert_eq!(
        store.validators("one", "a").unwrap().unwrap(),
        (input.etag.clone(), input.last_modified.clone())
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "원래제목".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "변경제목".into(),
                category: Some("장학".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    input.external_id = "b".into();
    let same_title = store.upsert_article("one", &input, false).unwrap().0;
    let other_source = store.upsert_article("two", &input, false).unwrap().0;
    assert_ne!(same_title, id);
    assert_ne!(same_title, other_source);
    assert!(!store.document(same_title, None).unwrap().item.saved);
    assert!(store.document(id, None).unwrap().item.saved);
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "변경제목".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        3
    );
    assert_archive_integrity(&store);
}

#[test]
fn equal_date_pages_have_deterministic_order_and_combined_filters() {
    let (_directory, store) = database();
    let mut ids = Vec::new();
    for n in 0..65 {
        let mut input = article(
            &n.to_string(),
            "같은 제목",
            "컴퓨터공학 Computer Science",
            "2026-10-01",
        );
        input.category = Some(if n % 2 == 0 { "학사" } else { "장학" }.into());
        let id = store
            .upsert_article(if n % 3 == 0 { "two" } else { "one" }, &input, true)
            .unwrap()
            .0;
        store
            .set_document_state(id, Some(n % 5 == 0), Some(n % 7 == 0))
            .unwrap();
        ids.push(id);
    }
    for ascending in [false, true] {
        let mut actual = Vec::new();
        for offset in [0, 30, 60] {
            let page = store
                .archive_query(&ArchiveQuery {
                    ascending,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(page.total, 65);
            actual.extend(page.items.into_iter().map(|item| item.id));
        }
        let expected: Vec<_> = if ascending {
            ids.clone()
        } else {
            ids.iter().rev().copied().collect()
        };
        assert_eq!(actual, expected);
    }
    let filtered = store
        .archive_query(&ArchiveQuery {
            query: "퓨터공 Computer".into(),
            source_id: Some("one".into()),
            category: Some("학사".into()),
            saved_only: true,
            unread_only: true,
            ..Default::default()
        })
        .unwrap();
    let expected: Vec<_> = (0..65)
        .rev()
        .filter(|n| n % 3 != 0 && n % 2 == 0 && n % 7 == 0 && n % 5 != 0)
        .map(|n| ids[n])
        .collect();
    assert_eq!(filtered.total, expected.len() as u64);
    assert_eq!(
        filtered
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        expected
    );
    assert_archive_integrity(&store);
}

#[test]
fn literal_search_limits_whitespace_quotes_and_fts_metacharacters_are_safe() {
    let (_directory, store) = database();
    let text = "인용 \"문자\" 경로 C:\\school\\files 100% _test_ C++ AND OR NEAR(foo) 장학금";
    store
        .upsert_article("one", &article("a", "문자 검색", text, "2026-10-01"), true)
        .unwrap();
    for query in [
        "\"문자\"",
        "C:\\school\\files",
        "100%",
        "_test_",
        "C++",
        "AND OR",
        "NEAR(foo)",
        "장학금\n\tC++",
        "문자 \"문자\"",
    ] {
        assert_eq!(
            store
                .archive_query(&ArchiveQuery {
                    query: query.into(),
                    ..Default::default()
                })
                .unwrap()
                .total,
            1,
            "{query:?}"
        );
    }
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "가".repeat(512),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "가".repeat(513),
                ..Default::default()
            })
            .unwrap_err(),
        "검색어는 512자 이내로 입력해 주세요."
    );
    assert!(store
        .archive_query(&ArchiveQuery {
            query: std::iter::repeat_n("문자", 32)
                .collect::<Vec<_>>()
                .join(" "),
            ..Default::default()
        })
        .is_ok());
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: std::iter::repeat_n("문자", 33)
                    .collect::<Vec<_>>()
                    .join(" "),
                ..Default::default()
            })
            .unwrap_err(),
        "검색 단어는 32개 이내로 입력해 주세요."
    );
    assert_archive_integrity(&store);
}

#[test]
fn resource_role_changes_require_revalidation_without_erasing_previous_copy() {
    let (_directory, store) = database();
    let mut input = article("a", "이미지 역할 변경", "포스터 안내", "2026-10-01");
    input.resources.push(ResourceInput {
        url: "https://www.sogang.ac.kr/file/a.png".into(),
        filename: "a.png".into(),
        kind: "attachment".into(),
    });
    let id = store.upsert_article("one", &input, true).unwrap().0;
    let previous = store.document(id, None).unwrap();
    let old_id = previous.resources[0].id;
    store
        .resource_result(
            old_id,
            "downloaded",
            Some("image/png"),
            Some(128),
            Some("resources/a.png"),
            Some("hash-a"),
            None,
        )
        .unwrap();
    input.resources[0].kind = "image".into();
    store.upsert_article("one", &input, false).unwrap();
    let current = store.document(id, None).unwrap();
    assert_eq!(current.resources[0].kind, "image");
    assert_eq!(current.resources[0].status, "pending");
    assert!(current.resources[0].local_path.is_none());
    let old = store.document(id, Some(previous.revision_id)).unwrap();
    assert_eq!(old.resources[0].status, "downloaded");
    assert_eq!(
        old.resources[0].local_path.as_deref(),
        Some("resources/a.png")
    );
    assert_eq!(old.resources[0].sha256.as_deref(), Some("hash-a"));
    assert_archive_integrity(&store);
}

#[test]
#[ignore = "explicit 5,000-document local performance/consistency observation; no network"]
fn large_archive_operations() {
    use std::time::Instant;
    const COUNT: usize = 5_000;
    let (_directory, store) = database();
    let sources: Vec<_> = (0..6)
        .map(|n| definition(&format!("bench-{n}"), true))
        .collect();
    store.catalog(&sources).unwrap();
    let started = Instant::now();
    let mut long_input = None;
    let mut long_id = 0;
    for n in 0..COUNT {
        let body = if n == 0 {
            format!(
                "{} 아주긴본문끝",
                "수강 신청 장학금 Computer Science C++ 인턴 모집. ".repeat(4_000)
            )
        } else {
            format!(
                "{} {}",
                "수강 신청 장학금 Computer Science C++ 인턴 모집. ".repeat(16),
                if n % 100 == 0 {
                    "양자컴퓨팅 희귀주제"
                } else {
                    "일반 안내"
                }
            )
        };
        let mut input = article(
            &n.to_string(),
            &format!("2026학년도 공지 {n}"),
            &body,
            &format!("2026-10-{:02}", 1 + n % 28),
        );
        input.category = Some(format!("분류{}", n % 5));
        let id = store
            .upsert_article(&sources[n % 6].id, &input, true)
            .unwrap()
            .0;
        store
            .set_document_state(id, Some(n % 11 == 0), Some(n % 13 == 0))
            .unwrap();
        if n == 0 {
            long_id = id;
            long_input = Some(input);
        }
    }
    println!(
        "benchmark dataset: {COUNT} documents, 6 sources, 5 categories; API ingestion {:?}",
        started.elapsed()
    );
    let cases = [
        ("inbox", ArchiveQuery::default(), COUNT as u64),
        (
            "offset-3000",
            ArchiveQuery {
                offset: 3_000,
                ..Default::default()
            },
            COUNT as u64,
        ),
        (
            "Korean-1",
            ArchiveQuery {
                query: "장".into(),
                ..Default::default()
            },
            COUNT as u64,
        ),
        (
            "Korean-2",
            ArchiveQuery {
                query: "학금".into(),
                ..Default::default()
            },
            COUNT as u64,
        ),
        (
            "Korean-3-common",
            ArchiveQuery {
                query: "장학금".into(),
                ..Default::default()
            },
            COUNT as u64,
        ),
        (
            "Korean-selective",
            ArchiveQuery {
                query: "양자컴퓨팅".into(),
                ..Default::default()
            },
            49,
        ),
        (
            "mixed",
            ArchiveQuery {
                query: "장학금 Computer".into(),
                ..Default::default()
            },
            COUNT as u64,
        ),
        (
            "source-category",
            ArchiveQuery {
                source_id: Some("bench-0".into()),
                category: Some("분류0".into()),
                ..Default::default()
            },
            (0..COUNT).filter(|n| n % 6 == 0 && n % 5 == 0).count() as u64,
        ),
        (
            "saved-unread",
            ArchiveQuery {
                saved_only: true,
                unread_only: true,
                ..Default::default()
            },
            (0..COUNT).filter(|n| n % 13 == 0 && n % 11 != 0).count() as u64,
        ),
    ];
    for (label, query, total) in cases {
        store.archive_query(&query).unwrap(); // Explicit warm-up, not timed.
        let mut elapsed = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let page = store.archive_query(&query).unwrap();
            elapsed.push(start.elapsed());
            assert_eq!(page.total, total, "{label}");
            assert!(page.items.len() <= 30, "{label}");
            assert_eq!(
                page.items.len(),
                (total.saturating_sub(u64::from(query.offset))).min(30) as usize,
                "{label}"
            );
            assert!(page
                .items
                .iter()
                .all(|item| item.snippet.chars().count() <= 242));
        }
        elapsed.sort();
        println!(
            "{label}: total={total}, seven warm debug samples min={:?} median={:?} max={:?}",
            elapsed[0], elapsed[3], elapsed[6]
        );
    }
    let start = Instant::now();
    let detail = store.document(long_id, None).unwrap();
    println!(
        "long document: {} Unicode chars, read {:?}",
        detail.body_text.chars().count(),
        start.elapsed()
    );
    assert!(detail.body_text.ends_with("아주긴본문끝"));
    let mut input = long_input.unwrap();
    input.body_text = "독립수정검색어 새 본문".into();
    input.body_html = format!("<p>{}</p>", input.body_text);
    let start = Instant::now();
    store.upsert_article("bench-0", &input, false).unwrap();
    println!("long-document revision + FTS update {:?}", start.elapsed());
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "아주긴본문끝".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .archive_query(&ArchiveQuery {
                query: "독립수정검색어".into(),
                ..Default::default()
            })
            .unwrap()
            .total,
        1
    );
    assert_eq!(store.document(long_id, None).unwrap().revisions.len(), 2);
    assert_archive_integrity(&store);
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
#[test]
fn skipped_articles_persist_validate_and_clear_on_archive() {
    let (directory, store) = database();
    let entry = |id: &str| ListingEntry {
        external_id: id.into(),
        url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        pinned: false,
        modified_at: None,
    };
    let hash = "a".repeat(64);
    for (source, entry, reason, sha) in [
        ("unsupported", entry("7"), "형식", hash.as_str()),
        ("one", entry("bad/id"), "형식", hash.as_str()),
        ("one", entry("7"), " ", hash.as_str()),
        ("one", entry("7"), "형식", "short"),
    ] {
        assert!(store
            .record_skipped(source, &entry, true, reason, sha)
            .is_err());
    }
    store
        .record_skipped("one", &entry("7"), true, "형식", &hash)
        .unwrap();
    // A repeated rejection updates the reason but keeps the original marking.
    store
        .record_skipped("one", &entry("7"), false, "다른 형식", &hash)
        .unwrap();
    store
        .record_skipped("two", &entry("8"), false, "형식", &hash)
        .unwrap();
    drop(store);
    let store = Store::open(&directory.path().join("db")).unwrap();
    let one = store.source("one").unwrap();
    assert_eq!(one.skipped_count, 1);
    let listed = store
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.definition.id == "one")
        .unwrap()
        .skipped;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].reason.as_str(), listed[0].archived),
        ("다른 형식", false)
    );
    let entries = store.skipped_entries("one").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0.external_id, "7");
    assert!(entries[0].1, "the first historical marking is kept");
    store
        .upsert_article("one", &article("7", "공지", "본문", "2026-01-01"), true)
        .unwrap();
    assert!(store
        .sources()
        .unwrap()
        .iter()
        .any(|s| s.skipped.first().is_some_and(|k| k.archived) && s.definition.id == "one"));
    store.clear_skipped("one", "7").unwrap();
    assert_eq!(store.source("one").unwrap().skipped_count, 0);
    assert_eq!(
        store.source("two").unwrap().skipped_count,
        1,
        "other sources are isolated"
    );
}
#[test]
fn schema_ten_upgrade_keeps_archive_and_adds_skipped_articles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        for migration in [
            include_str!("../../migrations/001_foundation.sql"),
            include_str!("../../migrations/002_archive.sql"),
            include_str!("../../migrations/003_intelligence.sql"),
            include_str!("../../migrations/004_personalization.sql"),
            include_str!("../../migrations/005_proactive.sql"),
            include_str!("../../migrations/006_notification_attempts.sql"),
            include_str!("../../migrations/007_notification_activation.sql"),
            include_str!("../../migrations/008_ai_execution_mode.sql"),
            include_str!("../../migrations/009_ai_model_directory.sql"),
            include_str!("../../migrations/010_update_check.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
    }
    {
        // Populate through the schema-10 tables only.
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", true)]).unwrap();
        store
            .upsert_article(
                "one",
                &article("5", "공지", "보관 본문", "2026-01-01"),
                true,
            )
            .unwrap();
        store
            .connection
            .execute_batch(
                "DROP TABLE skipped_articles; DROP TABLE ai_projectors; DROP TABLE notice_embeddings; DROP TABLE notice_classifications; DROP TABLE ai_remote_usage;
                 ALTER TABLE ai_models DROP COLUMN kind;
                 ALTER TABLE ai_preferences DROP COLUMN embedding_model_id;
                 ALTER TABLE ai_preferences DROP COLUMN classification_enabled;
                 DROP TABLE IF EXISTS cybercampus_tasks; DROP TABLE IF EXISTS cybercampus_courses;
                 ALTER TABLE settings DROP COLUMN onboarding_completed;
                 ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TABLE calendar_todos; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
                 PRAGMA user_version=10;",
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, super::SCHEMA_VERSION);
    let source = store.source("one").unwrap();
    assert_eq!(source.skipped_count, 0);
    let id = store.document_id("one", "5").unwrap().unwrap();
    assert!(store
        .document(id, None)
        .unwrap()
        .body_text
        .contains("보관 본문"));
}
#[test]
fn schema_eleven_upgrade_retries_mislabeled_and_newly_allowed_resources() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let blocked = "검증된 학교 공개 자료 주소가 아니어서 자동 다운로드하지 않았습니다.";
    let id = {
        let store = Store::open(&path).unwrap();
        store.catalog(&[definition("one", true)]).unwrap();
        let mut input = article("5", "공지", "본문", "2026-01-01");
        input.resources = ["a.png", "b.png", "c.png", "d.pdf"]
            .iter()
            .map(|name| ResourceInput {
                url: format!("https://cdn.example.com/{name}"),
                filename: (*name).into(),
                kind: if name.ends_with(".pdf") {
                    "attachment"
                } else {
                    "image"
                }
                .into(),
            })
            .collect();
        let (id, _) = store.upsert_article("one", &input, true).unwrap();
        store
            .connection
            .execute_batch(&format!(
                "UPDATE resources SET status='too_large',error='파일이 다운로드 크기 제한을 초과했습니다.' WHERE filename='a.png';
                 UPDATE resources SET status='unsupported',error='{blocked}' WHERE filename IN ('b.png','d.pdf');
                 UPDATE resources SET status='unsupported',error='안전한 이미지 형식(PNG, JPEG, GIF, WebP)이 아니어서 보관하지 않았습니다.' WHERE filename='c.png';
                 DROP TABLE ai_projectors;
                 DROP TABLE notice_embeddings; DROP TABLE notice_classifications; DROP TABLE ai_remote_usage;
                 ALTER TABLE ai_models DROP COLUMN kind;
                 ALTER TABLE ai_preferences DROP COLUMN embedding_model_id;
                 ALTER TABLE ai_preferences DROP COLUMN classification_enabled;
                 DROP TABLE IF EXISTS cybercampus_tasks; DROP TABLE IF EXISTS cybercampus_courses;
                 ALTER TABLE settings DROP COLUMN onboarding_completed;
                 ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TABLE calendar_todos; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
                 PRAGMA user_version=11;"
            ))
            .unwrap();
        id
    };
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, super::SCHEMA_VERSION);
    let states = store
        .document(id, None)
        .unwrap()
        .resources
        .into_iter()
        .map(|r| (r.filename, r.status, r.error.is_some()))
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            ("a.png".into(), "pending".into(), false),
            ("b.png".into(), "pending".into(), false),
            ("c.png".into(), "unsupported".into(), true),
            ("d.pdf".into(), "unsupported".into(), true),
        ]
    );
}
#[test]
fn retired_eng_newsletter_row_is_removed_but_archived_data_is_kept() {
    fn downgrade(path: &std::path::Path) {
        Connection::open(path)
            .unwrap()
            .execute_batch(
                "DROP TABLE cybercampus_tasks; DROP TABLE cybercampus_courses;
                 ALTER TABLE settings DROP COLUMN onboarding_completed;
                 ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TABLE calendar_todos; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
                 PRAGMA user_version=16",
            )
            .unwrap();
    }
    let empty = tempfile::tempdir().unwrap();
    let empty_path = empty.path().join("db");
    {
        let store = Store::open(&empty_path).unwrap();
        store
            .catalog(&[definition("eng-newsletter", false)])
            .unwrap();
        downgrade(&empty_path);
    }
    let store = Store::open(&empty_path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, super::SCHEMA_VERSION);
    assert!(store.sources().unwrap().is_empty());

    let archived = tempfile::tempdir().unwrap();
    let archived_path = archived.path().join("db");
    let id = {
        let store = Store::open(&archived_path).unwrap();
        store
            .catalog(&[definition("eng-newsletter", true)])
            .unwrap();
        let (id, _) = store
            .upsert_article(
                "eng-newsletter",
                &article("5", "뉴스레터", "보관 본문", "2026-01-01"),
                true,
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE sources SET supported=0 WHERE id='eng-newsletter'",
                [],
            )
            .unwrap();
        downgrade(&archived_path);
        id
    };
    let store = Store::open(&archived_path).unwrap();
    assert_eq!(store.sources().unwrap().len(), 1);
    assert!(store
        .document(id, None)
        .unwrap()
        .body_text
        .contains("보관 본문"));
}
fn subscribed_ids(store: &Store) -> Vec<String> {
    let mut ids: Vec<_> = store
        .sources()
        .unwrap()
        .into_iter()
        .filter(|source| source.subscribed)
        .map(|source| source.definition.id)
        .collect();
    ids.sort();
    ids
}
#[test]
fn source_groups_stop_their_subtree_and_restore_each_pick() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let store = Store::open(&path).unwrap();
    store.catalog(&crate::archive::definitions()).unwrap();
    for id in [
        "sogang-academic",
        "eng-research",
        "me-general",
        "computing-academic",
    ] {
        store.subscribe(id, true).unwrap();
    }
    let all = [
        "computing-academic",
        "eng-research",
        "me-general",
        "sogang-academic",
    ];
    assert_eq!(subscribed_ids(&store), all);

    // Level 2 stops itself and the departments under it, not its siblings.
    let changed = store.set_source_group("sogang-eng", false).unwrap();
    let mut changed: Vec<_> = changed.into_iter().map(|s| s.definition.id).collect();
    changed.sort();
    assert_eq!(changed, ["eng-research", "me-general"]);
    assert_eq!(
        subscribed_ids(&store),
        ["computing-academic", "sogang-academic"]
    );
    let me = store.source("me-general").unwrap();
    assert!(me.selected && !me.subscribed);
    assert_eq!(me.definition.group.as_deref(), Some("sogang-me"));
    assert_eq!(
        store.subscribe("me-awards", true).unwrap_err(),
        "상위 그룹을 먼저 켜 주세요."
    );

    // Level 1 stops everything; turning it back on keeps level 2 off.
    store.set_source_group("sogang", false).unwrap();
    assert!(subscribed_ids(&store).is_empty());
    assert!(store.subscribe("sogang-academic", false).is_err());
    store.set_source_group("sogang", true).unwrap();
    assert_eq!(
        subscribed_ids(&store),
        ["computing-academic", "sogang-academic"]
    );

    // State survives reopening and a catalog refresh.
    drop(store);
    let store = Store::open(&path).unwrap();
    store.catalog(&crate::archive::definitions()).unwrap();
    let groups = store.source_groups().unwrap();
    assert_eq!(groups[0].id, "sogang");
    assert!(
        !groups
            .iter()
            .find(|g| g.id == "sogang-eng")
            .unwrap()
            .enabled
    );
    assert_eq!(
        subscribed_ids(&store),
        ["computing-academic", "sogang-academic"]
    );
    store.set_source_group("sogang-eng", true).unwrap();
    assert_eq!(subscribed_ids(&store), all);
    // A department off leaves its parent's own boards subscribed.
    store.set_source_group("sogang-me", false).unwrap();
    assert_eq!(
        subscribed_ids(&store),
        ["computing-academic", "eng-research", "sogang-academic"]
    );
    assert_eq!(
        store.set_source_group("missing", false).unwrap_err(),
        "게시판 그룹을 찾을 수 없습니다."
    );
}
#[test]
fn schema_twenty_upgrade_keeps_every_subscription_with_groups_on() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let store = Store::open(&path).unwrap();
        store.catalog(&crate::archive::definitions()).unwrap();
        store.subscribe("se-news", true).unwrap();
        store.subscribe("dorm-menu", true).unwrap();
        store
            .connection
            .execute_batch(
                "ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TABLE calendar_todos; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
                 PRAGMA user_version=20;",
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, super::SCHEMA_VERSION);
    assert!(store.source("se-news").unwrap().selected);
    store.catalog(&crate::archive::definitions()).unwrap();
    assert_eq!(subscribed_ids(&store), ["dorm-menu", "se-news"]);
    assert!(store.source_groups().unwrap().iter().all(|g| g.enabled));
    assert_eq!(store.source_groups().unwrap().len(), 8);
}
#[test]
fn every_public_board_belongs_to_a_known_group() {
    let groups = crate::archive::groups();
    for (index, group) in groups.iter().enumerate() {
        // Parents come first so the settings tree can be built in order.
        assert!(group
            .parent
            .as_ref()
            .is_none_or(|parent| groups[..index].iter().any(|g| &g.id == parent)));
    }
    for source in crate::archive::definitions() {
        match source.group {
            None => assert_eq!(source.family, "cybercampus"),
            Some(group) => assert!(groups.iter().any(|g| g.id == group), "{group}"),
        }
    }
}
#[test]
fn trash_hides_and_restores_then_purge_leaves_a_tombstone_across_reopen() {
    let (directory, store) = database();
    let (kept, _) = store
        .upsert_article(
            "one",
            &article("1", "장학금 안내", "장학 본문", "2026-11-01"),
            false,
        )
        .unwrap();
    let (trashed, _) = store
        .upsert_article(
            "one",
            &article("2", "휴학 신청", "휴학 본문", "2026-11-02"),
            false,
        )
        .unwrap();
    store.set_document_state(trashed, None, Some(true)).unwrap();
    let listed = |store: &Store, trashed: bool| {
        store
            .archive_query(&ArchiveQuery {
                trashed,
                ..Default::default()
            })
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>()
    };

    store.trash_documents(&[trashed, trashed]).unwrap();
    assert_eq!(listed(&store, false), [kept]);
    assert_eq!(listed(&store, true), [trashed]);
    let search = store
        .archive_query(&ArchiveQuery {
            query: "휴학".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(search.items.is_empty());
    let summary = store.archive_summary().unwrap();
    assert_eq!(
        (summary.documents, summary.saved, summary.trashed),
        (1, 0, 1)
    );
    assert!(store
        .sources()
        .unwrap()
        .iter()
        .any(|s| s.definition.id == "one" && s.unread == 1));
    // The reader still opens it from 휴지통.
    let detail = store.document(trashed, None).unwrap();
    assert!(detail.item.trashed && detail.item.saved);
    assert!(store.trash_documents(&[999]).is_err());
    assert!(store.trash_documents(&[]).is_err());

    // Restoring brings back the notice with its saved state.
    store.restore_documents(&[trashed]).unwrap();
    assert_eq!(listed(&store, false), [trashed, kept]);
    assert_eq!(store.archive_summary().unwrap().saved, 1);

    // Only a notice in 휴지통 can be deleted permanently.
    assert!(store.purge_trashed(Some(&[kept])).is_err());
    store.trash_documents(&[trashed]).unwrap();
    store.purge_trashed(None).unwrap();
    assert!(store.document(trashed, None).is_err());
    assert!(store.notice_deleted("one", "2").unwrap());
    assert!(!store.notice_deleted("two", "2").unwrap());
    assert_eq!(store.archive_summary().unwrap().trashed, 0);
    assert_archive_integrity(&store);
    drop(store);

    let store = Store::open(&directory.path().join("db")).unwrap();
    assert!(store
        .upsert_article(
            "one",
            &article("2", "휴학 신청", "휴학 본문", "2026-11-02"),
            false
        )
        .is_err());
    // The same external ID on another board is a different notice.
    assert!(store
        .upsert_article(
            "two",
            &article("2", "휴학 신청", "휴학 본문", "2026-11-02"),
            false
        )
        .is_ok());
    assert_eq!(listed(&store, false).len(), 2);
}
#[test]
fn purge_returns_only_files_no_other_notice_uses() {
    let (_directory, store) = database();
    let mut ids = Vec::new();
    for external in ["1", "2"] {
        let (id, _) = store
            .upsert_article(
                "one",
                &article(external, "공지", "본문", "2026-11-01"),
                false,
            )
            .unwrap();
        ids.push(id);
    }
    let revision = |id: i64| store.document(id, None).unwrap().revision_id;
    let hash = "a".repeat(64);
    for (id, url, path) in [
        (
            ids[0],
            "https://www.sogang.ac.kr/a.png",
            "resources/shared.png",
        ),
        (
            ids[1],
            "https://www.sogang.ac.kr/b.png",
            "resources/shared.png",
        ),
        (
            ids[0],
            "https://www.sogang.ac.kr/c.png",
            "resources/own.png",
        ),
    ] {
        let resource = store
            .add_resource_for_test(id, revision(id), url, "x.png", "image")
            .unwrap();
        store
            .resource_result(
                resource,
                "downloaded",
                Some("image/png"),
                Some(1),
                Some(path),
                Some(&hash),
                None,
            )
            .unwrap();
    }
    store.trash_documents(&[ids[0]]).unwrap();
    assert_eq!(
        store.purge_trashed(Some(&[ids[0]])).unwrap(),
        ["resources/own.png"]
    );
    assert_eq!(store.document(ids[1], None).unwrap().resources.len(), 1);
}
#[test]
fn trashed_notices_leave_automatic_ai_work() {
    let (_directory, store) = database();
    let (kept, _) = store
        .upsert_article(
            "one",
            &article("1", "장학금 안내", "장학 본문", "2026-11-01"),
            false,
        )
        .unwrap();
    let (trashed, _) = store
        .upsert_article(
            "one",
            &article("2", "휴학 신청", "휴학 본문", "2026-11-02"),
            false,
        )
        .unwrap();
    store.trash_documents(&[trashed]).unwrap();
    let backlog = store
        .ai_backlog_candidates(10, false)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(backlog, [kept]);
    let recent = store
        .ai_recent_candidates("1970-01-01T00:00:00Z", 10, false)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    assert_eq!(recent, [kept]);
}
