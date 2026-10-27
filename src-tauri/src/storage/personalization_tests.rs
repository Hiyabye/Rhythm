use super::*;
use crate::archive::model::SourceDefinition;
use crate::storage::personalization::{
    topics, PersonalInterest, PersonalInterestInput, PersonalizationProfile,
};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

fn source_definition() -> SourceDefinition {
    SourceDefinition {
        id: "one".into(),
        name: "학사 공지".into(),
        organization: "서강대학교".into(),
        description: "테스트 정보원".into(),
        url: "https://example.edu/".into(),
        family: "main".into(),
        supported: true,
    }
}

fn add_document(
    store: &Store,
    id: i64,
    title: &str,
    category: &str,
    published_at: &str,
    collected_at: &str,
    historical: bool,
) {
    store
        .connection
        .execute(
            "INSERT INTO documents(
                 id,source_id,external_id,canonical_url,title,category,published_at,collected_at,
                 body_html,body_text,content_hash,historical
             ) VALUES(?1,'one',?2,?3,?4,?5,?6,?7,'<p>본문</p>','본문','hash',?8)",
            params![
                id,
                id.to_string(),
                format!("https://example.edu/{id}"),
                title,
                category,
                published_at,
                collected_at,
                historical
            ],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO document_revisions(
                 document_id,canonical_url,title,category,published_at,original,body_html,body_text,content_hash
             ) VALUES(?1,?2,?3,?4,?5,'{}','<p>본문</p>','본문','hash')",
            params![
                id,
                format!("https://example.edu/{id}"),
                title,
                category,
                published_at
            ],
        )
        .unwrap();
    store
        .connection
        .execute("INSERT INTO document_state(document_id) VALUES(?1)", [id])
        .unwrap();
}

fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.sqlite3")).unwrap();
    store
        .catalog_for_extension("test_extension", &[source_definition()])
        .unwrap();
    store.set_enabled("test_extension", true).unwrap();
    for (id, title, category) in [
        (1, "인공지능 연구 공지", "연구"),
        (2, "인공지능 장학 공지", "장학"),
        (3, "다른 행사 소식", "행사"),
    ] {
        add_document(
            &store,
            id,
            title,
            category,
            "2020-01-01",
            "2020-01-01T12:00:00Z",
            true,
        );
    }
    (directory, store)
}

fn date_offset(days_ago: i64) -> String {
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        / 86_400;
    let z = today - days_ago + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn save_interest(store: &Store, name: &str, priority: u8) -> PersonalInterest {
    store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: name.into(),
            aliases: Vec::new(),
            priority,
            polarity: "positive".into(),
        })
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.name == name)
        .unwrap()
}

