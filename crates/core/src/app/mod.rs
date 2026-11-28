//! The app as one value (#342): [`App::open`] runs the startup sequence and
//! owns every core handle; area handles such as [`App::settings`] hold the
//! methods the UI calls. A desktop shell (the Tauri app today, the GPUI app
//! later) keeps only windows, tray, menus and OS integration, and forwards
//! [`Event`]s from [`App::events`] to its UI.
//!
//! The fields are public while the command areas move here one by one; once
//! every area has its handle they become private.

mod archive;
mod calendar;
mod meals;
mod settings;
#[cfg(test)]
mod tests;

pub use archive::{original_link, Archive};
pub use calendar::Calendar;
pub use meals::Meals;
pub use settings::{storage_error, Settings, Snapshot};

use crate::{
    archive::sync,
    data_lock::{lock_data_dir, DataDirLock},
    extensions::Registry,
    feed, intelligence,
    operation_gate::{self, AdmissionGate},
    paths, proactive,
    shell::{Event, Shell, Ui},
    storage::Store,
    studium,
};
use std::{
    any::Any,
    fmt,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tokio::{runtime::Handle, sync::broadcast};

/// What the shell hands [`App::open`].
pub struct Config {
    /// The bundle identifier, which names the data directory.
    pub identifier: String,
    /// `Rhythm/<version>` for the app's own requests. Core cannot name it
    /// itself: its package version is 0.0.0.
    pub user_agent: &'static str,
    /// The runtime that background loops and notification receivers run on.
    pub runtime: Handle,
    pub shell: Arc<dyn Shell>,
}

/// Why [`App::open`] did not open the app.
#[derive(Debug)]
pub enum OpenError {
    /// Another Rhythm process owns the data directory: an ordinary rejected
    /// launch, not a crash.
    Locked(String),
    /// A restore staged by the last run could not be finished. Its journal and
    /// the previous files stay, so a later launch retries.
    Restore(String),
    Failed(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked(error) | Self::Restore(error) | Self::Failed(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for OpenError {}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a Studium view owner left open when Studium was turned off, captured
/// under the Store lock so the close that follows affects only that window.
pub struct CloseTicket(pub Box<dyn Any + Send>);

/// Shell windows that show Studium seats (the Tauri mini window and tray).
/// Maintenance pauses them with the other background work, and turning
/// Studium off closes them.
pub trait StudiumViews: Send + Sync + 'static {
    /// Starts the view loop; again after maintenance.
    fn start(self: Arc<Self>) -> Result<(), String>;
    /// Asks the views to redraw from the current settings and seats.
    fn refresh(&self);
    fn quiesce(&self) -> BoxFuture<'_, ()>;
    /// Restarts after a maintenance attempt that gave up before acquiring.
    fn resume_after_quiesce(self: Arc<Self>) -> BoxFuture<'static, ()>;
    /// Stops for good at exit.
    fn shutdown(&self) -> BoxFuture<'_, ()>;
    /// Called under the Store lock while Studium turns off.
    fn capture_close(&self) -> Option<CloseTicket>;
    /// Closes what `capture_close` captured, after the Store lock is released.
    fn close(self: Arc<Self>, ticket: CloseTicket) -> BoxFuture<'static, Result<(), String>>;
}

pub struct App {
    _instance_lock: DataDirLock,
    pub store: sync::SharedStore,
    pub data_root: PathBuf,
    pub proactive: Arc<proactive::Scheduler>,
    pub archive: Arc<sync::Controller>,
    pub feed: Arc<feed::Controller>,
    pub studium: Arc<studium::controller::Controller>,
    pub studium_notifications: Arc<studium::notification::Notifications>,
    pub network: crate::archive::Client,
    pub registry: Registry,
    pub classifier: Arc<intelligence::classification::Controller>,
    pub models: intelligence::models::Controller,
    pub hardware: intelligence::models::Hardware,
    /// Serializes model installs, removals and folder changes.
    pub ai_gate: tokio::sync::Mutex<()>,
    /// Set once at exit; background loops and new jobs stop on it.
    pub maintenance_shutdown: Arc<AtomicBool>,
    pub operation_gate: Arc<AdmissionGate>,
    /// What the restore finished at this launch reported, for the UI.
    pub restore_outcome: Option<crate::backup::RestoreOutcome>,
    user_agent: &'static str,
    ui: Ui,
    runtime: Handle,
    views: OnceLock<Arc<dyn StudiumViews>>,
    started: AtomicBool,
}

impl App {
    /// Locks the data directory, finishes a restore the last run staged, opens
    /// the store and builds every controller. Nothing runs in the background
    /// until [`App::start`].
    pub fn open(config: Config) -> Result<Self, OpenError> {
        let data_root = paths::data_root(&config.identifier).map_err(OpenError::Failed)?;
        let resources = paths::resource_dir().map_err(OpenError::Failed)?;
        Self::open_in(data_root, &resources, config)
    }

    fn open_in(
        data_root: PathBuf,
        resources: &std::path::Path,
        config: Config,
    ) -> Result<Self, OpenError> {
        let failed = |error: String| OpenError::Failed(error);
        let instance_lock = lock_data_dir(&data_root).map_err(OpenError::Locked)?;
        let restore_outcome =
            crate::backup::recover_pending_restore(&data_root).map_err(OpenError::Restore)?;
        crate::backup::remove_stale_backup_snapshots(&data_root);
        let store = Store::open(&paths::database(&data_root)).map_err(failed)?;
        store
            .catalog(&crate::archive::definitions())
            .map_err(failed)?;
        let store = Arc::new(Mutex::new(store));
        let ui = Ui::new(config.shell);
        let operation_gate = Arc::new(AdmissionGate::default());
        let archive = sync::Controller::for_identifier(&config.identifier);
        let studium = studium::controller::Controller::new();
        let studium_notifications = studium::notification::Notifications::new(
            store.clone(),
            Arc::downgrade(&studium),
            operation_gate.clone(),
            ui.clone(),
            config.runtime.clone(),
        );
        studium.attach_notifications(&studium_notifications);
        // The client keeps a `'static` User-Agent; one string per process.
        let archive_agent: &'static str = Box::leak(
            format!("{} (local university notice archive)", config.user_agent).into_boxed_str(),
        );
        let network = crate::archive::network::Client::new(archive_agent).map_err(failed)?;
        let feed = Arc::new(feed::Controller::default());
        let proactive = proactive::Scheduler::new(
            store.clone(),
            archive.clone(),
            feed.clone(),
            data_root.clone(),
            ui.clone(),
            config.runtime.clone(),
            config.user_agent,
            operation_gate.clone(),
        );
        // Bundled llama.cpp builds; the private runtime directory holds the
        // embedding server's socket or per-launch access key.
        let llama = paths::llama(resources);
        let runtime = paths::embedding_runtime(&data_root);
        // A server that a killed or crashed Rhythm left running still holds
        // its model in memory (#273).
        #[cfg(unix)]
        intelligence::stop_orphan(&runtime);
        let classifier = Arc::new(intelligence::classification::Controller::new(
            &llama, &runtime,
        ));
        let hardware = intelligence::models::hardware(&data_root);
        Ok(Self {
            _instance_lock: instance_lock,
            registry: Registry::new(archive.clone(), studium.clone(), feed.clone()),
            store,
            data_root,
            proactive: Arc::new(proactive),
            archive,
            feed,
            studium,
            studium_notifications,
            network,
            classifier,
            models: intelligence::models::Controller::new(config.user_agent),
            hardware,
            ai_gate: tokio::sync::Mutex::new(()),
            maintenance_shutdown: Arc::new(AtomicBool::new(false)),
            operation_gate,
            restore_outcome,
            user_agent: config.user_agent,
            ui,
            runtime: config.runtime,
            views: OnceLock::new(),
            started: AtomicBool::new(false),
        })
    }

    /// A receiver for every event sent after this call. Subscribe before
    /// [`App::start`] to miss none.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.ui.subscribe()
    }

    /// Starts the background work: seat alerts, the shell's Studium views, the
    /// classifier, the 휴지통 and notice-date loops and the scheduler. Call it
    /// once, after the shell can serve the UI's calls.
    pub fn start(&self, views: Option<Arc<dyn StudiumViews>>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(views) = views {
            let _ = self.views.set(views);
        }
        if let Err(error) = self.studium_notifications.start() {
            eprintln!("Studium: 알림 수신을 시작하지 못했습니다: {error}");
        }
        if let Some(views) = self.views.get() {
            if let Err(error) = views.clone().start() {
                eprintln!("Studium: 표시 수신 시작 실패: {error}");
            }
        }
        let shutdown = &self.maintenance_shutdown;
        self.runtime.spawn(intelligence::classification::run(
            self.classifier.clone(),
            self.store.clone(),
            self.models.clone(),
            self.data_root.clone(),
            shutdown.clone(),
        ));
        self.runtime.spawn(crate::archive::run_auto_trash(
            self.store.clone(),
            self.operation_gate.clone(),
            shutdown.clone(),
        ));
        self.runtime.spawn(crate::archive::run_notice_dates(
            self.store.clone(),
            self.operation_gate.clone(),
            shutdown.clone(),
        ));
        if let Err(error) = self.proactive.start() {
            // Proactive operation is optional. A scheduler start failure must
            // not take away archive, AI or settings access.
            eprintln!("Proactive: 스케줄러를 시작하지 못했습니다: {error}");
        }
    }

    pub fn settings(&self) -> Settings<'_> {
        Settings(self)
    }

    pub fn archive(&self) -> Archive<'_> {
        Archive(self)
    }

    pub fn calendar(&self) -> Calendar<'_> {
        Calendar(self)
    }

    pub fn meals(&self) -> Meals<'_> {
        Meals(self)
    }

    fn views(&self) -> Option<&Arc<dyn StudiumViews>> {
        self.views.get()
    }

    fn refresh_views(&self) {
        if let Some(views) = self.views() {
            views.refresh();
        }
    }

    /// Stops every job, loop and the embedding server for exit, waiting for
    /// each. The shell calls it once when the process is about to end.
    pub async fn shutdown(&self) {
        self.maintenance_shutdown.store(true, Ordering::Release);
        // Serialize cancellation against a start already admitted by the
        // gate but not yet registered. Starts recheck shutdown under Store.
        let studium_completion = studium::jobs::cancel_at_boundary(&self.store, &self.studium)
            .ok()
            .flatten();
        self.registry.stop_all();
        self.models.cancel();
        if let Some(views) = self.views() {
            views.shutdown().await;
        }
        self.studium_notifications.shutdown().await;
        self.proactive.shutdown().await;
        self.archive.wait().await;
        self.archive.bodies.wait().await;
        self.feed.wait().await;
        if let Some(completion) = studium_completion {
            completion.wait().await;
        } else {
            // A poisoned Store prevented capture, but Registry still
            // signalled cancellation; exit must join that job as well.
            self.studium.wait().await;
        }
        self.models.wait().await;
        self.classifier.shutdown().await;
    }

    /// Closes admission, stops every job and background loop and waits for
    /// them, for backup, restore and update installs. Dropping the guard
    /// restarts the work.
    pub async fn exclusive_maintenance(&self) -> Result<MaintenanceGuard, String> {
        let (reservation, studium_completion) =
            studium::jobs::reserve_maintenance(&self.operation_gate, &self.store, &self.studium)?;
        let mut preparation = MaintenancePreparation {
            reservation: Some(reservation),
            proactive: self.proactive.clone(),
            studium_notifications: self.studium_notifications.clone(),
            views: self.views().cloned(),
            runtime: self.runtime.clone(),
            armed: true,
        };
        self.archive.cancel();
        self.archive.bodies.cancel();
        self.feed.cancel();
        self.models.cancel();
        // A command admitted before the reservation may start a download
        // under the AI gate; cancel again once it has released the gate.
        let ai_gate = self.ai_gate.lock().await;
        self.models.cancel();
        drop(ai_gate);
        self.proactive.quiesce().await;
        self.studium_notifications.quiesce().await;
        if let Some(views) = self.views() {
            views.quiesce().await;
        }
        self.archive.wait().await;
        self.archive.bodies.wait().await;
        self.feed.wait().await;
        if let Some(completion) = studium_completion {
            completion.wait().await;
        }
        self.models.wait().await;
        let gate = preparation
            .reservation
            .take()
            .expect("maintenance reservation remains until exclusive acquisition")
            .acquire()
            .await?;
        // The guard takes over resuming now, so a caller dropped while the
        // classifier quiesces still restarts the work, exactly once.
        let mut guard = MaintenanceGuard {
            _gate: gate,
            proactive: self.proactive.clone(),
            studium_notifications: self.studium_notifications.clone(),
            views: self.views().cloned(),
            classifier_guard: None,
        };
        preparation.armed = false;
        // A classification batch writes SQLite; exclude it from backup/restore
        // snapshots and wait for one already in flight before returning.
        guard.classifier_guard = Some(self.classifier.quiesce().await);
        Ok(guard)
    }
}

