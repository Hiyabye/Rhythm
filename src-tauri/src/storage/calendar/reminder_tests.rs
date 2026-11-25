use super::*;
use crate::extensions;
use crate::storage::{
    UNDO_BODY_HASH, UNDO_CALENDAR_EVENTS, UNDO_FEED, UNDO_FEED_SYNC, UNDO_KNOWN_BOARDS,
    UNDO_PREVIOUS_MANIFEST, UNDO_REMINDERS,
};

fn fixture() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("reminders.db")).unwrap();
    store.set_enabled(extensions::CALENDAR, true).unwrap();
    store.set_enabled(extensions::PROACTIVE, true).unwrap();
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = true;
    settings.quiet_hours.enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    (directory, store)
}
fn todo(store: &Store, title: &str, day: &str, time: Option<&str>, minutes: Option<i64>) -> i64 {
    store
        .calendar_create_todo(&TodoInput {
            title: title.into(),
            day: day.into(),
            time: time.map(Into::into),
            reminder_minutes: minutes,
            ..TodoInput::default()
        })
        .unwrap()
        .id
}
fn at(value: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap()
}
fn due(store: &Store, now: &str) -> Vec<String> {
    store
        .calendar_due_reminders(at(now))
        .unwrap()
        .into_iter()
        .map(|reminder| reminder.title)
        .collect()
}
fn token(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
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
fn reminder_choices_depend_on_whether_the_todo_has_a_time() {
    let (_directory, store) = fixture();
    for minutes in TIMED_REMINDERS {
        todo(&store, "시간", "2026-11-10", Some("09:00"), Some(minutes));
    }
    for minutes in UNTIMED_REMINDERS {
        todo(&store, "종일", "2026-11-10", None, Some(minutes));
    }
    for (time, minutes) in [
        (Some("09:00"), 5),
        (Some("09:00"), -10),
        (None, 10),
        (None, 60),
    ] {
        let result = store.calendar_create_todo(&TodoInput {
            title: "x".into(),
            day: "2026-11-10".into(),
            time: time.map(Into::into),
            reminder_minutes: Some(minutes),
            ..TodoInput::default()
        });
        assert!(result.is_err(), "{time:?} {minutes}");
    }
    let saved = store.calendar_todos("2026-11-10", "2026-11-10").unwrap();
    assert_eq!(saved.len(), 7);
    assert_eq!(saved[0].reminder_minutes, Some(0), "untimed first");
}

#[test]
fn due_reminders_are_sent_late_only_while_the_todo_is_ahead() {
    let (_directory, store) = fixture();
    todo(&store, "10분 전", "2026-11-10", Some("09:00"), Some(10));
    todo(&store, "제시간", "2026-11-10", Some("09:00"), Some(0));
    todo(
        &store,
        "하루 전 (놓침)",
        "2026-11-11",
        Some("08:00"),
        Some(1440),
    );
    todo(&store, "지난 할 일", "2026-11-10", Some("08:30"), Some(10));
    todo(&store, "당일 아침", "2026-11-10", None, Some(0));
    todo(&store, "내일 종일", "2026-11-11", None, Some(1440));
    todo(&store, "어제 아침 알림", "2026-11-10", None, Some(1440));
    todo(&store, "알림 없음", "2026-11-10", Some("09:00"), None);
    let done = todo(&store, "끝냄", "2026-11-10", Some("09:00"), Some(10));
    store.calendar_set_todo_done(done, true).unwrap();
    assert_eq!(
        due(&store, "2026-11-10 08:55"),
        ["10분 전", "어제 아침 알림", "하루 전 (놓침)"]
    );
    // An untimed todo's reminder lasts until its day ends.
    assert_eq!(
        due(&store, "2026-11-10 23:59"),
        ["당일 아침", "어제 아침 알림", "하루 전 (놓침)", "내일 종일"]
    );
    assert_eq!(due(&store, "2026-11-11 08:00"), ["내일 종일"]);
}

#[test]
fn reminders_need_the_calendar_and_notifications() {
    let (_directory, store) = fixture();
    todo(&store, "보고서", "2026-11-10", Some("09:00"), Some(10));
    let mut settings = store.proactive_settings().unwrap();
    settings.notifications_enabled = false;
    store.proactive_save_settings(&settings).unwrap();
    assert!(due(&store, "2026-11-10 08:55").is_empty());
    settings.notifications_enabled = true;
    store.proactive_save_settings(&settings).unwrap();
    store.set_enabled(extensions::PROACTIVE, false).unwrap();
    assert!(due(&store, "2026-11-10 08:55").is_empty());
    store.set_enabled(extensions::PROACTIVE, true).unwrap();
    store.set_enabled(extensions::CALENDAR, false).unwrap();
    assert!(due(&store, "2026-11-10 08:55").is_empty());
    store.set_enabled(extensions::CALENDAR, true).unwrap();
    assert_eq!(due(&store, "2026-11-10 08:55"), ["보고서"]);
}

#[test]
fn a_reserved_reminder_latches_until_the_todo_moves() {
    let (_directory, store) = fixture();
    let id = todo(&store, "보고서", "2026-11-10", Some("09:00"), Some(10));
    let now = at("2026-11-10 08:55");
    assert_eq!(
        store
            .calendar_reminder_reserve(id, &token(1), at("2026-11-10 08:40"))
            .unwrap(),
        None
    );
    let reserved = store
        .calendar_reminder_reserve(id, &token(1), now)
        .unwrap()
        .unwrap();
    assert_eq!((reserved.title.as_str(), reserved.minutes), ("보고서", 10));
    assert!(due(&store, "2026-11-10 08:55").is_empty());
    assert_eq!(
        store.calendar_reminder_reserve(id, &token(2), now).unwrap(),
        None
    );
    assert_eq!(count(&store, "calendar_reminder_attempts"), 1);
    assert_eq!(
        store.proactive_recent_delivery_attempts(600).unwrap(),
        1,
        "shares the rate window"
    );

    let edit = |title: &str, time: &str, minutes: i64| {
        store
            .calendar_update_todo(
                id,
                &TodoInput {
                    title: title.into(),
                    day: "2026-11-10".into(),
                    time: Some(time.into()),
                    reminder_minutes: Some(minutes),
                    ..TodoInput::default()
                },
            )
            .unwrap()
    };
    edit("보고서 (수정)", "09:00", 10);
    assert!(
        due(&store, "2026-11-10 08:55").is_empty(),
        "a retitled todo stays latched"
    );
    edit("보고서 (수정)", "09:30", 30);
    assert_eq!(due(&store, "2026-11-10 09:05"), ["보고서 (수정)"]);
}

#[test]
fn quiet_hours_and_the_shared_rate_limit_hold_a_reminder_back() {
    let (_directory, store) = fixture();
    let id = todo(&store, "보고서", "2099-01-01", Some("09:00"), Some(1440));
    let now = at("2098-12-31 10:00");
    let mut settings = store.proactive_settings().unwrap();
    let minutes = store.proactive_local_minutes().unwrap();
    let (start, end) = (
        (minutes + 24 * 60 - 1) % (24 * 60),
        (minutes + 2) % (24 * 60),
    );
    settings.quiet_hours.enabled = true;
    settings.quiet_hours.start = format!("{:02}:{:02}", start / 60, start % 60);
    settings.quiet_hours.end = format!("{:02}:{:02}", end / 60, end % 60);
    store.proactive_save_settings(&settings).unwrap();
    assert_eq!(
        store.calendar_reminder_reserve(id, &token(1), now).unwrap(),
        None
    );
    settings.quiet_hours.enabled = false;
    store.proactive_save_settings(&settings).unwrap();

    for _ in 0..proactive::MAX_DELIVERY_ATTEMPTS {
        store
            .connection
            .execute(
                "INSERT INTO studium_alert_attempts(attempted_at) VALUES(strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                [],
            )
            .unwrap();
    }
    assert_eq!(
        store.calendar_reminder_reserve(id, &token(1), now).unwrap(),
        None
    );
    assert_eq!(count(&store, "calendar_reminder_attempts"), 0);
    assert_eq!(count(&store, "calendar_reminder_tokens"), 0);
    assert_eq!(
        due(&store, "2098-12-31 10:00"),
        ["보고서"],
        "held back, not dropped"
    );
    store
        .connection
        .execute("DELETE FROM studium_alert_attempts", [])
        .unwrap();
    assert!(store
        .calendar_reminder_reserve(id, &token(1), now)
        .unwrap()
        .is_some());
}

#[test]
fn a_click_token_opens_the_todo_day_once() {
    let (directory, store) = fixture();
    let id = todo(&store, "보고서", "2026-11-10", Some("09:00"), Some(10));
    let now = at("2026-11-10 08:55");
    store
        .calendar_reminder_reserve(id, &token(1), now)
        .unwrap()
        .unwrap();
    assert_eq!(
        store.calendar_reminder_consume(&"0".repeat(64)).unwrap(),
        None
    );
    assert_eq!(store.calendar_reminder_consume("bad").unwrap(), None);
    assert_eq!(
        store
            .calendar_reminder_consume(&token(1))
            .unwrap()
            .as_deref(),
        Some("2026-11-10")
    );
    assert_eq!(
        store.calendar_reminder_consume(&token(1)).unwrap(),
        None,
        "one use"
    );

    // Turning the calendar off, deleting the todo or reopening the store drops tokens.
    let second = todo(&store, "둘째", "2026-11-10", Some("09:00"), Some(10));
    store
        .calendar_reminder_reserve(second, &token(2), now)
        .unwrap()
        .unwrap();
    store.set_enabled(extensions::CALENDAR, false).unwrap();
    store.set_enabled(extensions::CALENDAR, true).unwrap();
    assert_eq!(store.calendar_reminder_consume(&token(2)).unwrap(), None);
    let third = todo(&store, "셋째", "2026-11-10", Some("09:00"), Some(10));
    store
        .calendar_reminder_reserve(third, &token(3), now)
        .unwrap()
        .unwrap();
    store.calendar_delete_todo(third).unwrap();
    assert_eq!(count(&store, "calendar_reminder_tokens"), 0);
    // Three reminders used the rate window; clear it for the last case.
    store
        .connection
        .execute("DELETE FROM calendar_reminder_attempts", [])
        .unwrap();
    let fourth = todo(&store, "넷째", "2026-11-10", Some("09:00"), Some(10));
    store
        .calendar_reminder_reserve(fourth, &token(4), now)
        .unwrap()
        .unwrap();
    drop(store);
    let store = Store::open(&directory.path().join("reminders.db")).unwrap();
    assert_eq!(store.calendar_reminder_consume(&token(4)).unwrap(), None);
}

#[test]
fn schema_thirty_four_upgrades_todos_without_reminders() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.db");
    {
        let store = Store::open(&path).unwrap();
        store.set_enabled(extensions::CALENDAR, true).unwrap();
        todo(&store, "예전 할 일", "2026-11-10", Some("09:00"), None);
        store
            .connection
            .execute_batch(&format!(
                "{UNDO_CALENDAR_EVENTS}{UNDO_KNOWN_BOARDS}{UNDO_PREVIOUS_MANIFEST}{UNDO_BODY_HASH}{UNDO_FEED_SYNC}{UNDO_FEED}{UNDO_REMINDERS} PRAGMA user_version=34;"
            ))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.info().unwrap().schema_version,
        crate::storage::SCHEMA_VERSION
    );
    let todos = store.calendar_todos("2026-11-10", "2026-11-10").unwrap();
    assert_eq!(
        (todos[0].title.as_str(), todos[0].reminder_minutes),
        ("예전 할 일", None)
    );
}
