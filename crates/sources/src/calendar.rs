//! The school's academic calendar (학사 일정, #319): the one published Outlook
//! ICS the collector fetches (`transport::Access::Calendar`) and its parser.
//! The collector publishes the events through the feed; the app never fetches
//! this URL.
use url::Url;

/// The feed's identity of this calendar (`event.upsert` `calendar_id`).
pub const ACADEMIC_CALENDAR_ID: &str = "sogang-academic-calendar";
/// The 캘린더 구독 link on `https://www.sogang.ac.kr/ko/academic-support/calendar`
/// (학사지원팀), published by the school's Outlook (user decision, #319).
pub const ACADEMIC_CALENDAR_URL: &str = "https://outlook.office365.com/owa/calendar/a6e147b5715c4625acfa62b58acd6efc@o365.sogang.ac.kr/1a77a92e19a94322b38942a04ce2416b5170855343168828714/calendar.ics";
const ROBOTS_URL: &str = "https://outlook.office365.com/robots.txt";
/// `X-WR-CALNAME` of that calendar; anything else is the wrong document.
const CALENDAR_NAME: &str = "서강대학교 학사일정";
/// Fewer events than this is a broken or truncated answer, never applied, so
/// it cannot remove the calendar (the school's has about 60 a year).
pub const MIN_EVENTS: usize = 10;
pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_EVENTS: usize = 2000;
const MAX_UID: usize = 512;
const MAX_TITLE_CHARS: usize = 200;
/// Matches the notice date rule (`dates::MAX_SPAN_DAYS`).
const MAX_SPAN_DAYS: i64 = 366;

/// One all-day event. `end_day` is inclusive and equals `start_day` for a
/// one-day event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub title: String,
    pub start_day: String,
    pub end_day: String,
}

/// The only URLs `Access::Calendar` may request: the ICS itself and that
/// host's robots.txt.
pub fn validate_url(url: &Url) -> Result<(), String> {
    if url.as_str() == ACADEMIC_CALENDAR_URL || url.as_str() == ROBOTS_URL {
        Ok(())
    } else {
        Err(crate::policy::BLOCKED.into())
    }
}

/// Parses the calendar. Fails on the wrong calendar, too few events or a
/// malformed file; skips single events it cannot represent (recurring,
/// cancelled, malformed or too long), which the caller may report.
pub fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    if bytes.len() > MAX_BYTES {
        return Err("the calendar is too large".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the calendar is not UTF-8")?;
    let lines = unfold(text);
    if lines.first().map(String::as_str) != Some("BEGIN:VCALENDAR") {
        return Err("not an iCalendar file".into());
    }
    let mut name = None;
    let mut events = Vec::new();
    let mut skipped = 0;
    let mut current: Option<Vec<(String, String)>> = None;
    let mut depth = 0usize;
    for line in &lines {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        let property = head.split(';').next().unwrap_or("").to_ascii_uppercase();
        match (property.as_str(), value) {
            ("BEGIN", "VEVENT") if current.is_none() => {
                current = Some(Vec::new());
                depth = 0;
            }
            ("BEGIN", _) if current.is_some() => depth += 1,
            ("END", "VEVENT") if current.is_some() && depth == 0 => {
                match event(&current.take().unwrap_or_default()) {
                    Some(event) => events.push(event),
                    None => skipped += 1,
                }
                if events.len() > MAX_EVENTS {
                    return Err("the calendar has too many events".into());
                }
            }
            ("END", _) if current.is_some() => depth = depth.saturating_sub(1),
            // Alarms and other nested components keep their own properties.
            _ if depth > 0 => {}
            _ if current.is_some() => {
                if let Some(properties) = current.as_mut() {
                    properties.push((head.to_owned(), value.to_owned()));
                }
            }
            ("X-WR-CALNAME", value) => name = Some(unescape(value)),
            _ => {}
        }
    }
    if current.is_some() {
        return Err("the calendar ends inside an event".into());
    }
    if name.as_deref() != Some(CALENDAR_NAME) {
        return Err(format!("unexpected calendar name {name:?}"));
    }
    let mut seen = std::collections::HashSet::new();
    if events.iter().any(|event| !seen.insert(event.uid.clone())) {
        return Err("the calendar repeats an event UID".into());
    }
    if events.len() < MIN_EVENTS {
        return Err(format!("the calendar has only {} events", events.len()));
    }
    events.sort_by(|a, b| (&a.start_day, &a.uid).cmp(&(&b.start_day, &b.uid)));
    Ok(Parsed { events, skipped })
}

#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    pub events: Vec<Event>,
    /// Events left out: recurring, cancelled, malformed or too long.
    pub skipped: usize,
}