fn activity_sequence(store: &Store) -> i64 {
    store
        .connection
        .query_row(
            "SELECT sequence FROM personalization_activity WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn v3_upgrade_preserves_archive_settings_and_ai_data_and_imports_interests() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.sqlite3");
    let legacy_interests = format!(
        "인공지능 · 장학금 · Computer{}Science · Computer   Science · computer science · 분산컴퓨팅 · distributed systems · 시스템프로그래밍 · systems programming · 연구기회 · research · 퀀트금융 · quantitative finance · System{}Programming",
        char::from(9),
        char::from(13)
    );
    {
        let connection = Connection::open(&path).unwrap();
        for migration in [
            include_str!("../../migrations/001_foundation.sql"),
            include_str!("../../migrations/002_archive.sql"),
            include_str!("../../migrations/003_intelligence.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        connection
            .execute(
                "UPDATE settings SET theme='dark',energy_profile='eco',interests=?1 WHERE id=1",
                [&legacy_interests],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sources(id,name,organization,description,url,family,supported)
             VALUES('old-source','학사','서강','공지','https://example.edu/','main',1)",
                [],
            )
            .unwrap();
        connection.execute(
            "UPDATE sources SET subscribed=1,next_page=7,discovered=42,archived=21 WHERE id='old-source'",
            [],
        ).unwrap();
        connection
            .execute(
                "INSERT INTO extension_preferences VALUES('sogang_notices',1)",
                [],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO documents(source_id,external_id,canonical_url,title,published_at,body_html,body_text,content_hash,historical)
             VALUES('old-source','1','https://example.edu/1','보존 공지','2020-01-01','','본문','hash',0)",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO document_revisions(document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash)
             VALUES(1,'https://example.edu/1','보존 공지','2020-01-01','원문','','본문','hash')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO resources(document_id,revision_id,url,filename,kind,status,mime,bytes,local_path,sha256)
             VALUES(1,1,'https://example.edu/guide.pdf','guide.pdf','attachment','downloaded','application/pdf',12,'resources/guide.pdf','digest')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO document_state(document_id,is_read,saved,read_at,saved_at) VALUES(1,1,1,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO ai_models(id,name,ownership,path,status) VALUES('model','로컬 모델','external','/tmp/model','installed')",
            [],
        ).unwrap();
        connection.execute(
            "UPDATE ai_preferences SET active_model_id='model',setup_completed=1,processing_mode='recent',automatic_limit=5,max_tokens=2048 WHERE id=1",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO ai_jobs(id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,status)
             VALUES(1,1,1,'notice_analysis','hash','{}',3,1,'config','completed')",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO ai_results(job_id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,result_json,partial,processed_chars,total_chars,duration_ms)
             VALUES(1,1,1,'notice_analysis','hash','{}',3,1,'config','{}',0,10,10,1)",
            [],
        ).unwrap();
    }

    let store = Store::open(&path).unwrap();
    assert_eq!(store.settings().unwrap().interests, legacy_interests);
    assert_eq!(store.settings().unwrap().theme, "dark");
    assert_eq!(store.settings().unwrap().energy_profile, "eco");
    assert!(store.enabled("sogang_notices").unwrap());
    assert!(store
        .connection
        .query_row(
            "SELECT subscribed FROM sources WHERE id='old-source'",
            [],
            |row| row.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT next_page FROM sources WHERE id='old-source'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        7
    );
    assert!(store.document(1, None).unwrap().item.is_read);
    assert!(store.document(1, None).unwrap().item.saved);
    assert_eq!(store.connection.query_row("SELECT COUNT(*) FROM resources WHERE status='downloaded' AND local_path='resources/guide.pdf'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT active_model_id FROM ai_preferences WHERE id=1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "model"
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM ai_jobs WHERE status='completed'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM ai_models WHERE id='model'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let snapshot = store.personalization_snapshot().unwrap();
    assert_eq!(snapshot.interests.len(), 7, "{:#?}", snapshot.interests);
    let ai = snapshot
        .interests
        .iter()
        .find(|interest| interest.name == "인공지능")
        .unwrap();
    assert!(ai.aliases.iter().any(|alias| alias == "AI"));
    assert_eq!(ai.polarity, "positive");
    let computer = snapshot
        .interests
        .iter()
        .find(|interest| interest.name == "Computer Science")
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT normalized FROM personalization_interests WHERE id=?1",
                [computer.id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "computer science"
    );
    let distributed = snapshot
        .interests
        .iter()
        .find(|interest| interest.name == "분산컴퓨팅")
        .unwrap();
    assert!(distributed
        .aliases
        .iter()
        .any(|alias| alias == "distributed systems"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT normalized FROM personalization_interests WHERE id=?1",
                [distributed.id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "distributed systems"
    );
    for (name, alias) in [
        ("시스템프로그래밍", "systems programming"),
        ("연구기회", "research opportunity"),
        ("퀀트금융", "quantitative finance"),
    ] {
        let interest = snapshot
            .interests
            .iter()
            .find(|interest| interest.name == name)
            .unwrap();
        assert!(interest.aliases.iter().any(|stored| stored == alias));
    }
    store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "computer science".into(),
            aliases: vec![],
            priority: 3,
            polarity: "positive".into(),
        })
        .unwrap();
    let after_edit = store.personalization_snapshot().unwrap();
    assert_eq!(after_edit.interests.len(), 7);
    assert!(store.document(1, None).unwrap().item.saved);
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM ai_results", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT extension_id FROM source_extensions WHERE source_id='old-source'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "sogang_notices"
    );
    assert_eq!(store.info().unwrap().schema_version, 4);

    store.personalization_reset().unwrap();
    assert_eq!(store.settings().unwrap().interests, "");
    assert!(store.document(1, None).unwrap().item.saved);
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM ai_results", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn korean_english_aliases_merge_as_one_explicit_interest() {
    let (_directory, store) = store();
    let first = store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "AI".into(),
            aliases: vec![],
            priority: 5,
            polarity: "positive".into(),
        })
        .unwrap();
    assert!(first.interests[0]
        .aliases
        .iter()
        .any(|alias| alias == "인공지능"));
    let second = store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "인공지능".into(),
            aliases: vec![],
            priority: 2,
            polarity: "positive".into(),
        })
        .unwrap();
    assert_eq!(second.interests.len(), 1);
    assert_eq!(second.interests[0].id, first.interests[0].id);
    assert_eq!(second.interests[0].priority, 2);
}

#[test]
fn profile_interests_feedback_and_learning_preference_persist_after_reopen() {
    let (directory, store) = store();
    let path = directory.path().join("rhythm.sqlite3");
    store
        .personalization_save_profile(&PersonalizationProfile {
            university: "서강대학교".into(),
            department: "컴퓨터공학과".into(),
            study_level: "학부 3학년".into(),
        })
        .unwrap();
    store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "분산 시스템".into(),
            aliases: vec!["distributed systems".into()],
            priority: 4,
            polarity: "positive".into(),
        })
        .unwrap();
    store.personalization_set_learning(false).unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    let snapshot = reopened.personalization_snapshot().unwrap();
    assert_eq!(snapshot.profile.department, "컴퓨터공학과");
    assert_eq!(snapshot.profile.study_level, "학부 3학년");
    assert!(!snapshot.learning_enabled);
    let interest = snapshot
        .interests
        .iter()
        .find(|interest| interest.name == "분산 시스템")
        .unwrap();
    assert_eq!(interest.priority, 4);
    assert!(interest
        .aliases
        .iter()
        .any(|alias| alias == "distributed systems"));
    assert_eq!(
        reopened.personalization_feedback(1).unwrap().as_deref(),
        Some("interested")
    );
}

