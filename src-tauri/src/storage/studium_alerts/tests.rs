use super::*;

fn fixture() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("alerts.db")).unwrap();
    store.set_enabled(extensions::STUDIUM, true).unwrap();
    store.set_enabled(extensions::PROACTIVE, true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = true;
    settings.quiet_hours.enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    (directory, store)
}

fn rule(kind: &str, minimum: u32) -> AlertRuleInput {
    AlertRuleInput {
        enabled: true,
        kind: kind.into(),
        minimum,
    }
}

fn observation(room_id: &str, remaining: u32, detail: bool) -> RoomState {
    let room = rooms::get(room_id).unwrap();
    RoomState {
        room_id: room_id.into(),
        status: if detail { "ready" } else { "count_only" }.into(),
        result: Some(SeatData {
            total: room.capacity,
            used: room.capacity - remaining,
            remaining,
            source_time: parser::now_timestamp(),
            seat_numbers: detail.then(|| (1..=remaining).collect()),
            source_url: if detail {
                room.url()
            } else {
                rooms::STATUS_URL.into()
            },
            layout: None,
            layout_error: None,
        }),
        error: None,
        checked_at: Some(parser::now_timestamp()),
    }
}

fn candidate(store: &Store, room_id: &str, remaining: u32) -> AlertCandidate {
    store
        .studium_alert_observe(1, &observation(room_id, remaining, true))
        .unwrap()
        .unwrap()
}

fn token(number: u64) -> String {
    format!("{number:064x}")
}