/// RFC 5545 §3.1: a line starting with a space or tab continues the previous one.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.trim_start_matches('\u{feff}').split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match (line.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ if line.is_empty() => {}
            _ => lines.push(line.to_owned()),
        }
    }
    lines
}

fn event(properties: &[(String, String)]) -> Option<Event> {
    let get = |name: &str| {
        properties
            .iter()
            .find(|(head, _)| {
                head.split(';')
                    .next()
                    .is_some_and(|p| p.eq_ignore_ascii_case(name))
            })
            .map(|(head, value)| (head.as_str(), value.as_str()))
    };
    // A recurrence would need expanding; the school's calendar has none.
    if get("RRULE").is_some() || get("RDATE").is_some() || get("RECURRENCE-ID").is_some() {
        return None;
    }
    if get("STATUS").is_some_and(|(_, status)| status.eq_ignore_ascii_case("CANCELLED")) {
        return None;
    }
    let uid = get("UID")?.1.trim();
    if uid.is_empty() || uid.len() > MAX_UID || uid.chars().any(char::is_control) {
        return None;
    }
    let title = unescape(get("SUMMARY")?.1);
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return None;
    }
    let (start_head, start_value) = get("DTSTART")?;
    let start = day(start_head, start_value)?;
    let end = match get("DTEND") {
        // DTEND is exclusive for a date; a timed end keeps its own day.
        Some((head, value)) if is_date(head) => previous_day(day(head, value)?)?,
        Some((head, value)) => day(head, value)?,
        None => start,
    };
    let span = days(end) - days(start);
    if !(0..MAX_SPAN_DAYS).contains(&span) {
        return None;
    }
    Some(Event {
        uid: uid.to_owned(),
        title,
        start_day: iso(start),
        end_day: iso(end),
    })
}

fn is_date(head: &str) -> bool {
    head.split(';')
        .skip(1)
        .any(|param| param.eq_ignore_ascii_case("VALUE=DATE"))
}

/// The day of a `DATE` or local `DATE-TIME` value. A UTC time (`Z`) is a
/// Seoul day: the school's calendar is in Korea (UTC+9, no daylight saving).
fn day(head: &str, value: &str) -> Option<(u32, u32, u32)> {
    let value = value.trim();
    let date = (
        value.get(0..4)?.parse().ok()?,
        value.get(4..6)?.parse().ok()?,
        value.get(6..8)?.parse().ok()?,
    );
    if !value.get(0..8)?.bytes().all(|b| b.is_ascii_digit()) || !valid(date) {
        return None;
    }
    if is_date(head) {
        return (value.len() == 8).then_some(date);
    }
    let time = value.strip_prefix(value.get(0..8)?)?.strip_prefix('T')?;
    let (clock, utc) = match time.strip_suffix('Z') {
        Some(clock) => (clock, true),
        None => (time, false),
    };
    if clock.len() != 6 || !clock.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour: u32 = clock[0..2].parse().ok()?;
    if utc && hour + 9 >= 24 {
        return next_day(date);
    }
    Some(date)
}

fn valid((year, month, day): (u32, u32, u32)) -> bool {
    (1900..=2200).contains(&year)
        && (1..=12).contains(&month)
        && (1..=month_days(year, month)).contains(&day)
}

fn month_days(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn previous_day((year, month, day): (u32, u32, u32)) -> Option<(u32, u32, u32)> {
    let date = match (month, day) {
        (1, 1) => (year - 1, 12, 31),
        (_, 1) => (year, month - 1, month_days(year, month - 1)),
        _ => (year, month, day - 1),
    };
    valid(date).then_some(date)
}

fn next_day((year, month, day): (u32, u32, u32)) -> Option<(u32, u32, u32)> {
    let date = if day < month_days(year, month) {
        (year, month, day + 1)
    } else if month < 12 {
        (year, month + 1, 1)
    } else {
        (year + 1, 1, 1)
    };
    valid(date).then_some(date)
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn days((year, month, day): (u32, u32, u32)) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year - 719_468
}

fn iso((year, month, day): (u32, u32, u32)) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// RFC 5545 TEXT escapes: `\\`, `\;`, `\,` and `\n`/`\N`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests;
