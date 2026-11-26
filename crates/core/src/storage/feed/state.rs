//! What the feed sync remembers between runs (migrations 038 and 041): the
//! feed's base URL, the last two accepted manifests' `generated_at`, board
//! health, and how the last attempt ended.
use super::super::Store;
use chrono::{DateTime, Utc};
use rhythm_feed::Manifest;
use rusqlite::params;
use serde::Serialize;

/// A manifest older than this shows the stale banner (user decision,
/// 2026-11-18); the collector publishes every 30 minutes.
const STALE_AFTER: chrono::Duration = chrono::Duration::hours(3);
/// Consecutive collector failures before a board is listed as failing (user
/// decision, 2026-11-18): about 1.5 hours of failed runs.
pub const FAILING_AFTER: u32 = 3;

#[derive(Debug, Default, PartialEq)]
pub struct FeedState {
    pub base: Option<String>,
    pub cursor: Option<u64>,
}

/// How a sync attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedResult {
    Ok,
    Offline,
    Failed,
    /// The feed is in a format this version cannot read.
    UpdateRequired,
}

impl FeedResult {
    fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Offline => "offline",
            Self::Failed => "failed",
            Self::UpdateRequired => "update_required",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeedBoard {
    pub source_id: String,
    pub name: String,
    pub last_success_at: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeedStatus {
    /// Filled in by the sync controller, not the store.
    pub running: bool,
    /// When the collector published the last accepted manifest.
    pub generated_at: Option<String>,
    /// The last sync that ended without an error.
    pub checked_at: Option<String>,
    pub stale: bool,
    /// `ok`, `offline`, `failed` or `update_required`; `None` before the first attempt.
    pub result: Option<String>,
    pub error: Option<String>,
    /// Subscribed boards the collector keeps failing on.
    pub failing_boards: Vec<FeedBoard>,
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

impl Store {
    pub fn feed_state(&self) -> Result<FeedState, String> {
        self.connection
            .query_row("SELECT base,cursor FROM feed_state WHERE id=1", [], |row| {
                Ok(FeedState {
                    base: row.get(0)?,
                    cursor: row.get::<_, Option<i64>>(1)?.map(|c| c as u64),
                })
            })
            .map_err(|e| e.to_string())
    }

    /// Accepts a verified manifest fetched from `base` (FEED.md step 2): one
    /// older than the last accepted is refused, so an old signed manifest
    /// cannot roll the mirror back. Stores its time, base and board health.
    pub fn feed_accept(&self, manifest: &Manifest, base: &str) -> Result<(), String> {
        let generated = parse_time(&manifest.generated_at)
            .ok_or("공지 피드의 생성 시각이 올바르지 않습니다.")?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let previous: Option<String> = transaction
            .query_row("SELECT generated_at FROM feed_state WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if previous
            .as_deref()
            .and_then(parse_time)
            .is_some_and(|previous| generated < previous)
        {
            return Err("받은 공지 피드가 이미 반영한 것보다 오래되었습니다.".into());
        }
        // A new manifest moves the last one into `previous_generated_at`, the
        // line between what the previous sync brought and what this one did.
        transaction
            .execute(
                "UPDATE feed_state SET previous_generated_at=CASE WHEN generated_at IS ?1
                   THEN previous_generated_at ELSE generated_at END,generated_at=?1,base=?2
                 WHERE id=1",
                params![manifest.generated_at, base],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute("DELETE FROM feed_boards", [])
            .map_err(|e| e.to_string())?;
        for board in &manifest.boards {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO feed_boards(source_id,last_success_at,consecutive_failures,history_complete)
                     VALUES(?1,?2,?3,?4)",
                    params![
                        board.source_id,
                        board.last_success_at,
                        board.consecutive_failures,
                        board.history_complete
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn feed_record(&self, result: FeedResult, error: Option<&str>) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE feed_state SET result=?1,error=?2,
                   checked_at=CASE WHEN ?1='ok' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE checked_at END
                 WHERE id=1",
                params![result.name(), error],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn feed_status(&self, now: DateTime<Utc>) -> Result<FeedStatus, String> {
        let (generated_at, checked_at, result, error) = self
            .connection
            .query_row(
                "SELECT generated_at,checked_at,result,error FROM feed_state WHERE id=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let stale = generated_at
            .as_deref()
            .and_then(parse_time)
            .is_some_and(|generated| now - generated > STALE_AFTER);
        let mut statement = self
            .connection
            .prepare(concat!(
                "SELECT b.source_id,",
                source_label!(),
                ",b.last_success_at,b.consecutive_failures
                 FROM feed_boards b JOIN sources s ON s.id=b.source_id
                 WHERE s.subscribed=1 AND b.consecutive_failures>=?1
                 ORDER BY 2,b.source_id"
            ))
            .map_err(|e| e.to_string())?;
        let failing_boards = statement
            .query_map([FAILING_AFTER], |r| {
                Ok(FeedBoard {
                    source_id: r.get(0)?,
                    name: r.get(1)?,
                    last_success_at: r.get(2)?,
                    consecutive_failures: r.get(3)?,
                })
            })
            .and_then(Iterator::collect)
            .map_err(|e| e.to_string())?;
        Ok(FeedStatus {
            running: false,
            generated_at,
            checked_at,
            stale,
            result,
            error,
            failing_boards,
        })
    }
}
