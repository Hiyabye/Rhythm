//! The GPUI side of `rhythm_core::shell`. Core calls the shell from its own
//! threads, while GPUI's app context lives only on the UI thread, so both
//! `Shell` calls become commands that a UI-thread task runs in order.

use gpui_kit::App;
use rhythm_core::shell::{Event, Shell};
use tokio::sync::{
    broadcast::{self, error::RecvError},
    mpsc,
};

enum Command {
    Run(Box<dyn FnOnce() + Send>),
    ShowMain,
}

pub(crate) struct GpuiShell(mpsc::UnboundedSender<Command>);

/// The UI-thread end of a [`GpuiShell`], handed to [`serve`].
pub(crate) struct Commands(mpsc::UnboundedReceiver<Command>);

impl GpuiShell {
    pub(crate) fn new() -> (Self, Commands) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self(sender), Commands(receiver))
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.0
            .send(command)
            .map_err(|_| "앱 창이 이미 닫혔습니다.".to_string())
    }
}

impl Shell for GpuiShell {
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
        self.send(Command::Run(task))
    }
    /// Core calls this from inside a `run_on_main_thread` task, which has no
    /// GPUI context, so it is queued behind that task.
    fn show_main_window(&self) -> Result<(), String> {
        self.send(Command::ShowMain)
    }
}

/// Runs the shell's commands on the UI thread for as long as the app lives.
pub(crate) fn serve(commands: Commands, cx: &mut App) {
    let Commands(mut receiver) = commands;
    cx.spawn(async move |cx| {
        while let Some(command) = receiver.recv().await {
            match command {
                Command::Run(task) => task(),
                Command::ShowMain => cx.update(crate::window::show_main),
            }
        }
    })
    .detach();
}

/// Applies core's events on the UI thread until the app's channel closes.
pub(crate) fn forward_events(mut events: broadcast::Receiver<Event>, cx: &mut App) {
    cx.spawn(async move |cx| loop {
        match events.recv().await {
            Ok(event) => cx.update(|cx| crate::window::apply_event(event, cx)),
            Err(RecvError::Lagged(missed)) => {
                eprintln!("화면에 알릴 이벤트 {missed}개를 놓쳤습니다.");
            }
            Err(RecvError::Closed) => break,
        }
    })
    .detach();
}
