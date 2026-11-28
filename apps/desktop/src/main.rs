//! The GPUI desktop app (v3, #339), built on `rhythm-core` beside the shipped
//! Tauri app until the cutover (#356). This shell (#343) opens the core app,
//! shows the main window with navigation from the extension registry and
//! stops the core app at quit.
// A release build is a GUI app on Windows, with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod nav;
mod shell;
mod startup;
mod window;

use backend::Backend;
use gpui_kit::*;
use rhythm_core::app::{App as Rhythm, Config};
use std::sync::Arc;

/// `Rhythm/0.0.0` until the cutover gives this crate the release version;
/// the release workflow checks only the Tauri app's.
const USER_AGENT: &str = concat!("Rhythm/", env!("CARGO_PKG_VERSION"));

/// A validation identifier until the cutover, so this binary never opens the
/// real profile. Being `.validation`, it also honors
/// `RHYTHM_VALIDATION_DATA_DIR` (`rhythm_core::paths`).
const IDENTIFIER: &str = "io.github.hiyabye.rhythm.gpui.validation";

actions!(rhythm, [Quit]);

fn main() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("rhythm-core")
        .build()
    {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            eprintln!("Rhythm을 시작하지 못했습니다: {error}");
            std::process::exit(1);
        }
    };
    let (shell, commands) = shell::GpuiShell::new();
    // Before the UI starts: it takes the data-directory lock, then opens the store.
    let opened = Rhythm::open(Config {
        identifier: IDENTIFIER.into(),
        user_agent: USER_AGENT,
        runtime: runtime.handle().clone(),
        shell: Arc::new(shell),
    });

    let application = gpui_kit::application();
    application.on_reopen(window::show_main);
    application.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        component::Theme::sync_system_appearance(None, cx);
        install_quit(cx);
        let app = match opened {
            Ok(app) => Arc::new(app),
            Err(error) => return startup::show(error, cx),
        };
        shell::serve(commands, cx);
        shell::forward_events(app.events(), cx);
        cx.set_global(Backend {
            app: app.clone(),
            runtime: runtime.clone(),
        });
        cx.on_app_quit(|cx| {
            // Blocks the UI thread until every job and loop has stopped, as
            // the Tauri app does at exit. The shell's commands only queue, so
            // nothing core waits for needs this thread.
            if let Some(backend) = cx.try_global::<Backend>() {
                backend.runtime.block_on(backend.app.shutdown());
            }
            async {}
        })
        .detach();
        window::show_main(cx);
        app.start(None);
    });
}

/// 종료 in the app menu and its shortcut. The full app menu is #344.
fn install_quit(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus(vec![Menu {
        name: "Rhythm".into(),
        items: vec![MenuItem::action("Rhythm 종료", Quit)],
        disabled: false,
    }]);
}
