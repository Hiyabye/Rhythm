//! Windows toast handoff. Activation is delivered separately through the
//! installed `rhythm-win:` protocol handler, including after process exit.
use windows::{
    core::{w, Interface, GUID, HSTRING, PCWSTR},
    Data::Xml::Dom::XmlDocument,
    Win32::{
        Foundation::{ERROR_SUCCESS, PROPERTYKEY},
        System::{
            Com::{
                CoCreateInstance, CoTaskMemFree, IPersistFile,
                StructuredStorage::{PropVariantClear, PropVariantToGUID, PropVariantToString},
                CLSCTX_INPROC_SERVER, STGM_READ,
            },
            Registry::{
                RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
                KEY_QUERY_VALUE, REG_SZ, REG_VALUE_TYPE,
            },
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED, RO_INIT_SINGLETHREADED},
        },
        UI::Shell::{
            FOLDERID_Programs, IShellLinkW, PropertiesSystem::IPropertyStore, SHGetKnownFolderPath,
            ShellLink, KF_FLAG_DEFAULT,
        },
    },
    UI::Notifications::{
        NotificationSetting, ToastNotification, ToastNotificationManager, ToastNotifier,
    },
};

const APP_ID: &str = "io.github.hiyabye.rhythm";
const PREFIX: &str = "rhythm-win://notification/";
const SETTING_NOT_FOUND: i32 = 0x80070490_u32 as i32;

/// Call before creating any window so Windows associates this process with
/// the same identity as the installed Start Menu shortcut.
pub(crate) fn initialize_process_identity() -> Result<(), String> {
    unsafe {
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(w!(
            "io.github.hiyabye.rhythm"
        ))
    }
    .map_err(|error| format!("{error:?}"))
}

#[derive(Debug, PartialEq, Eq)]
enum SettingState {
    Authorized,
    Denied,
    SetupRequired,
    Unavailable,
}

impl SettingState {
    fn status(&self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::Denied => "denied",
            Self::SetupRequired => "not_determined",
            Self::Unavailable => "unavailable",
        }
    }
}

fn classify_setting(setting: Result<NotificationSetting, i32>) -> SettingState {
    match setting {
        Ok(NotificationSetting::Enabled) => SettingState::Authorized,
        Ok(_) => SettingState::Denied,
        Err(SETTING_NOT_FOUND) => SettingState::SetupRequired,
        Err(_) => SettingState::Unavailable,
    }
}

// System.AppUserModel.ID, from the Windows property system.
const APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const TOAST_ACTIVATOR_CLSID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 26,
};
// A protocol-only stub, matching the installer. No COM server is registered.
const TOAST_STUB_CLSID: GUID = GUID::from_u128(0x2860d0e5_a45a_499e_a803_ea87ee435c32);

fn shortcut_matches_current_exe() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    shortcut_check_registered(&current).is_ok()
}

fn shortcut_check_registered(current: &std::path::Path) -> Result<(), String> {
    // COM is initialized by RuntimeGuard before entering this function.
    let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None) }
        .map_err(|e| format!("Programs known folder: {e:?}"))?;
    let folder_string = unsafe { folder.to_string() };
    unsafe { CoTaskMemFree(Some(folder.0.cast())) };
    let folder_string = folder_string.map_err(|_| "Programs folder has invalid UTF-16")?;
    let path = std::path::Path::new(&folder_string).join("Rhythm.lnk");
    shortcut_check_executable(&path, current)
}