fn count(store: &Store, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn rules_default_off_and_validate_changes_without_creating_notifications() {
    let (_directory, store) = fixture();
    let defaults = store.studium_alert_rules().unwrap();
    assert_eq!(defaults.len(), 7);
    assert!(defaults
        .values()
        .all(|rule| !rule.enabled && rule.version == 1));
    for invalid in [
        rule("bad", 1),
        rule("remaining", 0),
        rule("remaining", 103),
        rule("preferred", 2),
        rule("preferred", 1),
    ] {
        assert!(store.studium_save_alert_rule("5", &invalid).is_err());
        assert_eq!(store.studium_alert_rules().unwrap(), defaults);
    }
    assert!(store
        .studium_save_alert_rule("unknown", &rule("remaining", 1))
        .is_err());
    let changed = store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    assert_eq!(changed["5"].version, 2);
    assert_eq!(
        store
            .studium_save_alert_rule("5", &rule("remaining", 2))
            .unwrap(),
        changed
    );
    assert_eq!(count(&store, "studium_alert_tokens"), 0);
    assert_eq!(count(&store, "studium_alert_attempts"), 0);
    store.set_enabled(extensions::STUDIUM, false).unwrap();
    assert!(store
        .studium_save_alert_rule("5", &rule("remaining", 3))
        .is_err());
    assert_eq!(store.studium_alert_rules().unwrap(), changed);
}

#[test]
fn uncertainty_does_not_rearm_and_successful_nonmatch_rearms_across_reopen() {
    let (directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let first = candidate(&store, "5", 3);
    assert!(store.studium_alert_reserve(&first, &token(1)).unwrap());
    assert!(!store.studium_alert_reserve(&first, &token(2)).unwrap());
    let mut failed = observation("5", 0, true);
    failed.status = "failed".into();
    failed.result = None;
    assert!(store.studium_alert_observe(2, &failed).unwrap().is_none());
    let mut old = observation("5", 0, true);
    old.result.as_mut().unwrap().source_time =
        (chrono::Utc::now() - chrono::Duration::seconds(400)).to_rfc3339();
    assert!(store.studium_alert_observe(2, &old).unwrap().is_none());
    assert!(store
        .studium_alert_observe(3, &observation("5", 3, true))
        .unwrap()
        .is_none());
    drop(store);
    let store = Store::open(&directory.path().join("alerts.db")).unwrap();
    assert!(store
        .studium_alert_observe(4, &observation("5", 3, true))
        .unwrap()
        .is_none());
    assert!(store
        .studium_alert_observe(5, &observation("5", 0, true))
        .unwrap()
        .is_none());
    let next = candidate(&store, "5", 4);
    assert!(store.studium_alert_reserve(&next, &token(3)).unwrap());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 2);
}

#[test]
fn count_only_supports_remaining_but_preserves_unknown_preferred_latch() {
    let (_directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let count_only = store
        .studium_alert_observe(1, &observation("5", 3, false))
        .unwrap()
        .unwrap();
    assert!(store.studium_alert_reserve(&count_only, &token(1)).unwrap());
    store.studium_save_favorites("5", &[2]).unwrap();
    store
        .studium_save_alert_rule("5", &rule("preferred", 1))
        .unwrap();
    let preferred = candidate(&store, "5", 3);
    assert!(store.studium_alert_reserve(&preferred, &token(2)).unwrap());
    assert!(store
        .studium_alert_observe(2, &observation("5", 0, false))
        .unwrap()
        .is_none());
    assert!(store
        .studium_alert_observe(3, &observation("5", 3, true))
        .unwrap()
        .is_none());
    assert!(store
        .studium_alert_observe(4, &observation("5", 1, true))
        .unwrap()
        .is_none());
    assert!(store
        .studium_alert_observe(5, &observation("5", 2, true))
        .unwrap()
        .is_some());
}

#[test]
fn malformed_status_shape_and_stale_candidates_never_rearm_or_reserve() {
    let (_directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let issued = candidate(&store, "5", 3);
    assert!(store.studium_alert_reserve(&issued, &token(1)).unwrap());
    let mut wrong_status = observation("5", 0, true);
    wrong_status.status = "count_only".into();
    assert!(store
        .studium_alert_observe(2, &wrong_status)
        .unwrap()
        .is_none());
    let mut wrong_url = observation("5", 0, true);
    wrong_url.result.as_mut().unwrap().source_url = rooms::get("6").unwrap().url();
    assert!(store
        .studium_alert_observe(3, &wrong_url)
        .unwrap()
        .is_none());
    assert!(store
        .studium_alert_observe(4, &observation("5", 3, true))
        .unwrap()
        .is_none());
    store
        .studium_alert_observe(5, &observation("5", 0, true))
        .unwrap();
    let mut stale = candidate(&store, "5", 3);
    stale.data.source_time = (chrono::Utc::now() - chrono::Duration::seconds(400)).to_rfc3339();
    assert!(!store.studium_alert_reserve(&stale, &token(2)).unwrap());
    stale.data.source_time = (chrono::Utc::now() + chrono::Duration::seconds(120)).to_rfc3339();
    assert!(!store.studium_alert_reserve(&stale, &token(2)).unwrap());
    assert_eq!(count(&store, "studium_alert_attempts"), 1);
}

#[test]
fn favorites_change_versions_and_tokens_but_reordered_identical_saves_do_not() {
    let (_directory, store) = fixture();
    store.studium_save_favorites("5", &[1, 3]).unwrap();
    store
        .studium_save_alert_rule("5", &rule("preferred", 1))
        .unwrap();
    let old = candidate(&store, "5", 3);
    assert!(store.studium_alert_reserve(&old, &token(1)).unwrap());
    store.studium_save_favorites("5", &[3, 1]).unwrap();
    assert_eq!(
        store.studium_alert_rules().unwrap()["5"].version,
        old.version
    );
    assert_eq!(count(&store, "studium_alert_tokens"), 1);
    store.studium_save_favorites("5", &[2]).unwrap();
    assert_eq!(
        store.studium_alert_rules().unwrap()["5"].version,
        old.version + 1
    );
    assert_eq!(store.studium_alert_consume(&token(1)).unwrap(), None);
    assert!(!store.studium_alert_reserve(&old, &token(2)).unwrap());
    assert!(store
        .studium_alert_reserve(&candidate(&store, "5", 2), &token(2))
        .unwrap());
}

#[test]
fn quiet_hours_and_global_opt_out_leave_eligible_observations_unconsumed() {
    let (_directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let first = candidate(&store, "5", 3);
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    assert!(!store.studium_alert_reserve(&first, &token(1)).unwrap());
    settings.notifications_enabled = true;
    let now = store.proactive_local_minutes().unwrap();
    let start = (now + 24 * 60 - 1) % (24 * 60);
    let next = (now + 2) % (24 * 60);
    settings.quiet_hours.enabled = true;
    settings.quiet_hours.start = format!("{:02}:{:02}", start / 60, start % 60);
    settings.quiet_hours.end = format!("{:02}:{:02}", next / 60, next % 60);
    store.proactive_save_settings(&settings).unwrap();
    assert!(!store.studium_alert_reserve(&first, &token(1)).unwrap());
    assert_eq!(count(&store, "studium_alert_attempts"), 0);
    settings.quiet_hours.enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    assert!(store
        .studium_alert_reserve(&candidate(&store, "5", 3), &token(2))
        .unwrap());
}

#[test]
fn notice_and_seat_reservations_share_the_global_attempt_budget_in_both_directions() {
    let (_directory, store) = fixture();
    store
        .connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
         INSERT INTO proactive_events(id,document_id,revision_id,event_type,state,attempt_count)
         VALUES(1,11,21,'new','claimed',1),(2,12,22,'new','claimed',1),(3,13,23,'new','claimed',1);
         PRAGMA foreign_keys=ON;",
        )
        .unwrap();
    assert!(store
        .proactive_reserve_delivery_attempts(&[(1, 1), (2, 1)])
        .unwrap());
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    assert!(store
        .studium_alert_reserve(&candidate(&store, "5", 3), &token(1))
        .unwrap());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 3);
    assert!(!store
        .proactive_reserve_delivery_attempts(&[(3, 1)])
        .unwrap());
    store
        .studium_alert_observe(2, &observation("5", 0, true))
        .unwrap();
    assert!(!store
        .studium_alert_reserve(&candidate(&store, "5", 3), &token(2))
        .unwrap());
    store.proactive_clear_history().unwrap();
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 3);
}

#[test]
fn reservation_failure_rolls_back_token_attempt_and_latch_together() {
    let (_directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let first = candidate(&store, "5", 3);
    store.connection.execute_batch("CREATE TRIGGER fail_seat_attempt BEFORE INSERT ON studium_alert_attempts BEGIN SELECT RAISE(ABORT,'synthetic reservation failure'); END;").unwrap();
    assert!(store.studium_alert_reserve(&first, &token(1)).is_err());
    assert_eq!(count(&store, "studium_alert_tokens"), 0);
    assert_eq!(count(&store, "studium_alert_attempts"), 0);
    assert!(store
        .studium_alert_observe(2, &observation("5", 3, true))
        .unwrap()
        .is_some());
    store
        .connection
        .execute_batch("DROP TRIGGER fail_seat_attempt;")
        .unwrap();
    assert!(store.studium_alert_reserve(&first, &token(1)).unwrap());
    store.studium_alert_invalidate_tokens().unwrap();
    assert!(store
        .studium_alert_observe(3, &observation("5", 3, true))
        .unwrap()
        .is_none());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
}

#[test]
fn failed_rule_or_favorite_change_preserves_settings_and_existing_capabilities() {
    let (_directory, store) = fixture();
    store.studium_save_favorites("5", &[2, 8]).unwrap();
    let saved = store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    assert!(store
        .studium_alert_reserve(&candidate(&store, "5", 3), &token(1))
        .unwrap());
    store.connection.execute_batch("CREATE TRIGGER fail_token_reset BEFORE DELETE ON studium_alert_tokens BEGIN SELECT RAISE(ABORT,'synthetic token reset failure'); END;").unwrap();
    assert!(store
        .studium_save_alert_rule("5", &rule("remaining", 3))
        .is_err());
    assert_eq!(store.studium_alert_rules().unwrap(), saved);
    assert!(store.studium_save_favorites("5", &[3, 9]).is_err());
    assert_eq!(store.studium_favorites().unwrap()["5"], [2, 8]);
    assert_eq!(store.studium_alert_rules().unwrap(), saved);
    assert_eq!(count(&store, "studium_alert_tokens"), 1);
    assert_eq!(count(&store, "studium_alert_attempts"), 1);
}

#[test]
fn tokens_are_hashed_single_use_expiring_and_bound_to_current_selection_and_rule() {
    for invalidation in [
        "none",
        "expired",
        "rule",
        "studium",
        "proactive",
        "notifications",
        "selection",
    ] {
        let (_directory, store) = fixture();
        store
            .studium_save_alert_rule("5", &rule("remaining", 2))
            .unwrap();
        assert!(store
            .studium_alert_reserve(&candidate(&store, "5", 3), &token(1))
            .unwrap());
        let hash: String = store
            .connection
            .query_row("SELECT token_hash FROM studium_alert_tokens", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_ne!(hash, token(1));
        assert_eq!(store.studium_alert_consume("bad").unwrap(), None);
        assert_eq!(store.studium_alert_consume(&token(99)).unwrap(), None);
        match invalidation {
            "expired" => {
                store
                    .connection
                    .execute(
                        "UPDATE studium_alert_tokens SET expires_at='2000-01-01T00:00:00.000Z'",
                        [],
                    )
                    .unwrap();
            }
            "rule" => {
                store
                    .studium_save_alert_rule("5", &rule("remaining", 3))
                    .unwrap();
            }
            "studium" | "proactive" => {
                store.set_enabled(invalidation, false).unwrap();
                store.set_enabled(invalidation, true).unwrap();
            }
            "notifications" => {
                let mut settings = store.proactive_settings().unwrap();
                settings.notifications_enabled = false;
                store.proactive_save_settings(&settings).unwrap();
                settings.notifications_enabled = true;
                store.proactive_save_settings(&settings).unwrap();
            }
            "selection" => {
                store.studium_save_preferences(&["6".into()], "5").unwrap();
                store.studium_save_preferences(&["5".into()], "6").unwrap();
            }
            "none" => {}
            _ => unreachable!(),
        }
        assert_eq!(
            store.studium_alert_consume(&token(1)).unwrap(),
            (invalidation == "none").then(|| "5".to_string())
        );
        assert_eq!(store.studium_alert_consume(&token(1)).unwrap(), None);
    }
}

#[test]
fn capabilities_are_bounded_and_shared_clock_prunes_both_attempt_ledgers() {
    let (_directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    let first = candidate(&store, "5", 3);
    for number in 1..=MAX_TOKENS {
        store.connection.execute("INSERT INTO studium_alert_tokens VALUES(?1,'5',?2,strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 minutes'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![digest(&token(number as u64)), first.version]).unwrap();
    }
    assert!(!store.studium_alert_reserve(&first, &token(100)).unwrap());
    assert_eq!(count(&store, "studium_alert_tokens"), MAX_TOKENS);
    store.studium_alert_invalidate_tokens().unwrap();
    store.connection.execute_batch(
        "INSERT INTO studium_alert_attempts(attempted_at) VALUES(strftime('%Y-%m-%dT%H:%M:%fZ','now','-8 days'));
         INSERT INTO proactive_delivery_attempts VALUES(1,1,'new',1,strftime('%Y-%m-%dT%H:%M:%fZ','now','-8 days'));
         UPDATE proactive_delivery_clock SET max_observed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','+1 day');",
    ).unwrap();
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 0);
    assert_eq!(count(&store, "studium_alert_attempts"), 0);
    assert_eq!(count(&store, "proactive_delivery_attempts"), 0);
    let watermark: String = store
        .connection
        .query_row(
            "SELECT max_observed_at FROM proactive_delivery_clock",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(store.studium_alert_reserve(&first, &token(101)).unwrap());
    assert_eq!(store.proactive_recent_delivery_attempts(600).unwrap(), 1);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT attempted_at FROM studium_alert_attempts",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        watermark
    );
}

#[test]
fn populated_schema_twenty_eight_upgrade_and_settings_backup_preserve_rules_without_results() {
    let (directory, store) = fixture();
    store.studium_save_auto_interval(17).unwrap();
    store.studium_save_favorites("5", &[2, 8]).unwrap();
    let mut settings = store.settings().unwrap();
    settings.clock_format = "24h".into();
    settings.auto_trash_months = 6;
    store.save(&settings).unwrap();
    store.connection.execute_batch("DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; PRAGMA user_version=28;").unwrap();
    drop(store);
    let store = Store::open(&directory.path().join("alerts.db")).unwrap();
    assert_eq!(store.studium_auto_interval().unwrap(), 17);
    assert_eq!(store.studium_favorites().unwrap()["5"], [2, 8]);
    assert_eq!(store.settings().unwrap().clock_format, "24h");
    assert_eq!(store.settings().unwrap().auto_trash_months, 6);
    assert!(store
        .studium_alert_rules()
        .unwrap()
        .values()
        .all(|rule| !rule.enabled));
    let saved = store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    store
        .backup_to(&directory.path().join("backup.db"))
        .unwrap();
    let backup = Store::open(&directory.path().join("backup.db")).unwrap();
    assert_eq!(backup.studium_alert_rules().unwrap(), saved);
    assert_eq!(backup.settings().unwrap().clock_format, "24h");
    assert_eq!(backup.settings().unwrap().auto_trash_months, 6);
    for (table, expected) in [
        (
            "studium_alert_rules",
            vec!["room_id", "enabled", "kind", "minimum", "version", "armed"],
        ),
        ("studium_alert_attempts", vec!["id", "attempted_at"]),
        (
            "studium_alert_tokens",
            vec![
                "token_hash",
                "room_id",
                "rule_version",
                "expires_at",
                "created_at",
            ],
        ),
    ] {
        let columns: Vec<String> = backup
            .connection
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
            ))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns, expected);
    }
}

#[test]
fn alert_migration_failure_keeps_schema_twenty_eight_and_sentinel() {
    let (directory, store) = fixture();
    store.connection.execute_batch("DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; CREATE TABLE studium_alert_rules(sentinel TEXT); INSERT INTO studium_alert_rules VALUES('keep'); PRAGMA user_version=28;").unwrap();
    drop(store);
    assert!(Store::open(&directory.path().join("alerts.db")).is_err());
    let connection = Connection::open(directory.path().join("alerts.db")).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        28
    );
    assert_eq!(
        connection
            .query_row("SELECT sentinel FROM studium_alert_rules", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "keep"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='studium_alert_tokens'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn reopening_invalidates_tokens_left_by_a_process_that_skipped_shutdown() {
    let (directory, store) = fixture();
    store
        .studium_save_alert_rule("5", &rule("remaining", 2))
        .unwrap();
    assert!(store
        .studium_alert_reserve(&candidate(&store, "5", 3), &token(1))
        .unwrap());
    assert_eq!(count(&store, "studium_alert_tokens"), 1);
    // Dropping the store without quiesce models an updater exit or crash.
    drop(store);
    let store = Store::open(&directory.path().join("alerts.db")).unwrap();
    assert_eq!(count(&store, "studium_alert_tokens"), 0);
    assert_eq!(store.studium_alert_consume(&token(1)).unwrap(), None);
}
