//! The operating system's notification authorization (macOS
//! UNUserNotificationCenter; on Windows the installed connection). The app's
//! commands only wrap these.
#[cfg(target_os = "macos")]
const CALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[cfg(target_os = "macos")]
mod macos {
    use super::{Authorization, CALLBACK_TIMEOUT};
    use block2::RcBlock;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    use std::ptr::NonNull;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    pub(super) async fn authorization_status() -> Authorization {
        let receiver = authorization_status_receiver();
        await_status(receiver, "알림 권한 상태")
            .await
            .unwrap_or(Authorization::Unavailable)
    }

    fn authorization_status_receiver() -> oneshot::Receiver<Authorization> {
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

    pub(super) async fn request_permission() -> Authorization {
        let receiver = request_permission_receiver();
        match await_status(receiver, "알림 권한 요청").await {
            Some(true) => authorization_status().await,
            Some(false) | None => Authorization::Unavailable,
        }
    }

    /// Whether the system answered the request without an error.
    fn request_permission_receiver() -> oneshot::Receiver<bool> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (sender, receiver) = oneshot::channel();
        let sender = Mutex::new(Some(sender));
        let callback = RcBlock::new(move |_, error| {
            let result = error as usize == 0;
            if !result {
                eprintln!("macOS에서 알림 권한 요청을 처리하지 못했습니다.");
            }
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

    /// The callback's answer, or `None` when it timed out or never came.
    async fn await_status<T>(receiver: oneshot::Receiver<T>, operation: &str) -> Option<T> {
        match tokio::time::timeout(CALLBACK_TIMEOUT, receiver).await {
            Ok(Ok(status)) => Some(status),
            Ok(Err(_)) => {
                eprintln!("{operation} 응답 채널이 종료되었습니다.");
                None
            }
            Err(_) => {
                eprintln!("{operation} 시간이 초과되었습니다.");
                None
            }
        }
    }

    fn authorization_status_name(status: i64) -> Authorization {
        if status == UNAuthorizationStatus::NotDetermined.0 as i64 {
            Authorization::NotDetermined
        } else if status == UNAuthorizationStatus::Denied.0 as i64 {
            Authorization::Denied
        } else if status == UNAuthorizationStatus::Authorized.0 as i64 {
            Authorization::Authorized
        } else if status == UNAuthorizationStatus::Provisional.0 as i64 {
            Authorization::Provisional
        } else if status == UNAuthorizationStatus::Ephemeral.0 as i64 {
            Authorization::Ephemeral
        } else {
            Authorization::Unknown
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{authorization_status_name, Authorization};

        #[test]
        fn maps_all_known_authorization_statuses() {
            assert_eq!(authorization_status_name(0), Authorization::NotDetermined);
            assert_eq!(authorization_status_name(1), Authorization::Denied);
            assert_eq!(authorization_status_name(2), Authorization::Authorized);
            assert_eq!(authorization_status_name(3), Authorization::Provisional);
            assert_eq!(authorization_status_name(4), Authorization::Ephemeral);
            assert_eq!(authorization_status_name(-1), Authorization::Unknown);
            assert_eq!(authorization_status_name(5), Authorization::Unknown);
        }
    }
}

/// The operating system's notification authorization. The serialized names
/// are what the UI reads. macOS reports every state; Windows only
/// `authorized`, `denied`, `not_determined` (no notifier setting yet) and
/// `unavailable`; a timeout or a failed check is `unavailable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorization {
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
    Ephemeral,
    Unknown,
    Unavailable,
}

impl Authorization {
    /// Whether the system shows Rhythm's notifications now; provisional and
    /// ephemeral macOS authorization do.
    pub fn allows_delivery(self) -> bool {
        matches!(self, Self::Authorized | Self::Provisional | Self::Ephemeral)
    }
}

/// Reads the operating system's notification authorization without prompting.
/// On Windows this is notification availability plus installed protocol
/// ownership; other non-macOS hosts report unavailable.
pub async fn notification_authorization_status() -> Authorization {
    #[cfg(target_os = "macos")]
    {
        macos::authorization_status().await
    }
    #[cfg(windows)]
    {
        super::windows::authorization()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Authorization::Unavailable
    }
}

#[derive(serde::Serialize)]
pub struct NotificationConnectionResult {
    status: Authorization,
    test_notification_requested: bool,
}

/// Invoked only by an explicit user action. macOS asks for authorization;
/// Windows has no prompt, so it checks the installed connection and sends one
/// test notification without changing Windows settings.
pub async fn request_permission() -> Result<NotificationConnectionResult, String> {
    #[cfg(target_os = "macos")]
    let (status, test_notification_requested) = (macos::request_permission().await, false);
    #[cfg(windows)]
    let (status, test_notification_requested) = {
        let (status, requested) =
            tokio::task::spawn_blocking(super::windows::request_connection_test)
                .await
                .map_err(|_| "알림 연결을 확인하지 못했습니다.".to_string())??;
        (status, requested)
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let (status, test_notification_requested) = (Authorization::Unavailable, false);
    Ok(NotificationConnectionResult {
        status,
        test_notification_requested,
    })
}

#[cfg(all(test, not(any(target_os = "macos", windows))))]
mod tests {
    use super::Authorization;

    #[tokio::test]
    async fn notification_permission_is_unavailable_off_supported_platforms() {
        assert_eq!(
            super::notification_authorization_status().await,
            Authorization::Unavailable
        );
        let result = super::request_permission().await.unwrap();
        assert_eq!(result.status, Authorization::Unavailable);
        assert!(!result.test_notification_requested);
    }
}