pub struct MaintenanceGuard {
    _gate: operation_gate::MaintenanceGuard,
    proactive: Arc<proactive::Scheduler>,
    studium_notifications: Arc<studium::notification::Notifications>,
    views: Option<Arc<dyn StudiumViews>>,
    classifier_guard: Option<intelligence::classification::PauseGuard>,
}

impl MaintenanceGuard {
    /// Keeps maintenance closed and work stopped for the rest of the process.
    /// [`App::shutdown`] stops the classifier by taking its batch guard, so an
    /// exit requested while this guard is held must release that guard first,
    /// or the main thread blocks forever and the window stops responding.
    pub fn hold_until_exit(mut self) {
        if let Some(classifier_guard) = self.classifier_guard.take() {
            classifier_guard.release_for_exit();
        }
        std::mem::forget(self);
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        drop(self.classifier_guard.take());
        if let Err(error) = self.proactive.start() {
            eprintln!("Proactive: 유지보수 후 스케줄러를 다시 시작하지 못했습니다: {error}");
        }
        if let Err(error) = self.studium_notifications.start() {
            eprintln!("Studium: 유지보수 후 알림 수신을 다시 시작하지 못했습니다: {error}");
        }
        if let Some(views) = &self.views {
            if let Err(error) = views.clone().start() {
                eprintln!("Studium: 표시 재개 실패: {error}");
            }
        }
    }
}

struct MaintenancePreparation {
    reservation: Option<operation_gate::MaintenanceReservation>,
    proactive: Arc<proactive::Scheduler>,
    studium_notifications: Arc<studium::notification::Notifications>,
    views: Option<Arc<dyn StudiumViews>>,
    runtime: Handle,
    armed: bool,
}

impl Drop for MaintenancePreparation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let proactive = self.proactive.clone();
        let studium_notifications = self.studium_notifications.clone();
        let views = self.views.clone();
        self.runtime.spawn(async move {
            proactive.resume_after_quiesce().await;
            studium_notifications.resume_after_quiesce().await;
            if let Some(views) = views {
                views.resume_after_quiesce().await;
            }
        });
    }
}