fn shortcut_check_executable(
    path: &std::path::Path,
    current: &std::path::Path,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|e| format!("ShellLink creation: {e:?}"))?;
    let persist = link
        .cast::<IPersistFile>()
        .map_err(|e| format!("ShellLink persistence interface: {e:?}"))?;
    unsafe { persist.Load(PCWSTR(path_wide.as_ptr()), STGM_READ) }
        .map_err(|e| format!("ShellLink load: {e:?}"))?;
    let mut target = vec![0u16; 32768];
    unsafe { link.GetPath(&mut target, std::ptr::null_mut(), 0) }
        .map_err(|e| format!("ShellLink target: {e:?}"))?;
    let end = target
        .iter()
        .position(|&ch| ch == 0)
        .ok_or("ShellLink target is unterminated")?;
    let target =
        String::from_utf16(&target[..end]).map_err(|_| "ShellLink target has invalid UTF-16")?;
    if !paths_match(&target, current) {
        return Err("ShellLink target does not match the executable".into());
    }
    let properties = link
        .cast::<IPropertyStore>()
        .map_err(|e| format!("ShellLink property interface: {e:?}"))?;
    let mut value = unsafe { properties.GetValue(&APP_USER_MODEL_ID) }
        .map_err(|e| format!("ShellLink AUMID read: {e:?}"))?;
    let mut text = [0u16; 256];
    let result = unsafe { PropVariantToString(&value, &mut text) };
    unsafe {
        let _ = PropVariantClear(&mut value);
    }
    result.map_err(|e| format!("ShellLink AUMID conversion: {e:?}"))?;
    let id = text
        .iter()
        .position(|&ch| ch == 0)
        .and_then(|end| String::from_utf16(&text[..end]).ok())
        .ok_or("ShellLink AUMID is invalid")?;
    if id != APP_ID {
        return Err("ShellLink AUMID does not match".into());
    }
    let mut value = unsafe { properties.GetValue(&TOAST_ACTIVATOR_CLSID) }
        .map_err(|e| format!("ShellLink toast activator read: {e:?}"))?;
    let activator = unsafe { PropVariantToGUID(&value) };
    unsafe {
        let _ = PropVariantClear(&mut value);
    }
    let activator =
        activator.map_err(|e| format!("ShellLink toast activator conversion: {e:?}"))?;
    if activator != TOAST_STUB_CLSID {
        return Err("ShellLink toast activator does not match".into());
    }
    Ok(())
}

fn paths_match(path: &str, executable: &std::path::Path) -> bool {
    if !std::path::Path::new(path).is_absolute() {
        return false;
    }
    match (
        std::fs::canonicalize(path),
        std::fs::canonicalize(executable),
    ) {
        (Ok(registered), Ok(current)) => registered
            .to_string_lossy()
            .eq_ignore_ascii_case(&current.to_string_lossy()),
        _ => false,
    }
}

fn command_matches_executable(command: &str, executable: &std::path::Path) -> bool {
    let Some(path) = command
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix("\" \"%1\""))
    else {
        return false;
    };
    if path.contains('"') || !std::path::Path::new(path).is_absolute() {
        return false;
    }
    paths_match(path, executable)
}

fn protocol_registered_to_current_exe() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    protocol_registered_to_exe(&current)
}

fn protocol_registered_to_exe(executable: &std::path::Path) -> bool {
    let mut scheme_key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Classes\\rhythm-win"),
            None,
            KEY_QUERY_VALUE,
            &mut scheme_key,
        )
    };
    if opened != ERROR_SUCCESS {
        return false;
    }
    let mut scheme_type = REG_VALUE_TYPE::default();
    let flag = unsafe {
        RegQueryValueExW(
            scheme_key,
            w!("URL Protocol"),
            None,
            Some(&mut scheme_type),
            None,
            None,
        )
    };
    unsafe {
        let _ = RegCloseKey(scheme_key);
    }
    if flag != ERROR_SUCCESS || scheme_type != REG_SZ {
        return false;
    }
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Classes\\rhythm-win\\shell\\open\\command"),
            None,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return false;
    }
    let result = (|| {
        let mut size = 0u32;
        let mut value_type = REG_VALUE_TYPE::default();
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR::null(),
                None,
                Some(&mut value_type),
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS
            || value_type != REG_SZ
            || !(2..=65536).contains(&size)
            || !size.is_multiple_of(2)
        {
            return false;
        }
        let mut data = vec![0u16; (size / 2) as usize];
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR::null(),
                None,
                Some(&mut value_type),
                Some(data.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS
            || value_type != REG_SZ
            || size < 2
            || size as usize > data.len() * 2
            || !size.is_multiple_of(2)
        {
            return false;
        }
        data.truncate((size / 2) as usize);
        if data.pop() != Some(0) || data.contains(&0) {
            return false;
        }
        let Ok(command) = String::from_utf16(&data) else {
            return false;
        };
        command_matches_executable(&command, executable)
    })();
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

struct RuntimeGuard;
impl RuntimeGuard {
    fn initialize() -> Result<Self, String> {
        // A Tauri command can run on an existing STA or on an uninitialized
        // worker. A failed first call is never balanced; only success is.
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED).or_else(|error| {
                if error.code().0 == 0x80010106_u32 as i32 {
                    RoInitialize(RO_INIT_SINGLETHREADED)
                } else {
                    Err(error)
                }
            })
        }
        .map_err(|error| format!("{error:?}"))?;
        Ok(Self)
    }
}
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub(crate) fn token_from_uri(uri: &str) -> Option<&str> {
    let token = uri.strip_prefix(PREFIX)?;
    crate::storage::notification_activation::valid_token(token).then_some(token)
}

