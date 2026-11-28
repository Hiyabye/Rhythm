//! `App` against a temporary data directory with a no-op shell and fake
//! Studium views. No window, network request or embedding server.
use super::*;
use crate::extensions::STUDIUM;
use std::{path::Path, sync::atomic::AtomicUsize, time::Duration};

struct NoWindow;

impl Shell for NoWindow {
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        task();
        Ok(())
    }
    fn show_main_window(&self) -> Result<(), String> {
        Ok(())
    }
}

fn open(root: &Path) -> Result<App, OpenError> {
    App::open_in(
        root.join("data"),
        &root.join("resources"),
        Config {
            identifier: "io.github.hiyabye.rhythm.test".into(),
            user_agent: "Rhythm/test",
            runtime: Handle::current(),
            shell: Arc::new(NoWindow),
        },
    )
}

/// Counts what core asks of the shell's Studium views. `open` says whether a
/// window is open for `capture_close` to capture.
#[derive(Default)]
struct Views {
    open: AtomicBool,
    starts: AtomicUsize,
    refreshes: AtomicUsize,
    quiesces: AtomicUsize,
    shutdowns: AtomicUsize,
    closes: AtomicUsize,
}

const TICKET: u64 = 7;

impl StudiumViews for Views {
    fn start(self: Arc<Self>) -> Result<(), String> {
        self.starts.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn refresh(&self) {
        self.refreshes.fetch_add(1, Ordering::AcqRel);
    }
    fn quiesce(&self) -> BoxFuture<'_, ()> {
        self.quiesces.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {})
    }
    fn resume_after_quiesce(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.start();
        })
    }
    fn shutdown(&self) -> BoxFuture<'_, ()> {
        self.shutdowns.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {})
    }
    fn capture_close(&self) -> Option<CloseTicket> {
        self.open
            .load(Ordering::Acquire)
            .then(|| CloseTicket(Box::new(TICKET)))
    }
    fn close(self: Arc<Self>, ticket: CloseTicket) -> BoxFuture<'static, Result<(), String>> {
        Box::pin(async move {
            assert_eq!(*ticket.0.downcast::<u64>().unwrap(), TICKET);
            self.open.store(false, Ordering::Release);
            self.closes.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
    }
}

fn count(counter: &AtomicUsize) -> usize {
    counter.load(Ordering::Acquire)
}

async fn next(events: &mut broadcast::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("no event arrived")
        .unwrap()
}

#[tokio::test]
async fn saved_settings_reach_subscribers_and_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let mut events = app.events();
    let mut settings = app.settings().snapshot().await.unwrap().settings;
    settings.theme = "dark".into();
    settings.clock_format = "24h".into();
    let saved = app.settings().save(settings).await.unwrap();
    assert_eq!(saved.settings.theme, "dark");
    assert!(matches!(next(&mut events).await, Event::ThemeChanged(theme) if theme == "dark"));
    assert!(matches!(
        next(&mut events).await,
        Event::ClockFormatChanged(format) if format == "24h"
    ));
    app.shutdown().await;
    drop(app);

    let app = open(directory.path()).unwrap();
    let reopened = app.settings().snapshot().await.unwrap();
    assert_eq!(reopened.settings.theme, "dark");
    assert_eq!(reopened.settings.clock_format, "24h");
    app.shutdown().await;
}

#[tokio::test]
async fn a_second_open_of_the_same_data_directory_is_locked() {
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    assert!(matches!(open(directory.path()), Err(OpenError::Locked(_))));
    app.shutdown().await;
    drop(app);
    open(directory.path()).unwrap().shutdown().await;
}

