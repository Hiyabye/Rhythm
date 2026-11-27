//! How core reaches the desktop shell: `Shell` restores the main window when
//! the user clicks one of core's notifications, and [`Event`]s go out on a
//! broadcast channel that the shell subscribes to through `App::events`.

use std::sync::Arc;
use tokio::sync::broadcast;

/// Something the UI should react to. The Tauri app forwards each one to the
/// frontend under the event name noted on its variant.
#[derive(Clone, Debug)]
pub enum Event {
    /// A notice digest was clicked (`proactive://notification-clicked`).
    NoticeClicked(crate::proactive::NotificationActivation),
    /// A todo reminder was clicked (`calendar://reminder-clicked`): the todo's day.
    ReminderClicked(String),
    /// A seat alert was clicked (`studium://notification-clicked`): the room's ID.
    StudiumClicked(String),
    /// The saved theme (`rhythm://theme-changed`).
    ThemeChanged(String),
    /// The saved 12/24-hour setting (`rhythm://clock-format-changed`).
    ClockFormatChanged(String),
}

pub trait Shell: Send + Sync + 'static {
    /// Runs `task` on the UI thread, where window calls must happen.
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String>;
    /// Shows, unhides and focuses the main window. Call it from the UI thread.
    fn show_main_window(&self) -> Result<(), String>;
}

/// Events waiting for a slow subscriber before it starts missing them.
const EVENT_BUFFER: usize = 64;

/// The shell plus the event channel, which is what core's notification code
/// needs to restore the window and say what was clicked.
#[derive(Clone)]
pub struct Ui {
    shell: Arc<dyn Shell>,
    events: broadcast::Sender<Event>,
}

impl Ui {
    pub fn new(shell: Arc<dyn Shell>) -> Self {
        Self {
            shell,
            events: broadcast::channel(EVENT_BUFFER).0,
        }
    }
    pub fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.shell.run_on_main_thread(task)
    }
    pub fn show_main_window(&self) -> Result<(), String> {
        self.shell.show_main_window()
    }
    /// Sends `event` to every subscriber; with none it is dropped.
    pub fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}
