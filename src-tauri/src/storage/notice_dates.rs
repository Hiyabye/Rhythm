//! Dates found in notice text by `archive::dates`, stored for each notice's
//! current content. Writing an article or filling its body extracts them in the
//! same transaction; `refresh_notice_dates` covers notices archived before the
//! table existed or by an older extractor version. Dates are local derived data:
//! they never leave this device and never feed ranking or notifications.
use super::{calendar::day_number, trash::NOT_TRASHED, Store};
use crate::archive::dates::{self, Kind};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Bump when extraction rules change so stored dates are recomputed.
pub(crate) const EXTRACTOR_VERSION: i64 = 1;
/// The calendar's month grid and the week pane's two months ahead.
const MAX_RANGE_DAYS: i64 = 62;
/// Bounds one calendar request even with every notice's deadlines shown.
const MAX_DATES: i64 = 2000;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NoticeDate {
    pub document_id: i64,
    pub ordinal: i64,
    pub title: String,
    pub day: String,
    pub time: Option<String>,
    pub label: String,
    /// `deadline`, `start` or `date`.
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CalendarNoticeDates {
    pub all_deadlines: bool,
    pub dates: Vec<NoticeDate>,
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
    // Titles often carry the deadline (`채용 (~9/29(화)까지)`).
    for (ordinal, found) in dates::extract(&format!("{title}\n{body}"), &published)
        .iter()
        .enumerate()
    {
        connection
            .execute(
                "INSERT INTO notice_dates(document_id,ordinal,day,time,label,kind) VALUES(?1,?2,?3,?4,?5,?6)",
                params![document_id, ordinal as i64, found.day, found.time, found.label, kind_name(found.kind)],
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
        label: row.get(5)?,
        kind: row.get(6)?,
    })
}
const COLUMNS: &str = "n.document_id,n.ordinal,d.title,n.day,n.time,n.label,n.kind";

impl Store {
    /// Extracts dates for at most `limit` notices whose stored dates are missing
    /// or stale, and returns how many it processed. Callers repeat it with the
    /// store unlocked between batches until it returns 0.
    pub(crate) fn refresh_notice_dates(&self, limit: i64) -> Result<usize, String> {
        let stale: Vec<i64> = self
            .connection
            .prepare(
                "SELECT d.id FROM documents d
                 LEFT JOIN notice_date_extractions e ON e.document_id=d.id
                 WHERE e.document_id IS NULL OR e.content_hash<>d.content_hash OR e.version<>?1
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
    pub(crate) fn calendar_notice_dates(
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
                 WHERE n.day BETWEEN ?1 AND ?2 AND {NOT_TRASHED}
                   AND (COALESCE(u.saved,0)=1 OR (?3 AND n.kind='deadline'))
                 ORDER BY n.day, n.time IS NOT NULL, n.time, d.published_at DESC, n.document_id, n.ordinal
                 LIMIT ?4"
            ))
            .and_then(|mut statement| {
                statement
                    .query_map(params![from, to, all_deadlines, MAX_DATES], notice_date)?
                    .collect()
            })
            .map_err(|e| e.to_string())?;
        Ok(CalendarNoticeDates {
            all_deadlines,
            dates,
        })
    }

    /// A notice's own dates for its reader. A notice in 휴지통 has none to add.
    pub(crate) fn notice_dates(&self, document_id: i64) -> Result<Vec<NoticeDate>, String> {
        self.connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM notice_dates n JOIN documents d ON d.id=n.document_id
                 WHERE n.document_id=?1 AND {NOT_TRASHED} ORDER BY n.ordinal"
            ))
            .and_then(|mut statement| statement.query_map([document_id], notice_date)?.collect())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn calendar_all_deadlines(&self) -> Result<bool, String> {
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
    pub(crate) fn set_calendar_all_deadlines(&self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE settings SET calendar_all_deadlines=?1 WHERE id=1",
                [enabled],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