#[tokio::test]
async fn turning_studium_off_closes_the_window_captured_under_the_store_lock() {
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let views = Arc::new(Views::default());
    let _ = app.views.set(views.clone());

    let on = app.settings().set_extension(STUDIUM, true).await.unwrap();
    assert!(on.extensions.iter().any(|e| e.id == STUDIUM && e.enabled));
    views.open.store(true, Ordering::Release);
    let off = app.settings().set_extension(STUDIUM, false).await.unwrap();
    assert!(off.extensions.iter().any(|e| e.id == STUDIUM && !e.enabled));
    assert_eq!(count(&views.closes), 1);
    assert!(!views.open.load(Ordering::Acquire));
    // Both changes redraw the views.
    assert_eq!(count(&views.refreshes), 2);

    // With no window open there is nothing to close.
    app.settings().set_extension(STUDIUM, true).await.unwrap();
    app.settings().set_extension(STUDIUM, false).await.unwrap();
    assert_eq!(count(&views.closes), 1);
    app.shutdown().await;
}

#[tokio::test]
async fn maintenance_pauses_the_views_and_its_guard_restarts_them() {
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let views = Arc::new(Views::default());
    let _ = app.views.set(views.clone());

    let guard = app.exclusive_maintenance().await.unwrap();
    assert_eq!(count(&views.quiesces), 1);
    assert_eq!(count(&views.starts), 0);
    // Admission is closed while maintenance holds the data directory.
    assert!(app.settings().snapshot().await.is_err());
    drop(guard);
    assert_eq!(count(&views.starts), 1);
    assert!(app.settings().snapshot().await.is_ok());

    app.shutdown().await;
    assert_eq!(count(&views.shutdowns), 1);
}

#[tokio::test]
async fn a_maintenance_call_dropped_while_the_classifier_quiesces_resumes_the_work() {
    let directory = tempfile::tempdir().unwrap();
    let app = Arc::new(open(directory.path()).unwrap());
    let views = Arc::new(Views::default());
    let _ = app.views.set(views.clone());

    // A batch in flight keeps the classifier's quiesce waiting.
    let in_flight = app.classifier.quiesce().await;
    let task = tokio::spawn({
        let app = app.clone();
        async move { app.exclusive_maintenance().await.map(|_| ()) }
    });
    let waited = tokio::time::timeout(Duration::from_secs(5), async {
        while app.settings().snapshot().await.is_ok() || count(&views.quiesces) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "maintenance never closed the gate");
    // Past the exclusive acquisition, now waiting on the classifier.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!task.is_finished());
    assert_eq!(count(&views.starts), 0);

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(count(&views.starts), 1);
    assert!(app.settings().snapshot().await.is_ok());
    drop(in_flight);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count(&views.starts), 1, "resumed exactly once");
    app.shutdown().await;
}

#[tokio::test]
async fn a_source_subscribes_only_while_its_extension_is_on() {
    use crate::{archive::catalog::CYBERCAMPUS_SOURCE_ID, extensions::CYBERCAMPUS};
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let subscribed = |sources: &[crate::archive::model::SourceInfo]| {
        sources
            .iter()
            .find(|source| source.definition.id == CYBERCAMPUS_SOURCE_ID)
            .unwrap()
            .subscribed
    };
    let before = subscribed(&app.archive().sources().await.unwrap());

    let refused = app
        .archive()
        .subscribe(CYBERCAMPUS_SOURCE_ID, !before)
        .await;
    assert!(refused.unwrap_err().contains("확장 기능"));
    assert_eq!(subscribed(&app.archive().sources().await.unwrap()), before);

    app.settings()
        .set_extension(CYBERCAMPUS, true)
        .await
        .unwrap();
    let changed = app
        .archive()
        .subscribe(CYBERCAMPUS_SOURCE_ID, !before)
        .await
        .unwrap();
    assert_eq!(subscribed(&changed.sources), !before);
    app.shutdown().await;
}

