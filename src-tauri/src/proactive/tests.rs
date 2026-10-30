use super::{
    notification::{self, DeliveryItem},
    preserve_lifecycle_settings, scheduler,
};
use crate::storage::{
    personalization::PersonalizationNotificationScore,
    proactive::{NotificationSensitivity, ProactiveEventKind, ProactiveSettings, QuietHours},
};

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
        &item(true, 18, false, false)
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
fn inactive_sources_are_suppressed_before_any_notification_can_be_delivered() {
    use crate::storage::{proactive::ProactiveEventState, Store};
    use rusqlite::Connection;

    for disable_source in ["subscription", "extension"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rhythm.db");
        let store = Store::open(&path).unwrap();
        let mut settings = store.proactive_settings().unwrap();
        settings.notifications_enabled = true;
        settings.notification_sensitivity = NotificationSensitivity::All;
        store.proactive_save_settings(&settings).unwrap();

        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed)
                 VALUES('one','테스트 정보원','기관','설명','https://example.edu/','main',1,?1)",
                [disable_source != "subscription"],
            )
            .unwrap();
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

        assert!(notification::consider_candidates(&store, &settings)
            .unwrap()
            .is_empty());
        let history = store.proactive_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event.state, ProactiveEventState::Suppressed);
        assert_eq!(history[0].event.attempt_count, 0);
        assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
    }
}

#[test]
fn notification_digest_uses_stable_revision_events_without_ai_dates() {
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
    use crate::storage::Store;
    use rusqlite::Connection;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rhythm.db");
    let store = Store::open(&path).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = true;
    settings.notification_sensitivity = NotificationSensitivity::All;
    store.proactive_save_settings(&settings).unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed)
             VALUES('one','테스트 정보원','기관','설명','https://example.edu/','main',1,1)",
            [],
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
