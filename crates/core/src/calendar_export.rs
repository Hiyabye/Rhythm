//! iCalendar (`.ics`) export of the calendar (#191) for third-party calendar
//! apps: every todo not done yet and the open eClass deadlines the calendar
//! shows. Notice dates are left out; they are found by heuristics and can be
//! wrong. The file goes where the user chose and leaves Rhythm's control.
use crate::storage::{
    calendar::CalendarTodo, cybercampus::CybercampusTask, with_store as locked, SharedStore,
};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::Serialize;
use std::{fmt::Write as _, path::Path};

/// UIDs stay the same across exports, so a calendar that re-imports the file
/// can update its events instead of adding copies.
const UID_DOMAIN: &str = "io.github.hiyabye.rhythm";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CalendarExport {
    pub todos: usize,
    pub deadlines: usize,
}

/// Writes the `.ics` file to `destination`. The store lock is released
/// before the file is written; the caller holds an admission lease.
pub async fn export(store: &SharedStore, destination: &Path) -> Result<CalendarExport, String> {
    if destination.as_os_str().is_empty() {
        return Err("내보낼 파일 위치를 선택해 주세요.".into());
    }
    let (todos, tasks) = locked(store, |s| {
        let tasks = if s.cybercampus_deadlines_shown()? {
            s.cybercampus_tasks()?
        } else {
            Vec::new()
        };
        Ok((s.calendar_open_todos()?, tasks))
    })?;
    let (text, summary) = ics(&todos, &tasks, Utc::now());
    if summary.todos + summary.deadlines == 0 {
        return Err("내보낼 할 일이나 마감이 없습니다.".into());
    }
    tokio::fs::write(destination, text)
        .await
        .map_err(|_| "캘린더 파일을 저장하지 못했습니다. 저장 위치와 권한을 확인해 주세요.")?;
    Ok(summary)
}

fn kst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).expect("valid offset")
}
fn date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}
/// An eClass time: `YYYY-MM-DD` (the whole day) or `YYYY-MM-DDTHH:MM`, in Korea.
enum Due {
    Day(NaiveDate),
    At(DateTime<Utc>),
}
fn due(value: &str) -> Option<Due> {
    if value.len() == 10 {
        return date(value).map(Due::Day);
    }
    let local = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
    Some(Due::At(
        kst().from_local_datetime(&local).single()?.to_utc(),
    ))
}
/// The last moment a deadline still counts: a dated-only deadline lasts the day.
fn last_moment(value: &str) -> Option<DateTime<Utc>> {
    Some(match due(value)? {
        Due::Day(day) => kst()
            .from_local_datetime(&day.and_hms_opt(23, 59, 59)?)
            .single()?
            .to_utc(),
        Due::At(at) => at,
    })
}
/// The calendar's rule for an open deadline: unsubmitted, with a due time, and
/// that time (or its late window) not yet past.
fn open(task: &CybercampusTask, now: DateTime<Utc>) -> bool {
    let Some(due_at) = &task.due_at else {
        return false;
    };
    task.submitted_at.is_none()
        && last_moment(task.late_due_at.as_ref().unwrap_or(due_at)).is_some_and(|end| end >= now)
}

/// RFC 5545 TEXT escaping.
fn text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ';' => escaped.push_str("\\;"),
            ',' => escaped.push_str("\\,"),
            '\n' => escaped.push_str("\\n"),
            '\r' => {}
            other => escaped.push(other),
        }
    }
    escaped
}
/// Appends one content line, folded at 75 octets without splitting a UTF-8
/// character, with CRLF endings.
fn line(out: &mut String, content: &str) {
    let mut width = 0;
    for character in content.chars() {
        let size = character.len_utf8();
        if width + size > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(character);
        width += size;
    }
    out.push_str("\r\n");
}
fn day_value(day: NaiveDate) -> String {
    day.format("%Y%m%d").to_string()
}
fn all_day(out: &mut String, day: NaiveDate) {
    line(out, &format!("DTSTART;VALUE=DATE:{}", day_value(day)));
    line(
        out,
        &format!("DTEND;VALUE=DATE:{}", day_value(day + Duration::days(1))),
    );
}

