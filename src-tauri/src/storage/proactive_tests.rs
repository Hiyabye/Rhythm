use super::proactive::{
    NotificationSensitivity, ProactiveClaim, ProactiveEventKey, ProactiveEventKind,
    ProactiveEventState, ProactiveOutcome, ProactiveSettings, ProactiveTaskInput, QuietHours,
    SchedulerOutcome,
};
use super::Store;
use rusqlite::{params, Connection};
use std::path::Path;
use tempfile::TempDir;

fn store() -> (TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("rhythm.db")).unwrap();
    (directory, store)
}

fn count(store: &Store, sql: &str) -> i64 {
    store
        .connection
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn add_source(store: &Store, id: &str) {
    store
        .connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed)
             VALUES(?1,'테스트 정보원','기관','설명','https://example.edu/','main',1,1)",
            [id],
        )
        .unwrap();
}

fn add_revision(store: &Store, document_id: i64, ordinal: usize) -> i64 {
    store
        .connection
        .execute(
            "INSERT INTO document_revisions(
                 document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash
             ) VALUES(?1,?2,?3,'2026-01-01T00:00:00Z','{}','<p>본문</p>','본문',?4)",
            params![
                document_id,
                format!("https://example.edu/{document_id}?r={ordinal}"),
                format!("공지 {document_id} v{ordinal}"),
                format!("hash-{document_id}-{ordinal}")
            ],
        )
        .unwrap();
    store.connection.last_insert_rowid()
}

fn add_document(store: &Store, id: i64, source: &str, historical: bool) -> i64 {
    store
        .connection
        .execute(
            "INSERT INTO documents(
                 id,source_id,external_id,canonical_url,title,published_at,historical,
                 body_html,body_text,content_hash
             ) VALUES(?1,?2,?3,?4,?5,'2026-01-01T00:00:00Z',?6,'<p>본문</p>','본문','hash')",
            params![
                id,
                source,
                id.to_string(),
                format!("https://example.edu/{id}"),
                format!("공지 {id}"),
                historical
            ],
        )
        .unwrap();
    add_revision(store, id, 0)
}

fn key(document_id: i64, revision_id: i64, event_type: ProactiveEventKind) -> ProactiveEventKey {
    ProactiveEventKey {
        document_id,
        revision_id,
        event_type,
    }
}
fn claimed_event(store: &Store, key: &ProactiveEventKey) -> super::proactive::ProactiveEvent {
    let ProactiveClaim::Claimed {
        event_id,
        attempt_count: 0,
    } = store.proactive_claim_event(key).unwrap()
    else {
        panic!("expected pending registration");
    };
    store.proactive_claim_due_event(event_id).unwrap()
}

/// Builds a genuine schema-4 database so migration 5 is exercised on exactly
/// the state a released v0.4.0 installation would have.
fn version_four_database(path: &Path) {
    let connection = Connection::open(path).unwrap();
    for migration in [
        include_str!("../../migrations/001_foundation.sql"),
        include_str!("../../migrations/002_archive.sql"),
        include_str!("../../migrations/003_intelligence.sql"),
        include_str!("../../migrations/004_personalization.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute(
            "UPDATE settings SET theme='dark',energy_profile='eco',interests='컴퓨터 보안'
             WHERE id=1",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported)
             VALUES('one','학사 공지','서강대학교','설명','https://example.edu/','main',1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported)
             VALUES('legacy','이전 정보원','서강대학교','설명','https://example.edu/legacy','main',1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,history_complete)
             VALUES('done','완료 정보원','서강대학교','설명','https://example.edu/done','main',1,1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported)
             VALUES('empty','빈 정보원','서강대학교','설명','https://example.edu/empty','main',1)",
            [],
        )
        .unwrap();
    for (id, source, historical) in [(1_i64, "one", false), (2, "legacy", true)] {
        connection
            .execute(
                "INSERT INTO documents(
                     id,source_id,external_id,canonical_url,title,published_at,historical,
                     body_html,body_text,content_hash
                 ) VALUES(?1,?2,?3,?4,?5,'2026-01-01T00:00:00Z',?6,'<p>본문</p>','본문','hash')",
                params![
                    id,
                    source,
                    id.to_string(),
                    format!("https://example.edu/{id}"),
                    format!("공지 {id}"),
                    historical
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_revisions(
                     document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash
                 ) VALUES(?1,?2,?3,'2026-01-01T00:00:00Z','{}','<p>본문</p>','본문','hash')",
                params![
                    id,
                    format!("https://example.edu/{id}"),
                    format!("공지 {id}")
                ],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO extension_preferences(extension_id,enabled) VALUES('sogang_notices',1)",
            [],
        )
        .unwrap();
}

#[test]
fn migration_five_is_additive_and_keeps_version_four_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    version_four_database(&path);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.info().unwrap().schema_version, 5);
    assert_eq!(store.settings().unwrap().theme, "dark");
    assert_eq!(store.settings().unwrap().energy_profile, "eco");
    assert_eq!(store.archive_summary().unwrap().documents, 2);
    assert!(store.enabled("sogang_notices").unwrap());

    let defaults = store.proactive_settings().unwrap();
    assert!(!defaults.close_to_background);
    assert!(!defaults.launch_at_login);
    assert!(!defaults.start_hidden);
    assert!(!defaults.scheduler_enabled);
    assert!(!defaults.scheduler_paused);
    assert_eq!(defaults.scheduler_interval_minutes, 0);
    assert_eq!(defaults.scheduler_retry_minutes, 15);
    assert!(!defaults.notifications_enabled);
    assert_eq!(
        defaults.notification_sensitivity,
        NotificationSensitivity::Important
    );
    assert!(!defaults.quiet_hours.enabled);

    // Existing archive is the baseline: nothing from before the upgrade may
    // ever become an event.
    assert_eq!(store.proactive_baseline().unwrap(), 2);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 0);
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, 1, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Baseline { .. }
    ));

    assert!(store.seed_complete("one").unwrap());
    assert!(store.seed_complete("done").unwrap());
    // v0.4.0's ambiguous legacy seed (historical rows, incomplete history)
    // must stay incomplete so sync re-runs the bounded seed once.
    assert!(!store.seed_complete("legacy").unwrap());
    assert!(!store.seed_complete("empty").unwrap());

    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.info().unwrap().schema_version, 5);
    assert_eq!(reopened.proactive_baseline().unwrap(), 2);
    assert_eq!(reopened.archive_summary().unwrap().documents, 2);
}

