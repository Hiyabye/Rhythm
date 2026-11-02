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
    let available = matches!(authorization.as_str(), "authorized" | "granted");
    Ok(PlatformCapabilities {
        restore,
        notifications: FeatureSupport {
            available,
            reason: (!available)
                .then(|| "Windows 알림 설정과 앱 설치 상태를 확인해 주세요.".into()),
        },
        updater: FeatureSupport {
            available: false,
            reason: Some("새 설치 파일로 수동 업데이트합니다.".into()),
        },
    })
}
