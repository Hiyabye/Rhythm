//! Signed, architecture-specific updates from the single stable GitHub manifest.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct State {
    active: Arc<AtomicBool>,
    staged: std::sync::Mutex<Option<PendingUpdate>>,
    /// The newest update the last check found, for a window that subscribes
    /// to `updater://available` after the automatic check emitted it.
    available: std::sync::Mutex<Option<UpdateInfo>>,
}

struct PendingUpdate {
    update: Update,
    bytes: Vec<u8>,
    info: UpdateInfo,
}

struct Active(Arc<AtomicBool>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn acquire(state: &State) -> Result<Active, String> {
    state
        .active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| Active(state.active.clone()))
        .map_err(|_| "업데이트 작업이 이미 진행 중입니다.".to_string())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current_version: String,
    date: Option<String>,
    body: Option<String>,
}

fn info(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        date: update.date.as_ref().map(ToString::to_string),
        body: update.body.clone(),
    }
}
pub(crate) fn signing_key_configured(key: Option<&str>) -> bool {
    key.is_some_and(|key| !key.trim().is_empty())
}
/// Whether this build embeds a trusted update key. Without one the updater
/// plugin is not registered and Settings shows the manual update procedure.
#[tauri::command]
pub fn updater_configured() -> bool {
    signing_key_configured(option_env!("RHYTHM_UPDATER_PUBKEY"))
}
fn ensure_signing_key() -> Result<(), String> {
    if updater_configured() {
        Ok(())
    } else {
        Err("공식 업데이트 서명 키가 앱에 구성되지 않았습니다. 이 빌드에서는 업데이트를 사용할 수 없습니다.".into())
    }
}
/// Natural Korean for the failures a user can act on; plugin details are English.
fn update_error(stage: &str, error: &tauri_plugin_updater::Error) -> String {
    use std::io::ErrorKind;
    use tauri_plugin_updater::Error;
    // The macOS install moves the old bundle aside first; if the new one could
    // not be placed and the old one not put back, say where the old app is.
    if let Error::PreviousAppNotRestored(kept) = error {
        return format!(
            "{stage} 이전 앱을 원래 위치로 되돌리지 못했습니다. 이전 앱은 {}에 있습니다. 이 앱을 응용 프로그램 폴더로 옮기거나 새 버전을 직접 설치해 주세요. 데이터는 그대로입니다.",
            kept.display()
        );
    }
    let reason = match error {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => {
            "업데이트 파일의 서명이 이 앱의 공식 키와 일치하지 않아 설치하지 않았습니다. 현재 앱과 데이터는 그대로입니다."
        }
        Error::Reqwest(_) | Error::Network(_) | Error::Http(_) => {
            "네트워크 연결을 확인한 뒤 다시 시도해 주세요. 현재 앱과 데이터는 그대로입니다."
        }
        Error::UnsupportedArch | Error::UnsupportedOs | Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "이 기기에 맞는 업데이트가 아직 없습니다."
        }
        // A disk image or a translocated (quarantined, never moved) app is
        // read-only, and its temporary folder lies on another volume.
        Error::TempDirNotOnSameMountPoint => {
            "앱을 응용 프로그램 폴더로 옮겨 실행한 뒤 다시 시도해 주세요. 현재 앱과 데이터는 그대로입니다."
        }
        Error::Io(io) if io.kind() == ErrorKind::ReadOnlyFilesystem => {
            "앱을 응용 프로그램 폴더로 옮겨 실행한 뒤 다시 시도해 주세요. 현재 앱과 데이터는 그대로입니다."
        }
        Error::AuthenticationFailed => {
            "관리자 승인이 취소되었거나 실패했습니다. 현재 앱과 데이터는 그대로입니다."
        }
        Error::Io(io) if io.kind() == ErrorKind::PermissionDenied => {
            "앱이 설치된 폴더에 쓸 권한이 없거나 관리자 승인이 취소되었습니다. 현재 앱과 데이터는 그대로입니다."
        }
        Error::ReleaseNotFound | Error::Serialization(_) | Error::Semver(_) | Error::InvalidUpdaterFormat => {
            "업데이트 정보가 올바르지 않습니다. 잠시 후 다시 확인해 주세요."
        }
        _ => "현재 앱과 데이터는 그대로입니다. 잠시 후 다시 시도해 주세요.",
    };
    format!("{stage} {reason}")
}
fn require_confirmation(confirmed: bool) -> Result<(), String> {
    if confirmed {
        Ok(())
    } else {
        Err("업데이트 설치를 진행하려면 다시 시작 확인이 필요합니다.".into())
    }
}

