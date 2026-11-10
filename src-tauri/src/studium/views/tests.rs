//! Deterministic local geometry/summary/lifetime rules, not native-window or
//! operating-system tray evidence. No application window or transport exists.
use super::*;
use crate::studium::{
    model::{AutomaticStatus, Preferences, RoomState, SeatData},
    parser, rooms,
};
use std::{
    future::Future,
    task::{Context, Wake, Waker},
};

fn screen(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Screen {
    Screen {
        x,
        y,
        width,
        height,
        scale,
    }
}

#[test]
fn valid_physical_position_and_logical_size_survive_a_dpi_aware_restore() {
    let saved = MiniGeometry {
        x: -2400,
        y: 100,
        width: 380,
        height: 480,
    };
    let monitors = [
        screen(-2560, 0, 2560, 1440, 2.0),
        screen(0, 0, 1920, 1080, 1.0),
    ];
    assert_eq!(
        placement(Some(&saved), &monitors),
        Placement {
            width: 380,
            height: 480,
            position: Some((-2400, 100)),
        }
    );
}

#[test]
fn disconnected_monitors_and_dpi_changes_recover_centered_with_a_fitting_size() {
    let saved = MiniGeometry {
        x: 4000,
        y: -1500,
        width: 1000,
        height: 1200,
    };
    assert_eq!(
        placement(Some(&saved), &[screen(0, 0, 1200, 900, 2.0)]),
        Placement {
            width: 600,
            height: 450,
            position: None,
        }
    );
    let oversized = MiniGeometry {
        x: 100,
        y: 100,
        width: 1000,
        height: 1200,
    };
    assert_eq!(
        placement(Some(&oversized), &[screen(0, 0, 1200, 900, 2.0)]).position,
        None
    );
}

#[test]
fn no_monitor_metadata_and_invalid_scales_use_bounded_defaults() {
    for screens in [
        vec![],
        vec![screen(0, 0, 1920, 1080, f64::NAN)],
        vec![screen(0, 0, 0, 0, 0.0)],
    ] {
        assert_eq!(
            placement(None, &screens),
            Placement {
                width: 380,
                height: 480,
                position: None
            }
        );
    }
    let corrupt = MiniGeometry {
        x: i32::MAX,
        y: i32::MIN,
        width: u32::MAX,
        height: 0,
    };
    assert_eq!(
        placement(Some(&corrupt), &[]),
        Placement {
            width: 1200,
            height: 260,
            position: None
        }
    );
}

#[test]
fn a_delayed_disable_ticket_does_not_match_an_explicitly_reopened_window() {
    let mut state = WindowState::default();
    assert_eq!(
        state.status(),
        MiniStatus {
            open: false,
            always_on_top: false
        }
    );
    state.reserve_open().unwrap();
    let disabled = state.ticket();
    state.on_top = true;
    state.reserve_open().unwrap(); // Existing physical window, new user intent.
    assert!(!state.matches(disabled));
    assert!(state.status().open);
    assert!(state.status().always_on_top);
    state.open = false;
    state.on_top = false;
    state.closing = true;
    assert!(state.reserve_open().is_err());
    state.closing = false;
    state.reserve_open().unwrap();
    assert!(!state.status().always_on_top);
}

#[test]
fn an_old_physical_destroy_event_cannot_clear_a_new_window_session() {
    let mut state = WindowState::default();
    state.reserve_open().unwrap();
    state.session = Some(Arc::new(WindowSession {
        physical: 7,
        finished: AtomicBool::new(false),
        done: Notify::new(),
    }));
    state.destroyed(6);
    assert!(state.status().open);
    assert!(state.session.is_some());
    state.destroyed(7);
    assert_eq!(
        state.status(),
        MiniStatus {
            open: false,
            always_on_top: false
        }
    );
    assert!(state.session.is_none());
}

#[test]
fn destroy_cleanup_precedes_waking_a_waiting_reopen() {
    struct ReopenWake {
        windows: Arc<Mutex<WindowState>>,
        woken: AtomicBool,
    }
    impl Wake for ReopenWake {
        fn wake(self: Arc<Self>) {
            let state = self.windows.lock().unwrap();
            assert!(!state.closing);
            assert!(state.session.is_none());
            self.woken.store(true, Ordering::Release);
        }
    }
    let session = Arc::new(WindowSession {
        physical: 1,
        finished: AtomicBool::new(false),
        done: Notify::new(),
    });
    let windows = Arc::new(Mutex::new(WindowState {
        generation: 1,
        open: false,
        on_top: false,
        session: Some(session.clone()),
        closing: true,
    }));
    let wake = Arc::new(ReopenWake {
        windows: windows.clone(),
        woken: AtomicBool::new(false),
    });
    let waker = Waker::from(wake.clone());
    let mut context = Context::from_waker(&waker);
    let mut waiting = Box::pin(session.wait());
    assert!(waiting.as_mut().poll(&mut context).is_pending());
    finish_destroyed_state(&windows, &session);
    assert!(wake.woken.load(Ordering::Acquire));
    assert!(waiting.as_mut().poll(&mut context).is_ready());
}

#[test]
fn failed_creation_rolls_back_only_its_current_uncreated_generation() {
    let mut state = WindowState::default();
    let first = state.reserve_open().unwrap();
    state.rollback_open(first);
    assert!(!state.status().open);
    let old = state.reserve_open().unwrap();
    let current = state.reserve_open().unwrap();
    state.rollback_open(old);
    assert!(state.status().open);
    state.session = Some(Arc::new(WindowSession {
        physical: current,
        finished: AtomicBool::new(false),
        done: Notify::new(),
    }));
    state.rollback_open(current);
    assert!(state.status().open);
}

fn fixture() -> Snapshot {
    let catalog = rooms::all();
    let statuses = [
        "ready",
        "ready",
        "count_only",
        "failed",
        "loading",
        "not_checked",
        "cancelled",
    ];
    Snapshot {
        preferences: Preferences {
            room_ids: catalog.iter().map(|room| room.id.to_owned()).collect(),
            view_room: "5".into(),
        },
        results: catalog
            .iter()
            .zip(statuses)
            .map(|(room, status)| RoomState {
                room_id: room.id.into(),
                status: status.into(),
                error: None,
                checked_at: Some("2026-11-09T15:00:01Z".into()),
                result: matches!(status, "ready" | "count_only").then(|| SeatData {
                    total: room.capacity,
                    used: room.capacity,
                    remaining: 0,
                    source_time: "2026-11-09T15:00:00Z".into(),
                    seat_numbers: (status == "ready").then(Vec::new),
                    source_url: if status == "ready" {
                        room.url()
                    } else {
                        rooms::STATUS_URL.into()
                    },
                    layout: None,
                    layout_error: None,
                }),
            })
            .collect(),
        rooms: catalog,
        job_id: Some(42),
        running: true,
        automatic: AutomaticStatus {
            active: false,
            interval_minutes: 5,
            next_check_at: None,
            error: None,
        },
        favorites: Default::default(),
        alert_rules: Default::default(),
        alert_error: None,
    }
}

#[test]
fn tray_rows_preserve_catalog_order_verified_zero_and_every_unknown_state() {
    let snapshot = fixture();
    let rows = summary_rows(
        &snapshot,
        parser::timestamp_seconds("2026-11-09T15:00:00Z").unwrap(),
        "24h",
    );
    assert_eq!(rows.len(), 7);
    for (row, room) in rows.iter().zip(&snapshot.rooms) {
        assert!(row.starts_with(room.label));
    }
    assert!(rows[0].contains("빈 0석"));
    assert!(rows[0].contains("11/10 00:00"));
    assert!(rows[2].contains("빈 0석"));
    assert!(rows[2].contains("번호 미확인"));
    assert!(rows[3].ends_with("조회 실패"));
    assert!(rows[4].ends_with("확인 중"));
    assert!(rows[5].ends_with("미조회"));
    assert!(rows[6].ends_with("취소됨"));
    assert!(!rows[3].contains("0석"));
    assert!(!rows[5].contains("0석"));
}

#[test]
fn stale_tray_state_uses_source_time_without_substituting_receive_time() {
    let snapshot = fixture();
    let now = parser::timestamp_seconds("2026-11-09T15:05:01Z").unwrap();
    let rows = summary_rows(&snapshot, now, "24h");
    assert!(rows[0].contains("오래됨"));
    assert!(rows[0].contains("11/10 00:00"));
    assert!(rows[2].contains("오래됨"));
}

#[test]
fn tray_source_time_respects_the_clock_preference_at_korean_midnight_and_noon() {
    let mut snapshot = fixture();
    let midnight = parser::timestamp_seconds("2026-11-09T15:00:00Z").unwrap();
    assert!(summary_rows(&snapshot, midnight, "12h")[0].contains("11/10 오전 12:00"));
    assert!(summary_rows(&snapshot, midnight, "24h")[0].contains("11/10 00:00"));
    snapshot.results[0].result.as_mut().unwrap().source_time = "2026-11-09T03:00:00Z".into();
    let noon = parser::timestamp_seconds("2026-11-09T03:00:00Z").unwrap();
    assert!(summary_rows(&snapshot, noon, "12h")[0].contains("11/09 오후 12:00"));
    assert!(summary_rows(&snapshot, noon, "24h")[0].contains("11/09 12:00"));
}

#[test]
fn tooltip_is_bounded_in_utf16_while_the_menu_keeps_all_rooms() {
    let rows = vec!["열람실😀".repeat(50); 7];
    let tooltip = tray_tooltip(&rows);
    assert!(tooltip.encode_utf16().count() <= 120);
    assert!(tooltip.starts_with("Rhythm"));
    assert_eq!(rows.len(), 7);
    assert_eq!(tray_tooltip(&[]), "Rhythm");
}