#[test]
fn behavioral_topic_extraction_excludes_known_sensitive_terms() {
    let korean = topics("장애학생 보조공학 지원", None);
    assert!(korean.iter().all(|topic| !topic.contains("장애")));

    let english = topics("women's disability scholarship", None);
    assert!(english
        .iter()
        .all(|topic| !topic.contains("women") && !topic.contains("disability")));
    assert!(english.contains("scholarship"));
}
#[test]
fn learning_requires_distinct_documents_and_feedback_updates_are_idempotent() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    let sequence = activity_sequence(&store);
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    assert_eq!(activity_sequence(&store), sequence);
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));

    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    let learned = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();
    assert_eq!(learned.evidence_count, 2);
    assert!(learned
        .evidence_summary
        .as_deref()
        .unwrap()
        .contains("서로 다른 2개 공지에서 저장하거나 관심 표시한 행동을 근거로 제안"));

    store
        .personalization_set_feedback(1, Some("not_interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
    assert_eq!(
        store.personalization_feedback(2).unwrap().as_deref(),
        Some("interested")
    );
    store
        .personalization_set_feedback(3, Some("not_interested"))
        .unwrap();
    assert_eq!(
        store.personalization_feedback(3).unwrap().as_deref(),
        Some("not_interested")
    );
    store.personalization_set_feedback(3, None).unwrap();
    assert_eq!(store.personalization_feedback(3).unwrap(), None);
}
#[test]
fn negative_document_feedback_does_not_withdraw_independent_saved_evidence() {
    let (_directory, store) = store();
    store.set_document_state(1, None, Some(true)).unwrap();
    store.set_document_state(2, None, Some(true)).unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));

    store
        .personalization_set_feedback(1, Some("not_interested"))
        .unwrap();
    let learned = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();
    assert_eq!(learned.evidence_count, 2);
    assert_eq!(
        store.personalization_feedback(1).unwrap().as_deref(),
        Some("not_interested")
    );

    store.set_document_state(1, None, Some(false)).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}