#[test]
fn failed_migration_five_rolls_back_completely() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    version_four_database(&path);

    {
        let connection = Connection::open(&path).unwrap();
        // Collides with a late CREATE TABLE in migration 5.
        connection
            .execute_batch("CREATE TABLE proactive_events(sentinel TEXT);")
            .unwrap();
    }

    assert!(Store::open(&path).is_err());

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE name IN ('proactive_settings','proactive_scheduler','proactive_tasks',
                                'proactive_baseline','proactive_dedupe')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM proactive_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn default_settings_are_opt_in_and_validation_never_mutates() {
    let (_directory, store) = store();
    let defaults: ProactiveSettings = store.proactive_settings().unwrap();
    assert!(!defaults.updated_at.is_empty());
    assert_eq!(defaults.scheduler_interval_minutes, 0);
    assert!(!defaults.close_to_background);
    assert!(!defaults.scheduler_enabled);
    assert!(!defaults.notifications_enabled);
    assert_eq!(
        defaults.notification_sensitivity,
        NotificationSensitivity::Important
    );
    assert_eq!(defaults.quiet_hours.start, "22:00");
    assert_eq!(defaults.quiet_hours.end, "07:00");

    let local: String = store
        .connection
        .query_row("SELECT strftime('%H:%M','now','localtime')", [], |row| {
            row.get(0)
        })
        .unwrap();
    let (hour, minute) = local.split_once(':').unwrap();
    assert_eq!(
        store.proactive_local_minutes().unwrap(),
        hour.parse::<u16>().unwrap() * 60 + minute.parse::<u16>().unwrap()
    );

    let mut settings = defaults.clone();
    settings.close_to_background = true;
    settings.launch_at_login = true;
    settings.start_hidden = true;
    settings.scheduler_enabled = true;
    settings.scheduler_paused = true;
    settings.scheduler_interval_minutes = 60;
    settings.scheduler_retry_minutes = 10;
    settings.notifications_enabled = true;
    settings.notification_sensitivity = NotificationSensitivity::Interested;
    settings.quiet_hours = QuietHours {
        enabled: true,
        start: "9:00".into(),
        end: "17:30".into(),
    };
    let saved = store.proactive_save_settings(&settings).unwrap();
    assert!(saved.close_to_background);
    assert!(saved.start_hidden);
    assert_eq!(saved.scheduler_interval_minutes, 60);
    assert_eq!(
        saved.notification_sensitivity,
        NotificationSensitivity::Interested
    );
    assert_eq!(saved.quiet_hours.start, "09:00");
    assert_eq!(saved.quiet_hours.end, "17:30");

    let mut invalid = saved.clone();
    invalid.scheduler_interval_minutes = 5;
    assert!(store.proactive_save_settings(&invalid).is_err());
    invalid = saved.clone();
    invalid.scheduler_retry_minutes = 240;
    assert!(store.proactive_save_settings(&invalid).is_err());
    invalid = saved.clone();
    invalid.quiet_hours.enabled = true;
    invalid.quiet_hours.start = "24:00".into();
    assert!(store.proactive_save_settings(&invalid).is_err());
    invalid = saved.clone();
    invalid.quiet_hours.enabled = true;
    invalid.quiet_hours.start = "07:00".into();
    invalid.quiet_hours.end = "07:00".into();
    assert!(store.proactive_save_settings(&invalid).is_err());
    assert_eq!(store.proactive_settings().unwrap(), saved);

    let overnight = QuietHours {
        enabled: true,
        start: "22:00".into(),
        end: "07:00".into(),
    };
    assert!(overnight.contains(23 * 60));
    assert!(overnight.contains(6 * 60 + 59));
    assert!(!overnight.contains(7 * 60));
    assert!(!overnight.contains(12 * 60));
    let disabled = QuietHours {
        enabled: false,
        ..overnight.clone()
    };
    assert!(!disabled.contains(23 * 60));
}

