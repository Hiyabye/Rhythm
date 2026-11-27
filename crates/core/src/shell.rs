//! What core asks of the desktop shell when the user clicks one of its
//! notifications: restore the main window and tell it what was clicked. The
//! Tauri app implements it with its window API and `AppHandle::emit`.

/// A notification click for the main window.
#[derive(Clone, Debug)]
pub enum Event {
    /// A seat alert (`studium://notification-clicked`): the room's ID.
    StudiumClicked(String),
}

pub trait Shell: Send + Sync + 'static {
    /// Runs `task` on the UI thread, where window calls must happen.
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String>;
    /// Shows, unhides and focuses the main window. Call it from the UI thread.
    fn show_main_window(&self) -> Result<(), String>;
    /// Sends `event` to the main window.
    fn emit(&self, event: Event);
}
