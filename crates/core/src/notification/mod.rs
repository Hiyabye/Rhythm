//! Native notification plumbing shared by the scheduler's notices, todo
//! reminders and seat alerts: the OS authorization check and, on Windows, the
//! WinRT toast sender and its protocol activation.
pub mod permission;
#[cfg(windows)]
pub mod windows;

/// Whether the user really opened a macOS notification: the default action
/// (a click on the body), not a dismissal or the crate's 30-minute timeout.
/// Every macOS response handler (notices, todo reminders, seat alerts) ignores
/// the other responses.
#[cfg(target_os = "macos")]
pub(crate) fn is_activation(
    response: &mac_usernotifications::response::NotificationResponse,
) -> bool {
    use mac_usernotifications::response::CloseReason;
    response.is_default_action()
        && !matches!(
            response.close_reason,
            Some(CloseReason::Dismissed | CloseReason::Expired)
        )
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::is_activation;
    use mac_usernotifications::response::{CloseReason, NotificationResponse};

    fn response(default_action: bool, close_reason: Option<CloseReason>) -> NotificationResponse {
        let action_identifier = if default_action {
            // SAFETY: a valid NSString constant of the Apple framework.
            unsafe { objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string() }
        } else {
            "custom-action".to_string()
        };
        NotificationResponse {
            notification_id: "id".into(),
            action_identifier,
            reply_text: None,
            close_reason,
        }
    }

    #[test]
    fn only_a_default_action_without_a_passive_close_activates() {
        assert!(is_activation(&response(true, None)));
        assert!(!is_activation(&response(false, None)));
        assert!(!is_activation(&response(
            true,
            Some(CloseReason::Dismissed)
        )));
        assert!(!is_activation(&response(true, Some(CloseReason::Expired))));
        assert!(!is_activation(&response(false, Some(CloseReason::Expired))));
    }
}