pub(super) fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "알림 보안 토큰을 만들지 못했습니다.".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn authorization() -> &'static str {
    if !protocol_registered_to_current_exe() {
        return "unavailable";
    }
    let Ok(_runtime) = RuntimeGuard::initialize() else {
        return "unavailable";
    };
    if !shortcut_matches_current_exe() {
        return "unavailable";
    }
    let Ok(notifier) = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
    else {
        return "unavailable";
    };
    classify_setting(notifier.Setting().map_err(|error| error.code().0)).status()
}

/// User-invoked connection test, separate from the ordinary delivery ledger.
/// A not-found error is unknown, never permission. Only this explicit action
/// may submit a neutral test toast, which lets Windows initialize its sender.
pub(crate) fn request_connection_test() -> Result<(&'static str, bool), String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST_TEST: Mutex<Option<Instant>> = Mutex::new(None);

    if !protocol_registered_to_current_exe() {
        return Ok(("unavailable", false));
    }
    let _runtime = match RuntimeGuard::initialize() {
        Ok(runtime) => runtime,
        Err(_) => return Ok(("unavailable", false)),
    };
    if !shortcut_matches_current_exe() {
        return Ok(("unavailable", false));
    }
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
    {
        Ok(notifier) => notifier,
        Err(_) => return Ok(("unavailable", false)),
    };
    let state = classify_setting(notifier.Setting().map_err(|error| error.code().0));
    if state != SettingState::SetupRequired {
        return Ok((state.status(), false));
    }

    let mut last = LAST_TEST
        .try_lock()
        .map_err(|_| "알림 연결 시험이 이미 진행 중입니다.")?;
    if last.is_some_and(|time| time.elapsed() < Duration::from_secs(30)) {
        return Err("알림 연결 시험은 잠시 후 다시 시도해 주세요.".into());
    }
    let document = XmlDocument::new().map_err(|_| "알림 연결 시험을 준비하지 못했습니다.")?;
    document.LoadXml(&HSTRING::from(
        "<toast><visual><binding template=\"ToastGeneric\"><text>Rhythm Windows 알림 시험</text><text>설치된 앱의 알림 연결을 확인합니다</text></binding></visual></toast>"
    )).map_err(|_| "알림 연결 시험을 준비하지 못했습니다.")?;
    let toast = ToastNotification::CreateToastNotification(&document)
        .map_err(|_| "알림 연결 시험을 준비하지 못했습니다.")?;

    // Windows settings can change while constructing the toast. A known
    // disabled state or any unexpected error must still stop this request.
    let state = classify_setting(notifier.Setting().map_err(|error| error.code().0));
    if state != SettingState::SetupRequired {
        return Ok((state.status(), false));
    }
    *last = Some(Instant::now());
    notifier
        .Show(&toast)
        .map_err(|_| "알림 연결 시험 요청 결과를 확인하지 못했습니다.")?;
    // A successful Show is only a handoff. Give Windows a short bounded period
    // to initialize sender settings, then report whatever it actually returns.
    let mut state = SettingState::SetupRequired;
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(250));
        state = classify_setting(notifier.Setting().map_err(|error| error.code().0));
        if state != SettingState::SetupRequired {
            break;
        }
    }
    Ok((state.status(), true))
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(super) struct PreparedToast {
    notifier: ToastNotifier,
    toast: ToastNotification,
    // Declared last so the COM objects are released before uninitialization.
    _runtime: RuntimeGuard,
}

