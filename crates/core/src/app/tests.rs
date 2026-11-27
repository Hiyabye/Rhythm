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