#[tokio::test]
async fn the_calendar_opens_only_while_its_extension_is_on() {
    use crate::{
        extensions::{CALENDAR, WEEK},
        storage::calendar::TodoInput,
    };
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let (from, to) = ("2026-12-01", "2026-12-07");
    let todo = TodoInput {
        title: "보고서 제출".into(),
        day: "2026-12-03".into(),
        ..TodoInput::default()
    };
    let file = directory.path().join("calendar.ics");
    let off = |error: String| assert!(error.contains("확장 기능"), "{error}");

    // Everything off: the page's reads, switches, writes and export are closed.
    off(app.calendar().todos(from, to).await.unwrap_err());
    off(app.calendar().notice_dates(from, to).await.unwrap_err());
    off(app.calendar().set_all_deadlines(true).await.unwrap_err());
    off(app.calendar().set_academic(false).await.unwrap_err());
    off(app.calendar().create_todo(&todo).await.unwrap_err());
    off(app.calendar().export(&file).await.unwrap_err());
    assert!(!file.exists());
    // The reader is core: a notice's own dates need no extension.
    assert!(app.archive().document_dates(1).await.unwrap().is_empty());

    // The week pane shares the notice dates, nothing else.
    app.settings().set_extension(WEEK, true).await.unwrap();
    let dates = app.calendar().notice_dates(from, to).await.unwrap();
    assert!(!dates.all_deadlines && dates.academic);
    off(app.calendar().todos(from, to).await.unwrap_err());
    off(app.calendar().set_all_deadlines(true).await.unwrap_err());

    app.settings().set_extension(CALENDAR, true).await.unwrap();
    let created = app.calendar().create_todo(&todo).await.unwrap();
    assert_eq!(app.calendar().todos(from, to).await.unwrap(), [created]);
    app.calendar().set_all_deadlines(true).await.unwrap();
    app.calendar().set_academic(false).await.unwrap();
    let dates = app.calendar().notice_dates(from, to).await.unwrap();
    assert!(dates.all_deadlines && !dates.academic);
    let exported = app.calendar().export(&file).await.unwrap();
    assert_eq!((exported.todos, exported.deadlines), (1, 0));
    assert!(std::fs::read_to_string(&file)
        .unwrap()
        .contains("SUMMARY:보고서 제출"));

    // Off again: the todo and both switches are kept, and the week pane still
    // reads the dates.
    app.settings().set_extension(CALENDAR, false).await.unwrap();
    off(app.calendar().todos(from, to).await.unwrap_err());
    let dates = app.calendar().notice_dates(from, to).await.unwrap();
    assert!(dates.all_deadlines && !dates.academic);
    app.settings().set_extension(WEEK, false).await.unwrap();
    off(app.calendar().notice_dates(from, to).await.unwrap_err());
    app.settings().set_extension(CALENDAR, true).await.unwrap();
    assert_eq!(app.calendar().todos(from, to).await.unwrap().len(), 1);
    app.shutdown().await;
}

#[tokio::test]
async fn a_clicked_reminder_is_handed_over_once_and_only_while_the_calendar_is_on() {
    use crate::extensions::CALENDAR;
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();

    // A click that arrives with the calendar off is dropped, not kept for later.
    app.proactive.set_reminder_activation("2026-12-03");
    assert_eq!(app.calendar().reminder_activation().await.unwrap(), None);
    app.settings().set_extension(CALENDAR, true).await.unwrap();
    assert_eq!(app.calendar().reminder_activation().await.unwrap(), None);

    app.proactive.set_reminder_activation("2026-12-03");
    assert_eq!(
        app.calendar().reminder_activation().await.unwrap(),
        Some("2026-12-03".into())
    );
    assert_eq!(app.calendar().reminder_activation().await.unwrap(), None);
    app.shutdown().await;
}

#[tokio::test]
async fn the_menu_opens_only_while_its_extension_is_on() {
    use crate::extensions::MEALS;
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    assert!(app.meals().weeks().await.unwrap_err().contains("식단"));

    app.settings().set_extension(MEALS, true).await.unwrap();
    let meals = app.meals().weeks().await.unwrap();
    // No feed has synced, so there is no week; today is a calendar day.
    assert!(meals.weeks.is_empty());
    assert!(chrono::NaiveDate::parse_from_str(&meals.today, "%Y-%m-%d").is_ok());

    app.settings().set_extension(MEALS, false).await.unwrap();
    assert!(app.meals().weeks().await.is_err());
    app.shutdown().await;
}

