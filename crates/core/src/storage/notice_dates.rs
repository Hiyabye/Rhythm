//! Dates found in notice text by `archive::dates`, stored for each notice's
//! current content. Writing an article or filling its body extracts them in the
//! same transaction; `refresh_notice_dates` covers notices archived before the
//! table existed or by an older extractor version. Dates are local derived data:
//! they never leave this device and never feed ranking or notifications.
use super::{calendar::day_number, trash::NOT_TRASHED, Store};
use crate::archive::dates::{self, Kind, EXTRACTOR_VERSION, MAX_SPAN_DAYS};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// The calendar's month grid and the week pane's two months ahead.
const MAX_RANGE_DAYS: i64 = 62;
/// Bounds one calendar request even with every notice's deadlines shown.
const MAX_DATES: i64 = 2000;
/// The school calendar has about 60 events a year.
const MAX_EVENTS: i64 = 500;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NoticeDate {
    pub document_id: i64,
    pub ordinal: i64,
    pub title: String,
    pub day: String,
    pub time: Option<String>,
    /// The last day of a multi-day `date`.
    pub end_day: Option<String>,
    pub end_time: Option<String>,
    pub label: String,
    /// `deadline`, `start` or `date`.
    pub kind: String,
}
/// An academic calendar event from the feed (#319). `end_day` is inclusive.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarEvent {
    pub calendar_id: String,
    pub uid: String,
    pub title: String,
    pub start_day: String,
    pub end_day: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarNoticeDates {
    pub all_deadlines: bool,
    pub dates: Vec<NoticeDate>,
    /// Whether the 학사 일정 layer is shown (`settings.calendar_academic`).
    pub academic: bool,
    /// Events overlapping the range, empty while the layer is hidden.
    pub events: Vec<CalendarEvent>,
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Deadline => "deadline",
        Kind::Start => "start",
        Kind::Date => "date",
    }
}

