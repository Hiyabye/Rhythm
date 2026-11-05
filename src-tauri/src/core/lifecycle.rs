//! Native desktop lifecycle: tray, window restore, close behavior and login
//! registration.
//!
//! Persistence is owned by `storage::proactive`. This module reads the two
//! lifecycle preferences through [`LifecyclePreferences`], which `main.rs`
//! implements over the store, so a settings save from another command is
//! observed immediately instead of through a stale cache.
//!
//! Autostart registration is never performed during startup. It only changes
//! through the explicit commands in `main.rs`, and every change is read back
//! from the operating system before it is reported as successful.

use serde::Serialize;
use tauri::{menu::MenuBuilder, tray::TrayIconBuilder, AppHandle, CloseRequestApi, Manager};

/// Label of the single application window declared in `tauri.conf.json`.
pub const MAIN_WINDOW: &str = "main";
/// Passed only by the operating system's login registration.
pub const AUTOSTART_ARG: &str = "--autostart";
/// Manual debugging/launch flag for a hidden start. Login launches decide
/// visibility from the persisted `start_hidden` preference instead.
pub const HIDDEN_ARG: &str = "--hidden";
/// Bundle identifier of the distributed application.
const PRODUCTION_IDENTIFIER: &str = "io.github.hiyabye.rhythm";
/// Login entry already created by every released build.
///
/// macOS names the LaunchAgent file and label after it; Windows names the
/// per-user `Run` value after it. Production builds keep writing exactly this
/// entry, so an installed app keeps its login registration across upgrades.
#[cfg(not(windows))]
const LEGACY_AUTOSTART_NAME: &str = "Rhythm";
#[cfg(windows)]
const LEGACY_AUTOSTART_NAME: &str = "Rhythm.io.github.hiyabye.rhythm";

/// Login registration name for the build with `identifier`.
///
/// Only the production identifier owns the historical entry. Every other
/// identifier (a `.validation` build, a test identifier) gets its own entry
/// named after that identifier, so it can never read, replace or delete the
/// installed app's registration. The name ends up in a LaunchAgent file name
/// or a registry value name, so it keeps only ASCII alphanumerics, `-` and `.`.
pub fn autostart_name(identifier: &str) -> String {
    if identifier == PRODUCTION_IDENTIFIER {
        return LEGACY_AUTOSTART_NAME.into();
    }
    let cleaned: String = identifier
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.trim_matches(['-', '.']).is_empty() {
        // A config identifier that is empty or punctuation-only still needs a
        // distinct, usable name; it must never fall back to the legacy entry.
        "Rhythm-test-build".into()
    } else {
        cleaned
    }
}

const TRAY_SHOW: &str = "tray.show";
const TRAY_QUIT: &str = "tray.quit";
const TRAY_ID: &str = "rhythm-status";

/// The lifecycle preferences owned by persisted settings.
pub trait LifecyclePreferences: Send + Sync + 'static {
    /// Whether closing the main window should keep the application running in
    /// the background instead of quitting. Effective only when a tray icon was
    /// installed successfully.
    fn close_to_background(&self) -> bool;
    /// Whether an OS login launch should stay hidden. This governs only the
    /// `--autostart` launch path and never a normal manual launch.
    fn start_hidden(&self) -> bool;
}

/// How this process was started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaunchMode {
    pub autostart: bool,
    pub hidden: bool,
}

impl LaunchMode {
    /// Reads the real process arguments, excluding the executable path.
    /// Non-Unicode arguments cannot match our flags and are not a reason to
    /// abort startup.
    pub fn current() -> Self {
        let mut mode = Self::default();
        for argument in std::env::args_os().skip(1) {
            let argument = argument.to_string_lossy();
            if argument == AUTOSTART_ARG {
                mode.autostart = true;
            } else if argument == HIDDEN_ARG {
                mode.hidden = true;
            }
        }
        mode
    }

    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut mode = Self::default();
        for argument in args {
            match argument.as_ref() {
                AUTOSTART_ARG => mode.autostart = true,
                HIDDEN_ARG => mode.hidden = true,
                _ => {}
            }
        }
        mode
    }

    /// Login visibility comes from the explicit preference; a manual `--hidden`
    /// launch stays hidden regardless of that preference. A login launch cannot
    /// stay hidden without a tray because there would be no restore path.
    pub fn start_hidden(self, start_hidden_preference: bool, tray_available: bool) -> bool {
        if self.autostart {
            start_hidden_preference && tray_available
        } else {
            self.hidden
        }
    }
}

/// Actual OS registration state. Never derived from a persisted intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
}

#[cfg(windows)]
pub(crate) fn autostart_snapshot(
    app: &AppHandle,
) -> Result<super::windows_autostart::Snapshot, String> {
    super::windows_autostart::snapshot(&autostart_entry(app))
        .map_err(|error| autostart_error("확인하지", error))
}

