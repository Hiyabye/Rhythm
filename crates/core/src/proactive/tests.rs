use super::{
    notification::{self, DeliveryItem},
    preserve_lifecycle_settings, scheduler,
};
use crate::storage::{
    personalization::PersonalizationNotificationScore,
    proactive::{NotificationSensitivity, ProactiveEventKind, ProactiveSettings, QuietHours},
};

/// A store with notifications on for every sensitivity, one source (`one`) and
/// one notice with a revision.
fn seeded_store(
    subscribed: bool,
) -> (
    tempfile::TempDir,
    crate::storage::Store,
    crate::storage::proactive::ProactiveSettings,
) {
    use rusqlite::Connection;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let store = crate::storage::Store::open(&path).unwrap();
    store.set_enabled("proactive", true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = true;
    settings.notification_sensitivity = NotificationSensitivity::All;
    store.proactive_save_settings(&settings).unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed)
             VALUES('one','테스트 정보원','기관','설명','https://example.edu/','main',1,?1)",
            [subscribed],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO documents(
                 id,source_id,external_id,canonical_url,title,published_at,historical,
                 body_html,body_text,content_hash
             ) VALUES(1,'one','1','https://example.edu/1','공지','2026-01-01T00:00:00Z',
                      0,'<p>본문</p>','본문','hash')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO document_revisions(
                 document_id,canonical_url,title,published_at,original,body_html,body_text,content_hash
             ) VALUES(1,'https://example.edu/1','공지','2026-01-01T00:00:00Z',
                      '{}','<p>본문</p>','본문','hash')",
            [],
        )
        .unwrap();
    drop(connection);
    (directory, store, settings)
}

#[test]
fn generic_settings_save_preserves_persisted_lifecycle_values() {
    let persisted = ProactiveSettings {
        close_to_background: true,
        launch_at_login: false,
        start_hidden: true,
        ..ProactiveSettings::default()
    };
    let requested = ProactiveSettings {
        close_to_background: false,
        launch_at_login: true,
        start_hidden: false,
        scheduler_enabled: true,
        ..ProactiveSettings::default()
    };

    let merged = preserve_lifecycle_settings(requested, &persisted);

    assert!(merged.close_to_background);
    assert!(!merged.launch_at_login);
    assert!(merged.start_hidden);
    assert!(merged.scheduler_enabled);
}

#[test]
fn energy_profiles_set_the_declared_scheduler_defaults_and_keep_rate_limits_bounded() {
    assert_eq!(scheduler::profile_interval("eco"), 60);
    assert_eq!(scheduler::profile_interval("balanced"), 30);
    assert_eq!(scheduler::profile_interval("performance"), 15);
    assert_eq!(scheduler::profile_interval("unknown"), 30);
    assert_eq!(scheduler::resolved_interval(0, "eco"), 60);
    assert_eq!(scheduler::resolved_interval(0, "balanced"), 30);
    assert_eq!(scheduler::resolved_interval(0, "performance"), 15);
    for profile in ["eco", "balanced", "performance"] {
        assert_eq!(scheduler::resolved_interval(60, profile), 60);
    }
    assert_eq!(scheduler::resolved_interval(45, "eco"), 45);
    assert_eq!(scheduler::available_notification_slots(0), 3);
    assert_eq!(scheduler::available_notification_slots(2), 1);
    assert_eq!(scheduler::available_notification_slots(3), 0);
    assert_eq!(scheduler::available_notification_slots(u32::MAX), 0);
}

#[test]
fn scheduler_does_not_admit_after_stop_or_when_task_becomes_inactive() {
    let (sender, stop) = tokio::sync::watch::channel(false);
    let enabled = ProactiveSettings {
        scheduler_enabled: true,
        ..ProactiveSettings::default()
    };
    assert!(scheduler::can_admit_task(&stop, &enabled, true, true));

    let paused = ProactiveSettings {
        scheduler_paused: true,
        ..enabled.clone()
    };
    assert!(!scheduler::can_admit_task(&stop, &paused, true, true));
    let disabled = ProactiveSettings::default();
    assert!(!scheduler::can_admit_task(&stop, &disabled, true, true));
    assert!(!scheduler::can_admit_task(&stop, &enabled, false, true));
    assert!(!scheduler::can_admit_task(&stop, &enabled, true, false));

    sender.send(true).unwrap();
    assert!(!scheduler::can_admit_task(&stop, &enabled, true, true));
}

