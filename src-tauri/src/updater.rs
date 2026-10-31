//! Signed, architecture-specific updates from the single stable GitHub manifest.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct State {
    active: Arc<AtomicBool>,
    staged: std::sync::Mutex<Option<PendingUpdate>>,
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
fn ensure_signing_key() -> Result<(), String> {
    if signing_key_configured(option_env!("RHYTHM_UPDATER_PUBKEY")) {
        Ok(())
    } else {
        Err("공식 업데이트 서명 키가 앱에 구성되지 않았습니다. 이 빌드에서는 업데이트를 사용할 수 없습니다.".into())
    }
}
fn require_confirmation(confirmed: bool) -> Result<(), String> {
    if confirmed {
        Ok(())
    } else {
        Err("업데이트 설치를 진행하려면 다시 시작 확인이 필요합니다.".into())
    }
}

#[tauri::command]
pub async fn updater_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    ensure_signing_key()?;
    let update = app
        .updater()
        .map_err(|e| format!("업데이트 기능을 시작하지 못했습니다: {e}"))?
        .check()
        .await
        .map_err(|e| format!("업데이트 정보를 확인하지 못했습니다: {e}"))?;
    Ok(update.as_ref().map(info))
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
        .map_err(|e| format!("업데이트 정보를 다시 확인하지 못했습니다: {e}"))?
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
    }).await.map_err(|e| format!("서명된 업데이트를 내려받거나 검증하지 못했습니다: {e}"))?;
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
    let _operation = state.exclusive_maintenance().await?;
    let active = acquire(&updater)?;
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
        .map_err(|e| format!("서명된 업데이트를 설치하지 못했습니다: {e}"))?;
    let version = pending.info.version.clone();
    *staged = None;
    drop(staged);
    drop(active);
    app.emit(
        "updater://progress",
        serde_json::json!({"phase":"restarting", "version":version}),
    )
    .ok();
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
    fn updater_install_requires_explicit_confirmation() {
        assert!(super::require_confirmation(false).is_err());
        assert!(super::require_confirmation(true).is_ok());
    }
}