#[test]
fn learned_memory_expires_after_retention_window() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));

    store.connection.execute(
        "UPDATE personalization_evidence SET created_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','-181 days')",
        [],
    ).unwrap();
    let cached = store.personalization_snapshot().unwrap();
    assert!(cached
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
    assert!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM personalization_evidence", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
            > 0
    );
    store
        .connection
        .execute(
            "UPDATE personalization_activity SET last_recomputed_at='2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    let snapshot = store.personalization_snapshot().unwrap();
    assert!(!snapshot
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM personalization_evidence", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn learning_disabled_does_not_backfill_unchanged_feedback_or_saves() {
    let (_directory, store) = store();
    store.personalization_set_learning(false).unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    store.set_document_state(1, None, Some(true)).unwrap();
    store.set_document_state(2, None, Some(true)).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));

    store.personalization_set_learning(true).unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    store.set_document_state(2, None, Some(true)).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));

    store.personalization_set_feedback(1, None).unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
    store.personalization_set_feedback(2, None).unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}

#[test]
fn save_learning_is_idempotent_and_unsave_retracts_its_evidence() {
    let (_directory, store) = store();
    store.set_document_state(1, None, Some(true)).unwrap();
    let sequence = activity_sequence(&store);
    store.set_document_state(1, None, Some(true)).unwrap();
    assert_eq!(activity_sequence(&store), sequence);
    store.set_document_state(2, None, Some(true)).unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));

    store.personalization_set_learning(false).unwrap();
    store.set_document_state(1, None, Some(true)).unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
    store.set_document_state(1, None, Some(false)).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
    store.set_document_state(2, None, Some(false)).unwrap();
}
#[test]
fn reading_documents_does_not_create_behavioral_memory() {
    let (_directory, store) = store();
    store.set_document_state(1, Some(true), None).unwrap();
    store.set_document_state(2, Some(true), None).unwrap();
    assert_eq!(activity_sequence(&store), 0);
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
}

#[test]
fn profile_and_interests_change_ranking_with_grounded_reasons() {
    let (_directory, store) = store();
    store
        .connection
        .execute("UPDATE documents SET title='장학 공지' WHERE id=2", [])
        .unwrap();
    save_interest(&store, "장학", 5);
    let scholarship = store.personalization_recommendations(3).unwrap();
    assert_eq!(scholarship.items[0].item.id, 2);
    assert!(scholarship.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘장학’")));

    store.personalization_reset().unwrap();
    save_interest(&store, "AI", 5);
    let artificial_intelligence = store.personalization_recommendations(3).unwrap();
    assert_eq!(artificial_intelligence.items[0].item.id, 1);
    assert!(artificial_intelligence.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘AI’")));
    assert!(artificial_intelligence.items[0]
        .item
        .title
        .contains("인공지능"));
}

#[test]
fn ranking_uses_full_indexed_body_beyond_the_display_snippet() {
    let (_directory, store) = store();
    let body = format!("{}분산 시스템 연구 기회", "무관한 본문 내용 ".repeat(60));
    assert!(body.find("분산 시스템").unwrap() > 320);
    store
        .connection
        .execute(
            "UPDATE documents SET title='일반 소식',category='일반',body_text=?1 WHERE id=1",
            [&body],
        )
        .unwrap();
    save_interest(&store, "분산 시스템", 5);

    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(recommendations.items[0].item.id, 1);
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘분산 시스템’")));
    assert!(!recommendations.items[0]
        .item
        .snippet
        .contains("분산 시스템"));
}

#[test]
fn short_interest_terms_retrieve_old_documents_outside_recent_candidates() {
    let (_directory, store) = store();
    add_document(
        &store,
        4,
        "ML systems research opportunity",
        "Research",
        "2000-01-01",
        "2000-01-01T12:00:00Z",
        true,
    );
    for id in 10..520 {
        add_document(
            &store,
            id,
            &format!("Unrelated historical notice {id}"),
            "General",
            "2000-01-01",
            "2000-01-01T12:00:00Z",
            true,
        );
    }
    save_interest(&store, "ML", 5);

    let recommendations = store.personalization_recommendations(8).unwrap();
    let match_item = recommendations
        .items
        .iter()
        .find(|recommendation| recommendation.item.id == 4)
        .unwrap();
    assert!(match_item
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘ML’")));
}
#[test]
fn explicit_lower_interest_topics_reduce_relevance_and_can_be_corrected() {
    let (_directory, store) = store();
    store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "장학".into(),
            aliases: vec![],
            priority: 5,
            polarity: "negative".into(),
        })
        .unwrap();
    let lower_interest = store.personalization_recommendations(3).unwrap();
    assert_eq!(lower_interest.items.len(), 3);
    assert_eq!(lower_interest.items.last().unwrap().item.id, 2);
    assert!(lower_interest
        .items
        .last()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.contains("덜 보고 싶은 분야 ‘장학’")));

    save_interest(&store, "장학", 5);
    assert_eq!(
        store.personalization_recommendations(3).unwrap().items[0]
            .item
            .id,
        2
    );
}
#[test]
fn korean_english_interest_aliases_match_notice_terms() {
    let (_directory, store) = store();
    store.connection.execute(
        "UPDATE documents SET title='Scholarship for undergraduate study',category='Student funding' WHERE id=3",
        [],
    ).unwrap();
    let interest = save_interest(&store, "장학", 5);
    assert!(interest.aliases.iter().any(|alias| alias == "scholarship"));
    let recommendations = store.personalization_recommendations(3).unwrap();
    let english_notice = recommendations
        .items
        .iter()
        .find(|item| item.item.id == 3)
        .unwrap();
    assert!(english_notice
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘장학’")));

    store.personalization_reset().unwrap();
    store.connection.execute(
        "UPDATE documents SET title='Distributed systems internships',category='Computing' WHERE id=3",
        [],
    ).unwrap();
    let distributed = save_interest(&store, "분산 시스템", 5);
    assert!(distributed
        .aliases
        .iter()
        .any(|alias| alias == "distributed systems"));
    let recommendations = store.personalization_recommendations(3).unwrap();
    let english_notice = recommendations
        .items
        .iter()
        .find(|item| item.item.id == 3)
        .unwrap();
    assert!(english_notice
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘분산 시스템’")));
}
#[test]
fn explicit_priority_outranks_a_weak_learned_interest() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    save_interest(&store, "장학", 1);
    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(recommendations.items[0].item.id, 2);
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("장학")));
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("인공지능")));
}