#[test]
fn baseline_and_notification_opt_in_suppress_old_notices_but_allow_historical_revisions() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let historical = add_document(&store, 1, "one", true);
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, historical, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Historical { document_id: 1 }
    ));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 0);

    // Enabling notifications alone (scheduler still off) establishes the
    // baseline, so already archived notices cannot fire later.
    assert_eq!(store.proactive_baseline().unwrap(), 0);
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = true;
    store.proactive_save_settings(&settings).unwrap();
    assert_eq!(store.proactive_baseline().unwrap(), historical);
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, historical, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Baseline { .. }
    ));

    // Enabling the scheduler also advances it, even with no new revisions.
    let existing = add_document(&store, 2, "one", false);
    settings.scheduler_enabled = true;
    store.proactive_save_settings(&settings).unwrap();
    assert_eq!(store.proactive_baseline().unwrap(), existing);
    assert!(matches!(
        store
            .proactive_claim_event(&key(2, existing, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Baseline { .. }
    ));

    // A historical document's later revision is a real update after opt-in.
    let historical_revision = add_revision(&store, 1, 1);
    let event = claimed_event(
        &store,
        &key(1, historical_revision, ProactiveEventKind::Revision),
    );
    assert_eq!(event.document_id, 1);
    // Its New identity is still historical backfill and never notifies.
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, historical_revision, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Historical { document_id: 1 }
    ));

    let newer = add_revision(&store, 2, 1);
    assert!(matches!(
        store
            .proactive_claim_event(&key(2, newer, ProactiveEventKind::Revision))
            .unwrap(),
        ProactiveClaim::Claimed { .. }
    ));
}

#[test]
fn dedupe_is_atomic_and_survives_reopen_and_history_clear() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let revision;
    {
        let store = Store::open(&path).unwrap();
        add_source(&store, "one");
        revision = add_document(&store, 1, "one", false);
        let identity = key(1, revision, ProactiveEventKind::New);

        let event = claimed_event(&store, &identity);
        assert_eq!(event.state, ProactiveEventState::Claimed);
        assert_eq!(event.attempt_count, 1);
        assert!(matches!(
            store.proactive_claim_event(&identity).unwrap(),
            ProactiveClaim::Duplicate {
                previous_event_id: Some(id)
            } if id == event.id
        ));

        let delivered = store
            .proactive_mark_delivered(event.id, event.attempt_count)
            .unwrap();
        assert_eq!(delivered.state, ProactiveEventState::Delivered);
        assert!(delivered.delivered_at.is_some());
        assert_eq!(store.proactive_clear_history().unwrap(), 1);
        assert!(store.proactive_history(10).unwrap().is_empty());
    }

    let store = Store::open(&path).unwrap();
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, revision, ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Duplicate {
            previous_event_id: None
        }
    ));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 0);
    // Event type is part of the identity, not a free re-notification.
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, revision, ProactiveEventKind::Revision))
            .unwrap(),
        ProactiveClaim::Claimed { .. }
    ));
}

#[test]
fn invalid_claims_leave_no_partial_state() {
    let (_directory, store) = store();
    add_source(&store, "one");
    add_document(&store, 1, "one", false);
    let other = add_document(&store, 2, "one", false);

    assert!(store
        .proactive_claim_event(&key(1, other, ProactiveEventKind::New))
        .is_err());
    assert!(store
        .proactive_claim_event(&key(99, 1, ProactiveEventKind::New))
        .is_err());
    assert!(store
        .proactive_claim_event(&key(1, 0, ProactiveEventKind::New))
        .is_err());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 0);
}