/// Replaces a notice's dates with those of its current title and body.
pub(super) fn extract(connection: &Connection, document_id: i64) -> Result<(), String> {
    let (title, body, published, hash): (String, String, String, String) = connection
        .query_row(
            "SELECT title,body_text,published_at,content_hash FROM documents WHERE id=?1",
            [document_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| e.to_string())?;
    // Deleting the extraction row cascades to its dates.
    connection
        .execute(
            "DELETE FROM notice_date_extractions WHERE document_id=?1",
            [document_id],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "INSERT INTO notice_date_extractions(document_id,content_hash,version) VALUES(?1,?2,?3)",
            params![document_id, hash, EXTRACTOR_VERSION],
        )
        .map_err(|e| e.to_string())?;
    for (ordinal, found) in dates::extract_notice(&title, &body, &published)
        .iter()
        .enumerate()
    {
        connection
            .execute(
                "INSERT INTO notice_dates(document_id,ordinal,day,time,end_day,end_time,label,kind) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![document_id, ordinal as i64, found.day, found.time, found.end_day, found.end_time, found.label, kind_name(found.kind)],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Replaces a notice's dates with the ones the feed extracted from its full
/// body, which the app does not hold until the notice is opened.
pub(super) fn store_feed(
    connection: &Connection,
    document_id: i64,
    revision_hash: &str,
    version: u32,
    found: &[rhythm_feed::record::NoticeDate],
) -> Result<(), String> {
    use rhythm_feed::record::DateKind;
    connection
        .execute(
            "DELETE FROM notice_date_extractions WHERE document_id=?1",
            [document_id],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "INSERT INTO notice_date_extractions(document_id,content_hash,version) VALUES(?1,?2,?3)",
            params![document_id, revision_hash, version],
        )
        .map_err(|e| e.to_string())?;
    for (ordinal, date) in found.iter().enumerate() {
        let kind = match date.kind {
            DateKind::Deadline => Kind::Deadline,
            DateKind::Start => Kind::Start,
            DateKind::Date => Kind::Date,
        };
        connection
            .execute(
                "INSERT INTO notice_dates(document_id,ordinal,day,time,end_day,end_time,label,kind) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![document_id, ordinal as i64, date.day, date.time, date.end_day, date.end_time, date.label, kind_name(kind)],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn notice_date(row: &rusqlite::Row) -> rusqlite::Result<NoticeDate> {
    Ok(NoticeDate {
        document_id: row.get(0)?,
        ordinal: row.get(1)?,
        title: row.get(2)?,
        day: row.get(3)?,
        time: row.get(4)?,
        end_day: row.get(5)?,
        end_time: row.get(6)?,
        label: row.get(7)?,
        kind: row.get(8)?,
    })
}
const COLUMNS: &str =
    "n.document_id,n.ordinal,d.title,n.day,n.time,n.end_day,n.end_time,n.label,n.kind";

impl Store {
    /// Extracts dates for at most `limit` notices whose stored dates are missing
    /// or stale, and returns how many it processed. A feed notice not yet
    /// opened keeps the feed's dates: its text is only the excerpt. Callers repeat it with the
    /// store unlocked between batches until it returns 0.
    pub fn refresh_notice_dates(&self, limit: i64) -> Result<usize, String> {
        let stale: Vec<i64> = self
            .connection
            .prepare(
                "SELECT d.id FROM documents d
                 LEFT JOIN notice_date_extractions e ON e.document_id=d.id
                 WHERE d.body_pending=0
                   AND (e.document_id IS NULL OR e.content_hash<>d.content_hash OR e.version<>?1)
                 ORDER BY d.id DESC LIMIT ?2",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(params![EXTRACTOR_VERSION, limit], |r| r.get(0))?
                    .collect()
            })
            .map_err(|e| e.to_string())?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        for id in &stale {
            extract(&transaction, *id)?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(stale.len())
    }

    /// Dates between `from` and `to` for the calendar and week pane: every date
    /// of a saved notice, plus every notice's deadlines when the user chose so.
    /// A multi-day date that overlaps the range is included; it starts at most
    /// `MAX_SPAN_DAYS` before `from`, which keeps the day index usable.
    pub fn calendar_notice_dates(
        &self,
        from: &str,
        to: &str,
    ) -> Result<CalendarNoticeDates, String> {
        let span = day_number(from)
            .zip(day_number(to))
            .map(|(from, to)| to - from)
            .filter(|span| (0..=MAX_RANGE_DAYS).contains(span));
        if span.is_none() {
            return Err("달력 범위를 확인해 주세요.".into());
        }
        let all_deadlines = self.calendar_all_deadlines()?;
        let dates = self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM notice_dates n
                 JOIN documents d ON d.id=n.document_id
                 LEFT JOIN document_state u ON u.document_id=d.id
                 WHERE n.day BETWEEN date(?1,'-{MAX_SPAN_DAYS} days') AND ?2
                   AND COALESCE(n.end_day,n.day) >= ?1 AND {NOT_TRASHED}
                   AND (COALESCE(u.saved,0)=1 OR (?3 AND n.kind='deadline'
                        AND d.source_id IN (SELECT id FROM sources WHERE subscribed=1)))
                 ORDER BY n.day, n.time IS NOT NULL, n.time, d.published_at DESC, n.document_id, n.ordinal
                 LIMIT ?4"
            ))
            .and_then(|mut statement| {
                statement
                    .query_map(params![from, to, all_deadlines, MAX_DATES], notice_date)?
                    .collect()
            })
            .map_err(|e| e.to_string())?;
        let academic = self.calendar_academic()?;
        let events = if academic {
            self.connection
                .prepare(
                    "SELECT calendar_id,uid,title,start_day,end_day FROM calendar_events
                     WHERE start_day<=?2 AND end_day>=?1
                     ORDER BY start_day,end_day,title,uid LIMIT ?3",
                )
                .and_then(|mut statement| {
                    statement
                        .query_map(params![from, to, MAX_EVENTS], |r| {
                            Ok(CalendarEvent {
                                calendar_id: r.get(0)?,
                                uid: r.get(1)?,
                                title: r.get(2)?,
                                start_day: r.get(3)?,
                                end_day: r.get(4)?,
                            })
                        })?
                        .collect()
                })
                .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        Ok(CalendarNoticeDates {
            all_deadlines,
            dates,
            academic,
            events,
        })
    }

    pub fn calendar_academic(&self) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT calendar_academic FROM settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(true))
            .map_err(|e| e.to_string())
    }
    pub fn set_calendar_academic(&self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE settings SET calendar_academic=?1 WHERE id=1",
                [enabled],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A notice's own dates for its reader. A notice in 휴지통 has none to add.
    pub fn notice_dates(&self, document_id: i64) -> Result<Vec<NoticeDate>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM notice_dates n JOIN documents d ON d.id=n.document_id
                 WHERE n.document_id=?1 AND {NOT_TRASHED} ORDER BY n.ordinal"
            ))
            .and_then(|mut statement| statement.query_map([document_id], notice_date)?.collect())
            .map_err(|e| e.to_string())
    }

    pub fn calendar_all_deadlines(&self) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT calendar_all_deadlines FROM settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map(Option::unwrap_or_default)
            .map_err(|e| e.to_string())
    }
    pub fn set_calendar_all_deadlines(&self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE settings SET calendar_all_deadlines=?1 WHERE id=1",
                [enabled],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
