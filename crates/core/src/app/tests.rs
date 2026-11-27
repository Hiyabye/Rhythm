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