/// The `.ics` text for the todos and the open deadlines among `tasks`.
fn ics(
    todos: &[CalendarTodo],
    tasks: &[CybercampusTask],
    now: DateTime<Utc>,
) -> (String, CalendarExport) {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut out = String::new();
    line(&mut out, "BEGIN:VCALENDAR");
    line(&mut out, "VERSION:2.0");
    line(
        &mut out,
        &format!("PRODID:-//Rhythm//Rhythm {}//KO", env!("CARGO_PKG_VERSION")),
    );
    line(&mut out, "CALSCALE:GREGORIAN");
    line(&mut out, "METHOD:PUBLISH");
    line(&mut out, "X-WR-CALNAME:Rhythm");
    let mut summary = CalendarExport {
        todos: 0,
        deadlines: 0,
    };
    for todo in todos.iter().filter(|todo| !todo.done) {
        // Stored values are validated on write; anything else is skipped rather than guessed.
        let Some(day) = date(&todo.day) else { continue };
        let time = match &todo.time {
            Some(time) => match NaiveTime::parse_from_str(time, "%H:%M") {
                Ok(time) => Some(time),
                Err(_) => continue,
            },
            None => None,
        };
        line(&mut out, "BEGIN:VEVENT");
        line(&mut out, &format!("UID:todo-{}@{UID_DOMAIN}", todo.id));
        line(&mut out, &format!("DTSTAMP:{stamp}"));
        match time {
            // Todos keep the local wall-clock time, so the event is floating:
            // the importing calendar shows it at the same clock time.
            Some(time) => line(
                &mut out,
                &format!("DTSTART:{}", day.and_time(time).format("%Y%m%dT%H%M%S")),
            ),
            None => all_day(&mut out, day),
        }
        line(&mut out, &format!("SUMMARY:{}", text(&todo.title)));
        if !todo.memo.is_empty() {
            line(&mut out, &format!("DESCRIPTION:{}", text(&todo.memo)));
        }
        line(&mut out, "END:VEVENT");
        summary.todos += 1;
    }
    for task in tasks.iter().filter(|task| open(task, now)) {
        let Some(when) = task.due_at.as_deref().and_then(due) else {
            continue;
        };
        line(&mut out, "BEGIN:VEVENT");
        line(
            &mut out,
            &format!(
                "UID:eclass-{}-{}-{}@{UID_DOMAIN}",
                text(&task.course_key),
                text(&task.category),
                text(&task.item_id)
            ),
        );
        line(&mut out, &format!("DTSTAMP:{stamp}"));
        match when {
            Due::Day(day) => all_day(&mut out, day),
            Due::At(at) => line(
                &mut out,
                &format!("DTSTART:{}", at.format("%Y%m%dT%H%M%SZ")),
            ),
        }
        line(
            &mut out,
            &format!("SUMMARY:{}", text(&format!("{} 마감", task.title))),
        );
        let mut description = format!("{} · {}", task.course_name, task.label);
        if let Some(late) = &task.late_due_at {
            let _ = write!(description, "\n지각 제출 마감: {}", late.replace('T', " "));
        }
        line(&mut out, &format!("DESCRIPTION:{}", text(&description)));
        line(&mut out, "END:VEVENT");
        summary.deadlines += 1;
    }
    line(&mut out, "END:VCALENDAR");
    (out, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: i64, day: &str, time: Option<&str>, title: &str) -> CalendarTodo {
        CalendarTodo {
            id,
            title: title.into(),
            day: day.into(),
            time: time.map(Into::into),
            memo: String::new(),
            done: false,
            document_id: None,
            reminder_minutes: None,
        }
    }
    fn task(item: &str, due_at: Option<&str>) -> CybercampusTask {
        CybercampusTask {
            course_key: "A1".into(),
            course_name: "자료구조실습".into(),
            category: "report".into(),
            item_id: item.into(),
            label: "과제".into(),
            title: format!("실습 과제 {item}"),
            week: None,
            opens_at: None,
            due_at: due_at.map(Into::into),
            late_due_at: None,
            status: None,
            submitted_at: None,
            pending: false,
            updated_at: "2026-10-20T00:00:00Z".into(),
        }
    }
    fn now() -> DateTime<Utc> {
        "2026-11-16T03:04:05Z".parse().unwrap()
    }
    /// Unfolded content lines, as a reader sees them.
    fn lines(text: &str) -> Vec<String> {
        assert!(text.ends_with("\r\n"));
        text.replace("\r\n ", "")
            .trim_end_matches("\r\n")
            .split("\r\n")
            .map(Into::into)
            .collect()
    }

    #[test]
    fn todos_become_all_day_or_floating_events() {
        let mut memo = todo(7, "2026-11-10", Some("09:30"), "보고서 제출");
        memo.memo = "학과 사무실, 3층; 서류\n두 장".into();
        let mut done = todo(9, "2026-11-11", None, "끝낸 일");
        done.done = true;
        let (text, summary) = ics(
            &[todo(3, "2026-12-31", None, "장학금 서류"), memo, done],
            &[],
            now(),
        );
        assert_eq!(
            summary,
            CalendarExport {
                todos: 2,
                deadlines: 0
            }
        );
        assert_eq!(
            lines(&text),
            [
                "BEGIN:VCALENDAR",
                "VERSION:2.0",
                &format!("PRODID:-//Rhythm//Rhythm {}//KO", env!("CARGO_PKG_VERSION")),
                "CALSCALE:GREGORIAN",
                "METHOD:PUBLISH",
                "X-WR-CALNAME:Rhythm",
                "BEGIN:VEVENT",
                "UID:todo-3@io.github.hiyabye.rhythm",
                "DTSTAMP:20261116T030405Z",
                "DTSTART;VALUE=DATE:20261231",
                "DTEND;VALUE=DATE:20270101",
                "SUMMARY:장학금 서류",
                "END:VEVENT",
                "BEGIN:VEVENT",
                "UID:todo-7@io.github.hiyabye.rhythm",
                "DTSTAMP:20261116T030405Z",
                "DTSTART:20261110T093000",
                "SUMMARY:보고서 제출",
                "DESCRIPTION:학과 사무실\\, 3층\\; 서류\\n두 장",
                "END:VEVENT",
                "END:VCALENDAR",
            ]
        );
    }

    #[test]
    fn only_open_deadlines_are_exported_in_utc() {
        let mut late = task("2", Some("2026-11-15T23:59"));
        late.late_due_at = Some("2026-11-17T12:00".into());
        let mut submitted = task("3", Some("2026-11-20T23:59"));
        submitted.submitted_at = Some("2026-11-14T10:00".into());
        let tasks = [
            task("1", Some("2026-11-20T09:00")),
            late,
            submitted,
            task("4", Some("2026-11-15T23:59")),
            task("5", None),
            // A dated-only deadline lasts its whole day in Korea (until 14:59:59 UTC).
            task("6", Some("2026-11-16")),
            task("7", Some("2026-11-15")),
        ];
        let (text, summary) = ics(&[], &tasks, now());
        assert_eq!(
            summary,
            CalendarExport {
                todos: 0,
                deadlines: 3
            }
        );
        let lines = lines(&text);
        let uids: Vec<_> = lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.starts_with("UID:"))
            .collect();
        assert_eq!(
            uids,
            [
                "UID:eclass-A1-report-1@io.github.hiyabye.rhythm",
                "UID:eclass-A1-report-2@io.github.hiyabye.rhythm",
                "UID:eclass-A1-report-6@io.github.hiyabye.rhythm",
            ]
        );
        for expected in [
            "DTSTART:20261120T000000Z",
            "SUMMARY:실습 과제 1 마감",
            "DESCRIPTION:자료구조실습 · 과제",
            "DTSTART:20261115T145900Z",
            "DESCRIPTION:자료구조실습 · 과제\\n지각 제출 마감: 2026-11-17 12:00",
            "DTSTART;VALUE=DATE:20261116",
            "DTEND;VALUE=DATE:20261117",
        ] {
            assert!(lines.iter().any(|l| l == expected), "{expected}");
        }
        let end_of_day: DateTime<Utc> = "2026-11-16T15:00:00Z".parse().unwrap();
        assert!(!open(&task("6", Some("2026-11-16")), end_of_day));
    }

    #[test]
    fn long_lines_fold_at_75_octets_on_character_boundaries() {
        let title = "가".repeat(60);
        let (text, _) = ics(&[todo(1, "2026-11-10", None, &title)], &[], now());
        for physical in text.split("\r\n") {
            assert!(physical.len() <= 75, "{physical:?}");
        }
        assert!(lines(&text).contains(&format!("SUMMARY:{title}")));
    }
}