pub(super) fn prepare(title: &str, body: &str, token: &str) -> Result<PreparedToast, String> {
    if !protocol_registered_to_current_exe() {
        return Err(
            "Windows 알림 연결이 등록되지 않았습니다. 설치 프로그램으로 다시 설치해 주세요.".into(),
        );
    }
    let _runtime = RuntimeGuard::initialize()?;
    if !shortcut_matches_current_exe() {
        return Err(
            "Windows 알림 바로가기가 올바르지 않습니다. 설치 프로그램으로 다시 설치해 주세요."
                .into(),
        );
    }
    if token_from_uri(&format!("{PREFIX}{token}")).is_none() {
        return Err("알림 활성화 정보가 올바르지 않습니다.".into());
    }
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
        .map_err(|e| e.to_string())?;
    if notifier.Setting().map_err(|e| e.to_string())? != NotificationSetting::Enabled {
        return Err("Windows 알림이 꺼져 있습니다.".into());
    }
    let xml = format!("<toast activationType=\"protocol\" launch=\"{PREFIX}{token}\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>", escape_xml(title), escape_xml(body));
    let document = XmlDocument::new().map_err(|e| e.to_string())?;
    document
        .LoadXml(&HSTRING::from(xml))
        .map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&document).map_err(|e| e.to_string())?;
    Ok(PreparedToast {
        _runtime,
        notifier,
        toast,
    })
}

