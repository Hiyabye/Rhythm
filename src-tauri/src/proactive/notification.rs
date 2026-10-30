use crate::storage::{
    proactive::{NotificationSensitivity, ProactiveEvent, ProactiveEventKind, ProactiveSettings},
    Store,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;

pub(super) const CANDIDATE_LIMIT: usize = 100;

pub const RATE_LIMIT: u32 = 3;
pub const RATE_WINDOW_SECS: u64 = 600;

#[derive(Clone, Debug, Serialize)]
pub struct NotificationActivation {
    pub event_id: i64,
    pub document_id: i64,
    pub revision_id: i64,
    pub attempt_count: u32,
    pub event_ids: Vec<i64>,
    pub event_attempts: Vec<(i64, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeliveryItem {
    pub event_id: i64,
    pub document_id: i64,
    pub revision_id: i64,
    pub source_id: String,
    pub source_name: String,
    pub kind: ProactiveEventKind,
    pub title: String,
    pub attempt_count: u32,
}

pub(super) fn eligible(
    sensitivity: NotificationSensitivity,
    score: &crate::storage::personalization::PersonalizationNotificationScore,
) -> bool {
    if score.negative_feedback || !score.negative_interest_matches.is_empty() {
        return false;
    }
    match sensitivity {
        NotificationSensitivity::Important => score.has_preferences && score.score >= 18,
        NotificationSensitivity::Interested => score.explicit_interest_match,
        NotificationSensitivity::All => true,
    }
}

/// Read and score revisions without claiming them. A policy or quiet-hours
/// rejection must not consume the persistent dedupe identity.
pub(super) fn consider_candidates(
    store: &Store,
    settings: &ProactiveSettings,
) -> Result<Vec<DeliveryItem>, String> {
    if !settings.notifications_enabled {
        return Ok(Vec::new());
    }
    let candidates = store.proactive_candidates(CANDIDATE_LIMIT)?;
    let ids = candidates
        .iter()
        .map(|candidate| candidate.document_id)
        .collect::<Vec<_>>();
    let scores = store.personalization_notification_scores(&ids)?;
    let score_by_id = scores
        .into_iter()
        .map(|score| (score.document_id, score))
        .collect::<std::collections::HashMap<_, _>>();
    let mut eligible_candidates = Vec::new();
    for candidate in candidates {
        let score = score_by_id.get(&candidate.document_id);
        let reason = if !candidate.source_active {
            Some("정보원 구독 또는 확장 기능이 꺼져 있습니다.".to_string())
        } else if score.is_none_or(|score| !eligible(settings.notification_sensitivity, score)) {
            Some(
                score
                    .map(|score| score.reasons.join(", "))
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "현재 알림 기준에 맞지 않습니다.".into()),
            )
        } else {
            None
        };
        if let Some(reason) = reason {
            if let crate::storage::proactive::ProactiveClaim::Claimed {
                event_id,
                attempt_count,
            } = store.proactive_claim_event(&crate::storage::proactive::ProactiveEventKey {
                document_id: candidate.document_id,
                revision_id: candidate.revision_id,
                event_type: candidate.event_type,
            })? {
                store.proactive_suppress_event(event_id, Some(attempt_count), Some(&reason))?;
            }
        } else {
            eligible_candidates.push(DeliveryItem {
                event_id: 0,
                document_id: candidate.document_id,
                revision_id: candidate.revision_id,
                source_id: candidate.source_id,
                source_name: candidate.source_name,
                kind: candidate.event_type,
                title: candidate.title,
                attempt_count: 0,
            });
        }
    }
    Ok(eligible_candidates)
}

pub(super) fn due_events(store: &Store) -> Result<Vec<ProactiveEvent>, String> {
    store.proactive_due_events(CANDIDATE_LIMIT)
}

pub(super) fn local_quiet(store: &Store, settings: &ProactiveSettings) -> Result<bool, String> {
    Ok(settings
        .quiet_hours
        .contains(store.proactive_local_minutes()?))
}

pub(super) fn digest_text(items: &[DeliveryItem]) -> Option<(String, String)> {
    let first = items.first()?;
    let title = if items.len() == 1 {
        "새 소식이 도착했습니다".to_string()
    } else {
        format!("새 소식 {}건", items.len())
    };
    let body = if items.len() == 1 {
        format!("{} · {}", first.source_name, first.title)
    } else {
        format!(
            "{} · {} 외 {}건의 새 소식이 있습니다.",
            first.source_name,
            first.title,
            items.len() - 1
        )
    };
    Some((title, body))
}

pub(super) fn activation_for_items(items: &[DeliveryItem]) -> Option<NotificationActivation> {
    let first = items.first()?;
    Some(NotificationActivation {
        event_id: first.event_id,
        document_id: first.document_id,
        revision_id: first.revision_id,
        attempt_count: first.attempt_count,
        event_ids: items.iter().map(|item| item.event_id).collect(),
        event_attempts: items
            .iter()
            .map(|item| (item.event_id, item.attempt_count))
            .collect(),
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn show_digest(app: &AppHandle, items: &[DeliveryItem]) -> Result<(), String> {
    let Some(first) = items.first() else {
        return Ok(());
    };
    let (title, body) = digest_text(items).expect("nonempty digest");
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .extra("event_id", first.event_id)
        .extra("document_id", first.document_id)
        .extra("revision_id", first.revision_id)
        .extra("attempt_count", first.attempt_count)
        .extra("source_id", first.source_id.clone())
        .extra("source_name", first.source_name.clone())
        .extra(
            "digest_event_ids",
            items
                .iter()
                .map(|item| format!("{}:{}", item.event_id, item.attempt_count))
                .collect::<Vec<_>>()
                .join(","),
        )
        .show()
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub(super) async fn show_digest(
    app: &AppHandle,
    items: &[DeliveryItem],
) -> Result<Option<mac_usernotifications::NotificationHandle>, String> {
    use std::time::Duration;
    let Some((title, body)) = digest_text(items) else {
        return Ok(None);
    };
    let _ = app;
    // The crate closes the delivered alert when this elapses and resolves the
    // response as `Expired`. Thirty minutes keeps the notice visible while the
    // user is away without leaving a response task pending indefinitely.
    mac_usernotifications::Notification::new()
        .title(title)
        .message(body)
        .timeout(Duration::from_secs(1_800))
        .send()
        .await
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn action_activation(
    event: &tauri_plugin_notification::ActionPerformed,
) -> Option<NotificationActivation> {
    let extra = event.notification()?.extra();
    Some(NotificationActivation {
        event_id: extra.get("event_id")?.as_i64()?,
        document_id: extra.get("document_id")?.as_i64()?,
        revision_id: extra.get("revision_id")?.as_i64()?,
        attempt_count: extra.get("attempt_count")?.as_u64()?.try_into().ok()?,
        event_attempts: extra
            .get("digest_event_ids")?
            .as_str()?
            .split(',')
            .filter_map(|entry| {
                let (id, attempts) = entry.split_once(':')?;
                Some((id.parse::<i64>().ok()?, attempts.parse::<u32>().ok()?))
            })
            .collect(),
        event_ids: extra
            .get("digest_event_ids")?
            .as_str()?
            .split(',')
            .filter_map(|entry| entry.split_once(':')?.0.parse::<i64>().ok())
            .collect(),
    })
}

pub(super) fn emit_activation(app: &AppHandle, activation: NotificationActivation) {
    let _ = app.emit("proactive://notification-clicked", activation);
}
