//! Bounded maintenance while the application is open; not a background scheduler.
use crate::{
    intelligence::{analysis, idle},
    storage::intelligence::AiPreferences,
    AppState,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

/// Opt-in backlog work starts only after this much OS user inactivity.
const BACKLOG_IDLE_MINIMUM: Duration = Duration::from_secs(5 * 60);
/// Rolling-hour cap for backlog attempts, independent of the session cap.
const BACKLOG_HOURLY_LIMIT: usize = 20;
const BACKLOG_WINDOW: Duration = Duration::from_secs(60 * 60);

/// Remote automatic work is a separate, cost-bearing opt-in. `key_ready` is
/// the cached credential-store presence, never the key itself.
fn remote_ready(preferences: &AiPreferences, key_ready: bool) -> bool {
    preferences.engine == "openai-compatible"
        && key_ready
        && preferences.remote_consent
        && preferences.remote_automatic
        && preferences.remote_daily_limit > 0
        && preferences
            .remote_base_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && preferences
            .remote_model
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn automatic_budget(
    energy: &str,
    preferences: &AiPreferences,
    focused: bool,
    key_ready: bool,
) -> u32 {
    if !focused || preferences.processing_mode != "recent" {
        return 0;
    }
    if preferences.engine == "openai-compatible" {
        if energy == "eco" || !remote_ready(preferences, key_ready) {
            return 0;
        }
        // The exact cap is reserved per attempt; this only bounds the loop.
        return preferences.remote_daily_limit.min(20);
    }
    if !preferences.setup_completed || preferences.active_model_id.is_none() {
        return 0;
    }
    match energy {
        "balanced" => preferences.automatic_limit.min(3),
        "performance" => preferences.automatic_limit.min(20),
        // Eco never starts opportunistic work, including while on battery.
        _ => 0,
    }
}

/// Backlog is opt-in, idle-only and never runs in Eco. Both platform probes are
/// required; an unknown observation (None) never admits work.
fn backlog_allowed(
    energy: &str,
    preferences: &AiPreferences,
    ready: bool,
    key_ready: bool,
) -> bool {
    ready
        && preferences.processing_mode == "backlog"
        && energy != "eco"
        && if preferences.engine == "openai-compatible" {
            remote_ready(preferences, key_ready)
        } else {
            preferences.setup_completed && preferences.active_model_id.is_some()
        }
}

/// Drops timestamps older than the rolling hour and reports whether one more
/// backlog attempt fits under the cap.
fn backlog_window_allows(window: &mut VecDeque<Instant>, now: Instant) -> bool {
    while window
        .front()
        .is_some_and(|started| now.saturating_duration_since(*started) >= BACKLOG_WINDOW)
    {
        window.pop_front();
    }
    window.len() < BACKLOG_HOURLY_LIMIT
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
    let mut backlog_window = VecDeque::new();
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
        // Remote work needs a cached credential-store presence check. It is
        // only consulted when the mode and consent already allow automatic work.
        let key_ready = if preferences.engine == "openai-compatible"
            && preferences.remote_consent
            && preferences.remote_automatic
            && preferences.remote_daily_limit > 0
        {
            match super::remote::GenerationConfig::from_preferences(&preferences) {
                Ok(config) => {
                    let origin = config.endpoint.origin().to_owned();
                    let remote = state.remote.clone();
                    tokio::task::spawn_blocking(move || remote.present(&origin))
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false)
                }
                Err(_) => false,
            }
        } else {
            // The local engine has no credential-store prerequisite.
            true
        };
        if admitted < automatic_budget(&energy, &preferences, focused, key_ready) {
            let Some(boundary) = boundary.as_deref() else {
                continue;
            };
            // Bound recent ingestion/revision candidates, independent of publication
            // order. Initial historical backfill and already-attempted work are excluded.
            let candidate = recent_candidates(&state, &preferences, boundary, &attempted);
            let Some((document, revision)) = candidate else {
                continue;
            };
            let reserved = if preferences.engine == "openai-compatible" {
                if reserve_remote(&state, &preferences) != Some(true) {
                    continue;
                }
                true
            } else {
                false
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
                analysis::DIRECT_PRIORITY,
                lease,
                Some(state.remote.clone()),
            ) {
                Ok(response) if response.job.is_some() => {
                    admitted += 1;
                    last_activity = Instant::now();
                }
                Ok(_) => {
                    if reserved {
                        release_remote(&state);
                    }
                }
                Err(error) => {
                    if reserved {
                        release_remote(&state);
                    }
                    eprintln!("AI opportunistic admission: {error}");
                }
            }
            continue;
        }
        // Opt-in idle backlog. The platform probes run only when the mode is on.
        let backlog_ready = preferences.processing_mode == "backlog"
            && backlog_allowed(
                &energy,
                &preferences,
                idle::user_is_idle(BACKLOG_IDLE_MINIMUM) && idle::on_external_power(),
                key_ready,
            );
        if !backlog_ready || !backlog_window_allows(&mut backlog_window, Instant::now()) {
            continue;
        }
        let candidate = backlog_candidate(&state, &preferences, &attempted);
        let Some((document, revision)) = candidate else {
            continue;
        };
        let reserved = if preferences.engine == "openai-compatible" {
            if reserve_remote(&state, &preferences) != Some(true) {
                continue;
            }
            true
        } else {
            false
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
            analysis::BACKLOG_PRIORITY,
            lease,
            Some(state.remote.clone()),
        ) {
            Ok(response) if response.job.is_some() => {
                backlog_window.push_back(Instant::now());
                last_activity = Instant::now();
            }
            Ok(_) => {
                if reserved {
                    release_remote(&state);
                }
            }
            Err(error) => {
                if reserved {
                    release_remote(&state);
                }
                eprintln!("AI backlog admission: {error}");
            }
        }
    }
}