#[test]
fn delivery_attempts_are_fenced_and_terminal_after_three_failures() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let revision = add_document(&store, 1, "one", false);
    let event = claimed_event(&store, &key(1, revision, ProactiveEventKind::New));
    assert_eq!(event.attempt_count, 1);

    assert!(store.proactive_mark_delivered(event.id, 2).is_err());
    assert_eq!(
        store.proactive_event(event.id).unwrap().state,
        ProactiveEventState::Claimed
    );

    let deferred = store
        .proactive_defer_event(event.id, 1, 30, Some("quiet hours"))
        .unwrap();
    assert_eq!(deferred.state, ProactiveEventState::Deferred);
    assert!(deferred.deferred_until.is_some());
    assert!(!store
        .proactive_due_events(10)
        .unwrap()
        .iter()
        .any(|due| due.id == event.id));
    assert!(store.proactive_defer_event(event.id, 1, 30, None).is_err());

    store
        .connection
        .execute(
            "UPDATE proactive_events SET deferred_until='2000-01-01T00:00:00Z' WHERE id=?1",
            [event.id],
        )
        .unwrap();
    assert!(store
        .proactive_due_events(10)
        .unwrap()
        .iter()
        .any(|due| due.id == event.id));
    let claimed = store.proactive_claim_due_event(event.id).unwrap();
    assert_eq!(claimed.attempt_count, 2);
    let retry = store
        .proactive_fail_delivery(event.id, 2, "전달 실패")
        .unwrap();
    assert_eq!(retry.state, ProactiveEventState::Pending);
    assert_eq!(retry.attempt_count, 2);
    assert_eq!(retry.error.as_deref(), Some("전달 실패"));
    assert!(retry.deferred_until.is_some());

    store
        .connection
        .execute(
            "UPDATE proactive_events SET deferred_until='2000-01-01T00:00:00Z' WHERE id=?1",
            [event.id],
        )
        .unwrap();
    assert_eq!(
        store
            .proactive_claim_due_event(event.id)
            .unwrap()
            .attempt_count,
        3
    );
    let failed = store
        .proactive_fail_delivery(event.id, 3, "전달 실패")
        .unwrap();
    assert_eq!(failed.state, ProactiveEventState::Failed);
    assert!(store.proactive_claim_due_event(event.id).is_err());
    assert!(store.proactive_due_events(10).unwrap().is_empty());
}

#[test]
fn quiet_hour_deferral_preserves_zero_attempts_across_reopen_then_delivery_claims_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let revision;
    let event_id;
    {
        let store = Store::open(&path).unwrap();
        add_source(&store, "one");
        revision = add_document(&store, 1, "one", false);
        let ProactiveClaim::Claimed {
            event_id: id,
            attempt_count,
        } = store
            .proactive_claim_event(&key(1, revision, ProactiveEventKind::New))
            .unwrap()
        else {
            panic!("expected pending registration");
        };
        event_id = id;
        assert_eq!(attempt_count, 0);
        for _ in 0..3 {
            let deferred = store
                .proactive_defer_pending_event(
                    event_id,
                    0,
                    60,
                    Some("조용한 시간에는 알림을 보내지 않습니다."),
                )
                .unwrap();
            assert_eq!(deferred.state, ProactiveEventState::Deferred);
            assert_eq!(deferred.attempt_count, 0);
            assert!(deferred.last_attempt_at.is_none());
            assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
        }
    }

    let store = Store::open(&path).unwrap();
    store
        .connection
        .execute(
            "UPDATE proactive_events SET deferred_until='2000-01-01T00:00:00Z' WHERE id=?1",
            [event_id],
        )
        .unwrap();
    let due = store.proactive_due_events(10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].attempt_count, 0);
    let claimed = store.proactive_claim_due_event(event_id).unwrap();
    assert_eq!(claimed.attempt_count, 1);
    assert!(claimed.last_attempt_at.is_some());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
}

#[test]
fn quiet_hours_changes_release_only_quiet_deferred_events_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let mut changed_hours_id = 0;
    let mut disabled_id = 0;
    {
        let store = Store::open(&path).unwrap();
        add_source(&store, "one");

        let mut settings = store.proactive_settings().unwrap();
        settings.quiet_hours = QuietHours {
            enabled: true,
            start: "22:00".into(),
            end: "07:00".into(),
        };
        store.proactive_save_settings(&settings).unwrap();

        for (document_id, event_id) in [(1, &mut changed_hours_id), (2, &mut disabled_id)] {
            let revision = add_document(&store, document_id, "one", false);
            let ProactiveClaim::Claimed {
                event_id: id,
                attempt_count: 0,
            } = store
                .proactive_claim_event(&key(document_id, revision, ProactiveEventKind::New))
                .unwrap()
            else {
                panic!("expected pending registration");
            };
            *event_id = id;
            store
                .proactive_defer_pending_event(
                    id,
                    0,
                    60,
                    Some("조용한 시간에는 알림을 보내지 않습니다."),
                )
                .unwrap();
            store
                .connection
                .execute(
                    "UPDATE proactive_events
                     SET deferred_until='2999-01-01T00:00:00Z' WHERE id=?1",
                    [id],
                )
                .unwrap();
        }

        settings.quiet_hours.start = "21:00".into();
        store.proactive_save_settings(&settings).unwrap();
    }

    let store = Store::open(&path).unwrap();
    let changed_hours = store.proactive_event(changed_hours_id).unwrap();
    assert_eq!(changed_hours.state, ProactiveEventState::Deferred);
    assert_eq!(changed_hours.attempt_count, 0);
    assert!(changed_hours.deferred_until.is_none());
    assert!(store
        .proactive_due_events(10)
        .unwrap()
        .iter()
        .any(|event| event.id == changed_hours_id));

    let failure = claimed_event(
        &store,
        &key(
            3,
            add_document(&store, 3, "one", false),
            ProactiveEventKind::New,
        ),
    );
    let failure = store
        .proactive_fail_delivery(failure.id, failure.attempt_count, "전달 실패")
        .unwrap();
    let failure_id = failure.id;
    store
        .connection
        .execute(
            "UPDATE proactive_events
             SET deferred_until='2999-01-01T00:00:00Z' WHERE id=?1",
            [failure_id],
        )
        .unwrap();

    let before_disable = store.proactive_event(disabled_id).unwrap();
    assert_eq!(before_disable.state, ProactiveEventState::Deferred);
    assert_eq!(before_disable.attempt_count, 0);
    drop(store);

    let store = Store::open(&path).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.quiet_hours.enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    drop(store);

    let store = Store::open(&path).unwrap();
    let disabled = store.proactive_event(disabled_id).unwrap();
    let failure = store.proactive_event(failure_id).unwrap();
    assert_eq!(disabled.state, ProactiveEventState::Deferred);
    assert_eq!(disabled.attempt_count, 0);
    assert!(disabled.deferred_until.is_none());
    assert_eq!(failure.state, ProactiveEventState::Pending);
    assert_eq!(failure.attempt_count, 1);
    assert_eq!(
        failure.deferred_until.as_deref(),
        Some("2999-01-01T00:00:00Z")
    );
    let due = store.proactive_due_events(10).unwrap();
    assert!(due.iter().any(|event| event.id == disabled_id));
    assert!(!due.iter().any(|event| event.id == failure_id));
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
}