fn remember(updater: &State, found: Option<&UpdateInfo>) {
    if let Ok(mut available) = updater.available.lock() {
        *available = found.cloned();
    }
}

#[tauri::command]
pub async fn updater_check(
    app: AppHandle,
    updater: tauri::State<'_, State>,
) -> Result<Option<UpdateInfo>, String> {
    ensure_signing_key()?;
    let update = app
        .updater()
        .map_err(|e| format!("업데이트 기능을 시작하지 못했습니다: {e}"))?
        .check()
        .await
        .map_err(|e| update_error("업데이트 정보를 확인하지 못했습니다.", &e))?;
    let found = update.as_ref().map(info);
    remember(&updater, found.as_ref());
    Ok(found)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoCheck {
    enabled: bool,
    available: Option<UpdateInfo>,
}

/// The automatic-check preference and any update a check already found.
#[tauri::command]
pub fn updater_auto_check(
    updater: tauri::State<'_, State>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AutoCheck, String> {
    let enabled = state
        .store
        .lock()
        .map_err(|_| "설정 저장소를 사용할 수 없습니다.".to_string())?
        .update_check_enabled()?;
    let available = updater
        .available
        .lock()
        .map_err(|_| "업데이트 상태를 확인하지 못했습니다.".to_string())?
        .clone();
    Ok(AutoCheck { enabled, available })
}

#[tauri::command]
pub fn updater_set_auto_check(
    state: tauri::State<'_, crate::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "설정 저장소를 사용할 수 없습니다.".to_string())?;
    store.set_update_check(enabled)?;
    store.update_check_enabled()
}

/// The first wake leaves startup alone; later wakes are hourly, so a failed or
/// offline check retries within the hour and a successful one waits a day.
const AUTO_CHECK_FIRST_WAKE: Duration = Duration::from_secs(60);
const AUTO_CHECK_WAKE: Duration = Duration::from_secs(60 * 60);
const AUTO_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Checks for updates while the app is open, when the build trusts an update
/// key and the user has not turned automatic checks off. Failures stay quiet:
/// the manual check in Settings reports errors. It only announces; downloading
/// and installing remain explicit.
pub async fn run_auto_check(app: AppHandle, shutdown: Arc<AtomicBool>) {
    if !updater_configured() {
        return;
    }
    tokio::time::sleep(AUTO_CHECK_FIRST_WAKE).await;
    while !shutdown.load(Ordering::Acquire) {
        auto_check_once(&app).await;
        tokio::time::sleep(AUTO_CHECK_WAKE).await;
    }
}

async fn auto_check_once(app: &AppHandle) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    // Never hold the store lock across the network request.
    let due = state
        .store
        .lock()
        .ok()
        .and_then(|store| store.update_check_due().ok())
        .unwrap_or(false);
    if !due {
        return;
    }
    let Ok(updater) = app.updater_builder().timeout(AUTO_CHECK_TIMEOUT).build() else {
        return;
    };
    let Ok(update) = updater.check().await else {
        return;
    };
    if let Ok(store) = state.store.lock() {
        let _ = store.record_update_check();
    }
    let found = update.as_ref().map(info);
    remember(&app.state::<State>(), found.as_ref());
    if let Some(found) = found {
        let _ = app.emit("updater://available", found);
    }
}
/// Download and cryptographically verify the package while the app-wide
/// maintenance guard is held. Installation is a separate, confirmed action.
#[tauri::command]
pub async fn updater_install(
    app: AppHandle,
    updater: tauri::State<'_, State>,
    state: tauri::State<'_, crate::AppState>,
) -> Result<UpdateInfo, String> {
    ensure_signing_key()?;
    let _operation = state.exclusive_maintenance().await?;
    let active = acquire(&updater)?;
    let update = app
        .updater()
        .map_err(|e| format!("업데이트 기능을 시작하지 못했습니다: {e}"))?
        .check()
        .await
        .map_err(|e| update_error("업데이트 정보를 다시 확인하지 못했습니다.", &e))?
        .ok_or_else(|| "설치할 업데이트가 없습니다. 다시 확인해 주세요.".to_string())?;
    let info = info(&update);
    app.emit("updater://progress", serde_json::json!({"phase":"downloading", "version":info.version, "downloaded":0, "total":null})).ok();
    let app_progress = app.clone();
    let downloaded = std::sync::atomic::AtomicU64::new(0);
    let bytes = update.download(|chunk, total| {
        let bytes = downloaded.fetch_add(chunk as u64, Ordering::AcqRel).saturating_add(chunk as u64);
        let _ = app_progress.emit("updater://progress", serde_json::json!({"phase":"downloading", "version":info.version, "downloaded":bytes, "total":total}));
    }, || {
        let _ = app.emit("updater://progress", serde_json::json!({"phase":"verifying", "version":info.version}));
    }).await.map_err(|e| update_error("업데이트를 내려받거나 검증하지 못했습니다.", &e))?;
    *updater
        .staged
        .lock()
        .map_err(|_| "업데이트 준비 상태를 확인하지 못했습니다.".to_string())? =
        Some(PendingUpdate {
            update,
            bytes,
            info: info.clone(),
        });
    drop(active);
    app.emit(
        "updater://progress",
        serde_json::json!({"phase":"staged", "version":info.version}),
    )
    .ok();
    Ok(info)
}