/// Reserves one automatic remote attempt. `None` means the local engine or a
/// store lock that could not be taken; `Some(false)` means the daily cap.
fn reserve_remote(state: &AppState, preferences: &AiPreferences) -> Option<bool> {
    if preferences.engine != "openai-compatible" {
        return None;
    }
    state.store.try_lock().ok().and_then(|store| {
        store
            .ai_reserve_remote_automatic(preferences.remote_daily_limit)
            .ok()
    })
}

fn release_remote(state: &AppState) {
    if let Ok(store) = state.store.try_lock() {
        let _ = store.ai_release_remote_automatic();
    }
}

/// Candidate selection needs the selected model to be fully installed; every
/// lock is a `try_lock` so the maintenance loop never waits behind a user action.
fn candidate_store<'a>(
    state: &'a AppState,
    preferences: &AiPreferences,
) -> Option<std::sync::MutexGuard<'a, crate::Store>> {
    let store = state.store.try_lock().ok()?;
    // Remote analysis does not need a local generation model.
    if preferences.engine == "openai-compatible" {
        return Some(store);
    }
    let model = store
        .ai_model(preferences.active_model_id.as_deref()?)
        .ok()??;
    (model.status == "installed").then_some(store)
}

fn recent_candidates(
    state: &AppState,
    preferences: &AiPreferences,
    boundary: &str,
    attempted: &HashSet<(i64, i64)>,
) -> Option<(i64, i64)> {
    candidate_store(state, preferences)?
        .ai_recent_candidates(boundary, 20)
        .ok()?
        .into_iter()
        .find(|identity| !attempted.contains(identity))
}