#[test]
fn suppression_requires_the_current_attempt_and_keeps_identity() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let revision = add_document(&store, 1, "one", false);
    let identity = key(1, revision, ProactiveEventKind::New);
    let event = claimed_event(&store, &identity);
    assert!(store
        .proactive_suppress_event(event.id, Some(2), Some("민감도"))
        .is_err());
    let suppressed = store
        .proactive_suppress_event(event.id, Some(1), Some("민감도 기준 미달"))
        .unwrap();
    assert_eq!(suppressed.state, ProactiveEventState::Suppressed);
    assert_eq!(suppressed.reason.as_deref(), Some("민감도 기준 미달"));
    assert_eq!(store.proactive_history(10).unwrap().len(), 1);
    assert_eq!(store.proactive_history(10).unwrap()[0].title, "공지 1 v0");
    assert_eq!(store.proactive_history(10).unwrap()[0].source_id, "one");
    assert!(matches!(
        store.proactive_claim_event(&identity).unwrap(),
        ProactiveClaim::Duplicate {
            previous_event_id: Some(_)
        }
    ));
}

#[test]
fn history_is_bounded_and_clearing_it_never_replays() {
    let (_directory, store) = store();
    add_source(&store, "one");
    add_document(&store, 1, "one", false);
    for _ in 0..204 {
        add_revision(&store, 1, 1);
    }
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM document_revisions"),
        205
    );

    let mut statement = store
        .connection
        .prepare("SELECT id FROM document_revisions ORDER BY id")
        .unwrap();
    let revisions: Vec<i64> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(statement);
    for revision in &revisions {
        let event = claimed_event(&store, &key(1, *revision, ProactiveEventKind::New));
        store
            .proactive_mark_delivered(event.id, event.attempt_count)
            .unwrap();
    }

    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 200);
    assert_eq!(store.proactive_history(500).unwrap().len(), 200);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 205);
    assert_eq!(store.proactive_clear_history().unwrap(), 200);
    assert!(store.proactive_history(10).unwrap().is_empty());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_dedupe"), 205);
    assert!(matches!(
        store
            .proactive_claim_event(&key(1, revisions[0], ProactiveEventKind::New))
            .unwrap(),
        ProactiveClaim::Duplicate {
            previous_event_id: None
        }
    ));
}

#[test]
fn candidates_are_bounded_filtered_and_dedupe_aware() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let first = add_document(&store, 1, "one", false);

    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].document_id, 1);
    assert_eq!(candidates[0].revision_id, first);
    assert_eq!(candidates[0].event_type, ProactiveEventKind::New);
    assert_eq!(candidates[0].title, "공지 1 v0");
    assert_eq!(candidates[0].source_id, "one");
    assert_eq!(candidates[0].source_name, "테스트 정보원");
    assert!(!candidates[0].historical);

    let update = add_revision(&store, 1, 1);
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[1].revision_id, update);
    assert_eq!(candidates[1].event_type, ProactiveEventKind::Revision);

    let event = claimed_event(&store, &key(1, first, ProactiveEventKind::New));
    assert_eq!(event.attempt_count, 1);
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].revision_id, update);

    // Historical document creation never becomes a candidate, but its later
    // revision does.
    let historical_first = add_document(&store, 2, "one", true);
    let candidates = store.proactive_candidates(10).unwrap();
    assert!(!candidates
        .iter()
        .any(|candidate| candidate.revision_id == historical_first));
    let historical_update = add_revision(&store, 2, 1);
    let candidates = store.proactive_candidates(10).unwrap();
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.revision_id == historical_update)
        .expect("historical revision candidate");
    assert_eq!(candidate.event_type, ProactiveEventKind::Revision);
    assert!(candidate.historical);

    // A revision that only repeats the predecessor's title/body is meta-only.
    store
        .connection
        .execute(
            "INSERT INTO document_revisions(
                 document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash
             ) SELECT document_id,canonical_url||'?meta',title,published_at,original,
                      body_html,body_text,content_hash
               FROM document_revisions WHERE id=?1",
            [update],
        )
        .unwrap();
    let meta_only = store.connection.last_insert_rowid();
    assert!(!store
        .proactive_candidates(10)
        .unwrap()
        .iter()
        .any(|candidate| candidate.revision_id == meta_only));

    // Advancing the baseline makes every archived revision ineligible.
    let baseline = store.proactive_establish_baseline().unwrap();
    assert_eq!(baseline, meta_only);
    assert!(store.proactive_candidates(10).unwrap().is_empty());

    let after = add_revision(&store, 1, 2);
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].revision_id, after);
    assert_eq!(candidates[0].event_type, ProactiveEventKind::Revision);
    assert_eq!(store.proactive_candidates(0).unwrap().len(), 1);
}

