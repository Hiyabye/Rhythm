//! Bounded maintenance while the application is open; not a background scheduler.
use crate::{storage::intelligence::AiPreferences, AppState};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

fn automatic_budget(energy: &str, preferences: &AiPreferences, focused: bool) -> u32 {
    if !focused
        || !preferences.setup_completed
        || preferences.active_model_id.is_none()
        || preferences.processing_mode != "recent"
    {
        return 0;
    }
    match energy {
        "balanced" => preferences.automatic_limit.min(3),
        "performance" => preferences.automatic_limit.min(20),
        // Eco never starts opportunistic work, including while on battery.
        _ => 0,
    }
}
fn idle_limit(energy: &str) -> Duration {
    Duration::from_secs(match energy {
        "eco" => 30,
        "performance" => 600,
        _ => 120,
    })
}

// SQLite collection timestamps are UTC ISO strings with three fractional digits.
// Deriving the boundary from SystemTime avoids a platform command or another dependency.
fn session_boundary(time: SystemTime) -> Option<String> {
    let since_epoch = time.duration_since(UNIX_EPOCH).ok()?;
    let seconds = since_epoch.as_secs();
    let mut days = seconds / 86400;
    let mut year = 1970;
    let leap = |year: u64| {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    };
    loop {
        let count = if leap(year) { 366 } else { 365 };
        if days < count {
            break;
        }
        days -= count;
        year += 1;
        if year > 9999 {
            return None;
        }
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= months[month] {
        days -= months[month];
        month += 1;
    }
    Some(format!(
        "{year:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        month + 1,
        days + 1,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        since_epoch.subsec_millis()
    ))
}
/// Start once after `AppState` is managed. The owner must set `shutdown=true` before
/// stopping controllers/worker on exit. No task is started outside the app's lifetime.
pub async fn run(handle: tauri::AppHandle, shutdown: Arc<AtomicBool>) {
    let boundary = session_boundary(SystemTime::now());
    let mut attempted = HashSet::new();
    let mut admitted = 0;
    let mut last_activity = Instant::now();
    let mut previous_model = None;
    let mut previous_job = None;
    let mut previous_operation = 0;
    while !shutdown.load(Ordering::Acquire) {
        // No immediate startup inference. A model is loaded only by explicit setup,
        // explicit analysis, or the opt-in focused-window policy below.
        tokio::time::sleep(Duration::from_secs(5)).await;
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let Some(state) = handle.try_state::<AppState>() else {
            break;
        };
        let Ok(lease) = state.operation_gate.shared().await else {
            continue;
        };
        let focused = handle
            .get_webview_window("main")
            .and_then(|window| window.is_focused().ok())
            .unwrap_or(false);
        if state.analysis.running() || state.models.busy() {
            last_activity = Instant::now();
            continue;
        }
        // Read policy only after admission is serialized with user preference
        // changes. A snapshot taken before this gate could admit work after opt-out.
        // Never wait behind a user operation, and preserve its lock order.
        let Ok(_gate) = state.ai_gate.try_lock() else {
            last_activity = Instant::now();
            continue;
        };
        if shutdown.load(Ordering::Acquire) || state.analysis.running() {
            continue;
        }
        let operation = state.ai_operation.activity_generation();
        if previous_operation != operation {
            previous_operation = operation;
            last_activity = Instant::now();
        }
        let context = state.store.try_lock().ok().and_then(|store| {
            Some((
                store.settings().ok()?.energy_profile,
                store.ai_preferences().ok()?,
                store
                    .ai_jobs(1)
                    .ok()?
                    .first()
                    .map(|job| (job.id, job.status.clone(), job.updated_at.clone())),
            ))
        });
        let Some((energy, preferences, newest_job)) = context else {
            continue;
        };
        if previous_job != newest_job {
            previous_job = newest_job;
            last_activity = Instant::now();
        }
        {
            let Ok(mut worker) = state.worker.try_lock() else {
                last_activity = Instant::now();
                continue;
            };
            let status = worker.status();
            if previous_model != status.model {
                previous_model = status.model.clone();
                last_activity = Instant::now();
            }
            if status.model.is_some() && last_activity.elapsed() >= idle_limit(&energy) {
                if let Err(error) = worker.unload().await {
                    eprintln!("AI idle unload: {error}");
                }
                previous_model = None;
                last_activity = Instant::now();
            }
        }
        if admitted >= automatic_budget(&energy, &preferences, focused) {
            continue;
        }
        let Some(boundary) = boundary.as_deref() else {
            continue;
        };
        // Bound recent ingestion/revision candidates, independent of publication
        // order. Initial historical backfill and already-attempted work are excluded.
        let candidate = candidates(&state, &preferences, boundary, &attempted);
        let Some((document, revision)) = candidate else {
            continue;
        };
        attempted.insert((document, revision));
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        match state.analysis.start_admitted(
            state.store.clone(),
            state.worker.clone(),
            state.data_root.clone(),
            document,
            revision,
            false,
            lease,
        ) {
            Ok(response) if response.job.is_some() => {
                admitted += 1;
                last_activity = Instant::now();
            }
            Ok(_) => (),
            Err(error) => eprintln!("AI opportunistic admission: {error}"),
        }
    }
}

fn candidates(
    state: &AppState,
    preferences: &AiPreferences,
    boundary: &str,
    attempted: &HashSet<(i64, i64)>,
) -> Option<(i64, i64)> {
    let store = state.store.try_lock().ok()?;
    let model = store
        .ai_model(preferences.active_model_id.as_deref()?)
        .ok()??;
    if model.status != "installed" {
        return None;
    }
    store
        .ai_recent_candidates(boundary, 20)
        .ok()?
        .into_iter()
        .find(|identity| !attempted.contains(identity))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preferences() -> AiPreferences {
        AiPreferences {
            active_model_id: Some("model".into()),
            setup_completed: true,
            processing_mode: "recent".into(),
            automatic_limit: 20,
            max_tokens: 1024,
        }
    }
    #[test]
    fn maintenance_future_can_run_on_the_tauri_async_runtime() {
        fn assert_send<T: Send>(_: T) {}
        fn check(handle: tauri::AppHandle) {
            assert_send(run(handle, Arc::new(AtomicBool::new(false))));
        }
        let _ = check as fn(tauri::AppHandle);
    }
    #[test]
    fn automatic_policy_requires_focus_setup_opt_in_and_non_eco() {
        let mut p = preferences();
        assert_eq!(automatic_budget("eco", &p, true), 0);
        assert_eq!(automatic_budget("balanced", &p, true), 3);
        assert_eq!(automatic_budget("performance", &p, true), 20);
        assert_eq!(automatic_budget("performance", &p, false), 0);
        p.processing_mode = "explicit".into();
        assert_eq!(automatic_budget("performance", &p, true), 0);
        p.processing_mode = "recent".into();
        p.setup_completed = false;
        assert_eq!(automatic_budget("balanced", &p, true), 0);
        p.setup_completed = true;
        p.active_model_id = None;
        assert_eq!(automatic_budget("balanced", &p, true), 0);
    }
    #[test]
    fn automatic_work_is_session_bounded() {
        let mut p = preferences();
        p.automatic_limit = 1;
        assert_eq!(automatic_budget("balanced", &p, true), 1);
        p.automatic_limit = 0;
        assert_eq!(automatic_budget("performance", &p, true), 0);
        p.automatic_limit = u32::MAX;
        assert_eq!(automatic_budget("performance", &p, true), 20);
    }
    #[test]
    fn utc_boundary_and_residency_policy_are_deterministic() {
        assert_eq!(
            session_boundary(UNIX_EPOCH).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
        assert_eq!(
            session_boundary(UNIX_EPOCH + Duration::from_millis(951825600123)).as_deref(),
            Some("2000-02-29T12:00:00.123Z")
        );
        assert_eq!(
            session_boundary(UNIX_EPOCH + Duration::from_secs(1709251200)).as_deref(),
            Some("2024-03-01T00:00:00.000Z")
        );
        assert_eq!(idle_limit("eco"), Duration::from_secs(30));
        assert_eq!(idle_limit("balanced"), Duration::from_secs(120));
        assert_eq!(idle_limit("performance"), Duration::from_secs(600));
    }
}