#[tauri::command]
pub async fn updater_restart(
    app: AppHandle,
    updater: tauri::State<'_, State>,
    state: tauri::State<'_, crate::AppState>,
    confirmed: bool,
) -> Result<(), String> {
    require_confirmation(confirmed)?;
    ensure_signing_key()?;
    let operation = state.exclusive_maintenance().await?;
    let active = acquire(&updater)?;
    // Maintenance stops jobs but not a loaded model's server, which runs from
    // the install folder. The Windows installer would find its executable
    // locked (the plugin exits the app with `process::exit`, skipping the
    // normal shutdown), and on macOS it would keep running from the replaced
    // bundle. Admission stays closed, so nothing restarts it before install.
    state
        .worker
        .lock()
        .await
        .stop_for_inventory()
        .await
        .map_err(|e| format!("AI 엔진을 멈추지 못해 업데이트를 설치하지 않았습니다. {e}"))?;
    let mut staged = updater
        .staged
        .lock()
        .map_err(|_| "업데이트 준비 상태를 확인하지 못했습니다.".to_string())?;
    let pending = staged.as_ref().ok_or_else(|| {
        "검증된 업데이트가 없습니다. 먼저 업데이트를 다운로드해 주세요.".to_string()
    })?;
    app.emit(
        "updater://progress",
        serde_json::json!({"phase":"installing", "version":pending.info.version}),
    )
    .ok();
    pending
        .update
        .install(&pending.bytes)
        .map_err(|e| update_error("업데이트를 설치하지 못했습니다.", &e))?;
    let version = pending.info.version.clone();
    *staged = None;
    drop(staged);
    drop(active);
    app.emit(
        "updater://progress",
        serde_json::json!({"phase":"restarting", "version":version}),
    )
    .ok();
    // `restart` never returns off the main thread: it waits for the exit
    // handler, which must be able to stop the classifier.
    operation.hold_until_exit();
    app.restart();
}