// `start` and `automatic_start` are never called here: they use the real seat
// client, so a regression in their admission would send a school request from a
// test. `studium::jobs::tests` covers their refusals with a fake transport.
#[tokio::test]
async fn studium_opens_only_while_its_extension_is_on_and_keeps_its_settings() {
    use crate::studium::model::AlertRuleInput;
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    let views = Arc::new(Views::default());
    let _ = app.views.set(views.clone());
    // In catalog order, as the store returns them.
    let rooms = ["19".to_string(), "5".to_string()];
    let rule = AlertRuleInput {
        enabled: true,
        kind: "remaining".into(),
        minimum: 3,
    };
    let off = |error: String| assert!(error.contains("확장 기능"), "{error}");

    // Everything off: the page's read and every settings write are closed.
    off(app.studium().snapshot().await.unwrap_err());
    off(app.studium_mini().snapshot().await.unwrap_err());
    off(app
        .studium()
        .save_preferences(&rooms, "19")
        .await
        .unwrap_err());
    off(app.studium().set_view_room("5").await.unwrap_err());
    off(app.studium().save_interval(5).await.unwrap_err());
    off(app
        .studium()
        .save_favorites("5", &[3, 1])
        .await
        .unwrap_err());
    off(app.studium().save_alert("5", &rule).await.unwrap_err());
    assert_eq!(count(&views.refreshes), 0);
    assert_eq!(app.studium().notification_activation().await.unwrap(), None);

    app.settings().set_extension(STUDIUM, true).await.unwrap();
    let fresh = app.studium().snapshot().await.unwrap();
    assert!(!fresh.running && fresh.job_id.is_none() && !fresh.automatic.active);
    let before = count(&views.refreshes);

    let saved = app.studium().save_preferences(&rooms, "19").await.unwrap();
    assert_eq!(saved.preferences.room_ids, rooms);
    assert_eq!(saved.preferences.view_room, "19");
    let saved = app.studium().set_view_room("5").await.unwrap();
    assert_eq!(saved.preferences.view_room, "5");
    let saved = app.studium().save_interval(5).await.unwrap();
    assert_eq!(saved.automatic.interval_minutes, 5);
    let saved = app.studium().save_favorites("5", &[3, 1]).await.unwrap();
    assert_eq!(saved.favorites["5"], [1, 3]);
    let saved = app.studium().save_alert("5", &rule).await.unwrap();
    assert!(saved.alert_rules["5"].enabled && saved.alert_rules["5"].minimum == 3);
    // Each save asks the shell's views to redraw, and none starts a check.
    assert_eq!(count(&views.refreshes), before + 5);
    assert!(!saved.running && saved.job_id.is_none() && !saved.automatic.active);

    // The mini window's handle reads the same state and may switch the room.
    let foundation = app.studium_mini().app_snapshot().await.unwrap();
    assert!(foundation
        .extensions
        .iter()
        .any(|e| e.id == STUDIUM && e.enabled));
    let mini = app.studium_mini().set_view_room("19").await.unwrap();
    assert_eq!(mini.preferences.view_room, "19");
    assert_eq!(
        app.studium_mini()
            .snapshot()
            .await
            .unwrap()
            .preferences
            .view_room,
        "19"
    );

    // Off keeps every setting; a reopen keeps them too.
    app.settings().set_extension(STUDIUM, false).await.unwrap();
    off(app.studium().snapshot().await.unwrap_err());
    app.settings().set_extension(STUDIUM, true).await.unwrap();
    app.shutdown().await;
    drop(app);
    let app = open(directory.path()).unwrap();
    let kept = app.studium().snapshot().await.unwrap();
    assert_eq!(kept.preferences.room_ids, rooms);
    assert_eq!(kept.preferences.view_room, "19");
    assert_eq!(kept.automatic.interval_minutes, 5);
    assert_eq!(kept.favorites["5"], [1, 3]);
    assert!(kept.alert_rules["5"].enabled);
    assert!(!kept.running && !kept.automatic.active);
    app.shutdown().await;
}