#[test]
fn revision_notification_history_keeps_each_revision_title() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let first = add_document(&store, 1, "one", false);
    store
        .connection
        .execute(
            "UPDATE document_revisions SET title='Original notice'
             WHERE id=?1",
            [first],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE documents SET title='Original notice' WHERE id=1",
            [],
        )
        .unwrap();

    let first_event = claimed_event(&store, &key(1, first, ProactiveEventKind::New));
    store
        .proactive_mark_attempted(first_event.id, first_event.attempt_count)
        .unwrap();

    let second = add_revision(&store, 1, 1);
    store
        .connection
        .execute(
            "UPDATE document_revisions SET title='Revised notice'
             WHERE id=?1",
            [second],
        )
        .unwrap();
    store
        .connection
        .execute("UPDATE documents SET title='Revised notice' WHERE id=1", [])
        .unwrap();

    let revision_candidate = store
        .proactive_candidates(10)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.revision_id == second)
        .expect("revision notification candidate");
    assert_eq!(revision_candidate.title, "Revised notice");
    assert_eq!(revision_candidate.document_id, 1);
    assert_eq!(revision_candidate.source_id, "one");

    let second_event = claimed_event(&store, &key(1, second, ProactiveEventKind::Revision));
    assert_eq!(
        store
            .proactive_event_candidate(second_event.id)
            .unwrap()
            .title,
        "Revised notice"
    );
    store
        .proactive_mark_attempted(second_event.id, second_event.attempt_count)
        .unwrap();

    let history = store.proactive_history(10).unwrap();
    let first_history = history
        .iter()
        .find(|item| item.event.revision_id == first)
        .expect("initial notification history");
    let second_history = history
        .iter()
        .find(|item| item.event.revision_id == second)
        .expect("revision notification history");
    assert_eq!(first_history.title, "Original notice");
    assert_eq!(second_history.title, "Revised notice");
    assert_eq!(first_history.event.document_id, 1);
    assert_eq!(second_history.event.document_id, 1);
    assert_eq!(first_history.source_id, "one");
    assert_eq!(second_history.source_id, "one");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM proactive_events"), 2);
}

#[test]
fn candidates_keep_unsubscribed_source_revisions_for_suppression() {
    let (_directory, store) = store();
    add_source(&store, "one");
    add_document(&store, 1, "one", false);
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].source_active);

    store
        .connection
        .execute("UPDATE sources SET subscribed=0 WHERE id='one'", [])
        .unwrap();
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(!candidates[0].source_active);

    store
        .connection
        .execute("UPDATE sources SET subscribed=1 WHERE id='one'", [])
        .unwrap();
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].source_active);
}

#[test]
fn candidates_keep_disabled_extension_revisions_for_suppression() {
    let (_directory, store) = store();
    add_source(&store, "one");
    add_source(&store, "two");
    store
        .connection
        .execute(
            "INSERT INTO source_extensions(source_id,extension_id) VALUES('two','test_extension')",
            [],
        )
        .unwrap();
    store.set_enabled("test_extension", false).unwrap();
    add_document(&store, 1, "one", false);
    add_document(&store, 2, "two", false);

    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates
        .iter()
        .any(|candidate| { candidate.document_id == 1 && candidate.source_active }));
    assert!(candidates
        .iter()
        .any(|candidate| { candidate.document_id == 2 && !candidate.source_active }));

    store.set_enabled("test_extension", true).unwrap();
    let candidates = store.proactive_candidates(10).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|candidate| candidate.source_active));
}