#[cfg(test)]
mod tests {
    #[test]
    fn stable_manifest_is_a_single_fixed_https_endpoint() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let endpoints = config["plugins"]["updater"]["endpoints"]
            .as_array()
            .unwrap();
        assert_eq!(endpoints.len(), 1);
        let url = url::Url::parse(endpoints[0].as_str().unwrap()).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(
            url.path(),
            "/Hiyabye/Rhythm/releases/latest/download/latest.json"
        );
        assert!(url.query().is_none());
    }

    #[test]
    fn updater_operation_admission_is_single_flight_and_released_on_drop() {
        let state = super::State::default();
        let first = super::acquire(&state).unwrap();
        assert!(super::acquire(&state).is_err());
        drop(first);
        assert!(super::acquire(&state).is_ok());
    }

    #[test]
    fn missing_or_blank_public_keys_never_enable_update_trust() {
        assert!(!super::signing_key_configured(None));
        assert!(!super::signing_key_configured(Some("  \n")));
        assert!(super::signing_key_configured(Some("test-public-key")));
    }

    #[test]
    fn configured_updater_requires_artifact_signed_version() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["plugins"]["updater"]["requireSignedVersion"], true);
    }

    #[test]
    fn minisign_artifact_fixture_authenticates_bytes_and_binds_version() {
        // Temporary test key; its private half is not kept in this repository.
        const PUBLIC_KEY: &str =
            "untrusted comment: minisign public key: A49EF4746FA6E6D7\nRWTX5qZvdPSepDGWg2bz8n6ArQnBMwgQK1geVosorcbLzcunD+KJ+cYe";
        const SIGNATURE: &str = "untrusted comment: signature from tauri secret key\nRUTX5qZvdPSepPF2zsjvP/L/TQtBLRnWRvaq+4omK1UEugtQp/USrGiI1v762sDy3id3aor7bRC9ETyeot6lMHHrWoZ5hjUM1Qc=\ntrusted comment: timestamp:1791649051\tfile:payload\tversion:2.0.0\nDDq2CLOEl5cQmC2eJRHlnZPVViOZsu1CjpPKQoei4A/XBfkDqGO0B+QMrcrHgEQ5qV3mne75tdFWhJpqUXh9AA==";

        let public_key = minisign_verify::PublicKey::decode(PUBLIC_KEY).unwrap();
        let signature = minisign_verify::Signature::decode(SIGNATURE).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let artifact = directory.path().join("controlled-update.bin");
        std::fs::write(&artifact, b"test").unwrap();
        let bytes = std::fs::read(&artifact).unwrap();
        public_key.verify(&bytes, &signature, true).unwrap();
        std::fs::write(&artifact, b"tampered").unwrap();
        let tampered = std::fs::read(&artifact).unwrap();
        assert!(public_key.verify(&tampered, &signature, true).is_err());

        let version = signature
            .trusted_comment()
            .split('\t')
            .find_map(|field| field.strip_prefix("version:"));
        assert_eq!(version, Some("2.0.0"));
        assert_ne!(version, Some("3.0.0"));
    }

    #[test]
    fn settings_show_the_built_application_version() {
        let settings = include_str!("../../src/lib/UpdaterSettings.svelte");
        // The redesigned shell shows the version only in Settings → About.
        assert!(settings.contains(&format!("<span>{}</span>", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn signature_failures_are_reported_in_korean_without_plugin_text() {
        let error = tauri_plugin_updater::Error::SignedVersionMismatch {
            signed: "0.9.2".into(),
            announced: "0.9.3".into(),
        };
        let message = super::update_error("업데이트를 내려받거나 검증하지 못했습니다.", &error);
        assert!(message.contains("서명이 이 앱의 공식 키와 일치하지 않아 설치하지 않았습니다"));
        assert!(!message.is_ascii() && !message.contains("signature"));
    }

    #[test]
    fn install_location_failures_name_the_action_to_take() {
        use std::io::{Error as IoError, ErrorKind};
        use tauri_plugin_updater::Error;
        let stage = "업데이트를 설치하지 못했습니다.";
        let move_app = "응용 프로그램 폴더로 옮겨 실행한 뒤";
        assert!(super::update_error(stage, &Error::TempDirNotOnSameMountPoint).contains(move_app));
        let read_only = Error::Io(IoError::from(ErrorKind::ReadOnlyFilesystem));
        assert!(super::update_error(stage, &read_only).contains(move_app));
        let denied = Error::Io(IoError::from(ErrorKind::PermissionDenied));
        assert!(super::update_error(stage, &denied).contains("쓸 권한이 없거나"));
        assert!(super::update_error(stage, &Error::AuthenticationFailed).contains("관리자 승인"));
        let kept = Error::PreviousAppNotRestored("/Users/someone/Rhythm.app".into());
        let message = super::update_error(stage, &kept);
        assert!(message.starts_with(stage));
        assert!(message.contains("/Users/someone/Rhythm.app에 있습니다"));
        assert!(!message.contains("그대로입니다. 잠시"));
    }

    #[test]
    fn missing_platform_targets_are_not_described_as_mac_only() {
        let message = super::update_error(
            "업데이트 정보를 확인하지 못했습니다.",
            &tauri_plugin_updater::Error::TargetNotFound("windows-x86_64".into()),
        );
        assert!(message.contains("이 기기에 맞는 업데이트"));
        assert!(!message.contains("Mac"));
    }

    #[test]
    fn windows_updates_run_the_installer_in_passive_mode() {
        // Passive shows only progress, skips the uninstall and shortcut pages
        // through /UPDATE, and relaunches the app with /R.
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.windows.conf.json")).unwrap();
        assert_eq!(
            config["plugins"]["updater"]["windows"]["installMode"],
            "passive"
        );
    }

    #[test]
    fn updater_install_requires_explicit_confirmation() {
        assert!(super::require_confirmation(false).is_err());
        assert!(super::require_confirmation(true).is_ok());
    }
}
