//! Calendar todo reminders (#189), sent from the scheduler loop while Rhythm is
//! open. Store decides what is due and reserves each handoff, so a reminder held
//! back by quiet hours, the shared rate limit or a full response queue is simply
//! tried again on a later pass while its todo is still ahead. A reserved
//! reminder whose handoff fails is never sent again.
use super::scheduler::{lock_store, NotificationState};
use crate::{archive::sync::SharedStore, storage::calendar::DueReminder};
use chrono::NaiveDate;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

#[cfg(any(windows, test))]
pub(crate) fn token_prefix() -> &'static str {
    crate::core::windows_identity::current()
        .reminder_prefix
        .as_str()
}
#[cfg(any(windows, target_os = "macos"))]
const UNCERTAIN: &str = "할 일 알림 전달 결과를 확인하지 못했습니다. 자동 재전송하지 않습니다.";

pub(super) async fn deliver(
    store: &SharedStore,
    app: &AppHandle,
    notifications: &mut NotificationState,
    stop: &watch::Receiver<bool>,
) -> Result<(), String> {
    let now = chrono::Local::now().naive_local();
    let (due, clock) = {
        let store = lock_store(store)?;
        (
            store.calendar_due_reminders(now)?,
            store.settings()?.clock_format,
        )
    };
    if due.is_empty() {
        return Ok(());
    }
    let authorization = crate::notification_permission::notification_authorization_status().await;
    if !super::scheduler::notification_authorization_permits(&authorization, false) {
        return Ok(());
    }
    for reminder in due {
        if *stop.borrow() || notifications.response_tasks.is_full() {
            break;
        }
        let token = token()?;
        #[cfg(windows)]
        {
            let _ = app;
            // Preparing the toast can fail without consuming the latch or a rate slot.
            let (title, body) = text(&reminder, &clock);
            let store = lock_store(store)?;
            let prepared = super::windows_notification::prepare_reminder(&title, &body, &token)?;
            if store
                .calendar_reminder_reserve(reminder.todo_id, &token, now)?
                .is_none()
            {
                continue;
            }
            // The synchronous handoff shares Store with settings and disable.
            super::windows_notification::show(prepared).map_err(|_| UNCERTAIN.to_string())?;
        }
        #[cfg(target_os = "macos")]
        {
            let Some(reminder) =
                lock_store(store)?.calendar_reminder_reserve(reminder.todo_id, &token, now)?
            else {
                continue;
            };
            let (title, body) = text(&reminder, &clock);
            let handle = mac_usernotifications::Notification::new()
                .title(title)
                .message(body)
                .timeout(std::time::Duration::from_secs(1_800))
                .send()
                .await
                .map_err(|_| UNCERTAIN.to_string())?;
            let store = store.clone();
            let app = app.clone();
            let pending = notifications.pending_reminder.clone();
            let accepting = notifications.accepting_callbacks.clone();
            let callback_gate = notifications.callback_gate.clone();
            let operation_gate = notifications.operation_gate.clone();
            let mut stop = stop.clone();
            notifications.response_tasks.spawn(async move {
                use mac_usernotifications::response::CloseReason;
                let response = tokio::select! {
                    _ = stop.changed() => return,
                    response = handle.response() => response,
                };
                let Ok(response) = response else { return };
                if !response.is_default_action()
                    || matches!(
                        response.close_reason,
                        Some(CloseReason::Dismissed | CloseReason::Expired)
                    )
                {
                    return;
                }
                let Ok(_lease) = operation_gate.shared().await else {
                    return;
                };
                let Ok(_callback) = callback_gate.lock() else {
                    return;
                };
                if !accepting.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                if let Err(error) = activate(&store, &app, &pending, &token) {
                    eprintln!("Calendar reminder activation: {error}");
                }
            });
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (app, &reminder, &clock, token);
            return Ok(());
        }
    }
    Ok(())
}

/// Consumes a clicked reminder's token, then shows the window and asks the
/// calendar to open the todo's day. The day also waits in `pending` for a
/// window that subscribes later.
pub(super) fn activate(
    store: &SharedStore,
    app: &AppHandle,
    pending: &Mutex<Option<String>>,
    token: &str,
) -> Result<bool, String> {
    let Some(day) = lock_store(store)?.calendar_reminder_consume(token)? else {
        return Ok(false);
    };
    *pending
        .lock()
        .map_err(|_| "할 일 알림 이동 정보를 저장할 수 없습니다.")? = Some(day.clone());
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(error) = crate::core::lifecycle::show_main_window(&handle) {
            eprintln!("Lifecycle: 알림 창 복원 실패: {error}");
        }
    })
    .map_err(|e| e.to_string())?;
    let _ = app.emit("calendar://reminder-clicked", day);
    Ok(true)
}

fn token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "할 일 알림 보안 토큰을 만들지 못했습니다.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(any(windows, test))]
pub(crate) fn token_from_uri(uri: &str) -> Option<&str> {
    crate::core::windows_identity::current().reminder_token_from_uri(uri)
}

/// Title and body in the user's 12/24-hour format, like the calendar shows them.
fn text(reminder: &DueReminder, clock: &str) -> (String, String) {
    let day = NaiveDate::parse_from_str(&reminder.day, "%Y-%m-%d")
        .map(|day| day.format("%-m월 %-d일").to_string())
        .unwrap_or_else(|_| reminder.day.clone());
    let when = match &reminder.time {
        Some(time) => format!("{day} {}", wall_time(time, clock)),
        None => day,
    };
    ("할 일 알림".into(), format!("{}\n{when}", reminder.title))
}
/// Mirrors `wallTime` in `src/lib/archive-format.ts`.
fn wall_time(time: &str, clock: &str) -> String {
    if clock == "24h" {
        return time.to_owned();
    }
    let hour: u32 = time
        .get(..2)
        .and_then(|hour| hour.parse().ok())
        .unwrap_or(0);
    let minutes = time.get(3..5).unwrap_or("00");
    format!(
        "{} {}:{minutes}",
        if hour < 12 { "오전" } else { "오후" },
        match hour % 12 {
            0 => 12,
            hour => hour,
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reminder(time: Option<&str>) -> DueReminder {
        DueReminder {
            todo_id: 1,
            title: "보고서 제출".into(),
            day: "2026-11-05".into(),
            time: time.map(Into::into),
            minutes: 10,
        }
    }

    #[test]
    fn text_follows_the_clock_format() {
        assert_eq!(
            text(&reminder(Some("21:05")), "12h"),
            (
                "할 일 알림".into(),
                "보고서 제출\n11월 5일 오후 9:05".into()
            )
        );
        assert_eq!(
            text(&reminder(Some("00:30")), "12h").1,
            "보고서 제출\n11월 5일 오전 12:30"
        );
        assert_eq!(
            text(&reminder(Some("09:00")), "24h").1,
            "보고서 제출\n11월 5일 09:00"
        );
        assert_eq!(text(&reminder(None), "12h").1, "보고서 제출\n11월 5일");
    }

    #[test]
    fn only_reminder_links_are_accepted() {
        let token = "a".repeat(64);
        let prefix = token_prefix();
        assert_eq!(
            token_from_uri(&format!("{prefix}{token}")),
            Some(token.as_str())
        );
        assert_eq!(
            token_from_uri(&format!("rhythm-win://notification/{token}")),
            None
        );
        assert_eq!(token_from_uri(&format!("{prefix}{}", "g".repeat(64))), None);
        assert_eq!(token_from_uri(prefix), None);
    }
}