#[test]
fn scheduler_rechecks_each_batch_admission_after_stop_pause_or_disable() {
    let (sender, stop) = tokio::sync::watch::channel(false);
    let active = ProactiveSettings {
        scheduler_enabled: true,
        ..ProactiveSettings::default()
    };

    let mut admissions = 0;
    for index in 0..3 {
        if !scheduler::can_admit_task(&stop, &active, true, true) {
            break;
        }
        admissions += 1;
        if index == 0 {
            sender.send(true).unwrap();
        }
    }
    assert_eq!(admissions, 1);

    for inactive in [
        ProactiveSettings {
            scheduler_paused: true,
            ..active.clone()
        },
        ProactiveSettings::default(),
    ] {
        let (_, stop) = tokio::sync::watch::channel(false);
        let mut admissions = 0;
        for prefs in [&active, &inactive] {
            if !scheduler::can_admit_task(&stop, prefs, true, true) {
                break;
            }
            admissions += 1;
        }
        assert_eq!(admissions, 1);
    }
}
#[test]
fn quiet_hours_use_local_window_semantics_and_end_boundary_is_exclusive() {
    let quiet = QuietHours {
        enabled: true,
        start: "22:00".into(),
        end: "07:00".into(),
    };
    assert!(quiet.contains(22 * 60));
    assert!(quiet.contains(23 * 60 + 59));
    assert!(quiet.contains(0));
    assert!(quiet.contains(6 * 60 + 59));
    assert!(!quiet.contains(7 * 60));
    let daytime = QuietHours {
        enabled: true,
        start: "09:00".into(),
        end: "17:00".into(),
    };
    assert!(daytime.contains(12 * 60));
    assert!(!daytime.contains(17 * 60));
}

#[test]
fn quiet_events_are_deferred_until_local_window_end() {
    let overnight = QuietHours {
        enabled: true,
        start: "22:00".into(),
        end: "07:00".into(),
    };
    assert_eq!(
        scheduler::quiet_end_delay_minutes(&overnight, 22 * 60),
        Some(9 * 60)
    );
    assert_eq!(
        scheduler::quiet_end_delay_minutes(&overnight, 6 * 60 + 30),
        Some(30)
    );
    assert_eq!(scheduler::quiet_end_delay_minutes(&overnight, 7 * 60), None);

    let daytime = QuietHours {
        enabled: true,
        start: "09:00".into(),
        end: "17:00".into(),
    };
    assert_eq!(
        scheduler::quiet_end_delay_minutes(&daytime, 12 * 60 + 15),
        Some(4 * 60 + 45)
    );
    assert_eq!(scheduler::quiet_end_delay_minutes(&daytime, 17 * 60), None);
}

#[test]
fn notifications_are_discovered_without_scheduler_enablement() {
    let prefs = ProactiveSettings {
        notifications_enabled: true,
        scheduler_enabled: false,
        ..ProactiveSettings::default()
    };
    assert!(scheduler::should_discover_notifications(&prefs));
    assert!(!scheduler::can_admit_task(
        &tokio::sync::watch::channel(false).1,
        &prefs,
        true,
        true
    ));
}

#[test]
fn authorization_policy_matches_each_platform_sender() {
    if cfg!(windows) {
        for status in ["unavailable", "denied", "not_determined", "unknown"] {
            assert!(!scheduler::notification_authorization_permits(
                status, false
            ));
            assert!(!scheduler::notification_authorization_permits(status, true));
        }
        assert!(scheduler::notification_authorization_permits(
            "authorized",
            false
        ));
    } else {
        for status in ["denied", "not_determined"] {
            assert!(!scheduler::notification_authorization_permits(
                status, false
            ));
            // Quiet hours still claim and defer so the events are not lost.
            assert!(scheduler::notification_authorization_permits(status, true));
        }
        for status in ["authorized", "provisional", "ephemeral", "unavailable"] {
            assert!(scheduler::notification_authorization_permits(status, false));
        }
    }
}

