//! Calendar events and health (migration 008, #319). A fetched calendar is
//! the whole current list: new and changed events are stored, and an event it
//! no longer lists is removed only when a fetch at least `MISSING_CONFIRM`
//! after the first one still lacks it, the deletion rule notices follow. An
//! event that ended before the fetched calendar's earliest event aged out of
//! the published window and stays.
use super::{State, MISSING_CONFIRM};
use rhythm_sources::calendar::Event;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

/// What one successful fetch changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub added: usize,
    pub changed: usize,
    /// Absent for the first time.
    pub missing: usize,
    pub removed: usize,
}

impl State {
    /// Applies a successfully fetched calendar and records the success.
    pub fn calendar_apply(
        &self,
        calendar_id: &str,
        events: &[Event],
        now: &str,
    ) -> Result<Applied, String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            transaction.execute(
                "INSERT INTO calendars(calendar_id,last_success_at) VALUES(?1,?2)
                 ON CONFLICT(calendar_id) DO UPDATE SET last_success_at=excluded.last_success_at,
                   consecutive_failures=0,last_error=NULL",
                params![calendar_id, now],
            )?;
            let mut applied = Applied::default();
            for event in events {
                let stored: Option<(String, String, String, bool)> = transaction
                    .query_row(
                        "SELECT title,start_day,end_day,
                           missing_since IS NOT NULL OR removed_at IS NOT NULL
                         FROM calendar_events WHERE calendar_id=?1 AND uid=?2",
                        params![calendar_id, event.uid],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                match stored {
                    None => {
                        transaction.execute(
                            "INSERT INTO calendar_events(calendar_id,uid,title,start_day,end_day,
                               first_seen_at,updated_at)
                             VALUES(?1,?2,?3,?4,?5,?6,?6)",
                            params![
                                calendar_id,
                                event.uid,
                                event.title,
                                event.start_day,
                                event.end_day,
                                now
                            ],
                        )?;
                        applied.added += 1;
                    }
                    Some((title, start, end, absent)) => {
                        let same =
                            title == event.title && start == event.start_day && end == event.end_day;
                        if !same || absent {
                            transaction.execute(
                                "UPDATE calendar_events SET title=?3,start_day=?4,end_day=?5,
                                   updated_at=CASE WHEN ?7 THEN updated_at ELSE ?6 END,
                                   missing_since=NULL,removed_at=NULL
                                 WHERE calendar_id=?1 AND uid=?2",
                                params![
                                    calendar_id,
                                    event.uid,
                                    event.title,
                                    event.start_day,
                                    event.end_day,
                                    now,
                                    same
                                ],
                            )?;
                            applied.changed += usize::from(!same);
                        }
                    }
                }
            }
            let listed: HashSet<&str> = events.iter().map(|e| e.uid.as_str()).collect();
            let window_start = events.iter().map(|e| e.start_day.as_str()).min();
            let mut absent = Vec::new();
            {
                let mut statement = transaction.prepare(
                    "SELECT uid,missing_since IS NOT NULL,
                       COALESCE(missing_since<=strftime('%Y-%m-%dT%H:%M:%fZ',?3,?4),0)
                     FROM calendar_events
                     WHERE calendar_id=?1 AND removed_at IS NULL AND end_day>=?2",
                )?;
                let mut rows = statement.query(params![
                    calendar_id,
                    window_start.unwrap_or("9999-12-31"),
                    now,
                    MISSING_CONFIRM
                ])?;
                while let Some(row) = rows.next()? {
                    let uid: String = row.get(0)?;
                    if !listed.contains(uid.as_str()) {
                        absent.push((uid, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?));
                    }
                }
            }
            for (uid, reported, confirmed) in absent {
                if !reported {
                    transaction.execute(
                        "UPDATE calendar_events SET missing_since=?3 WHERE calendar_id=?1 AND uid=?2",
                        params![calendar_id, uid, now],
                    )?;
                    applied.missing += 1;
                } else if confirmed {
                    transaction.execute(
                        "UPDATE calendar_events SET removed_at=?3 WHERE calendar_id=?1 AND uid=?2",
                        params![calendar_id, uid, now],
                    )?;
                    applied.removed += 1;
                }
            }
            transaction.commit()?;
            Ok(applied)
        })
    }

    /// Records a failed fetch; the stored events stay as they are.
    pub fn calendar_failed(&self, calendar_id: &str, error: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "INSERT INTO calendars(calendar_id,consecutive_failures,last_error) VALUES(?1,1,?2)
                 ON CONFLICT(calendar_id) DO UPDATE SET
                   consecutive_failures=consecutive_failures+1,last_error=excluded.last_error",
                params![calendar_id, error],
            )
        })
        .map(drop)
    }
}