#[test]
fn negative_feedback_is_document_scoped_and_beats_positive_relevance() {
    let (_directory, store) = store();
    save_interest(&store, "장학", 5);
    store
        .personalization_set_feedback(2, Some("not_interested"))
        .unwrap();
    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(recommendations.items.last().unwrap().item.id, 2);
    assert!(recommendations
        .items
        .last()
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 없음")));
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
}
#[test]
fn subscribed_source_is_a_small_explainable_preference() {
    let (_directory, store) = store();
    store
        .catalog_for_extension(
            "test_extension",
            &[
                source_definition(),
                SourceDefinition {
                    id: "two".into(),
                    name: "다른 정보원".into(),
                    organization: "연구소".into(),
                    description: "두 번째 정보원".into(),
                    url: "https://research.example.edu/".into(),
                    family: "research".into(),
                    supported: true,
                },
            ],
        )
        .unwrap();
    store
        .connection
        .execute("UPDATE documents SET source_id='two' WHERE id=3", [])
        .unwrap();
    store.subscribe("one", true).unwrap();

    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(recommendations.items[0].item.source_id, "one");
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason == "구독 중인 정보원"));
    save_interest(&store, "연구소", 5);
    let organization_match = store.personalization_recommendations(3).unwrap();
    assert_eq!(organization_match.items[0].item.source_id, "two");
    assert!(organization_match.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘연구소’")));
}