#[test]
fn sensitivity_policies_suppress_cold_start_and_negative_signals() {
    let item = |has_preferences, score, explicit_interest_match, negative_feedback| {
        PersonalizationNotificationScore {
            document_id: 10,
            score,
            reasons: Vec::new(),
            has_preferences,
            positive_interest_matches: Vec::new(),
            explicit_interest_match,
            negative_interest_matches: Vec::new(),
            feedback: None,
            positive_feedback: false,
            negative_feedback,
        }
    };
    assert!(!notification::eligible(
        NotificationSensitivity::Important,
        &item(false, 100, false, false)
    ));
    assert!(notification::eligible(
        NotificationSensitivity::Important,
        &item(true, 16, false, false)
    ));
    assert!(!notification::eligible(
        NotificationSensitivity::Important,
        &item(true, 15, false, false)
    ));
    assert!(!notification::eligible(
        NotificationSensitivity::Interested,
        &item(true, 80, false, false)
    ));
    assert!(notification::eligible(
        NotificationSensitivity::Interested,
        &item(false, 0, true, false)
    ));
    assert!(!notification::eligible(
        NotificationSensitivity::All,
        &item(true, 80, true, true)
    ));
}

#[test]
fn inactive_sources_are_settled_without_history_and_never_replayed() {
    use rusqlite::Connection;

    for disable_source in ["subscription", "extension"] {
        let (directory, store, settings) = seeded_store(disable_source != "subscription");
        let connection = Connection::open(directory.path().join("rhythm.db")).unwrap();
        if disable_source == "extension" {
            connection
                .execute(
                    "INSERT INTO source_extensions(source_id,extension_id)
                     VALUES('one','test_extension')",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO extension_preferences(extension_id,enabled)
                     VALUES('test_extension',0)",
                    [],
                )
                .unwrap();
        }

        assert!(notification::consider_candidates(&store, &settings)
            .unwrap()
            .is_empty());
        // The feed mirrors every board: an inactive one leaves no history.
        assert!(store.proactive_history(10).unwrap().is_empty());
        assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);

        // Activating the source later does not announce what it missed.
        connection
            .execute_batch(
                "UPDATE sources SET subscribed=1; UPDATE extension_preferences SET enabled=1;",
            )
            .unwrap();
        assert!(store.proactive_candidates(10).unwrap().is_empty());
        assert!(notification::consider_candidates(&store, &settings)
            .unwrap()
            .is_empty());
    }
}

#[test]
fn notification_digest_text_uses_stable_revision_events() {
    let item = |event_id, revision_id, title: &str| DeliveryItem {
        event_id,
        document_id: 5,
        revision_id,
        source_id: "notices".into(),
        source_name: "공지".into(),
        kind: ProactiveEventKind::Revision,
        title: title.into(),
        attempt_count: 1,
    };
    assert_eq!(notification::digest_text(&[]), None);
    assert_eq!(
        notification::digest_text(&[item(3, 71, "장학 공지")]),
        Some(("새 소식이 도착했습니다".into(), "공지 · 장학 공지".into()))
    );
    assert_eq!(
        notification::digest_text(&[item(3, 71, "장학 공지"), item(4, 72, "수정 공지")]),
        Some((
            "새 소식 2건".into(),
            "공지 · 장학 공지 외 1건의 새 소식이 있습니다.".into()
        ))
    );
}

#[test]
fn candidate_preparation_does_not_claim_or_report_delivery() {
    let (_directory, store, settings) = seeded_store(true);

    let candidates = notification::consider_candidates(&store, &settings).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].document_id, 1);
    assert_eq!(candidates[0].revision_id, 1);
    assert_eq!(candidates[0].event_id, 0);
    assert_eq!(candidates[0].attempt_count, 0);
    assert!(store.proactive_history(10).unwrap().is_empty());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
}

#[test]
fn activation_preserves_single_notice_and_digest_identity_and_attempts() {
    let item = |event_id, document_id, revision_id, attempt_count| DeliveryItem {
        event_id,
        document_id,
        revision_id,
        source_id: "notices".into(),
        source_name: "공지".into(),
        kind: ProactiveEventKind::Revision,
        title: "장학 공지".into(),
        attempt_count,
    };

    assert!(notification::activation_for_items(&[]).is_none());
    let single_item = item(17, 5, 71, 2);
    let single = notification::activation_for_items(std::slice::from_ref(&single_item)).unwrap();
    assert_eq!(single.event_id, 17);
    assert_eq!(single.document_id, 5);
    assert_eq!(single.revision_id, 71);
    assert_eq!(single.attempt_count, 2);
    assert_eq!(single.event_ids, vec![17]);
    assert_eq!(single.event_attempts, vec![(17, 2)]);

    let digest = notification::activation_for_items(&[
        item(17, 5, 71, 2),
        item(18, 6, 72, 1),
        item(19, 7, 73, 3),
    ])
    .unwrap();
    assert_eq!(digest.event_id, 17);
    assert_eq!(digest.document_id, 5);
    assert_eq!(digest.revision_id, 71);
    assert_eq!(digest.attempt_count, 2);
    assert_eq!(digest.event_ids, vec![17, 18, 19]);
    assert_eq!(digest.event_attempts, vec![(17, 2), (18, 1), (19, 3)]);
}

