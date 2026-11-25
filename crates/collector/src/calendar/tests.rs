use super::*;
use rhythm_sources::transport::FetchError;
use std::sync::Mutex;

const FIXTURE: &str = include_str!("../../../sources/src/fixtures/academic-calendar.ics");
const ROBOTS: &str = "https://outlook.office365.com/robots.txt";

/// Answers the robots.txt and the ICS; records every request.
struct Fake {
    robots: Result<String, u16>,
    ics: Mutex<(String, String)>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn new(ics: &str) -> Self {
        Self {
            // The real transport turns the host's redirect into an empty policy.
            robots: Ok(String::new()),
            ics: Mutex::new((ics.to_owned(), "text/calendar".into())),
            calls: Mutex::default(),
        }
    }
    fn serve(&self, ics: &str, mime: &str) {
        *self.ics.lock().unwrap() = (ics.to_owned(), mime.to_owned());
    }
}

impl Transport for Fake {
    async fn fetch(
        &self,
        url: &Url,
        _validators: Option<(Option<String>, Option<String>)>,
        limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        assert_eq!(access, Access::Calendar);
        self.calls.lock().unwrap().push(url.to_string());
        let (body, mime) = if url.as_str() == ROBOTS {
            match &self.robots {
                Ok(body) => (body.clone(), "text/plain".to_owned()),
                Err(status) => return Err(FetchError::http(*status, "robots".into())),
            }
        } else {
            assert_eq!(url.as_str(), ACADEMIC_CALENDAR_URL);
            self.ics.lock().unwrap().clone()
        };
        assert!(body.len() <= limit);
        Ok(Fetch::Body {
            bytes: body.into_bytes(),
            etag: None,
            last_modified: None,
            mime: Some(mime),
        })
    }
}

/// The fixture without the events whose `SUMMARY` line or `DTSTART` year
/// matches `drop`.
fn without(drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut block = String::new();
    let mut inside = false;
    for line in FIXTURE.split_inclusive('\n') {
        if line.starts_with("BEGIN:VEVENT") {
            inside = true;
        }
        if inside {
            block.push_str(line);
        } else {
            out.push_str(line);
        }
        if line.starts_with("END:VEVENT") {
            inside = false;
            if !block.lines().any(&drop) {
                out.push_str(&block);
            }
            block.clear();
        }
    }
    out
}