#[tokio::test]
async fn a_running_seat_check_refuses_new_targets_and_is_cancelled_by_its_id() {
    use crate::studium::{
        jobs::{start_with_transport, tests::BlockedTransport},
        model::RoomStatus,
    };
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    app.settings().set_extension(STUDIUM, true).await.unwrap();
    // The handle's own `start` uses the real seat client; the same admission
    // with a transport that never answers keeps the check in flight.
    let transport = BlockedTransport::new();
    let started = start_with_transport(
        &app.studium,
        &app.operation_gate,
        &app.maintenance_shutdown,
        &app.store,
        transport.clone(),
    )
    .await
    .unwrap();
    transport.entered().await;
    let job = started.job_id.unwrap();

    let running = app.studium().snapshot().await.unwrap();
    assert!(running.running && running.job_id == Some(job));
    assert!(running
        .results
        .iter()
        .all(|room| room.status == RoomStatus::Loading));
    let refused = app.studium().save_preferences(&["19".into()], "19").await;
    assert!(refused.unwrap_err().contains("좌석 확인 중"));
    // Favorites may change during a check; the check keeps its generation.
    let saved = app.studium().save_favorites("5", &[7]).await.unwrap();
    assert!(saved.running && saved.job_id == Some(job));

    // Another id cancels nothing.
    assert!(app.studium().cancel(job + 1).await.unwrap().running);
    app.studium().cancel(job).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), app.studium.wait())
        .await
        .expect("the cancelled check never ended");
    let ended = app.studium().snapshot().await.unwrap();
    assert!(!ended.running);
    assert!(ended
        .results
        .iter()
        .all(|room| room.status == RoomStatus::Cancelled && room.result.is_none()));
    assert_eq!(transport.requests(), 1);
    app.shutdown().await;
}

/// The core is the all-off state: nothing an extension owns is on or running.
/// The classifier loop, scheduler and Studium receiver start idle, so only
/// their effective settings and the absence of work and processes are asserted.
async fn assert_core_only(app: &App) {
    {
        let store = app.store.lock().unwrap();
        for extension in crate::extensions::BUILT_IN {
            assert!(!store.enabled(extension.id).unwrap(), "{}", extension.id);
        }
        assert!(
            !store
                .ai_effective_preferences()
                .unwrap()
                .classification_enabled
        );
        let proactive = store.proactive_effective_settings().unwrap();
        assert!(
            !proactive.scheduler_enabled
                && !proactive.notifications_enabled
                && !proactive.close_to_background
                && !proactive.start_hidden
        );
    }
    let snapshot = app.settings().snapshot().await.unwrap();
    assert_eq!(snapshot.extensions.len(), 11);
    assert!(snapshot.extensions.iter().all(|e| !e.enabled));
    assert!(!app.archive.status().unwrap().running);
    assert!(!app.feed.running());
    assert!(!app.models.busy());
    assert!(app.studium.completion().is_none());
    tokio::time::timeout(Duration::from_secs(1), app.archive.bodies.wait())
        .await
        .expect("a body fetch is running");
    assert!(matches!(app.classifier.status().state, "idle" | "disabled"));
    // No embedding server: its private run directory holds no socket or pid.
    let run = app.data_root.join("run").join("embedding");
    if let Ok(mut entries) = std::fs::read_dir(run) {
        assert!(entries.next().is_none());
    }
}