#[tokio::test]
async fn pending_notification_responses_are_bounded_nonblocking_and_cancelled_on_shutdown() {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use tokio::sync::{oneshot, watch};

    let mut response_tasks = scheduler::ResponseTasks::default();
    let (shutdown_sender, stop) = watch::channel(false);
    let accepting_callbacks = Arc::new(AtomicBool::new(true));
    let callback_gate = Arc::new(Mutex::new(()));
    let callback_ran = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicUsize::new(0));
    let mut response_senders = Vec::new();
    let mut admitted = 0;

    loop {
        let (response_sender, response_receiver) = oneshot::channel::<()>();
        let callback_acceptance = accepting_callbacks.clone();
        let callback_gate_task = callback_gate.clone();
        let callback_ran_task = callback_ran.clone();
        let completed_task = completed.clone();
        let mut task_stop = stop.clone();
        if !response_tasks.spawn(async move {
            tokio::select! {
                _ = task_stop.changed() => {}
                response = response_receiver => {
                    let Ok(_callback_guard) = callback_gate_task.lock() else {
                        return;
                    };
                    if response.is_ok() && callback_acceptance.load(Ordering::Acquire) {
                        callback_ran_task.store(true, Ordering::Release);
                    }
                }
            }
            completed_task.fetch_add(1, Ordering::Release);
        }) {
            break;
        }
        admitted += 1;
        response_senders.push(response_sender);
    }
    assert!(admitted > 0, "response waiters should have a nonzero bound");
    assert!(response_tasks.is_full());

    let mut ticks = tokio::time::interval(std::time::Duration::from_millis(1));
    ticks.tick().await;
    tokio::time::timeout(std::time::Duration::from_millis(100), ticks.tick())
        .await
        .expect("another scheduler tick should happen while bounded responses are pending");
    assert!(response_tasks.is_full());

    let _shutdown_guard = callback_gate.lock().unwrap();
    accepting_callbacks.store(false, Ordering::Release);
    shutdown_sender.send(true).unwrap();
    drop(_shutdown_guard);
    response_tasks.drain().await;
    drop(response_senders);
    assert_eq!(completed.load(Ordering::Acquire), admitted);
    assert!(!response_tasks.is_full());
    assert!(!callback_ran.load(Ordering::Acquire));
}
fn claimed_delivery_fixture() -> (
    tempfile::TempDir,
    std::sync::Arc<std::sync::Mutex<crate::storage::Store>>,
    DeliveryItem,
) {
    use crate::storage::proactive::{ProactiveClaim, ProactiveEventKey};
    use std::sync::{Arc, Mutex};

    let (directory, store, _settings) = seeded_store(true);

    let (event_id, _) = match store
        .proactive_claim_event(&ProactiveEventKey {
            document_id: 1,
            revision_id: 1,
            event_type: ProactiveEventKind::New,
        })
        .unwrap()
    {
        ProactiveClaim::Claimed {
            event_id,
            attempt_count,
        } => (event_id, attempt_count),
        _ => panic!("expected the fresh event to be claimed"),
    };
    let claimed = store.proactive_claim_due_event(event_id).unwrap();
    let candidate = store.proactive_event_candidate(event_id).unwrap();
    let item = DeliveryItem {
        event_id,
        document_id: candidate.document_id,
        revision_id: candidate.revision_id,
        source_id: candidate.source_id,
        source_name: candidate.source_name,
        kind: candidate.event_type,
        title: candidate.title,
        attempt_count: claimed.attempt_count,
    };

    let shared = Arc::new(Mutex::new(store));
    (directory, shared, item)
}