#[test]
fn publication_freshness_collection_recency_and_backfill_are_distinct() {
    let (_directory, store) = store();
    let today = date_offset(0);
    let old = date_offset(60);
    store
        .connection
        .execute(
            "UPDATE documents SET published_at=?1,collected_at=?2,historical=0 WHERE id=1",
            params![today, format!("{today}T12:00:00Z")],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET published_at=?1,collected_at=?2,historical=1 WHERE id=2",
            params![today, format!("{today}T12:00:00Z")],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET last_revision_at=?1 WHERE id=2",
            [format!("{today}T12:00:00Z")],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET published_at=?1,collected_at=?2,historical=0 WHERE id=3",
            params![old, format!("{today}T12:00:00Z")],
        )
        .unwrap();
    save_interest(&store, "distributed systems", 1);

    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(
        recommendations
            .items
            .iter()
            .map(|item| item.item.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason == "최근 게시"));
    assert!(recommendations.items[1]
        .reasons
        .iter()
        .any(|reason| reason == "최근 내용이 갱신된 공지"));
    assert!(recommendations.items[2]
        .reasons
        .iter()
        .any(|reason| reason == "최근 보관된 오래된 공지"));
}
#[test]
fn recently_updated_historical_documents_are_bounded_candidates() {
    let (_directory, store) = store();
    let today = date_offset(0);
    store.connection.execute(
        "UPDATE documents SET published_at='2020-01-01',collected_at='2020-01-01T00:00:00Z',historical=1 WHERE id<=3",
        [],
    ).unwrap();
    for id in 4..=650 {
        add_document(
            &store,
            id,
            "Historical notice",
            "학사",
            "2020-01-01",
            "2020-01-01T00:00:00Z",
            true,
        );
    }
    store
        .connection
        .execute(
            "UPDATE documents SET title='Updated historical notice',last_revision_at=?1 WHERE id=1",
            [format!("{today}T12:00:00Z")],
        )
        .unwrap();
    save_interest(&store, "quantitative finance", 1);

    let recommendations = store.personalization_recommendations(1).unwrap();
    assert_eq!(recommendations.items[0].item.id, 1);
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason == "최근 내용이 갱신된 공지"));
}

#[test]
fn malformed_publication_dates_do_not_receive_freshness_reasons() {
    let (_directory, store) = store();
    store.connection.execute("UPDATE documents SET published_at='2026-02-31',collected_at='2020-01-01T00:00:00Z',historical=0 WHERE id=1", []).unwrap();
    save_interest(&store, "systems", 1);
    let item = store
        .personalization_recommendations(3)
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.item.id == 1)
        .unwrap();
    assert!(!item.reasons.iter().any(|reason| reason.contains("최근")));
}
#[test]
fn partial_ai_deadline_candidates_do_not_change_source_based_ranking() {
    let (_directory, store) = store();
    save_interest(&store, "인공지능", 5);
    let before = store.personalization_recommendations(3).unwrap();
    store.connection.execute(
        "INSERT INTO ai_jobs(id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,status)
         VALUES(1,1,1,'notice_analysis','model-hash','{}',3,1,'config','completed')",
        [],
    ).unwrap();
    store.connection.execute(
        "INSERT INTO ai_results(job_id,document_id,revision_id,task,model_sha256,model_identity_json,prompt_version,schema_version,configuration_hash,result_json,partial,processed_chars,total_chars,duration_ms)
         VALUES(1,1,1,'notice_analysis','model-hash','{}',3,1,'config',
                '{\"facts\":[{\"kind\":\"deadline\",\"label\":\"마감\",\"value\":\"2099-02-31\",\"evidence_id\":\"source-1\"}]}',
                1,10,10,1)",
        [],
    ).unwrap();
    let after = store.personalization_recommendations(3).unwrap();
    assert_eq!(
        before
            .items
            .iter()
            .map(|item| item.item.id)
            .collect::<Vec<_>>(),
        after
            .items
            .iter()
            .map(|item| item.item.id)
            .collect::<Vec<_>>()
    );
    assert!(after
        .items
        .iter()
        .all(|item| !item.reasons.iter().any(|reason| reason.contains("마감"))));
}
#[test]
fn old_behavioral_evidence_and_document_feedback_no_longer_dominate_ranking() {
    let (_directory, store) = store();
    save_interest(&store, "AI", 1);
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    assert_eq!(
        store.personalization_recommendations(3).unwrap().items[0]
            .item
            .id,
        1
    );
    store.connection.execute(
        "UPDATE personalization_evidence SET created_at='2000-01-01T00:00:00Z' WHERE document_id=1",
        [],
    ).unwrap();
    store.connection.execute(
        "UPDATE personalization_feedback SET updated_at='2000-01-01T00:00:00Z' WHERE document_id=1",
        [],
    ).unwrap();

    store
        .connection
        .execute(
            "UPDATE personalization_activity SET last_recomputed_at='2000-01-01T00:00:00Z'",
            [],
        )
        .unwrap();
    let snapshot = store.personalization_snapshot().unwrap();
    assert!(!snapshot
        .interests
        .iter()
        .any(|interest| interest.origin == "learned"));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM personalization_evidence WHERE document_id=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    let recommendations = store.personalization_recommendations(3).unwrap();
    assert_eq!(recommendations.items[0].item.id, 2);
    assert!(!recommendations
        .items
        .iter()
        .find(|item| item.item.id == 1)
        .unwrap()
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 표시한 공지")));
}

#[test]
fn disabled_extension_items_leave_recommendations_but_remain_in_archive() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store.set_enabled("test_extension", false).unwrap();
    assert!(store
        .personalization_recommendations(10)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        store.personalization_feedback(1).unwrap().as_deref(),
        Some("interested")
    );
    assert_eq!(
        store
            .archive_query(&crate::archive::model::ArchiveQuery::default())
            .unwrap()
            .items
            .len(),
        3
    );

    store.set_enabled("test_extension", true).unwrap();
    assert!(!store
        .personalization_recommendations(10)
        .unwrap()
        .items
        .is_empty());
}
#[test]
fn missing_extension_preference_is_disabled_for_recommendations_only() {
    let (_directory, store) = store();
    store
        .connection
        .execute(
            "DELETE FROM extension_preferences WHERE extension_id='test_extension'",
            [],
        )
        .unwrap();
    assert!(store
        .personalization_recommendations(10)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        store
            .archive_query(&crate::archive::model::ArchiveQuery::default())
            .unwrap()
            .items
            .len(),
        3
    );
}