#[tokio::test]
async fn a_fresh_profile_runs_only_the_core_and_returns_to_it_after_each_extension() {
    let directory = tempfile::tempdir().unwrap();
    let app = open(directory.path()).unwrap();
    app.start(Some(Arc::new(Views::default())));
    assert_core_only(&app).await;

    for extension in crate::extensions::BUILT_IN {
        app.settings()
            .set_extension(extension.id, true)
            .await
            .unwrap();
        app.settings()
            .set_extension(extension.id, false)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_core_only(&app).await;
    }
    app.shutdown().await;
}

#[test]
fn every_extension_is_declared_once_and_its_links_exist() {
    use crate::extensions::{Tokens, Work, BUILT_IN};
    assert_eq!(BUILT_IN.len(), 11);
    let mut ids: Vec<_> = BUILT_IN.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), BUILT_IN.len());
    for extension in BUILT_IN {
        assert!(crate::extensions::is_built_in(extension.id));
        // Everything a disable awaits it also signals, except the classifier,
        // which stops itself and is only woken.
        for work in extension.settles {
            assert!(
                *work == Work::Classifier || extension.cancels.contains(work),
                "{} settles work it never cancels",
                extension.id
            );
        }
    }
    // The cross-extension links: proactive's disable clears both token kinds.
    let clears = |id: &str| crate::extensions::declaration(id).unwrap().clears;
    assert_eq!(
        clears(crate::extensions::PROACTIVE),
        [Tokens::StudiumAlerts, Tokens::CalendarReminders]
    );
    assert_eq!(clears(crate::extensions::STUDIUM), [Tokens::StudiumAlerts]);
    assert_eq!(
        clears(crate::extensions::CALENDAR),
        [Tokens::CalendarReminders]
    );
}

/// A feed transport that holds the sync until it is cancelled, so a feed job
/// is running when shutdown or maintenance asks it to stop.
struct HeldFeed;

impl crate::feed::transport::Transport for HeldFeed {
    async fn get(
        &self,
        _url: &url::Url,
        _limit: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<u8>, crate::feed::transport::FetchError> {
        crate::job::cancelled(cancel, Duration::from_millis(5)).await;
        Err(crate::feed::transport::FetchError::Cancelled)
    }
}

/// Starts a feed sync that only a cancel can end and returns once it runs.
async fn hold_feed(app: &Arc<App>) -> tokio::task::JoinHandle<Result<bool, String>> {
    let held = app.clone();
    let task =
        tokio::spawn(async move { held.feed.run(&held.store, &held.data_root, &HeldFeed).await });
    while !app.feed.running() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    task
}

// Pins that exit and maintenance each cancel and join a running job. Only the
// feed sync can be held through public API and a fake transport; the archive
// job (needs an eClass login), body fetches (need a stored pending notice) and
// model downloads (need a real catalog download) have no such hook, so a job
// type missing from either list is caught here only for the feed. Even then
// the scheduler's quiesce also cancels the feed, so this pins the outcome
// (the job is joined), not that each site's own feed cancel is present.
#[tokio::test]
async fn shutdown_and_maintenance_leave_no_feed_job_running() {
    let directory = tempfile::tempdir().unwrap();
    let app = Arc::new(open(directory.path()).unwrap());
    // A sync only starts while the notices extension is on.
    crate::storage::with_store(&app.store, |s| {
        s.set_enabled(crate::extensions::NOTICES, true)
    })
    .unwrap();

    let task = hold_feed(&app).await;
    let guard = tokio::time::timeout(Duration::from_secs(10), app.exclusive_maintenance())
        .await
        .expect("maintenance waits for the feed job")
        .unwrap();
    assert!(!app.feed.running());
    assert_eq!(task.await.unwrap(), Ok(true));
    drop(guard);

    let task = hold_feed(&app).await;
    tokio::time::timeout(Duration::from_secs(10), app.shutdown())
        .await
        .expect("shutdown waits for the feed job");
    assert!(!app.feed.running());
    assert_eq!(task.await.unwrap(), Ok(true));
}