#[test]
fn final_notification_gate_defers_when_notifications_were_disabled_after_claim() {
    use crate::storage::proactive::ProactiveEventState;
    let (_directory, shared, item) = claimed_delivery_fixture();
    let event_id = item.event_id;
    {
        let store = shared.lock().unwrap();
        let mut settings = store.proactive_settings().unwrap();
        settings.notifications_enabled = false;
        store.proactive_save_settings(&settings).unwrap();
    }
    scheduler::handoff_validated(
        &shared,
        vec![item],
        &"aa".repeat(32),
        |_, _| -> Result<(), String> {
            panic!("disabled notifications must never reach OS preparation")
        },
        |_| panic!("disabled notifications must never reach OS handoff"),
    )
    .unwrap();
    let event = shared.lock().unwrap().proactive_event(event_id).unwrap();
    assert_eq!(event.state, ProactiveEventState::Deferred);
    assert_eq!(event.attempt_count, 0);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .proactive_recent_delivery_attempts(600)
            .unwrap(),
        0
    );
}

#[test]
fn reserved_macos_batch_defers_disabled_notifications_and_reserves_enabled_ones() {
    use crate::storage::proactive::ProactiveEventState;
    let (_directory, shared, item) = claimed_delivery_fixture();
    let event_id = item.event_id;
    let set_enabled = |enabled| {
        let store = shared.lock().unwrap();
        let mut settings = store.proactive_settings().unwrap();
        settings.notifications_enabled = enabled;
        store.proactive_save_settings(&settings).unwrap();
    };
    set_enabled(false);
    assert!(
        scheduler::reserve_delivery_batch(&shared, vec![item.clone()])
            .unwrap()
            .is_empty()
    );
    let event = shared.lock().unwrap().proactive_event(event_id).unwrap();
    assert_eq!(event.state, ProactiveEventState::Deferred);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .proactive_recent_delivery_attempts(600)
            .unwrap(),
        0
    );

    let (_directory, shared, item) = claimed_delivery_fixture();
    let reserved = scheduler::reserve_delivery_batch(&shared, vec![item]).unwrap();
    assert_eq!(reserved.len(), 1);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .proactive_recent_delivery_attempts(600)
            .unwrap(),
        1
    );
}

