/// Reads Windows notification availability and installed protocol ownership.
pub(crate) async fn notification_authorization_status() -> String {
    #[cfg(windows)]
    {
        crate::proactive::windows_notification::authorization().to_owned()
    }
    #[cfg(not(windows))]
    {
        "unavailable".to_owned()
    }
}

#[tauri::command]
pub(crate) async fn notification_authorization() -> String {
    notification_authorization_status().await
}

#[derive(serde::Serialize)]
pub(crate) struct NotificationConnectionResult {
    status: String,
    test_notification_requested: bool,
}

/// Invoked only by the user's connection-test button. This never changes
/// Windows settings or reports that an OS permission prompt was displayed.
#[tauri::command]
pub(crate) async fn notification_request_permission() -> Result<NotificationConnectionResult, String>
{
    #[cfg(windows)]
    let (status, test_notification_requested) =
        crate::proactive::windows_notification::request_connection_test()?;
    #[cfg(not(windows))]
    let (status, test_notification_requested) = ("unavailable", false);
    Ok(NotificationConnectionResult {
        status: status.to_owned(),
        test_notification_requested,
    })
}