#[test]
fn recent_delivery_attempts_persist_and_honor_the_window() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let event_id;
    {
        let store = Store::open(&path).unwrap();
        add_source(&store, "one");
        let revision = add_document(&store, 1, "one", false);
        let identity = key(1, revision, ProactiveEventKind::New);
        let event = claimed_event(&store, &identity);
        event_id = event.id;
        assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
        // Re-claiming the same identity is not another attempt.
        assert!(matches!(
            store.proactive_claim_event(&identity).unwrap(),
            ProactiveClaim::Duplicate { .. }
        ));
        assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
        store
            .proactive_mark_delivered(event_id, event.attempt_count)
            .unwrap();
        assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
        assert!(store.proactive_recent_delivery_attempts(0).is_err());
        assert!(store.proactive_recent_delivery_attempts(u64::MAX).is_err());
    }

    let store = Store::open(&path).unwrap();
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
    store
        .connection
        .execute(
            "UPDATE proactive_events SET last_attempt_at='2000-01-01T00:00:00Z' WHERE id=?1",
            [event_id],
        )
        .unwrap();
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
}

#[test]
fn expired_max_attempt_claims_become_terminal_unknown_failures() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let revision = add_document(&store, 1, "one", false);
    let event = claimed_event(&store, &key(1, revision, ProactiveEventKind::New));
    store
        .connection
        .execute(
            "UPDATE proactive_events SET attempt_count=3,
               claimed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
            [event.id],
        )
        .unwrap();
    // Still inside the claim lease: an in-flight delivery is not finalized.
    assert_eq!(store.proactive_finalize_expired_claims().unwrap(), 0);
    assert_eq!(
        store.proactive_event(event.id).unwrap().state,
        ProactiveEventState::Claimed
    );

    store
        .connection
        .execute(
            "UPDATE proactive_events SET claimed_at='2000-01-01T00:00:00Z' WHERE id=?1",
            [event.id],
        )
        .unwrap();
    assert_eq!(store.proactive_finalize_expired_claims().unwrap(), 1);
    let failed = store.proactive_event(event.id).unwrap();
    assert_eq!(failed.state, ProactiveEventState::Failed);
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("확인하지 못했습니다")));
    assert_eq!(store.proactive_history(10).unwrap().len(), 1);
    assert_eq!(store.proactive_finalize_expired_claims().unwrap(), 0);

    // Stale claims below the attempt limit stay retryable, not failed.
    let second_revision = add_document(&store, 2, "one", false);
    let second = claimed_event(&store, &key(2, second_revision, ProactiveEventKind::New));
    store
        .connection
        .execute(
            "UPDATE proactive_events SET claimed_at='2000-01-01T00:00:00Z' WHERE id=?1",
            [second.id],
        )
        .unwrap();
    assert_eq!(store.proactive_finalize_expired_claims().unwrap(), 0);
    assert!(store
        .proactive_due_events(10)
        .unwrap()
        .iter()
        .any(|due| due.id == second.id));
    assert_eq!(
        store
            .proactive_claim_due_event(second.id)
            .unwrap()
            .attempt_count,
        2
    );
}

#[test]
fn event_candidate_returns_archive_metadata() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let revision = add_document(&store, 1, "one", false);
    let event = claimed_event(&store, &key(1, revision, ProactiveEventKind::New));
    let candidate = store.proactive_event_candidate(event.id).unwrap();
    assert_eq!(candidate.document_id, 1);
    assert_eq!(candidate.revision_id, revision);
    assert_eq!(candidate.event_type, ProactiveEventKind::New);
    assert_eq!(candidate.title, "공지 1 v0");
    assert_eq!(candidate.source_id, "one");
    assert_eq!(candidate.source_name, "테스트 정보원");
    assert!(!candidate.historical);
    assert!(store.proactive_event_candidate(9999).is_err());
}

#[test]
fn attempted_state_is_terminal_honest_and_promotable() {
    let (_directory, store) = store();
    add_source(&store, "one");
    let revision = add_document(&store, 1, "one", false);
    let event = claimed_event(&store, &key(1, revision, ProactiveEventKind::New));

    let attempted = store
        .proactive_mark_attempted(event.id, event.attempt_count)
        .unwrap();
    assert_eq!(attempted.state, ProactiveEventState::Attempted);
    assert!(attempted.delivered_at.is_none());
    let history = store.proactive_history(10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].event.state, ProactiveEventState::Attempted);
    assert!(store
        .proactive_suppress_event(event.id, Some(event.attempt_count), None)
        .is_err());
    assert!(store.proactive_claim_due_event(event.id).is_err());

    // A later platform receipt can upgrade the record to confirmed delivery.
    let delivered = store
        .proactive_mark_delivered(event.id, event.attempt_count)
        .unwrap();
    assert_eq!(delivered.state, ProactiveEventState::Delivered);
    assert!(delivered.delivered_at.is_some());

    let second_revision = add_document(&store, 2, "one", false);
    let second = claimed_event(&store, &key(2, second_revision, ProactiveEventKind::New));
    store
        .proactive_mark_attempted(second.id, second.attempt_count)
        .unwrap();
    assert_eq!(store.proactive_clear_history().unwrap(), 2);
    assert!(store.proactive_history(10).unwrap().is_empty());
}