#[test]
fn settings_change_waits_until_validated_handoff_finishes() {
    let (_directory, shared, item) = claimed_delivery_fixture();
    let changing = shared.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (changed_tx, changed_rx) = std::sync::mpsc::channel();
    let mut worker = None;
    scheduler::handoff_validated(
        &shared,
        vec![item],
        &"bb".repeat(32),
        |store, _| {
            worker = Some(std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let store = changing.lock().unwrap();
                let mut settings = store.proactive_settings().unwrap();
                settings.notifications_enabled = false;
                store.proactive_save_settings(&settings).unwrap();
                changed_tx.send(()).unwrap();
            }));
            started_rx.recv().unwrap();
            assert!(matches!(
                changed_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            assert!(store.proactive_settings().unwrap().notifications_enabled);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap();
    worker.unwrap().join().unwrap();
    changed_rx.recv().unwrap();
    assert!(
        !shared
            .lock()
            .unwrap()
            .proactive_settings()
            .unwrap()
            .notifications_enabled
    );
}

#[test]
fn unavailable_before_show_defers_without_consuming_an_attempt() {
    use crate::storage::proactive::ProactiveEventState;
    let (_directory, shared, item) = claimed_delivery_fixture();
    let event_id = item.event_id;
    let result = scheduler::handoff_validated(
        &shared,
        vec![item],
        &"cc".repeat(32),
        |_, _| Err::<(), String>("Windows notifications unavailable".into()),
        |_| panic!("Show must not run after preparation fails"),
    );
    assert!(result.is_err());
    let store = shared.lock().unwrap();
    let event = store.proactive_event(event_id).unwrap();
    assert_eq!(event.state, ProactiveEventState::Deferred);
    assert_eq!(event.attempt_count, 0);
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
    assert!(store
        .notification_token_consume(&"cc".repeat(32))
        .unwrap()
        .is_none());
}

#[test]
fn failed_show_is_counted_as_an_attempt() {
    let (_directory, shared, item) = claimed_delivery_fixture();
    let event_id = item.event_id;
    let result = scheduler::handoff_validated(
        &shared,
        vec![item],
        &"dd".repeat(32),
        |_, _| Ok(()),
        |_| Err("Show failed".into()),
    );
    assert!(result.is_err());
    let store = shared.lock().unwrap();
    assert_eq!(store.proactive_event(event_id).unwrap().attempt_count, 1);
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
    assert!(store
        .notification_token_consume(&"dd".repeat(32))
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn scheduled_controller_collects_cybercampus_and_runs_again() {
    use crate::{
        archive::{
            self,
            cybercampus::{credentials::Credentials, fake::Fake, Connector},
            sync::Controller,
        },
        storage::{
            proactive::{ProactiveOutcome, ProactiveTaskInput},
            Store,
        },
    };
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.sqlite3");
    let store = Store::open(&path).unwrap();
    store.set_enabled("proactive", true).unwrap();
    store.catalog(&archive::definitions()).unwrap();
    store.set_enabled("cybercampus", true).unwrap();
    store.subscribe("cybercampus", true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.scheduler_enabled = true;
    settings.scheduler_paused = false;
    settings.notifications_enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    let task_id = "source:cybercampus";
    store
        .proactive_save_task(&ProactiveTaskInput {
            task_id: task_id.into(),
            interval_minutes: 15,
            enabled: true,
        })
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE proactive_tasks SET next_run_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE task_id=?1",
            [task_id],
        )
        .unwrap();

    let shared = Arc::new(Mutex::new(store));
    let controller = Controller::with_connector(Connector::memory());
    controller
        .cybercampus
        .vault
        .save(&Credentials::new("20990001", "right").unwrap())
        .unwrap();
    let transport = Arc::new(Fake::default());
    let (stop_sender, mut stop) = tokio::sync::watch::channel(false);
    let _keep_stop = stop_sender;
    let due = shared.lock().unwrap().proactive_due_tasks().unwrap();
    assert_eq!(due.len(), 1);
    let first = scheduler::run_scheduled_task(
        &shared,
        &controller,
        directory.path(),
        &transport,
        &due[0],
        &mut stop,
    )
    .await
    .unwrap();
    assert_eq!(
        first,
        ProactiveOutcome::Success,
        "{:?}",
        controller.status().unwrap()
    );
    let completed = shared
        .lock()
        .unwrap()
        .proactive_task(task_id)
        .unwrap()
        .unwrap();
    assert_eq!(completed.last_outcome, Some(ProactiveOutcome::Success));
    assert!(!shared
        .lock()
        .unwrap()
        .proactive_due_tasks()
        .unwrap()
        .iter()
        .any(|task| task.task_id == task_id));

    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE proactive_tasks SET next_run_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE task_id=?1",
            [task_id],
        )
        .unwrap();
    let due_again = shared.lock().unwrap().proactive_due_tasks().unwrap();
    let second = scheduler::run_scheduled_task(
        &shared,
        &controller,
        directory.path(),
        &transport,
        &due_again[0],
        &mut stop,
    )
    .await
    .unwrap();
    assert_eq!(second, ProactiveOutcome::Success);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .archive_query(&crate::archive::model::ArchiveQuery::default())
            .unwrap()
            .total,
        2
    );
    let (_, completed_job) = controller
        .start_tracked_with_transport(
            shared.clone(),
            directory.path().to_path_buf(),
            transport.clone(),
            Some(vec!["cybercampus".into()]),
            "incremental".into(),
        )
        .unwrap();
    controller.wait().await;
    let (_, later_job) = controller
        .start_tracked_with_transport(
            shared.clone(),
            directory.path().to_path_buf(),
            transport.clone(),
            Some(vec!["cybercampus".into()]),
            "incremental".into(),
        )
        .unwrap();
    let preserved = scheduler::cancel_or_take_scheduled_outcome(&controller, completed_job)
        .await
        .unwrap()
        .expect("completed job outcome must survive a lost cancellation");
    assert!(preserved.result.is_ok());
    assert!(!preserved.cancelled);
    controller.wait().await;
    let later_outcome = controller.take_task_outcome(later_job).unwrap().unwrap();
    assert!(later_outcome.result.is_ok());
}

#[test]
fn scheduled_tasks_need_the_proactive_and_owning_extensions() {
    use crate::archive;
    use std::sync::{Arc, Mutex};
    let (_directory, store) = crate::storage::temp_store();
    store.catalog(&archive::definitions()).unwrap();
    store.set_enabled("sogang_notices", true).unwrap();
    store.set_enabled("proactive", true).unwrap();
    store.subscribe("sogang-academic", true).unwrap();
    store.subscribe("cybercampus", true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.scheduler_enabled = true;
    settings.notifications_enabled = true;
    settings.close_to_background = true;
    store.proactive_save_settings(&settings).unwrap();
    let store = Arc::new(Mutex::new(store));
    let active = |id| scheduler::task_currently_active(&store, id).unwrap();
    assert!(active("sogang-academic"));
    assert!(!active("cybercampus"));
    store
        .lock()
        .unwrap()
        .set_enabled("proactive", false)
        .unwrap();
    assert!(!active("sogang-academic"));
    // The stored choices survive; only what runs is masked.
    let guard = store.lock().unwrap();
    let effective = guard.proactive_effective_settings().unwrap();
    assert!(!effective.scheduler_enabled && !effective.notifications_enabled);
    assert!(!effective.close_to_background);
    let stored = guard.proactive_settings().unwrap();
    assert!(stored.scheduler_enabled && stored.notifications_enabled && stored.close_to_background);
}

#[test]
fn only_cybercampus_gets_a_scheduled_task() {
    use crate::archive;
    use std::sync::{Arc, Mutex};
    let (_directory, store) = crate::storage::temp_store();
    store.catalog(&archive::definitions()).unwrap();
    for extension in ["sogang_notices", "cybercampus", "proactive"] {
        store.set_enabled(extension, true).unwrap();
    }
    store.subscribe("sogang-academic", true).unwrap();
    store.subscribe("cybercampus", true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.scheduler_enabled = true;
    store.proactive_save_settings(&settings).unwrap();
    let prefs = store.proactive_effective_settings().unwrap();
    let energy = store.settings().unwrap();
    let store = Arc::new(Mutex::new(store));
    scheduler::reconcile_tasks(&store, &prefs, &energy).unwrap();
    let tasks = store.lock().unwrap().proactive_tasks().unwrap();
    // A subscribed public board comes from the feed and gets no task.
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, "source:cybercampus");
    assert!(tasks[0].enabled);
}

#[test]
fn claimed_events_are_released_without_a_spent_attempt_when_a_later_step_fails() {
    use crate::storage::proactive::{ProactiveClaim, ProactiveEventKey, ProactiveEventState};
    use std::sync::{Arc, Mutex};

    let (directory, store, settings) = seeded_store(true);
    let ProactiveClaim::Claimed { event_id, .. } = store
        .proactive_claim_event(&ProactiveEventKey {
            document_id: 1,
            revision_id: 1,
            event_type: ProactiveEventKind::New,
        })
        .unwrap()
    else {
        panic!("expected the fresh event to be claimed");
    };
    // Fresh-candidate discovery reads `proactive_baseline`; the due-event
    // claim before it does not, so the failure lands after the first claim.
    rusqlite::Connection::open(directory.path().join("rhythm.db"))
        .unwrap()
        .execute_batch("ALTER TABLE proactive_baseline RENAME TO proactive_baseline_gone")
        .unwrap();
    let shared = Arc::new(Mutex::new(store));

    assert!(scheduler::claim_delivery_items(&shared, &settings, 3, None).is_err());

    let event = shared.lock().unwrap().proactive_event(event_id).unwrap();
    assert_eq!(event.state, ProactiveEventState::Deferred);
    assert_eq!(event.attempt_count, 0);
}

#[test]
fn an_error_after_the_claim_hands_the_event_back_without_a_spent_attempt() {
    use crate::storage::proactive::ProactiveEventState;
    for windows_path in [false, true] {
        let (directory, shared, item) = claimed_delivery_fixture();
        let event_id = item.event_id;
        assert_eq!(item.attempt_count, 1);
        // The final policy read fails after the claim spent an attempt.
        rusqlite::Connection::open(directory.path().join("rhythm.db"))
            .unwrap()
            .execute_batch("ALTER TABLE documents RENAME TO documents_gone")
            .unwrap();
        let failed = if windows_path {
            scheduler::handoff_validated(
                &shared,
                vec![item],
                &"ee".repeat(32),
                |_, _| -> Result<(), String> {
                    panic!("no preparation after a failed revalidation")
                },
                |_| panic!("no handoff after a failed revalidation"),
            )
            .is_err()
        } else {
            scheduler::reserve_delivery_batch(&shared, vec![item]).is_err()
        };
        assert!(failed, "windows path: {windows_path}");
        let event = shared.lock().unwrap().proactive_event(event_id).unwrap();
        assert_eq!(event.state, ProactiveEventState::Deferred);
        assert_eq!(event.attempt_count, 0, "windows path: {windows_path}");
    }
}