#[test]
fn cold_start_is_useful_unpersonalized_and_deterministic() {
    let (_directory, store) = store();
    let first = store.personalization_recommendations(2).unwrap();
    let second = store.personalization_recommendations(2).unwrap();
    assert!(!first.has_preferences);
    assert_eq!(first.items.len(), 2);
    assert!(first.items.iter().all(|item| item.reasons.is_empty()));
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| item.item.id)
            .collect::<Vec<_>>(),
        second
            .items
            .iter()
            .map(|item| item.item.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn deleting_memory_suppresses_unchanged_evidence_until_new_activity() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    let learned = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();
    store.personalization_remove_interest(learned.id).unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));

    store.personalization_set_feedback(1, None).unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    assert!(store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}
#[test]
fn renaming_a_learned_memory_suppresses_its_old_topic_from_unchanged_evidence() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    let learned = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();

    store
        .personalization_save_interest(&PersonalInterestInput {
            id: Some(learned.id),
            name: "시스템 프로그래밍".into(),
            aliases: vec!["AI".into()],
            priority: 3,
            polarity: "positive".into(),
        })
        .unwrap();
    store
        .personalization_set_feedback(3, Some("not_interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
    let published = date_offset(1);
    let collected = format!("{published}T12:00:00Z");
    add_document(
        &store,
        4,
        "인공지능 연구 기회 새 공지",
        "연구",
        &published,
        &collected,
        false,
    );
    store
        .personalization_set_feedback(4, Some("interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}

#[test]
fn deleting_an_explicit_interest_also_suppresses_retained_behavioral_evidence() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    let explicit = store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "인공지능".into(),
            aliases: vec![],
            priority: 3,
            polarity: "positive".into(),
        })
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.name == "인공지능")
        .unwrap();
    store.personalization_remove_interest(explicit.id).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}

#[test]
fn deleting_explicit_interest_suppresses_unchanged_evidence_for_its_aliases() {
    let (_directory, store) = store();
    let explicit = store
        .personalization_save_interest(&PersonalInterestInput {
            id: None,
            name: "Machine Learning".into(),
            aliases: vec!["AI".into()],
            priority: 3,
            polarity: "positive".into(),
        })
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.name == "Machine Learning")
        .unwrap();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));

    store.personalization_remove_interest(explicit.id).unwrap();
    assert!(!store
        .personalization_snapshot()
        .unwrap()
        .interests
        .iter()
        .any(|interest| interest.origin == "learned" && interest.name == "인공지능"));
}

