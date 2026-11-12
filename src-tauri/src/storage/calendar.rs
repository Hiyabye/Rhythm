//! Todos the user makes on the calendar. `day` and `time` are local wall-clock
//! values; they never leave this device. A todo made from a notice keeps a
//! link to it (`document_id`), shown only while the notice is outside 휴지통.
use super::Store;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

const MAX_TITLE: usize = 200;
const MAX_MEMO: usize = 2000;
const MAX_TODOS: i64 = 5000;
/// A month grid shows at most six weeks.
const MAX_RANGE_DAYS: i64 = 62;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalendarTodo {
    pub id: i64,
    pub title: String,
    pub day: String,
    pub time: Option<String>,
    pub memo: String,
    pub done: bool,
    pub document_id: Option<i64>,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TodoInput {
    pub title: String,
    pub day: String,
    pub time: Option<String>,
    #[serde(default)]
    pub memo: String,
    /// Read only when the todo is created; editing keeps the link it has.
    #[serde(default)]
    pub document_id: Option<i64>,
}

/// Days since 1970-01-01 of a valid `YYYY-MM-DD`, or `None`.
fn day_number(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = &value[range];
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if year < 1970 || !(1..=length).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}
fn valid_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && value[..2].parse::<u8>().is_ok_and(|hour| hour < 24)
        && value[3..].parse::<u8>().is_ok_and(|minute| minute < 60)
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
}
fn validated(input: &TodoInput) -> Result<(String, Option<&str>, String), String> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(format!(
            "할 일 제목은 1자에서 {MAX_TITLE}자 사이로 입력해 주세요."
        ));
    }
    if day_number(&input.day).is_none() {
        return Err("할 일 날짜를 확인해 주세요.".into());
    }
    let time = input.time.as_deref().filter(|time| !time.is_empty());
    if time.is_some_and(|time| !valid_time(time)) {
        return Err("할 일 시간을 확인해 주세요.".into());
    }
    let memo = input.memo.trim();
    if memo.chars().count() > MAX_MEMO {
        return Err(format!("메모는 {MAX_MEMO}자 이하로 입력해 주세요."));
    }
    Ok((title.to_owned(), time, memo.to_owned()))
}
fn todo(row: &Row) -> rusqlite::Result<CalendarTodo> {
    Ok(CalendarTodo {
        id: row.get(0)?,
        title: row.get(1)?,
        day: row.get(2)?,
        time: row.get(3)?,
        memo: row.get(4)?,
        done: row.get(5)?,
        document_id: row.get(6)?,
    })
}
// A trashed notice hides its link; restoring the notice brings it back.
const COLUMNS: &str = "id,title,day,time,memo,done,
    CASE WHEN EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=calendar_todos.document_id)
      THEN NULL ELSE document_id END";
const NOT_FOUND: &str = "할 일을 찾을 수 없습니다. 이미 삭제되었을 수 있습니다.";