#[cfg(windows)]
pub(crate) fn restore_autostart(
    snapshot: &super::windows_autostart::Snapshot,
) -> Result<(), String> {
    super::windows_autostart::restore(snapshot).map_err(|error| autostart_error("복구하지", error))
}

/// Installs the tray icon (the monochrome Beat template on macOS) and its
/// native menu.
///
/// A failure is reported to the caller; the application continues without a
/// tray, and close-to-background is then refused by the command layer.
pub fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .text(TRAY_SHOW, "Rhythm 열기")
        .separator()
        .text(TRAY_QUIT, "Rhythm 종료")
        .build()?;
    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray-template.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/32x32.png"))?;
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("Rhythm")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TRAY_SHOW => restore_main_window(app),
            TRAY_QUIT => app.exit(0),
            _ => {}
        });
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);
    builder.build(app)?;
    Ok(())
}

fn main_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW)
        .ok_or(tauri::Error::WindowNotFound)
}

/// Shows, unminimizes and focuses the existing window. Used for tray restore,
/// macOS reopen and a second instance; it never creates another window or core.
pub fn show_main_window(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = app.show() {
            // The window can still be restored even if app unhiding fails.
            eprintln!("Lifecycle: 앱을 다시 표시하지 못했습니다: {error}");
        }
    }
    let window = main_window(app)?;
    window.show()?;
    window.unminimize()?;
    if let Err(error) = window.set_focus() {
        // The window is already visible; a focus failure is not a failed restore.
        eprintln!("Lifecycle: 창에 포커스를 주지 못했습니다: {error}");
    }
    Ok(())
}

/// Shows the window without requesting focus. Used for an explicitly visible
/// login launch so starting at login does not steal focus from other apps.
pub fn reveal_main_window(app: &AppHandle) -> tauri::Result<()> {
    let window = main_window(app)?;
    window.show()?;
    window.unminimize()?;
    Ok(())
}

fn hide_main_window(app: &AppHandle) -> tauri::Result<()> {
    main_window(app)?.hide()
}

/// Handles a close request for the main window.
///
/// With close-to-background disabled (the conservative default) the window
/// close is an explicit quit, which runs the existing `RunEvent::Exit`
/// cleanup. With it enabled the window is hidden and the application keeps
/// running for tray restore, macOS reopen and second-instance activation.
pub fn handle_main_close(
    app: &AppHandle,
    label: &str,
    api: &CloseRequestApi,
    close_to_background: bool,
) {
    if label != MAIN_WINDOW {
        return;
    }
    if close_to_background {
        api.prevent_close();
        if let Err(error) = hide_main_window(app) {
            eprintln!("Lifecycle: 창을 백그라운드로 숨기지 못해 종료합니다: {error}");
            app.exit(0);
        }
    } else {
        app.exit(0);
    }
}

/// macOS dock-icon activation. `has_visible_windows` is false when the window
/// was hidden or closed to the background.
#[cfg(target_os = "macos")]
pub fn handle_reopen(app: &AppHandle, has_visible_windows: bool) {
    if !has_visible_windows {
        restore_main_window(app);
    }
}

fn restore_main_window(app: &AppHandle) {
    if let Err(error) = show_main_window(app) {
        eprintln!("Lifecycle: 기존 창을 다시 표시하지 못했습니다: {error}");
    }
}

/// A second launch hands its arguments to the running instance and exits
/// before opening the data directory. Restore the first instance's window for
/// an ordinary launch; an operating-system login launch restores it without
/// requesting focus.
pub fn handle_second_instance(app: &AppHandle, launch: LaunchMode) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if launch.autostart {
            if let Err(error) = reveal_main_window(&handle) {
                eprintln!("Lifecycle: 기존 창을 다시 표시하지 못했습니다: {error}");
            }
        } else {
            restore_main_window(&handle);
        }
    }) {
        eprintln!("Lifecycle: 두 번째 실행 요청을 처리하지 못했습니다: {error}");
    }
}

fn autostart_error(action: &str, error: impl std::fmt::Display) -> String {
    eprintln!("Autostart ({action}): {error}");
    #[cfg(windows)]
    let place = "Windows 시작 앱 설정";
    #[cfg(not(windows))]
    let place = "운영체제의 로그인 항목 설정";
    format!("로그인 시 자동 실행을 {action} 못했습니다. {place}과 앱 설치 위치를 확인해 주세요.")
}

/// The login entry name this build must own.
#[cfg(windows)]
fn autostart_entry(app: &AppHandle) -> String {
    autostart_name(&app.config().identifier)
}

/// Reads the actual OS login-registration state for this build's entry.
#[cfg(windows)]
pub fn autostart_status(app: &AppHandle) -> Result<AutostartStatus, String> {
    super::windows_autostart::status(&autostart_entry(app))
        .map(|enabled| AutostartStatus { enabled })
        .map_err(|error| autostart_error("확인하지", error))
}