#[test]
fn learned_interest_id_remains_stable_when_evidence_count_changes() {
    let (_directory, store) = store();
    store
        .personalization_set_feedback(1, Some("interested"))
        .unwrap();
    store
        .personalization_set_feedback(2, Some("interested"))
        .unwrap();
    let before = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET title='인공지능 시스템 공지' WHERE id=3",
            [],
        )
        .unwrap();
    store
        .personalization_set_feedback(3, Some("interested"))
        .unwrap();
    let after = store
        .personalization_snapshot()
        .unwrap()
        .interests
        .into_iter()
        .find(|interest| interest.origin == "learned" && interest.name == "인공지능")
        .unwrap();
    assert_eq!(before.id, after.id);
    assert_eq!(after.evidence_count, 3);
}

#[test]
fn source_diversity_is_bounded_and_explained_when_it_changes_selection() {
    let (_directory, store) = store();
    store
        .catalog_for_extension(
            "test_extension",
            &[
                source_definition(),
                SourceDefinition {
                    id: "two".into(),
                    name: "연구 공지".into(),
                    organization: "연구소".into(),
                    description: "두 번째 정보원".into(),
                    url: "https://research.example.edu/".into(),
                    family: "research".into(),
                    supported: true,
                },
            ],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET source_id='two',title='일반 연구 안내' WHERE id=3",
            [],
        )
        .unwrap();
    for id in [4, 5] {
        add_document(
            &store,
            id,
            "AI systems research",
            "research",
            "2020-01-01",
            "2020-01-01T12:00:00Z",
            true,
        );
    }
    save_interest(&store, "인공지능", 5);
    let recommendations = store.personalization_recommendations(4).unwrap();
    assert!(
        recommendations
            .items
            .iter()
            .filter(|item| item.item.source_id == "one")
            .count()
            <= 3
    );
    assert!(recommendations
        .items
        .iter()
        .any(|item| item.item.source_id == "two"
            && item
                .reasons
                .iter()
                .any(|reason| reason.contains("출처 편중"))));
}

#[test]
fn ranking_candidate_retrieval_is_bounded_and_uses_local_fts_for_older_matches() {
    let (_directory, store) = store();
    store
        .connection
        .execute(
            "UPDATE documents SET title='RareSystemsTopic' WHERE id=1",
            [],
        )
        .unwrap();
    let tx = store.connection.unchecked_transaction().unwrap();
    for id in 4..2004 {
        tx.execute(
            "INSERT INTO documents(id,source_id,external_id,canonical_url,title,published_at,collected_at,body_html,body_text,content_hash,historical)
             VALUES(?1,'one',?2,?3,'일반 공지','2020-01-01','2020-01-01T00:00:00Z','본문','본문','hash',1)",
            params![id, id.to_string(), format!("https://example.edu/{id}")],
        ).unwrap();
    }
    tx.commit().unwrap();
    for index in 0..32 {
        let topic = if index == 0 {
            "RareSystemsTopic".to_string()
        } else {
            format!("otherSystemsTopic{index}")
        };
        save_interest(&store, &topic, 5);
    }
    let start = std::time::Instant::now();
    let result = store.personalization_recommendations(10).unwrap();
    eprintln!(
        "bounded personalization ranking for 2,003 documents: {:?}",
        start.elapsed()
    );
    assert_eq!(result.items.len(), 10);
    assert!(result.items.iter().any(|item| item.item.id == 1));
}
#[test]
fn short_korean_interest_finds_old_category_matches_outside_recent_candidates() {
    let (_directory, store) = store();
    store
        .connection
        .execute(
            "UPDATE documents SET title='Older funding notice',category='장학' WHERE id=1",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET title='Different event',category='행사' WHERE id=2",
            [],
        )
        .unwrap();
    let tx = store.connection.unchecked_transaction().unwrap();
    for id in 4..604 {
        tx.execute(
            "INSERT INTO documents(id,source_id,external_id,canonical_url,title,category,published_at,collected_at,body_html,body_text,content_hash,historical)
             VALUES(?1,'one',?2,?3,'일반 공지','일반','2020-01-01','2020-01-01T00:00:00Z','본문','본문','hash',1)",
            params![id, id.to_string(), format!("https://example.edu/{id}")],
        ).unwrap();
    }
    tx.commit().unwrap();
    save_interest(&store, "장학", 5);

    let recommendations = store.personalization_recommendations(1).unwrap();
    assert_eq!(recommendations.items[0].item.id, 1);
    assert!(recommendations.items[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("관심 분야 ‘장학’")));
}
