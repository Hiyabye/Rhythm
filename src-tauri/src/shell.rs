//! The Tauri side of `rhythm_core::shell`: the main window, and the core
//! events forwarded to the frontend under the names it listens to.
use rhythm_core::shell::{Event, Shell};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::{error::RecvError, Receiver};

pub(crate) struct AppShell(pub(crate) AppHandle);

impl Shell for AppShell {
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.0.run_on_main_thread(task).map_err(|e| e.to_string())
    }
    fn show_main_window(&self) -> Result<(), String> {
        crate::core::lifecycle::show_main_window(&self.0).map_err(|e| e.to_string())
    }
}

/// Emits every core event to the frontend until the app's channel closes.
pub(crate) fn forward_events(app: AppHandle, mut events: Receiver<Event>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => emit(&app, event),
                Err(RecvError::Lagged(missed)) => {
                    eprintln!("화면에 알릴 이벤트 {missed}개를 놓쳤습니다.");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn emit(app: &AppHandle, event: Event) {
    let _ = match event {
        Event::NoticeClicked(activation) => {
            app.emit("proactive://notification-clicked", activation)
        }
        Event::ReminderClicked(day) => app.emit("calendar://reminder-clicked", day),
        Event::StudiumClicked(room) => app.emit("studium://notification-clicked", room),
        Event::ThemeChanged(theme) => app.emit("rhythm://theme-changed", theme),
        Event::ClockFormatChanged(format) => app.emit("rhythm://clock-format-changed", format),
    };
}