pub(super) fn show(prepared: PreparedToast) -> Result<(), String> {
    // After this call starts, an error is an attempted handoff; do not retract
    // the durable attempt because the OS result can be ambiguous.
    prepared
        .notifier
        .Show(&prepared.toast)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_classification_distinguishes_unknown_from_permission() {
        assert_eq!(
            classify_setting(Ok(NotificationSetting::Enabled)),
            SettingState::Authorized
        );
        for setting in [
            NotificationSetting::DisabledForApplication,
            NotificationSetting::DisabledForUser,
            NotificationSetting::DisabledByGroupPolicy,
            NotificationSetting::DisabledByManifest,
        ] {
            assert_eq!(classify_setting(Ok(setting)), SettingState::Denied);
        }
        assert_eq!(
            classify_setting(Err(SETTING_NOT_FOUND)),
            SettingState::SetupRequired
        );
        assert_eq!(
            classify_setting(Err(0x80004005_u32 as i32)),
            SettingState::Unavailable
        );
        assert_ne!(
            classify_setting(Err(SETTING_NOT_FOUND)).status(),
            "authorized"
        );
    }
    #[test]
    fn protocol_parser_is_strict_and_xml_text_is_escaped() {
        let token = "ab".repeat(32);
        assert_eq!(
            token_from_uri(&format!("{PREFIX}{token}")),
            Some(token.as_str())
        );
        for suffix in ["?x=1", "/extra", "#x"] {
            assert!(token_from_uri(&format!("{PREFIX}{token}{suffix}")).is_none());
        }
        assert!(token_from_uri(&format!("{PREFIX}{}", token.to_uppercase())).is_none());
        assert_eq!(escape_xml("<&>\"'"), "&lt;&amp;&gt;&quot;&apos;");
        assert_ne!(new_token().unwrap(), new_token().unwrap());
    }

    #[test]
    fn installed_protocol_command_requires_exact_safe_quoting_and_current_exe() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Rhythm test.exe");
        let other = dir.path().join("other.exe");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(&other, b"x").unwrap();
        let command = format!("\"{}\" \"%1\"", exe.display());
        assert!(command_matches_executable(&command, &exe));
        assert!(!command_matches_executable(&command, &other));
        assert!(!command_matches_executable(
            &format!("{} \"%1\"", exe.display()),
            &exe
        ));
        assert!(!command_matches_executable(&(command + " --extra"), &exe));
    }

    #[test]
    fn shortcut_validation_rejects_missing_and_invalid_links() {
        let _runtime = RuntimeGuard::initialize().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rhythm.exe");
        let shortcut = dir.path().join("Rhythm.lnk");
        std::fs::write(&exe, b"x").unwrap();
        assert!(shortcut_check_executable(&shortcut, &exe).is_err());
        std::fs::write(&shortcut, b"not a shell link").unwrap();
        assert!(shortcut_check_executable(&shortcut, &exe).is_err());
    }

    #[test]
    #[ignore = "read-only diagnostic against an explicitly named installed executable"]
    fn installed_notification_diagnostics() {
        let executable = std::path::PathBuf::from(
            std::env::var_os("RHYTHM_NOTIFICATION_DIAGNOSTIC_EXE")
                .expect("set RHYTHM_NOTIFICATION_DIAGNOSTIC_EXE to the installed rhythm.exe"),
        );
        assert!(executable.is_absolute() && executable.is_file());
        let protocol = protocol_registered_to_exe(&executable);
        println!("protocol_matches_executable={protocol}");
        let runtime = RuntimeGuard::initialize();
        if let Err(error) = &runtime {
            println!("winrt_initialize=error: {error}");
        } else {
            println!("winrt_initialize=ok");
        }
        let mut shortcut_ok = false;
        let mut notifications_enabled = false;
        if let Ok(_runtime) = runtime {
            match shortcut_check_registered(&executable) {
                Ok(()) => {
                    println!("shortcut=ok");
                    shortcut_ok = true;
                }
                Err(error) => println!("shortcut=error: {error}"),
            }
            match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)) {
                Ok(notifier) => match notifier.Setting() {
                    Ok(setting) => {
                        println!("notifier_setting={setting:?}");
                        notifications_enabled = setting == NotificationSetting::Enabled;
                    }
                    Err(error) => println!("notifier_setting=error: {error:?}"),
                },
                Err(error) => println!("notifier_creation=error: {error:?}"),
            }
            // This changes only the short-lived diagnostic process identity.
            // It does not write registration or Windows notification settings.
            match unsafe {
                windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(w!(
                    "io.github.hiyabye.rhythm"
                ))
            } {
                Ok(()) => {
                    println!("set_process_aumid=ok");
                    match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
                        APP_ID,
                    ))
                    .and_then(|notifier| notifier.Setting())
                    {
                        Ok(setting) => {
                            println!("notifier_setting_after_process_aumid={setting:?}");
                            notifications_enabled |= setting == NotificationSetting::Enabled;
                        }
                        Err(error) => {
                            println!("notifier_setting_after_process_aumid=error: {error:?}")
                        }
                    }
                }
                Err(error) => println!("set_process_aumid=error: {error:?}"),
            }
        }
        assert!(
            protocol && shortcut_ok && notifications_enabled,
            "native notification prerequisites failed"
        );
    }

    #[test]
    #[ignore = "explicitly authorized one-time native notification experiment"]
    fn diagnostic_show_registration_test() {
        assert_eq!(
            std::env::var("RHYTHM_NOTIFICATION_TEST_SHOW").as_deref(),
            Ok("1")
        );
        let executable = std::path::PathBuf::from(
            std::env::var_os("RHYTHM_NOTIFICATION_DIAGNOSTIC_EXE")
                .expect("set RHYTHM_NOTIFICATION_DIAGNOSTIC_EXE to installed rhythm.exe"),
        );
        assert!(executable.is_absolute() && executable.is_file());
        assert!(
            protocol_registered_to_exe(&executable),
            "protocol does not match installation"
        );
        unsafe {
            windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(w!(
                "io.github.hiyabye.rhythm"
            ))
        }
        .expect("set diagnostic process AppUserModelID");
        let _runtime = RuntimeGuard::initialize().expect("initialize WinRT");
        shortcut_check_registered(&executable).expect("installed shortcut prerequisites");
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
            .expect("create notification manager");
        match notifier.Setting() {
            Ok(NotificationSetting::Enabled) => {
                println!("setting_before=Enabled; no experiment needed; no toast sent");
                return;
            }
            Ok(setting) => panic!("setting_before={setting:?}; refusing to send a test toast"),
            Err(error) if error.code().0 == 0x80070490_u32 as i32 => {
                println!("setting_before=0x80070490; running explicitly requested single test");
            }
            Err(error) => panic!("setting_before={error:?}; refusing to send a test toast"),
        }
        let document = XmlDocument::new().expect("create test toast XML");
        document.LoadXml(&HSTRING::from(
            "<toast><visual><binding template=\"ToastGeneric\"><text>Rhythm Windows 알림 시험</text><text>설치된 앱의 알림 연결을 확인합니다</text></binding></visual></toast>"
        )).expect("load test toast XML");
        let toast =
            ToastNotification::CreateToastNotification(&document).expect("create test toast");
        notifier.Show(&toast).expect("hand off one test toast");
        println!("handoff=ok (screen display not confirmed)");
        for attempt in 1..=10 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            match notifier.Setting() {
                Ok(setting) => {
                    println!("setting_after_{attempt}={setting:?}");
                    assert_eq!(
                        setting,
                        NotificationSetting::Enabled,
                        "Windows did not enable notifications"
                    );
                    return;
                }
                Err(error) => println!("setting_after_{attempt}=error: {error:?}"),
            }
        }
        panic!("Windows notification setting remained unavailable after the test handoff");
    }
}