#[test]
fn task_scheduling_outcomes_and_scheduler_status_persist() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    {
        let store = Store::open(&path).unwrap();
        let task = store
            .proactive_save_task(&ProactiveTaskInput {
                task_id: "source:one".into(),
                interval_minutes: 30,
                enabled: true,
            })
            .unwrap();
        assert!(task.enabled);
        assert!(task.next_run_at.is_some());
        assert_eq!(task.consecutive_failures, 0);
        let disabled = store
            .proactive_save_task(&ProactiveTaskInput {
                task_id: "source:two".into(),
                interval_minutes: 60,
                enabled: false,
            })
            .unwrap();
        assert!(disabled.next_run_at.is_none());
        assert!(!disabled.enabled);
        assert!(store.proactive_due_tasks().unwrap().is_empty());

        store
            .connection
            .execute(
                "UPDATE proactive_tasks SET next_run_at='2000-01-01T00:00:00Z'
                 WHERE task_id='source:one'",
                [],
            )
            .unwrap();
        let due = store.proactive_due_tasks().unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].task_id, "source:one");
        assert_eq!(store.proactive_scheduler_status().unwrap().due_tasks, 1);

        let failure = store
            .proactive_record_task_outcome(
                "source:one",
                ProactiveOutcome::Failure,
                Some("수집 실패"),
            )
            .unwrap();
        assert_eq!(failure.consecutive_failures, 1);
        assert_eq!(failure.last_outcome, Some(ProactiveOutcome::Failure));
        assert_eq!(failure.last_error.as_deref(), Some("수집 실패"));
        assert!(failure.last_run_at.is_some());
        assert!(!store
            .proactive_due_tasks()
            .unwrap()
            .iter()
            .any(|t| t.task_id == "source:one"));
        let second = store
            .proactive_record_task_outcome("source:one", ProactiveOutcome::Failure, None)
            .unwrap();
        assert_eq!(second.consecutive_failures, 2);

        let success = store
            .proactive_record_task_outcome("source:one", ProactiveOutcome::Success, None)
            .unwrap();
        assert_eq!(success.consecutive_failures, 0);
        assert_eq!(success.last_outcome, Some(ProactiveOutcome::Success));
        assert!(success.last_error.is_none());

        let stopped = store
            .proactive_set_task_enabled("source:one", false)
            .unwrap();
        assert!(stopped.next_run_at.is_none());
        let restarted = store
            .proactive_set_task_enabled("source:one", true)
            .unwrap();
        assert!(restarted.next_run_at.is_some());
        assert_eq!(restarted.consecutive_failures, 0);

        let status = store
            .proactive_record_scheduler_run(SchedulerOutcome::Failure, Some("실행 실패"))
            .unwrap();
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.last_outcome, Some(SchedulerOutcome::Failure));
        assert!(status.next_task_at.is_some());
        let status = store
            .proactive_record_scheduler_run(SchedulerOutcome::Failure, None)
            .unwrap();
        assert_eq!(status.consecutive_failures, 2);
        let status = store
            .proactive_record_scheduler_run(SchedulerOutcome::Success, None)
            .unwrap();
        assert_eq!(status.consecutive_failures, 0);
        assert_eq!(status.last_outcome, Some(SchedulerOutcome::Success));
        assert_eq!(status.pending_events, 0);
        assert_eq!(store.proactive_tasks().unwrap().len(), 2);
        assert!(store
            .proactive_save_task(&ProactiveTaskInput {
                task_id: "bad task".into(),
                interval_minutes: 30,
                enabled: true,
            })
            .is_err());
        assert!(store
            .proactive_save_task(&ProactiveTaskInput {
                task_id: "source:x".into(),
                interval_minutes: 1,
                enabled: true,
            })
            .is_err());
        assert!(store
            .proactive_record_task_outcome("missing", ProactiveOutcome::Success, None)
            .is_err());
        assert!(store.proactive_set_task_enabled("missing", false).is_err());
    }

    let store = Store::open(&path).unwrap();
    let task = store.proactive_task("source:one").unwrap().unwrap();
    assert!(task.enabled);
    assert_eq!(task.interval_minutes, 30);
    assert_eq!(store.proactive_tasks().unwrap().len(), 2);
    let status = store.proactive_scheduler_status().unwrap();
    assert_eq!(status.last_outcome, Some(SchedulerOutcome::Success));
    assert_eq!(status.consecutive_failures, 0);
}

#[test]
fn seed_bootstrap_state_is_durable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    {
        let store = Store::open(&path).unwrap();
        add_source(&store, "one");
        assert!(!store.seed_complete("one").unwrap());
        store.mark_seed_complete("one").unwrap();
        assert!(store.seed_complete("one").unwrap());
        store.mark_seed_complete("one").unwrap();
        assert!(store.seed_complete("missing").is_err());
        assert!(store.mark_seed_complete("missing").is_err());
    }
    let store = Store::open(&path).unwrap();
    assert!(store.seed_complete("one").unwrap());
}
