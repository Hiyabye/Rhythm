//! The Tauri side of `rhythm_core::shell::Shell`: the main window and the
//! events the frontend listens to.
use rhythm_core::shell::{Event, Shell};
use tauri::{AppHandle, Emitter};

pub(crate) struct AppShell(pub(crate) AppHandle);

impl Shell for AppShell {
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.0.run_on_main_thread(task).map_err(|e| e.to_string())
    }
    fn show_main_window(&self) -> Result<(), String> {
        crate::core::lifecycle::show_main_window(&self.0).map_err(|e| e.to_string())
    }
    fn emit(&self, event: Event) {
        let _ = match event {
            Event::NoticeClicked(activation) => {
                self.0.emit("proactive://notification-clicked", activation)
            }
            Event::ReminderClicked(day) => self.0.emit("calendar://reminder-clicked", day),
            Event::StudiumClicked(room) => self.0.emit("studium://notification-clicked", room),
        };
    }
}
