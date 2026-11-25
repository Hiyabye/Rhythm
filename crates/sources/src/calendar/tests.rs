use super::*;

const FIXTURE: &[u8] = include_bytes!("../fixtures/academic-calendar.ics");

fn find<'a>(events: &'a [Event], title: &str, start: &str) -> &'a Event {
    events
        .iter()
        .find(|e| e.title == title && e.start_day == start)
        .unwrap_or_else(|| panic!("no {title} on {start}"))
}

#[test]
fn parses_the_published_academic_calendar() {
    let parsed = parse(FIXTURE).unwrap();
    assert_eq!(parsed.events.len(), 65);
    assert_eq!(parsed.skipped, 0);
    // One day: DTSTART 20251221, DTEND 20251222 (exclusive).
    let mass = find(&parsed.events, "종강미사(오전 11시)", "2025-12-21");
    assert_eq!(mass.end_day, "2025-12-21");
    assert!(mass.uid.starts_with("040000008200E00074C5B7101A82E008"));
    // An exclusive end in the next year and in the next month.
    let grades = find(&parsed.events, "학기말성적 확인", "2025-12-30");
    assert_eq!(grades.end_day, "2026-01-02");
    let scholarship = find(&parsed.events, "2026학년도 1학기 장학금 신청", "2025-11-01");
    assert_eq!(scholarship.end_day, "2025-11-30");
    // Escaped commas, and a trailing space in the source title.
    find(&parsed.events, "겨울방학 시작,계절수업 개강", "2025-12-22");
    let leave = find(&parsed.events, "2026학년도 2학기 휴복학 신청", "2026-07-23");
    assert_eq!(leave.end_day, "2026-08-07");
    assert!(parsed
        .events
        .windows(2)
        .all(|pair| pair[0].start_day <= pair[1].start_day));
}

/// A calendar with `events` and the expected name, CRLF-terminated.
fn calendar(events: &[&str]) -> Vec<u8> {
    let mut text =
        String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:서강대학교 학사일정\r\n");
    for event in events {
        text.push_str("BEGIN:VEVENT\r\n");
        text.push_str(&event.replace('\n', "\r\n"));
        text.push_str("\r\nEND:VEVENT\r\n");
    }
    text.push_str("END:VCALENDAR\r\n");
    text.into_bytes()
}

fn filler(count: usize) -> Vec<String> {
    (0..count)
        .map(|n| {
            format!(
                "UID:filler-{n}\nSUMMARY:행사 {n}\nDTSTART;VALUE=DATE:202603{:02}\nDTEND;VALUE=DATE:202603{:02}",
                n + 1,
                n + 2
            )
        })
        .collect()
}

fn with_filler(extra: &[&str]) -> Vec<u8> {
    let filler = filler(MIN_EVENTS);
    let mut all: Vec<&str> = filler.iter().map(String::as_str).collect();
    all.extend_from_slice(extra);
    calendar(&all)
}

#[test]
fn unfolds_lines_and_reads_times() {
    let parsed = parse(&with_filler(&[
        // A folded UID and SUMMARY, a nested alarm, an escaped newline.
        "UID:long-\n id\nSUMMARY:성적\n  입력\\n마감\nDTSTART;VALUE=DATE:20260630\nBEGIN:VALARM\nDTSTART;VALUE=DATE:20200101\nEND:VALARM",
        // A UTC time after 15:00 is the next Seoul day; a local time keeps its day.
        "UID:utc\nSUMMARY:야간\nDTSTART:20261231T160000Z\nDTEND:20261231T170000Z",
        "UID:local\nSUMMARY:오전\nDTSTART;TZID=Korea Standard Time:20260302T100000\nDTEND;TZID=Korea Standard Time:20260302T110000",
    ]))
    .unwrap();
    let folded = parsed.events.iter().find(|e| e.uid == "long-id").unwrap();
    assert_eq!(folded.title, "성적 입력 마감");
    assert_eq!(
        (folded.start_day.as_str(), folded.end_day.as_str()),
        ("2026-06-30", "2026-06-30")
    );
    let utc = parsed.events.iter().find(|e| e.uid == "utc").unwrap();
    assert_eq!(
        (utc.start_day.as_str(), utc.end_day.as_str()),
        ("2027-01-01", "2027-01-01")
    );
    let local = parsed.events.iter().find(|e| e.uid == "local").unwrap();
    assert_eq!(local.start_day, "2026-03-02");
}

#[test]
fn skips_events_it_cannot_represent() {
    let parsed = parse(&with_filler(&[
        "UID:weekly\nSUMMARY:매주\nDTSTART;VALUE=DATE:20260302\nRRULE:FREQ=WEEKLY",
        "UID:cancelled\nSUMMARY:취소\nDTSTART;VALUE=DATE:20260302\nSTATUS:CANCELLED",
        "UID:no-title\nDTSTART;VALUE=DATE:20260302",
        "UID:bad-date\nSUMMARY:날짜\nDTSTART;VALUE=DATE:20260230",
        "UID:backwards\nSUMMARY:역순\nDTSTART;VALUE=DATE:20260310\nDTEND;VALUE=DATE:20260301",
        "UID:too-long\nSUMMARY:긴 기간\nDTSTART;VALUE=DATE:20260101\nDTEND;VALUE=DATE:20270201",
        "SUMMARY:UID 없음\nDTSTART;VALUE=DATE:20260302",
    ]))
    .unwrap();
    assert_eq!(parsed.events.len(), MIN_EVENTS);
    assert_eq!(parsed.skipped, 7);
}

#[test]
fn refuses_the_wrong_or_a_broken_calendar() {
    let filler = filler(MIN_EVENTS);
    let events: Vec<&str> = filler.iter().map(String::as_str).collect();
    let renamed = String::from_utf8(calendar(&events))
        .unwrap()
        .replace("서강대학교 학사일정", "다른 일정");
    assert!(parse(renamed.as_bytes()).is_err());
    assert!(parse(&calendar(&events[..MIN_EVENTS - 1])).is_err());
    let mut repeated = events.clone();
    repeated.push(events[0]);
    assert!(parse(&calendar(&repeated)).is_err());
    let truncated = String::from_utf8(calendar(&events)).unwrap();
    let truncated = &truncated[..truncated.rfind("END:VEVENT").unwrap()];
    assert!(parse(truncated.as_bytes()).is_err());
    assert!(parse(b"<html>login</html>").is_err());
    assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
}

#[test]
fn only_the_calendar_and_its_robots_are_reachable() {
    for allowed in [
        ACADEMIC_CALENDAR_URL,
        "https://outlook.office365.com/robots.txt",
    ] {
        assert!(
            validate_url(&Url::parse(allowed).unwrap()).is_ok(),
            "{allowed}"
        );
    }
    for blocked in [
        "https://outlook.office365.com/mail/",
        "https://outlook.office365.com/owa/calendar/other/calendar.ics",
        &format!("{ACADEMIC_CALENDAR_URL}?x=1"),
        &ACADEMIC_CALENDAR_URL.replace("https://", "http://"),
        &ACADEMIC_CALENDAR_URL.replace("office365.com/", "office365.com:8443/"),
    ] {
        assert!(
            validate_url(&Url::parse(blocked).unwrap()).is_err(),
            "{blocked}"
        );
    }
}
