#[cfg(target_os = "macos")]
const CALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[cfg(target_os = "macos")]
mod macos {
    use super::CALLBACK_TIMEOUT;
    use block2::RcBlock;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    use std::ptr::NonNull;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    pub(super) async fn authorization_status() -> String {
        let receiver = authorization_status_receiver();
        await_status(receiver, "알림 권한 상태").await
    }

    fn authorization_status_receiver() -> oneshot::Receiver<&'static str> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (sender, receiver) = oneshot::channel();
        let sender = Mutex::new(Some(sender));
        let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            let status = authorization_status_name(
                unsafe { settings.as_ref() }.authorizationStatus().0 as i64,
            );
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(status);
            }
        });
        center.getNotificationSettingsWithCompletionHandler(&callback);
        receiver
    }

    pub(super) async fn request_permission() -> String {
        let receiver = request_permission_receiver();
        let result = await_status(receiver, "알림 권한 요청").await;
        if result == "requested" {
            authorization_status().await
        } else {
            result
        }
    }

    fn request_permission_receiver() -> oneshot::Receiver<&'static str> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (sender, receiver) = oneshot::channel();
        let sender = Mutex::new(Some(sender));
        let callback = RcBlock::new(move |_, error| {
            let result = if error as usize == 0 {
                "requested"
            } else {
                eprintln!("macOS에서 알림 권한 요청을 처리하지 못했습니다.");
                "unavailable"
            };
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(result);
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge,
            &callback,
        );
        receiver
    }

    async fn await_status(receiver: oneshot::Receiver<&'static str>, operation: &str) -> String {
        match tokio::time::timeout(CALLBACK_TIMEOUT, receiver).await {
            Ok(Ok(status)) => status.to_owned(),
            Ok(Err(_)) => {
                eprintln!("{operation} 응답 채널이 종료되었습니다.");
                "unavailable".to_owned()
            }
            Err(_) => {
                eprintln!("{operation} 시간이 초과되었습니다.");
                "unavailable".to_owned()
            }
        }
    }

    fn authorization_status_name(status: i64) -> &'static str {
        if status == UNAuthorizationStatus::NotDetermined.0 as i64 {
            "not_determined"
        } else if status == UNAuthorizationStatus::Denied.0 as i64 {
            "denied"
        } else if status == UNAuthorizationStatus::Authorized.0 as i64 {
            "authorized"
        } else if status == UNAuthorizationStatus::Provisional.0 as i64 {
            "provisional"
        } else if status == UNAuthorizationStatus::Ephemeral.0 as i64 {
            "ephemeral"
        } else {
            "unknown"
        }
    }

    #[cfg(test)]
    mod tests {
        use super::authorization_status_name;

        #[test]
        fn maps_all_known_authorization_statuses() {
            assert_eq!(authorization_status_name(0), "not_determined");
            assert_eq!(authorization_status_name(1), "denied");
            assert_eq!(authorization_status_name(2), "authorized");
            assert_eq!(authorization_status_name(3), "provisional");
            assert_eq!(authorization_status_name(4), "ephemeral");
            assert_eq!(authorization_status_name(-1), "unknown");
            assert_eq!(authorization_status_name(5), "unknown");
        }
    }
}

/// Reads the operating system's notification authorization without prompting.
/// On Windows this is notification availability plus installed protocol
/// ownership; other non-macOS hosts report unavailable.
pub(crate) async fn notification_authorization_status() -> String {
    #[cfg(target_os = "macos")]
    {
        macos::authorization_status().await
    }
    #[cfg(windows)]
    {
        crate::proactive::windows_notification::authorization().to_owned()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        "unavailable".to_owned()
    }
}

/// Tauri command for checking native notification authorization without prompting.
#[tauri::command]
pub(crate) async fn notification_authorization() -> String {
    notification_authorization_status().await
}

#[derive(serde::Serialize)]
pub(crate) struct NotificationConnectionResult {
    status: String,
    test_notification_requested: bool,
}

/// Invoked only by an explicit user action. macOS asks for authorization;
/// Windows has no prompt, so it checks the installed connection and sends one
/// test notification without changing Windows settings.
#[tauri::command]
pub(crate) async fn notification_request_permission() -> Result<NotificationConnectionResult, String>
{
    #[cfg(target_os = "macos")]
    let (status, test_notification_requested) = (macos::request_permission().await, false);
    #[cfg(windows)]
    let (status, test_notification_requested) = {
        let (status, requested) = tokio::task::spawn_blocking(
            crate::proactive::windows_notification::request_connection_test,
        )
        .await
        .map_err(|_| "알림 연결을 확인하지 못했습니다.".to_string())??;
        (status.to_owned(), requested)
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let (status, test_notification_requested) = ("unavailable".to_owned(), false);
    Ok(NotificationConnectionResult {
        status,
        test_notification_requested,
    })
}

#[cfg(all(test, not(any(target_os = "macos", windows))))]
mod tests {
    #[tokio::test]
    async fn notification_permission_is_unavailable_off_supported_platforms() {
        assert_eq!(
            super::notification_authorization_status().await,
            "unavailable"
        );
        let result = super::notification_request_permission().await.unwrap();
        assert_eq!(result.status, "unavailable");
        assert!(!result.test_notification_requested);
    }
}