fn event_row(state: &State, title: &str, start: &str) -> (String, Option<String>, Option<String>) {
    state
        .with(|c| {
            c.query_row(
                "SELECT end_day,missing_since,removed_at FROM calendar_events
                 WHERE title=?1 AND start_day=?2",
                [title, start],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap()
}

fn health(state: &State) -> (Option<String>, u32, Option<String>) {
    state
        .with(|c| {
            c.query_row(
                "SELECT last_success_at,consecutive_failures,last_error FROM calendars",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .unwrap()
}

const T0: &str = "2026-11-25T01:00:00.000Z";
const T1: &str = "2026-11-25T02:00:00.000Z";
const T2: &str = "2026-11-26T02:00:00.000Z";

#[tokio::test]
async fn a_fetched_calendar_is_stored_and_an_unchanged_one_changes_nothing() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(FIXTURE);
    let cancel = AtomicBool::new(false);
    let report = run(&state, &fake, T0, &cancel).await.unwrap();
    assert_eq!(report.events, 65);
    assert_eq!(report.applied.added, 65);
    assert_eq!(
        event_row(&state, "학기말성적 확인", "2025-12-30").0,
        "2026-01-02"
    );
    assert_eq!(health(&state), (Some(T0.into()), 0, None));
    assert_eq!(*fake.calls.lock().unwrap(), [ROBOTS, ACADEMIC_CALENDAR_URL]);

    let again = run(&state, &fake, T1, &cancel).await.unwrap();
    assert_eq!(again.applied, CalendarApplied::default());
    let changed = FIXTURE.replace(
        "SUMMARY:개강미사(오전10시 30분)",
        "SUMMARY:개강미사(오전 11시)",
    );
    fake.serve(&changed, "text/calendar");
    let report = run(&state, &fake, T1, &cancel).await.unwrap();
    assert_eq!((report.applied.added, report.applied.changed), (0, 1));
}

#[tokio::test]
async fn an_event_is_removed_only_when_still_absent_a_day_later() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(FIXTURE);
    let cancel = AtomicBool::new(false);
    run(&state, &fake, T0, &cancel).await.unwrap();

    fake.serve(
        &without(|line| line == "DTSTART;VALUE=DATE:20251221"),
        "text/calendar",
    );
    let first = run(&state, &fake, T0, &cancel).await.unwrap();
    assert_eq!((first.applied.missing, first.applied.removed), (1, 0));
    let soon = run(&state, &fake, T1, &cancel).await.unwrap();
    assert_eq!((soon.applied.missing, soon.applied.removed), (0, 0));
    let later = run(&state, &fake, T2, &cancel).await.unwrap();
    assert_eq!(later.applied.removed, 1);
    assert_eq!(
        event_row(&state, "종강미사(오전 11시)", "2025-12-21").2,
        Some(T2.into())
    );

    // Listed again: restored, neither new nor changed.
    fake.serve(FIXTURE, "text/calendar");
    let back = run(&state, &fake, T2, &cancel).await.unwrap();
    assert_eq!(back.applied, CalendarApplied::default());
    assert_eq!(
        event_row(&state, "종강미사(오전 11시)", "2025-12-21").2,
        None
    );
}

#[tokio::test]
async fn events_before_the_calendars_window_age_out_and_stay() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(FIXTURE);
    let cancel = AtomicBool::new(false);
    run(&state, &fake, T0, &cancel).await.unwrap();
    // The calendar now starts on 2026-01-12; every 2025 event ended before.
    fake.serve(
        &without(|line| line.starts_with("DTSTART;VALUE=DATE:2025")),
        "text/calendar",
    );
    for now in [T0, T2] {
        let report = run(&state, &fake, now, &cancel).await.unwrap();
        assert_eq!(report.events, 59);
        assert_eq!((report.applied.missing, report.applied.removed), (0, 0));
    }
    assert_eq!(
        event_row(&state, "학기말성적 확인", "2025-12-30"),
        ("2026-01-02".into(), None, None)
    );
}

#[tokio::test]
async fn a_failed_fetch_records_health_and_keeps_the_events() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new(FIXTURE);
    let cancel = AtomicBool::new(false);
    run(&state, &fake, T0, &cancel).await.unwrap();
    // A login page, a wrong calendar, a truncated one, and an empty answer.
    let truncated = &FIXTURE[..FIXTURE.find("BEGIN:VEVENT").unwrap()];
    for (body, mime) in [
        ("<html>sign in</html>", "text/html"),
        (
            &FIXTURE.replace("서강대학교 학사일정", "다른 일정")[..],
            "text/calendar",
        ),
        (truncated, "text/calendar"),
        ("", "text/calendar"),
    ] {
        fake.serve(body, mime);
        assert!(run(&state, &fake, T2, &cancel).await.is_err());
    }
    let (last_success, failures, error) = health(&state);
    assert_eq!((last_success.as_deref(), failures), (Some(T0), 4));
    assert!(error.is_some());
    let kept: i64 = state
        .with(|c| {
            c.query_row(
                "SELECT count(*) FROM calendar_events WHERE missing_since IS NULL AND removed_at IS NULL",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(kept, 65);
    fake.serve(FIXTURE, "text/calendar");
    run(&state, &fake, T2, &cancel).await.unwrap();
    assert_eq!(health(&state), (Some(T2.into()), 0, None));
}

#[tokio::test]
async fn robots_rules_are_obeyed_and_a_policy_error_stops_the_fetch() {
    let cancel = AtomicBool::new(false);
    for robots in [Ok("User-agent: *\nDisallow: /owa/\n".to_owned()), Err(403)] {
        let state = State::open_in_memory().unwrap();
        let fake = Fake {
            robots,
            ..Fake::new(FIXTURE)
        };
        assert!(run(&state, &fake, T0, &cancel).await.is_err());
        assert_eq!(*fake.calls.lock().unwrap(), [ROBOTS]);
        assert_eq!(health(&state).1, 1);
    }
}

/// Live: the real transport against Outlook, two requests (robots.txt and
/// the ICS) into an in-memory state. Needs approval (crates/collector/AGENTS.md).
#[tokio::test]
#[ignore]
async fn live_academic_calendar() {
    let client = rhythm_sources::transport::Client::new(concat!(
        "Rhythm/",
        env!("CARGO_PKG_VERSION"),
        " (local university notice archive)"
    ))
    .unwrap();
    let state = State::open_in_memory().unwrap();
    let report = run(&state, &client, T0, &AtomicBool::new(false))
        .await
        .unwrap();
    println!("{}", summary(&Ok(report)));
}