impl Store {
    /// With the 캘린더 extension off its todos are kept but cannot change.
    fn calendar_writable(&self) -> Result<(), String> {
        if self.enabled(crate::extensions::CALENDAR)? {
            Ok(())
        } else {
            Err("캘린더 확장 기능이 꺼져 있습니다.".into())
        }
    }
    /// Todos dated `from` through `to`, untimed first within a day.
    pub fn calendar_todos(&self, from: &str, to: &str) -> Result<Vec<CalendarTodo>, String> {
        let span = day_number(from)
            .zip(day_number(to))
            .map(|(from, to)| to - from)
            .filter(|span| (0..=MAX_RANGE_DAYS).contains(span));
        if span.is_none() {
            return Err("달력 범위를 확인해 주세요.".into());
        }
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM calendar_todos WHERE day BETWEEN ?1 AND ?2
                 ORDER BY day, time IS NOT NULL, time, id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([from, to], todo)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    fn calendar_todo(&self, id: i64) -> Result<CalendarTodo, String> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM calendar_todos WHERE id=?1"),
                [id],
                todo,
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| NOT_FOUND.into())
    }
    pub fn calendar_create_todo(&self, input: &TodoInput) -> Result<CalendarTodo, String> {
        self.calendar_writable()?;
        let (title, time, memo) = validated(input)?;
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM calendar_todos", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count >= MAX_TODOS {
            return Err(format!(
                "할 일은 {MAX_TODOS}개까지 저장할 수 있습니다. 지난 할 일을 지워 주세요."
            ));
        }
        if let Some(document) = input.document_id {
            let exists = self
                .connection
                .query_row(
                    &format!(
                        "SELECT 1 FROM documents d WHERE d.id=?1 AND {}",
                        super::trash::NOT_TRASHED
                    ),
                    [document],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if exists.is_none() {
                return Err(
                    "연결할 공지를 찾을 수 없습니다. 휴지통으로 옮겼거나 삭제했을 수 있습니다."
                        .into(),
                );
            }
        }
        self.connection
            .execute(
                "INSERT INTO calendar_todos(title,day,time,memo,document_id) VALUES(?1,?2,?3,?4,?5)",
                params![title, input.day, time, memo, input.document_id],
            )
            .map_err(|e| e.to_string())?;
        self.calendar_todo(self.connection.last_insert_rowid())
    }
    pub fn calendar_update_todo(&self, id: i64, input: &TodoInput) -> Result<CalendarTodo, String> {
        self.calendar_writable()?;
        let (title, time, memo) = validated(input)?;
        let changed = self
            .connection
            .execute(
                "UPDATE calendar_todos SET title=?2,day=?3,time=?4,memo=?5,
                   updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                params![id, title, input.day, time, memo],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(NOT_FOUND.into());
        }
        self.calendar_todo(id)
    }
    pub fn calendar_set_todo_done(&self, id: i64, done: bool) -> Result<CalendarTodo, String> {
        self.calendar_writable()?;
        let changed = self
            .connection
            .execute(
                "UPDATE calendar_todos SET done=?2,
                   updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                params![id, done],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err(NOT_FOUND.into());
        }
        self.calendar_todo(id)
    }
    /// Deleting a todo that is already gone succeeds, so a repeated click is harmless.
    pub fn calendar_delete_todo(&self, id: i64) -> Result<(), String> {
        self.calendar_writable()?;
        self.connection
            .execute("DELETE FROM calendar_todos WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &std::path::Path) -> Store {
        let store = Store::open(path).unwrap();
        store
            .set_enabled(crate::extensions::CALENDAR, true)
            .unwrap();
        store
    }
    fn input(title: &str, day: &str, time: Option<&str>) -> TodoInput {
        TodoInput {
            title: title.into(),
            day: day.into(),
            time: time.map(Into::into),
            memo: String::new(),
            document_id: None,
        }
    }

    #[test]
    fn todos_persist_in_day_order_and_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rhythm.db");
        {
            let store = open(&path);
            store
                .calendar_create_todo(&input("저녁 약속", "2026-11-10", Some("18:30")))
                .unwrap();
            let morning = store
                .calendar_create_todo(&input("  보고서 제출  ", "2026-11-10", Some("09:00")))
                .unwrap();
            assert_eq!(morning.title, "보고서 제출");
            store
                .calendar_create_todo(&input("하루 종일", "2026-11-10", None))
                .unwrap();
            store
                .calendar_create_todo(&input("다음 달", "2026-12-01", Some("")))
                .unwrap();
            store.calendar_set_todo_done(morning.id, true).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let todos = store.calendar_todos("2026-11-01", "2026-11-30").unwrap();
        assert_eq!(
            todos
                .iter()
                .map(|t| (t.title.as_str(), t.time.as_deref(), t.done))
                .collect::<Vec<_>>(),
            [
                ("하루 종일", None, false),
                ("보고서 제출", Some("09:00"), true),
                ("저녁 약속", Some("18:30"), false),
            ]
        );
        let december = store.calendar_todos("2026-12-01", "2026-12-01").unwrap();
        assert_eq!(december[0].time, None, "an empty time is stored as untimed");
    }

    #[test]
    fn update_and_delete_change_only_the_named_todo() {
        let directory = tempfile::tempdir().unwrap();
        let store = open(&directory.path().join("rhythm.db"));
        let first = store
            .calendar_create_todo(&input("첫째", "2026-11-10", None))
            .unwrap();
        let second = store
            .calendar_create_todo(&input("둘째", "2026-11-11", None))
            .unwrap();
        let moved = store
            .calendar_update_todo(
                first.id,
                &TodoInput {
                    memo: "도서관".into(),
                    ..input("첫째 (수정)", "2026-11-12", Some("13:05"))
                },
            )
            .unwrap();
        assert_eq!(
            (
                moved.day.as_str(),
                moved.time.as_deref(),
                moved.memo.as_str()
            ),
            ("2026-11-12", Some("13:05"), "도서관")
        );
        store.calendar_delete_todo(second.id).unwrap();
        store.calendar_delete_todo(second.id).unwrap();
        assert_eq!(
            store.calendar_todos("2026-11-01", "2026-11-30").unwrap(),
            [moved]
        );
        assert!(store
            .calendar_update_todo(second.id, &input("x", "2026-11-11", None))
            .is_err());
        assert!(store.calendar_set_todo_done(second.id, true).is_err());
    }

    #[test]
    fn invalid_input_and_ranges_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let store = open(&directory.path().join("rhythm.db"));
        for bad in [
            input("   ", "2026-11-10", None),
            input(&"가".repeat(MAX_TITLE + 1), "2026-11-10", None),
            input("x", "2026-02-29", None),
            input("x", "2026-13-01", None),
            input("x", "2026-1-01", None),
            input("x", "+026-11-10", None),
            input("x", "2026-11-10", Some("24:00")),
            input("x", "2026-11-10", Some("9:00")),
            input("x", "2026-11-10", Some("09:60")),
            TodoInput {
                memo: "a".repeat(MAX_MEMO + 1),
                ..input("x", "2026-11-10", None)
            },
        ] {
            assert!(store.calendar_create_todo(&bad).is_err(), "{bad:?}");
        }
        assert!(store
            .calendar_create_todo(&input(&"가".repeat(MAX_TITLE), "2028-02-29", Some("23:59")))
            .is_ok());
        assert!(store.calendar_todos("2026-11-30", "2026-11-01").is_err());
        assert!(store.calendar_todos("2026-11-01", "2027-01-03").is_err());
        assert!(store.calendar_todos("2026-11-01", "bad").is_err());
        assert!(store.calendar_todos("2026-10-26", "2026-12-06").is_ok());
    }

    #[test]
    fn disabled_extension_keeps_todos_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rhythm.db");
        let store = Store::open(&path).unwrap();
        assert!(
            !store.enabled(crate::extensions::CALENDAR).unwrap(),
            "the calendar starts off"
        );
        assert!(store
            .calendar_create_todo(&input("x", "2026-11-10", None))
            .is_err());
        store
            .set_enabled(crate::extensions::CALENDAR, true)
            .unwrap();
        let kept = store
            .calendar_create_todo(&input("x", "2026-11-10", None))
            .unwrap();
        store
            .set_enabled(crate::extensions::CALENDAR, false)
            .unwrap();
        assert!(store.calendar_set_todo_done(kept.id, true).is_err());
        assert!(store
            .calendar_update_todo(kept.id, &input("y", "2026-11-10", None))
            .is_err());
        assert!(store.calendar_delete_todo(kept.id).is_err());
        assert_eq!(
            store.calendar_todos("2026-11-10", "2026-11-10").unwrap(),
            [kept]
        );
    }

    #[test]
    fn day_numbers_count_real_days() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2000-03-01"), Some(11017));
        assert_eq!(
            day_number("2027-01-01").unwrap() - day_number("2026-12-31").unwrap(),
            1
        );
    }
}