/// Enables or disables this build's OS login registration. The registry module
/// re-reads the Run and StartupApproved values; a silent failure is an error.
#[cfg(windows)]
pub fn set_autostart(app: &AppHandle, enabled: bool) -> Result<AutostartStatus, String> {
    super::windows_autostart::set(&autostart_entry(app), enabled)
        .map(|enabled| AutostartStatus { enabled })
        .map_err(|error| autostart_error(if enabled { "켜지" } else { "끄지" }, error))
}

/// Reads the actual OS login-registration state.
#[cfg(not(windows))]
pub fn autostart_status(app: &AppHandle) -> Result<AutostartStatus, String> {
    use tauri_plugin_autostart::ManagerExt as _;

    app.autolaunch()
        .is_enabled()
        .map(|enabled| AutostartStatus { enabled })
        .map_err(|error| autostart_error("확인하지", error))
}

/// Enables or disables OS login registration and verifies the result by
/// reading the OS state again. A silent platform failure is an error, not a
/// success.
#[cfg(not(windows))]
pub fn set_autostart(app: &AppHandle, enabled: bool) -> Result<AutostartStatus, String> {
    use tauri_plugin_autostart::ManagerExt as _;

    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(error) = result {
        return Err(autostart_error(
            if enabled { "켜지" } else { "끄지" },
            error,
        ));
    }
    let actual = manager
        .is_enabled()
        .map_err(|error| autostart_error("확인하지", error))?;
    if actual != enabled {
        return Err(autostart_error(
            if enabled {
                "등록하지"
            } else {
                "해제하지"
            },
            "운영체제가 변경을 반영하지 않았습니다.",
        ));
    }
    Ok(AutostartStatus { enabled: actual })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_flags_are_parsed_independently() {
        assert_eq!(LaunchMode::from_args(["app"]), LaunchMode::default());
        assert!(LaunchMode::from_args(["app", AUTOSTART_ARG]).autostart);
        assert!(LaunchMode::from_args(["app", HIDDEN_ARG]).hidden);
        assert_eq!(
            LaunchMode::from_args(["app", AUTOSTART_ARG, HIDDEN_ARG]),
            LaunchMode {
                autostart: true,
                hidden: true
            }
        );
    }

    #[test]
    fn login_visibility_uses_only_the_preference_and_requires_a_tray_to_hide() {
        let login = LaunchMode::from_args([AUTOSTART_ARG]);
        assert!(!login.start_hidden(false, true));
        assert!(login.start_hidden(true, true));
        assert!(!login.start_hidden(true, false));

        // The OS login path is governed by the saved preference, not the
        // manual-only debugging flag.
        let login_with_manual_flag = LaunchMode::from_args([AUTOSTART_ARG, HIDDEN_ARG]);
        assert!(!login_with_manual_flag.start_hidden(false, true));
        assert!(login_with_manual_flag.start_hidden(true, true));

        let manual = LaunchMode::from_args([HIDDEN_ARG]);
        assert!(manual.start_hidden(false, true));
        assert!(manual.start_hidden(true, true));
        assert!(manual.start_hidden(true, false));

        assert!(!LaunchMode::default().start_hidden(true, false));
    }

    #[test]
    fn production_identifier_keeps_the_entry_installed_builds_already_use() {
        // Upgrading must not lose the user's login registration: the installed
        // entry name is unchanged, so the OS state check still finds it.
        assert_eq!(autostart_name(PRODUCTION_IDENTIFIER), LEGACY_AUTOSTART_NAME);
        assert_eq!(
            autostart_name(PRODUCTION_IDENTIFIER),
            match cfg!(windows) {
                true => "Rhythm.io.github.hiyabye.rhythm",
                false => "Rhythm",
            }
        );
    }

    #[test]
    fn the_packaged_identifier_is_the_production_one() {
        // The upgrade path depends on `tauri.conf.json` keeping the identifier
        // that maps to the login entry released builds already created.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], PRODUCTION_IDENTIFIER);
    }

    #[test]
    fn other_identifiers_never_touch_the_installed_entry() {
        for identifier in [
            "io.github.hiyabye.rhythm.validation",
            "io.github.hiyabye.rhythm-test",
            "com.example.rhythm.test",
        ] {
            let name = autostart_name(identifier);
            assert_eq!(name, identifier);
            assert_ne!(name, LEGACY_AUTOSTART_NAME);
            assert_ne!(name, autostart_name(PRODUCTION_IDENTIFIER));
        }
    }

    #[test]
    fn entry_names_are_safe_for_device_files_and_registry_values() {
        let name = autostart_name("com.example.rhythm test/빌드");
        assert_eq!(name, "com.example.rhythm-test---");
        assert!(name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.')));
        // An unusable identifier still gets a distinct name instead of the
        // production entry.
        for identifier in ["", "/", "..", "  "] {
            let name = autostart_name(identifier);
            assert!(!name.is_empty());
            assert_ne!(name, LEGACY_AUTOSTART_NAME);
        }
    }
}
