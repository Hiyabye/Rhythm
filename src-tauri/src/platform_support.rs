use serde::Serialize;
#[derive(Serialize)]
pub struct FeatureSupport {
    available: bool,
    reason: Option<String>,
}
#[derive(Serialize)]
pub struct PlatformCapabilities {
    notifications: FeatureSupport,
    restore: FeatureSupport,
    updater: FeatureSupport,
}
#[tauri::command]
pub async fn platform_capabilities(
    state: tauri::State<'_, crate::AppState>,
) -> Result<PlatformCapabilities, String> {
    let restore = match crate::backup::backup_restore_support(state) {
        Ok(()) => FeatureSupport {
            available: true,
            reason: None,
        },
        Err(reason) => FeatureSupport {
            available: false,
            reason: Some(reason),
        },
    };
    let authorization = crate::notification_permission::notification_authorization_status().await;
    // macOS provisional/ephemeral authorization also permits delivery.
    let available = matches!(
        authorization.as_str(),
        "authorized" | "granted" | "provisional" | "ephemeral"
    );
    #[cfg(windows)]
    let notification_hint = "Windows 알림 설정과 앱 설치 상태를 확인해 주세요.";
    #[cfg(not(windows))]
    let notification_hint = "운영체제의 알림 설정에서 Rhythm 알림을 허용해 주세요.";
    let updater = crate::updater::updater_configured();
    Ok(PlatformCapabilities {
        restore,
        notifications: FeatureSupport {
            available,
            reason: (!available).then(|| notification_hint.into()),
        },
        updater: FeatureSupport {
            available: updater,
            reason: (!updater).then(|| "새 버전을 내려받아 직접 설치합니다.".into()),
        },
    })
}