/// Newest publication first across the whole archive, including the initial
/// backfill. Revisions with any prior attempt or result are excluded by the query.
fn backlog_candidate(
    state: &AppState,
    preferences: &AiPreferences,
    attempted: &HashSet<(i64, i64)>,
) -> Option<(i64, i64)> {
    candidate_store(state, preferences)?
        .ai_backlog_candidates(BACKLOG_HOURLY_LIMIT as u32)
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
            execution_mode: "auto".into(),
            model_directory: None,
            embedding_model_id: None,
            classification_enabled: true,
            engine: "local".into(),
            remote_base_url: None,
            remote_model: None,
            remote_embedding_model: None,
            remote_timeout_seconds: 120,
            remote_consent: false,
            remote_automatic: false,
            remote_daily_limit: 20,
            remote_embeddings: false,
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
        assert_eq!(automatic_budget("eco", &p, true, true), 0);
        assert_eq!(automatic_budget("balanced", &p, true, true), 3);
        assert_eq!(automatic_budget("performance", &p, true, true), 20);
        assert_eq!(automatic_budget("performance", &p, false, true), 0);
        p.processing_mode = "explicit".into();
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.processing_mode = "backlog".into();
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.processing_mode = "recent".into();
        p.setup_completed = false;
        assert_eq!(automatic_budget("balanced", &p, true, true), 0);
        p.setup_completed = true;
        p.active_model_id = None;
        assert_eq!(automatic_budget("balanced", &p, true, true), 0);
    }
    #[test]
    fn remote_automatic_work_requires_separate_consent_key_and_cap() {
        let mut p = preferences();
        p.engine = "openai-compatible".into();
        p.remote_base_url = Some("https://api.example.com".into());
        p.remote_model = Some("model".into());
        p.remote_consent = true;
        p.remote_automatic = true;
        p.remote_daily_limit = 5;
        // A missing credential-store entry never admits automatic remote work.
        assert_eq!(automatic_budget("performance", &p, true, false), 0);
        assert_eq!(automatic_budget("performance", &p, true, true), 5);
        p.remote_automatic = false;
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.remote_automatic = true;
        p.remote_consent = false;
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.remote_consent = true;
        p.remote_daily_limit = 0;
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.remote_daily_limit = 5;
        // Eco still suppresses opportunistic work.
        assert_eq!(automatic_budget("eco", &p, true, true), 0);
        p.processing_mode = "backlog".into();
        assert!(backlog_allowed("balanced", &p, true, true));
        assert!(!backlog_allowed("balanced", &p, true, false));
        assert!(!backlog_allowed("eco", &p, true, true));
    }

    #[test]
    fn automatic_work_is_session_bounded() {
        let mut p = preferences();
        p.automatic_limit = 1;
        assert_eq!(automatic_budget("balanced", &p, true, true), 1);
        p.automatic_limit = 0;
        assert_eq!(automatic_budget("performance", &p, true, true), 0);
        p.automatic_limit = u32::MAX;
        assert_eq!(automatic_budget("performance", &p, true, true), 20);
    }
    #[test]
    fn backlog_requires_opt_in_idle_power_and_non_eco() {
        let mut p = preferences();
        p.processing_mode = "backlog".into();
        assert!(backlog_allowed("balanced", &p, true, true));
        assert!(!backlog_allowed("eco", &p, true, true));
        assert!(!backlog_allowed("balanced", &p, false, true));
        p.processing_mode = "recent".into();
        assert!(!backlog_allowed("performance", &p, true, true));
        p.processing_mode = "backlog".into();
        p.setup_completed = false;
        assert!(!backlog_allowed("balanced", &p, true, true));
        p.setup_completed = true;
        p.active_model_id = None;
        assert!(!backlog_allowed("balanced", &p, true, true));
    }
    #[test]
    fn backlog_hourly_window_prunes_and_caps() {
        let now = Instant::now();
        let mut window = VecDeque::new();
        for _ in 0..BACKLOG_HOURLY_LIMIT {
            assert!(backlog_window_allows(&mut window, now));
            window.push_back(now);
        }
        assert!(!backlog_window_allows(&mut window, now));
        // Entries older than an hour are dropped before the cap is applied.
        assert!(backlog_window_allows(
            &mut window,
            now + BACKLOG_WINDOW + Duration::from_secs(1)
        ));
        assert_eq!(window.len(), 0);
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
