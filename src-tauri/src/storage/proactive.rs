use super::Store;
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    Connection, OptionalExtension, Row, ToSql,
};
use serde::{Deserialize, Serialize};

// Proactive state is deliberately opt-in. Every default here keeps Rhythm in
// the foreground: no autostart, no hidden window, no background collection and
// no notifications. Notice content is never copied; events reference
// documents/document_revisions and every method releases its transaction
// before returning.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
const MIN_SCHEDULER_INTERVAL: u32 = 15;
const MAX_SCHEDULER_INTERVAL: u32 = 720;
const MIN_SCHEDULER_RETRY: u32 = 5;
const MAX_SCHEDULER_RETRY: u32 = 120;
const MIN_TASK_INTERVAL: u32 = 5;
const MAX_TASK_INTERVAL: u32 = 1440;
const DEFAULT_QUIET_START: u32 = 22 * 60;
const DEFAULT_QUIET_END: u32 = 7 * 60;
const MAX_DELIVERY_ATTEMPTS: u32 = 3;
const DELIVERY_CLAIM_LEASE_MINUTES: u32 = 5;
const MAX_FAILURE_DELAY_MINUTES: i64 = 240;
const HISTORY_LIMIT: i64 = 200;
const MAX_HISTORY_QUERY: usize = 200;
const MAX_DUE_EVENTS: usize = 100;
const MAX_CANDIDATES: usize = 200;
const MAX_RATE_WINDOW_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_DUE_TASKS: i64 = 50;
const MAX_TASKS_QUERY: i64 = 500;
const MAX_REASON_CHARS: usize = 200;
const MAX_ERROR_CHARS: usize = 500;
const MAX_TASK_ID_CHARS: usize = 128;

const SETTINGS_COLUMNS: &str = "close_to_background,launch_at_login,start_hidden,\
scheduler_enabled,scheduler_paused,scheduler_interval_minutes,scheduler_retry_minutes,\
notifications_enabled,notification_sensitivity,quiet_hours_enabled,\
quiet_hours_start_minutes,quiet_hours_end_minutes,updated_at";
const TASK_COLUMNS: &str = "task_id,interval_minutes,enabled,next_run_at,last_run_at,\
last_outcome,consecutive_failures,last_error,created_at,updated_at";
const EVENT_COLUMNS: &str = "id,document_id,revision_id,event_type,state,attempt_count,\
claimed_at,last_attempt_at,deferred_until,delivered_at,error,reason,created_at,updated_at";
const HISTORY_COLUMNS: &str = "e.id,e.document_id,e.revision_id,e.event_type,e.state,\
e.attempt_count,e.claimed_at,e.last_attempt_at,e.deferred_until,e.delivered_at,e.error,\
e.reason,e.created_at,e.updated_at,r.title,d.category,d.source_id";
const TERMINAL_STATES_SQL: &str = "'attempted','delivered','suppressed','failed'";

macro_rules! sql_text_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $name {
            pub(crate) fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
            fn from_db(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                Self::from_db(value.as_str()?).ok_or(FromSqlError::InvalidType)
            }
        }
        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::Borrowed(ValueRef::Text(
                    self.as_str().as_bytes(),
                )))
            }
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSensitivity {
    Important,
    Interested,
    All,
}

sql_text_enum!(NotificationSensitivity {
    Important => "important",
    Interested => "interested",
    All => "all",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactiveEventKind {
    New,
    Revision,
}

sql_text_enum!(ProactiveEventKind {
    New => "new",
    Revision => "revision",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactiveEventState {
    Pending,
    Claimed,
    Attempted,
    Deferred,
    Delivered,
    Suppressed,
    Failed,
}

impl ProactiveEventState {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Attempted | Self::Delivered | Self::Suppressed | Self::Failed
        )
    }
}

sql_text_enum!(ProactiveEventState {
    Pending => "pending",
    Claimed => "claimed",
    Attempted => "attempted",
    Deferred => "deferred",
    Delivered => "delivered",
    Suppressed => "suppressed",
    Failed => "failed",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProactiveOutcome {
    Success,
    Failure,
    Skipped,
    Cancelled,
}

sql_text_enum!(ProactiveOutcome {
    Success => "success",
    Failure => "failure",
    Skipped => "skipped",
    Cancelled => "cancelled",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerOutcome {
    Success,
    Partial,
    Skipped,
    Failure,
    Cancelled,
}

sql_text_enum!(SchedulerOutcome {
    Success => "success",
    Partial => "partial",
    Skipped => "skipped",
    Failure => "failure",
    Cancelled => "cancelled",
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    pub enabled: bool,
    /// Local wall-clock `HH:MM`, 24-hour. Stored canonically after validation.
    pub start: String,
    pub end: String,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: format_clock(DEFAULT_QUIET_START as u16),
            end: format_clock(DEFAULT_QUIET_END as u16),
        }
    }
}

impl QuietHours {
    /// Ordered window in local minutes since midnight, or `None` when disabled.
    pub fn window(&self) -> Option<(u16, u16)> {
        if !self.enabled {
            return None;
        }
        let start = parse_clock(&self.start)?;
        let end = parse_clock(&self.end)?;
        (start != end).then_some((start, end))
    }

    /// Whether a local wall-clock minute falls inside the (possibly overnight)
    /// quiet window. The end boundary is exclusive.
    pub fn contains(&self, minutes_since_midnight: u16) -> bool {
        if minutes_since_midnight >= 24 * 60 {
            return false;
        }
        let Some((start, end)) = self.window() else {
            return false;
        };
        if start < end {
            (start..end).contains(&minutes_since_midnight)
        } else {
            minutes_since_midnight >= start || minutes_since_midnight < end
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveSettings {
    pub close_to_background: bool,
    pub launch_at_login: bool,
    pub start_hidden: bool,
    pub scheduler_enabled: bool,
    pub scheduler_paused: bool,
    pub scheduler_interval_minutes: u32,
    pub scheduler_retry_minutes: u32,
    pub notifications_enabled: bool,
    pub notification_sensitivity: NotificationSensitivity,
    pub quiet_hours: QuietHours,
    /// Server-controlled UTC timestamp; ignored on save.
    pub updated_at: String,
}

impl Default for ProactiveSettings {
    fn default() -> Self {
        Self {
            close_to_background: false,
            launch_at_login: false,
            start_hidden: false,
            scheduler_enabled: false,
            scheduler_paused: false,
            scheduler_interval_minutes: 0,
            scheduler_retry_minutes: 15,
            notifications_enabled: false,
            notification_sensitivity: NotificationSensitivity::Important,
            quiet_hours: QuietHours::default(),
            updated_at: String::new(),
        }
    }
}

/// Stable notification identity. One document revision yields at most one
/// event per kind, forever (the dedupe ledger outlives history retention).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveEventKey {
    pub document_id: i64,
    pub revision_id: i64,
    pub event_type: ProactiveEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveEvent {
    pub id: i64,
    pub document_id: i64,
    pub revision_id: i64,
    pub event_type: ProactiveEventKind,
    pub state: ProactiveEventState,
    /// Delivery attempts started so far; also the fencing token returned from
    /// claim calls and required by completion calls.
    pub attempt_count: u32,
    pub claimed_at: Option<String>,
    pub last_attempt_at: Option<String>,
    pub deferred_until: Option<String>,
    pub delivered_at: Option<String>,
    pub error: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProactiveClaim {
    /// New row recorded and claimed as attempt 1.
    Claimed { event_id: i64, attempt_count: u32 },
    /// Revision already exists in the archive at/below the notification
    /// baseline. Nothing was recorded; old archives are never replayed.
    Baseline {
        revision_id: i64,
        baseline_revision_id: i64,
    },
    /// Historical backfill notice of kind New. Nothing was recorded or
    /// notified. Historical revisions may still notify once above baseline.
    Historical { document_id: i64 },
    /// Identity already recorded (possibly its history row was cleared).
    Duplicate { previous_event_id: Option<i64> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveCandidate {
    pub document_id: i64,
    pub revision_id: i64,
    pub event_type: ProactiveEventKind,
    pub title: String,
    pub source_id: String,
    pub source_name: String,
    pub category: Option<String>,
    pub published_at: String,
    pub collected_at: String,
    pub historical: bool,
    pub source_active: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProactiveHistoryItem {
    #[serde(flatten)]
    pub event: ProactiveEvent,
    pub title: String,
    pub category: Option<String>,
    pub source_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveTaskInput {
    pub task_id: String,
    pub interval_minutes: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveTask {
    pub task_id: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_outcome: Option<ProactiveOutcome>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProactiveSchedulerStatus {
    pub enabled: bool,
    pub paused: bool,
    pub interval_minutes: u32,
    pub retry_minutes: u32,
    pub notifications_enabled: bool,
    pub notification_sensitivity: NotificationSensitivity,
    pub quiet_hours: QuietHours,
    /// Outstanding events: pending, deferred and claimed-but-unfinished.
    pub pending_events: u32,
    pub due_tasks: u32,
    pub next_task_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_outcome: Option<SchedulerOutcome>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

fn parse_clock(value: &str) -> Option<u16> {
    let (hour, minute) = value.trim().split_once(':')?;
    if hour.is_empty()
        || hour.len() > 2
        || minute.len() != 2
        || !hour.chars().all(|c| c.is_ascii_digit())
        || !minute.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let hour: u16 = hour.parse().ok()?;
    let minute: u16 = minute.parse().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

fn format_clock(minutes: u16) -> String {
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn valid_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_TASK_ID_CHARS
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn settings_row(row: &Row<'_>) -> rusqlite::Result<ProactiveSettings> {
    Ok(ProactiveSettings {
        close_to_background: row.get(0)?,
        launch_at_login: row.get(1)?,
        start_hidden: row.get(2)?,
        scheduler_enabled: row.get(3)?,
        scheduler_paused: row.get(4)?,
        scheduler_interval_minutes: row.get::<_, i64>(5)? as u32,
        scheduler_retry_minutes: row.get::<_, i64>(6)? as u32,
        notifications_enabled: row.get(7)?,
        notification_sensitivity: row.get(8)?,
        quiet_hours: QuietHours {
            enabled: row.get(9)?,
            start: format_clock(row.get::<_, i64>(10)? as u16),
            end: format_clock(row.get::<_, i64>(11)? as u16),
        },
        updated_at: row.get(12)?,
    })
}

fn read_settings(connection: &Connection) -> Result<ProactiveSettings, String> {
    connection
        .query_row(
            &format!("SELECT {SETTINGS_COLUMNS} FROM proactive_settings WHERE id=1"),
            [],
            settings_row,
        )
        .map_err(|e| e.to_string())
}

fn task_row(row: &Row<'_>) -> rusqlite::Result<ProactiveTask> {
    Ok(ProactiveTask {
        task_id: row.get(0)?,
        interval_minutes: row.get::<_, i64>(1)? as u32,
        enabled: row.get(2)?,
        next_run_at: row.get(3)?,
        last_run_at: row.get(4)?,
        last_outcome: row.get(5)?,
        consecutive_failures: row.get::<_, i64>(6)? as u32,
        last_error: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn event_row_at(row: &Row<'_>, base: usize) -> rusqlite::Result<ProactiveEvent> {
    Ok(ProactiveEvent {
        id: row.get(base)?,
        document_id: row.get(base + 1)?,
        revision_id: row.get(base + 2)?,
        event_type: row.get(base + 3)?,
        state: row.get(base + 4)?,
        attempt_count: row.get::<_, i64>(base + 5)? as u32,
        claimed_at: row.get(base + 6)?,
        last_attempt_at: row.get(base + 7)?,
        deferred_until: row.get(base + 8)?,
        delivered_at: row.get(base + 9)?,
        error: row.get(base + 10)?,
        reason: row.get(base + 11)?,
        created_at: row.get(base + 12)?,
        updated_at: row.get(base + 13)?,
    })
}

fn event_row(row: &Row<'_>) -> rusqlite::Result<ProactiveEvent> {
    event_row_at(row, 0)
}

fn candidate_row(row: &Row<'_>) -> rusqlite::Result<ProactiveCandidate> {
    Ok(ProactiveCandidate {
        document_id: row.get(0)?,
        revision_id: row.get(1)?,
        event_type: row.get(2)?,
        title: row.get(3)?,
        source_id: row.get(4)?,
        source_name: row.get(5)?,
        category: row.get(6)?,
        published_at: row.get(7)?,
        collected_at: row.get(8)?,
        historical: row.get(9)?,
        source_active: row.get(10)?,
    })
}

fn event_by_id(connection: &Connection, id: i64) -> Result<ProactiveEvent, String> {
    match connection
        .query_row(
            &format!("SELECT {EVENT_COLUMNS} FROM proactive_events WHERE id=?1"),
            [id],
            event_row,
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        Some(event) => Ok(event),
        None => Err("알림 기록을 찾을 수 없습니다.".into()),
    }
}

fn task_by_id(connection: &Connection, task_id: &str) -> Result<Option<ProactiveTask>, String> {
    connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM proactive_tasks WHERE task_id=?1"),
            [task_id],
            task_row,
        )
        .optional()
        .map_err(|e| e.to_string())
}

/// Keeps terminal history bounded. Dedupe lives in proactive_dedupe, so this
/// never makes an already-notified revision eligible again.
fn prune_history(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            &format!(
                "DELETE FROM proactive_events
                 WHERE state IN ({TERMINAL_STATES_SQL})
                   AND id NOT IN (
                     SELECT id FROM proactive_events
                     WHERE state IN ({TERMINAL_STATES_SQL})
                     ORDER BY updated_at DESC,id DESC LIMIT ?1
                   )"
            ),
            [HISTORY_LIMIT],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn advance_baseline(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "UPDATE proactive_baseline
             SET baseline_revision_id=MAX(
                 baseline_revision_id,
                 COALESCE((SELECT MAX(id) FROM document_revisions),0)
             )
             WHERE id=1",
            [],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

impl Store {
    pub fn proactive_settings(&self) -> Result<ProactiveSettings, String> {
        read_settings(&self.connection)
    }

    /// Full validated replacement. The first enable of either the scheduler or
    /// notifications also advances the notification baseline in the same
    /// transaction, so notices archived before opt-in are never replayed.
    pub fn proactive_save_settings(
        &self,
        settings: &ProactiveSettings,
    ) -> Result<ProactiveSettings, String> {
        if (settings.scheduler_interval_minutes != 0
            && !(MIN_SCHEDULER_INTERVAL..=MAX_SCHEDULER_INTERVAL)
                .contains(&settings.scheduler_interval_minutes))
            || !(MIN_SCHEDULER_RETRY..=MAX_SCHEDULER_RETRY)
                .contains(&settings.scheduler_retry_minutes)
        {
            return Err("스케줄 간격을 확인해 주세요.".into());
        }
        let start = parse_clock(&settings.quiet_hours.start)
            .ok_or_else(|| "조용한 시간 시작을 확인해 주세요.".to_string())?;
        let end = parse_clock(&settings.quiet_hours.end)
            .ok_or_else(|| "조용한 시간 종료를 확인해 주세요.".to_string())?;
        if settings.quiet_hours.enabled && start == end {
            return Err("조용한 시간의 시작과 종료가 같을 수 없습니다.".into());
        }
        let start = settings.quiet_hours.start.trim();
        let end = settings.quiet_hours.end.trim();
        let start = parse_clock(start).unwrap_or(DEFAULT_QUIET_START as u16);
        let end = parse_clock(end).unwrap_or(DEFAULT_QUIET_END as u16);
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let (
            was_scheduler_enabled,
            was_notifications_enabled,
            old_quiet_enabled,
            old_quiet_start,
            old_quiet_end,
        ): (bool, bool, bool, i64, i64) = transaction
            .query_row(
                "SELECT scheduler_enabled,notifications_enabled,quiet_hours_enabled,
                        quiet_hours_start_minutes,quiet_hours_end_minutes
                 FROM proactive_settings WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let quiet_hours_changed = (old_quiet_enabled, old_quiet_start, old_quiet_end)
            != (
                settings.quiet_hours.enabled,
                i64::from(start),
                i64::from(end),
            );
        transaction
            .execute(
                &format!(
                    "UPDATE proactive_settings SET
                     close_to_background=?1,launch_at_login=?2,start_hidden=?3,
                     scheduler_enabled=?4,scheduler_paused=?5,
                     scheduler_interval_minutes=?6,scheduler_retry_minutes=?7,
                     notifications_enabled=?8,notification_sensitivity=?9,
                     quiet_hours_enabled=?10,quiet_hours_start_minutes=?11,
                     quiet_hours_end_minutes=?12,updated_at={NOW}
                     WHERE id=1"
                ),
                params![
                    settings.close_to_background,
                    settings.launch_at_login,
                    settings.start_hidden,
                    settings.scheduler_enabled,
                    settings.scheduler_paused,
                    settings.scheduler_interval_minutes as i64,
                    settings.scheduler_retry_minutes as i64,
                    settings.notifications_enabled,
                    settings.notification_sensitivity,
                    settings.quiet_hours.enabled,
                    start as i64,
                    end as i64,
                ],
            )
            .map_err(|e| e.to_string())?;
        if quiet_hours_changed {
            transaction
                .execute(
                    "UPDATE proactive_events SET deferred_until=NULL,updated_at=strftime(
                         '%Y-%m-%dT%H:%M:%fZ','now'
                     )
                     WHERE state='deferred'
                       AND reason='조용한 시간에는 알림을 보내지 않습니다.'",
                    [],
                )
                .map_err(|e| e.to_string())?;
        }
        if (!was_scheduler_enabled && settings.scheduler_enabled)
            || (!was_notifications_enabled && settings.notifications_enabled)
        {
            advance_baseline(&transaction)?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        read_settings(&self.connection)
    }

    #[cfg(test)]
    pub fn proactive_baseline(&self) -> Result<i64, String> {
        self.connection
            .query_row(
                "SELECT baseline_revision_id FROM proactive_baseline WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }

    /// Current local wall-clock minute since midnight, evaluated by SQLite so
    /// quiet-hours checks use the machine's local timezone. Pair with
    /// `QuietHours::contains`.
    pub fn proactive_local_minutes(&self) -> Result<u16, String> {
        let minutes: i64 = self
            .connection
            .query_row(
                "SELECT CAST(strftime('%H','now','localtime') AS INTEGER)*60
                      + CAST(strftime('%M','now','localtime') AS INTEGER)",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        u16::try_from(minutes).map_err(|_| "현재 시각을 확인할 수 없습니다.".into())
    }

    /// Test helper for advancing the baseline outside the settings opt-in path.
    #[cfg(test)]
    pub fn proactive_establish_baseline(&self) -> Result<i64, String> {
        advance_baseline(&self.connection)?;
        self.proactive_baseline()
    }

    /// Atomically records an eligible identity as pending. Delivery attempts
    /// begin only when `proactive_claim_due_event` hands it to the caller.
    pub fn proactive_claim_event(&self, key: &ProactiveEventKey) -> Result<ProactiveClaim, String> {
        if key.document_id <= 0 || key.revision_id <= 0 {
            return Err("알림 대상을 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let historical: Option<bool> = transaction
            .query_row(
                "SELECT historical FROM documents WHERE id=?1",
                [key.document_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(historical) = historical else {
            return Err("공지를 찾을 수 없습니다.".into());
        };
        let revision: Option<i64> = transaction
            .query_row(
                "SELECT id FROM document_revisions WHERE id=?1 AND document_id=?2",
                params![key.revision_id, key.document_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if revision.is_none() {
            return Err("공지 수정본을 찾을 수 없습니다.".into());
        }
        let baseline: i64 = transaction
            .query_row(
                "SELECT baseline_revision_id FROM proactive_baseline WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if key.revision_id <= baseline {
            return Ok(ProactiveClaim::Baseline {
                revision_id: key.revision_id,
                baseline_revision_id: baseline,
            });
        }
        if historical && key.event_type == ProactiveEventKind::New {
            return Ok(ProactiveClaim::Historical {
                document_id: key.document_id,
            });
        }
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO proactive_dedupe(document_id,revision_id,event_type)
                 VALUES(?1,?2,?3)",
                params![key.document_id, key.revision_id, key.event_type],
            )
            .map_err(|e| e.to_string())?;
        if inserted == 0 {
            let previous_event_id: Option<i64> = transaction
                .query_row(
                    "SELECT id FROM proactive_events
                     WHERE document_id=?1 AND revision_id=?2 AND event_type=?3",
                    params![key.document_id, key.revision_id, key.event_type],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            return Ok(ProactiveClaim::Duplicate { previous_event_id });
        }
        transaction
            .execute(
                "INSERT INTO proactive_events(
                     document_id,revision_id,event_type,state,attempt_count
                 ) VALUES(?1,?2,?3,'pending',0)",
                params![key.document_id, key.revision_id, key.event_type],
            )
            .map_err(|e| e.to_string())?;
        let event_id = transaction.last_insert_rowid();
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(ProactiveClaim::Claimed {
            event_id,
            attempt_count: 0,
        })
    }
    #[cfg(test)]
    pub fn proactive_event(&self, id: i64) -> Result<ProactiveEvent, String> {
        event_by_id(&self.connection, id)
    }

    /// Bounded discovery of notification-worthy revisions, oldest first.
    /// Excludes revisions at/below the baseline, historical document creation
    /// (New), unsubscribed or extension-disabled sources and identities already
    /// in the dedupe ledger; revisions whose title and body match the immediate
    /// predecessor are meta-only and skipped. Historical post-baseline revisions
    /// are eligible.
    pub fn proactive_candidates(&self, limit: usize) -> Result<Vec<ProactiveCandidate>, String> {
        let limit = limit.clamp(1, MAX_CANDIDATES) as i64;
        let mut statement = self
            .connection
            .prepare(
                "WITH baseline(value) AS (
                   SELECT baseline_revision_id FROM proactive_baseline WHERE id=1
                 ),
                 first_revision(document_id,revision_id) AS (
                   SELECT document_id,MIN(id) FROM document_revisions GROUP BY document_id
                 ),
                 revision_previous(
                   document_id,revision_id,prev_title,prev_body_html,prev_body_text
                 ) AS (
                   SELECT r.document_id,r.id,p.title,p.body_html,p.body_text
                   FROM document_revisions r
                   LEFT JOIN document_revisions p ON p.id=(
                     SELECT MAX(prev.id) FROM document_revisions prev
                     WHERE prev.document_id=r.document_id AND prev.id<r.id
                   )
                 ),
                 candidates(
                   document_id,revision_id,event_type,title,source_id,source_name,
                   category,published_at,collected_at,historical,source_active
                 ) AS (
                   SELECT r.document_id,r.id,
                     CASE WHEN f.revision_id=r.id THEN 'new' ELSE 'revision' END,
                    r.title,d.source_id,s.name,d.category,d.published_at,
                    r.collected_at,d.historical,
                    CASE WHEN s.supported=1 AND s.subscribed=1 AND (
                      NOT EXISTS(SELECT 1 FROM source_extensions se WHERE se.source_id=s.id)
                      OR EXISTS(
                        SELECT 1 FROM source_extensions se
                        JOIN extension_preferences ep ON ep.extension_id=se.extension_id
                        WHERE se.source_id=s.id AND ep.enabled=1
                      )
                    ) THEN 1 ELSE 0 END
                   FROM document_revisions r
                   JOIN documents d ON d.id=r.document_id
                   JOIN sources s ON s.id=d.source_id
                   JOIN baseline b ON r.id>b.value
                   JOIN first_revision f ON f.document_id=r.document_id
                   JOIN revision_previous rp
                     ON rp.document_id=r.document_id AND rp.revision_id=r.id
                   WHERE NOT (d.historical=1 AND f.revision_id=r.id)
                     AND (
                       COALESCE(r.title<>rp.prev_title,1)
                       OR COALESCE(r.body_html<>rp.prev_body_html,1)
                       OR COALESCE(r.body_text<>rp.prev_body_text,1)
                     )
                 )
                 SELECT c.document_id,c.revision_id,c.event_type,c.title,c.source_id,
                        c.source_name,c.category,c.published_at,c.collected_at,c.historical,
                        c.source_active
                 FROM candidates c
                 WHERE NOT EXISTS(
                   SELECT 1 FROM proactive_dedupe dd
                   WHERE dd.document_id=c.document_id AND dd.revision_id=c.revision_id
                     AND dd.event_type=c.event_type
                 )
                 ORDER BY c.revision_id LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([limit], candidate_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    /// Title/source metadata for one recorded event, so a restart can render
    /// the notification without re-querying the archive itself.
    pub fn proactive_event_candidate(&self, id: i64) -> Result<ProactiveCandidate, String> {
        match self
            .connection
            .query_row(
                "SELECT e.document_id,e.revision_id,e.event_type,r.title,d.source_id,
                        s.name,d.category,d.published_at,r.collected_at,d.historical,
                        CASE WHEN s.supported=1 AND s.subscribed=1 AND (
                          NOT EXISTS(SELECT 1 FROM source_extensions se WHERE se.source_id=s.id)
                          OR EXISTS(
                            SELECT 1 FROM source_extensions se
                            JOIN extension_preferences ep ON ep.extension_id=se.extension_id
                            WHERE se.source_id=s.id AND ep.enabled=1
                          )
                        ) THEN 1 ELSE 0 END
                 FROM proactive_events e
                 JOIN documents d ON d.id=e.document_id
                 JOIN sources s ON s.id=d.source_id
                 JOIN document_revisions r
                   ON r.id=e.revision_id AND r.document_id=e.document_id
                 WHERE e.id=?1",
                [id],
                candidate_row,
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            Some(candidate) => Ok(candidate),
            None => Err("알림 기록을 찾을 수 없습니다.".into()),
        }
    }

    /// Recovery for crashes after the final allowed attempt: stale claimed rows
    /// at the attempt limit can no longer be retried, so they become terminal
    /// `failed` records with an unknown-outcome error. Idempotent; call before
    /// due-event sweeps and at startup. Stale claims below the limit remain
    /// retryable through `proactive_due_events`.
    pub fn proactive_finalize_expired_claims(&self) -> Result<u32, String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='failed',
                     error='알림 전달 결과를 확인하지 못했습니다.',
                     deferred_until=NULL,updated_at={NOW}
                     WHERE state='claimed'
                       AND attempt_count>=?1
                       AND claimed_at IS NOT NULL
                       AND claimed_at<=strftime(
                         '%Y-%m-%dT%H:%M:%fZ','now','-'||?2||' minutes'
                       )"
                ),
                params![
                    MAX_DELIVERY_ATTEMPTS as i64,
                    DELIVERY_CLAIM_LEASE_MINUTES as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed > 0 {
            prune_history(&transaction)?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(changed as u32)
    }

    /// Distinct events whose most recent delivery attempt started within the
    /// window (persisted, restart-safe; a retried event counts once). Use this
    /// before claiming a new delivery to enforce a notification rate limit.
    pub fn proactive_recent_delivery_attempts(&self, window_seconds: u64) -> Result<u32, String> {
        if window_seconds == 0 || window_seconds > MAX_RATE_WINDOW_SECONDS {
            return Err("알림 속도 제한 기간을 확인해 주세요.".into());
        }
        let count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM proactive_events
                 WHERE last_attempt_at IS NOT NULL
                   AND last_attempt_at>=strftime(
                     '%Y-%m-%dT%H:%M:%fZ','now','-'||?1||' seconds'
                   )",
                [window_seconds as i64],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        u32::try_from(count).map_err(|_| "알림 시도를 확인할 수 없습니다.".into())
    }

    /// Pending/deferred events whose deferral elapsed, plus claimed attempts
    /// whose lease expired (crash recovery). Ordered by due time, then id.
    pub fn proactive_due_events(&self, limit: usize) -> Result<Vec<ProactiveEvent>, String> {
        let limit = limit.clamp(1, MAX_DUE_EVENTS) as i64;
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM proactive_events
                 WHERE attempt_count<?3
                   AND (
                     (state IN ('pending','deferred')
                       AND (deferred_until IS NULL OR deferred_until<={NOW}))
                     OR (state='claimed' AND claimed_at IS NOT NULL
                       AND claimed_at<=strftime('%Y-%m-%dT%H:%M:%fZ','now','-'||?1||' minutes'))
                   )
                 ORDER BY COALESCE(deferred_until,claimed_at,created_at),id
                 LIMIT ?2"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(
                params![
                    DELIVERY_CLAIM_LEASE_MINUTES as i64,
                    limit,
                    MAX_DELIVERY_ATTEMPTS as i64
                ],
                event_row,
            )
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    /// Starts the next delivery attempt for a due event, returning the row
    /// whose `attempt_count` fences all completion calls.
    pub fn proactive_claim_due_event(&self, id: i64) -> Result<ProactiveEvent, String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let event = event_by_id(&transaction, id)?;
        if event.attempt_count >= MAX_DELIVERY_ATTEMPTS {
            return Err("알림 전달 시도 횟수를 초과했습니다.".into());
        }
        let now: String = transaction
            .query_row(&format!("SELECT {NOW}"), [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let lease_cutoff: String = transaction
            .query_row(
                "SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now','-'||?1||' minutes')",
                [DELIVERY_CLAIM_LEASE_MINUTES as i64],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let available = match event.state {
            ProactiveEventState::Pending | ProactiveEventState::Deferred => event
                .deferred_until
                .as_deref()
                .is_none_or(|until| until <= now.as_str()),
            ProactiveEventState::Claimed => event
                .claimed_at
                .as_deref()
                .is_some_and(|claimed| claimed <= lease_cutoff.as_str()),
            ProactiveEventState::Attempted
            | ProactiveEventState::Delivered
            | ProactiveEventState::Suppressed
            | ProactiveEventState::Failed => false,
        };
        if !available {
            return Err("아직 전달할 알림이 아닙니다.".into());
        }
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='claimed',attempt_count=attempt_count+1,
                     claimed_at={NOW},last_attempt_at={NOW},
                     deferred_until=NULL,error=NULL,updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state=?3"
                ),
                params![id, event.attempt_count as i64, event.state],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Records that a notification was handed to the OS but no receipt was
    /// observed. Terminal and honest: use this instead of `mark_delivered`
    /// when the platform cannot confirm display.
    pub fn proactive_mark_attempted(
        &self,
        id: i64,
        attempt: u32,
    ) -> Result<ProactiveEvent, String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='attempted',delivered_at=NULL,deferred_until=NULL,
                     error=NULL,updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state='claimed'"
                ),
                params![id, attempt as i64],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 전달 상태가 이미 변경되었습니다.".into());
        }
        prune_history(&transaction)?;
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Records a confirmed display. Also promotes an `attempted` event when
    /// the platform confirms the receipt later.
    pub fn proactive_mark_delivered(
        &self,
        id: i64,
        attempt: u32,
    ) -> Result<ProactiveEvent, String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='delivered',delivered_at={NOW},deferred_until=NULL,
                     error=NULL,updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state IN ('claimed','attempted')"
                ),
                params![id, attempt as i64],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 전달 상태가 이미 변경되었습니다.".into());
        }
        prune_history(&transaction)?;
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Marks an event as suppressed. A claimed event requires its fencing
    /// attempt; pending/deferred events may omit it.
    pub fn proactive_suppress_event(
        &self,
        id: i64,
        attempt: Option<u32>,
        reason: Option<&str>,
    ) -> Result<ProactiveEvent, String> {
        let reason = reason.map(str::trim).filter(|value| !value.is_empty());
        if reason.is_some_and(|value| value.chars().count() > MAX_REASON_CHARS) {
            return Err("알림 억제 사유가 너무 깁니다.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let event = event_by_id(&transaction, id)?;
        if event.state.is_terminal() {
            return Err("이미 종료된 알림입니다.".into());
        }
        if event.state == ProactiveEventState::Claimed {
            if attempt != Some(event.attempt_count) {
                return Err("알림 상태가 이미 변경되었습니다.".into());
            }
        } else if attempt.is_some_and(|attempt| attempt != event.attempt_count) {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='suppressed',reason=?3,error=NULL,deferred_until=NULL,
                     updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state=?4"
                ),
                params![id, event.attempt_count as i64, reason, event.state],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        prune_history(&transaction)?;
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Postpones a claimed delivery without consuming a failure attempt.
    pub fn proactive_defer_event(
        &self,
        id: i64,
        attempt: u32,
        minutes: u32,
        reason: Option<&str>,
    ) -> Result<ProactiveEvent, String> {
        let reason = reason.map(str::trim).filter(|value| !value.is_empty());
        if reason.is_some_and(|value| value.chars().count() > MAX_REASON_CHARS) {
            return Err("알림 연기 사유가 너무 깁니다.".into());
        }
        if !(1..=1440).contains(&minutes) {
            return Err("연기 시간을 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='deferred',
                     deferred_until=strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||?3||' minutes'),
                     reason=?4,error=NULL,updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state='claimed'"
                ),
                params![id, attempt as i64, minutes as i64, reason],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }
    /// Defers an event that is not currently claimed without creating an
    /// attempt. The attempt value fences retries after prior real attempts.
    pub fn proactive_defer_pending_event(
        &self,
        id: i64,
        attempt: u32,
        minutes: u32,
        reason: Option<&str>,
    ) -> Result<ProactiveEvent, String> {
        let reason = reason.map(str::trim).filter(|value| !value.is_empty());
        if reason.is_some_and(|value| value.chars().count() > MAX_REASON_CHARS) {
            return Err("알림 연기 사유가 너무 깁니다.".into());
        }
        if !(1..=1440).contains(&minutes) {
            return Err("연기 시간을 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                &format!(
                    "UPDATE proactive_events SET
                     state='deferred',
                     deferred_until=strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||?3||' minutes'),
                     reason=?4,error=NULL,updated_at={NOW}
                     WHERE id=?1 AND attempt_count=?2 AND state IN ('pending','deferred')"
                ),
                params![id, attempt as i64, minutes as i64, reason],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Records a failed delivery attempt. The third failure is terminal;
    /// earlier failures retry with capped exponential backoff from
    /// `scheduler_retry_minutes`.
    pub fn proactive_fail_delivery(
        &self,
        id: i64,
        attempt: u32,
        error: &str,
    ) -> Result<ProactiveEvent, String> {
        let error = error.trim();
        if error.chars().count() > MAX_ERROR_CHARS {
            return Err("알림 오류 내용이 너무 깁니다.".into());
        }
        let error = if error.is_empty() {
            "알림을 전달하지 못했습니다."
        } else {
            error
        };
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let event = event_by_id(&transaction, id)?;
        if event.state != ProactiveEventState::Claimed || event.attempt_count != attempt {
            return Err("알림 상태가 이미 변경되었습니다.".into());
        }
        let retry_minutes: i64 = transaction
            .query_row(
                "SELECT scheduler_retry_minutes FROM proactive_settings WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if event.attempt_count >= MAX_DELIVERY_ATTEMPTS {
            let changed = transaction
                .execute(
                    &format!(
                        "UPDATE proactive_events SET
                         state='failed',error=?2,deferred_until=NULL,updated_at={NOW}
                         WHERE id=?1 AND attempt_count=?3 AND state='claimed'"
                    ),
                    params![id, error, attempt as i64],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("알림 상태가 이미 변경되었습니다.".into());
            }
        } else {
            let exponent = i64::from(event.attempt_count.saturating_sub(1).min(4));
            let delay = (retry_minutes * (1_i64 << exponent)).clamp(1, MAX_FAILURE_DELAY_MINUTES);
            let changed = transaction
                .execute(
                    &format!(
                        "UPDATE proactive_events SET
                         state='pending',error=?2,
                         deferred_until=strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||?4||' minutes'),
                         updated_at={NOW}
                         WHERE id=?1 AND attempt_count=?3 AND state='claimed'"
                    ),
                    params![id, error, attempt as i64, delay],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("알림 상태가 이미 변경되었습니다.".into());
            }
        }
        prune_history(&transaction)?;
        let event = event_by_id(&transaction, id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(event)
    }

    /// Newest terminal events (attempted/delivered/suppressed/failed) with the
    /// notice title joined from the archive instead of duplicated here.
    pub fn proactive_history(&self, limit: usize) -> Result<Vec<ProactiveHistoryItem>, String> {
        let limit = limit.clamp(1, MAX_HISTORY_QUERY) as i64;
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {HISTORY_COLUMNS}
                 FROM proactive_events e
                 JOIN documents d ON d.id=e.document_id
                 JOIN document_revisions r
                   ON r.id=e.revision_id AND r.document_id=e.document_id
                 WHERE e.state IN ({TERMINAL_STATES_SQL})
                 ORDER BY e.updated_at DESC,e.id DESC LIMIT ?1"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([limit], |row| {
                Ok(ProactiveHistoryItem {
                    event: event_row(row)?,
                    title: row.get(14)?,
                    category: row.get(15)?,
                    source_id: row.get(16)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    /// Clears terminal history only. Pending work, dedupe, baseline and
    /// archive rows are untouched, so nothing is replayed.
    pub fn proactive_clear_history(&self) -> Result<u64, String> {
        self.connection
            .execute(
                &format!(
                    "DELETE FROM proactive_events
                     WHERE state IN ({TERMINAL_STATES_SQL})"
                ),
                [],
            )
            .map(|changed| changed as u64)
            .map_err(|e| e.to_string())
    }

    pub fn proactive_tasks(&self) -> Result<Vec<ProactiveTask>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM proactive_tasks ORDER BY task_id LIMIT ?1"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([MAX_TASKS_QUERY], task_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    #[cfg(test)]
    pub fn proactive_task(&self, task_id: &str) -> Result<Option<ProactiveTask>, String> {
        task_by_id(&self.connection, task_id)
    }

    /// Creates or updates a task. First creation, interval changes and
    /// re-enabling schedule the next run one interval from now; an unchanged
    /// enabled task keeps its existing schedule.
    pub fn proactive_save_task(&self, input: &ProactiveTaskInput) -> Result<ProactiveTask, String> {
        let task_id = input.task_id.trim();
        if !valid_task_id(task_id) {
            return Err("예약 작업 식별자를 확인해 주세요.".into());
        }
        if !(MIN_TASK_INTERVAL..=MAX_TASK_INTERVAL).contains(&input.interval_minutes) {
            return Err("예약 작업 간격을 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                &format!(
                    "INSERT INTO proactive_tasks(task_id,interval_minutes,enabled,next_run_at)
                     VALUES(
                       ?1,?2,?3,
                       CASE WHEN ?3=1
                         THEN strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||?2||' minutes')
                         ELSE NULL END
                     )
                     ON CONFLICT(task_id) DO UPDATE SET
                       interval_minutes=excluded.interval_minutes,
                       enabled=excluded.enabled,
                       next_run_at=CASE
                         WHEN excluded.enabled=0 THEN NULL
                         WHEN proactive_tasks.enabled=0 THEN excluded.next_run_at
                         WHEN proactive_tasks.next_run_at IS NULL THEN excluded.next_run_at
                         WHEN proactive_tasks.interval_minutes<>excluded.interval_minutes
                           THEN excluded.next_run_at
                         ELSE proactive_tasks.next_run_at END,
                       consecutive_failures=CASE
                         WHEN proactive_tasks.enabled=0 AND excluded.enabled=1 THEN 0
                         ELSE proactive_tasks.consecutive_failures END,
                       updated_at={NOW}"
                ),
                params![task_id, input.interval_minutes as i64, input.enabled],
            )
            .map_err(|e| e.to_string())?;
        let task = task_by_id(&transaction, task_id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        task.ok_or_else(|| "예약 작업을 찾을 수 없습니다.".to_string())
    }

    #[cfg(test)]
    pub fn proactive_set_task_enabled(
        &self,
        task_id: &str,
        enabled: bool,
    ) -> Result<ProactiveTask, String> {
        if !valid_task_id(task_id.trim()) {
            return Err("예약 작업 식별자를 확인해 주세요.".into());
        }
        let task_id = task_id.trim();
        let changed = self
            .connection
            .execute(
                &format!(
                    "UPDATE proactive_tasks SET
                     enabled=?2,
                     next_run_at=CASE WHEN ?2=1
                       THEN COALESCE(
                         next_run_at,
                         strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||interval_minutes||' minutes')
                       )
                       ELSE NULL END,
                     consecutive_failures=CASE
                       WHEN ?2=1 AND enabled=0 THEN 0
                       ELSE consecutive_failures END,
                     updated_at={NOW}
                     WHERE task_id=?1"
                ),
                params![task_id, enabled],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("예약 작업을 찾을 수 없습니다.".into());
        }
        self.proactive_task(task_id)?
            .ok_or_else(|| "예약 작업을 찾을 수 없습니다.".to_string())
    }

    pub fn proactive_due_tasks(&self) -> Result<Vec<ProactiveTask>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM proactive_tasks
                 WHERE enabled=1 AND next_run_at IS NOT NULL AND next_run_at<={NOW}
                 ORDER BY next_run_at,task_id LIMIT ?1"
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([MAX_DUE_TASKS], task_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    /// Records a task attempt. Success/skipped/cancelled schedule the next run
    /// one interval out; failure applies capped exponential backoff derived
    /// from `scheduler_retry_minutes` (never beyond the task interval).
    pub fn proactive_record_task_outcome(
        &self,
        task_id: &str,
        outcome: ProactiveOutcome,
        error: Option<&str>,
    ) -> Result<ProactiveTask, String> {
        let task_id = task_id.trim();
        let error = error.map(str::trim).filter(|value| !value.is_empty());
        if error.is_some_and(|value| value.chars().count() > MAX_ERROR_CHARS) {
            return Err("예약 작업 오류 내용이 너무 깁니다.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let task = task_by_id(&transaction, task_id)?
            .ok_or_else(|| "예약 작업을 찾을 수 없습니다.".to_string())?;
        let failures = if outcome == ProactiveOutcome::Failure {
            task.consecutive_failures.saturating_add(1)
        } else {
            0
        };
        let delay = if outcome == ProactiveOutcome::Failure {
            let retry_minutes: i64 = transaction
                .query_row(
                    "SELECT scheduler_retry_minutes FROM proactive_settings WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let exponent = i64::from(failures.saturating_sub(1).min(4));
            (retry_minutes * (1_i64 << exponent)).clamp(
                1,
                MAX_FAILURE_DELAY_MINUTES.min(i64::from(task.interval_minutes)),
            )
        } else {
            i64::from(task.interval_minutes)
        };
        let stored_error = (outcome == ProactiveOutcome::Failure)
            .then_some(error)
            .flatten();
        transaction
            .execute(
                &format!(
                    "UPDATE proactive_tasks SET
                     last_run_at={NOW},last_outcome=?2,last_error=?3,
                     consecutive_failures=?4,
                     next_run_at=CASE WHEN enabled=1
                       THEN strftime('%Y-%m-%dT%H:%M:%fZ','now','+'||?5||' minutes')
                       ELSE NULL END,
                     updated_at={NOW}
                     WHERE task_id=?1"
                ),
                params![task_id, outcome, stored_error, i64::from(failures), delay],
            )
            .map_err(|e| e.to_string())?;
        let task = task_by_id(&transaction, task_id)?;
        transaction.commit().map_err(|e| e.to_string())?;
        task.ok_or_else(|| "예약 작업을 찾을 수 없습니다.".to_string())
    }

    pub fn proactive_scheduler_status(&self) -> Result<ProactiveSchedulerStatus, String> {
        let settings = read_settings(&self.connection)?;
        let (last_run_at, last_outcome, last_error, consecutive_failures) = self
            .connection
            .query_row(
                "SELECT last_run_at,last_outcome,last_error,consecutive_failures
                 FROM proactive_scheduler WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<SchedulerOutcome>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let pending_events: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM proactive_events
                 WHERE state IN ('pending','deferred','claimed')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let (due_tasks, next_task_at): (i64, Option<String>) = self
            .connection
            .query_row(
                &format!(
                    "SELECT
                       COALESCE(SUM(CASE WHEN next_run_at<={NOW} THEN 1 ELSE 0 END),0),
                       MIN(next_run_at)
                     FROM proactive_tasks
                     WHERE enabled=1 AND next_run_at IS NOT NULL"
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        Ok(ProactiveSchedulerStatus {
            enabled: settings.scheduler_enabled,
            paused: settings.scheduler_paused,
            interval_minutes: settings.scheduler_interval_minutes,
            retry_minutes: settings.scheduler_retry_minutes,
            notifications_enabled: settings.notifications_enabled,
            notification_sensitivity: settings.notification_sensitivity,
            quiet_hours: settings.quiet_hours,
            pending_events: pending_events as u32,
            due_tasks: due_tasks as u32,
            next_task_at,
            last_run_at,
            last_outcome,
            last_error,
            consecutive_failures: consecutive_failures as u32,
        })
    }

    pub fn proactive_record_scheduler_run(
        &self,
        outcome: SchedulerOutcome,
        error: Option<&str>,
    ) -> Result<ProactiveSchedulerStatus, String> {
        let error = error.map(str::trim).filter(|value| !value.is_empty());
        if error.is_some_and(|value| value.chars().count() > MAX_ERROR_CHARS) {
            return Err("스케줄러 오류 내용이 너무 깁니다.".into());
        }
        let stored_error = (outcome == SchedulerOutcome::Failure)
            .then_some(error)
            .flatten();
        self.connection
            .execute(
                &format!(
                    "UPDATE proactive_scheduler SET
                     last_run_at={NOW},last_outcome=?1,last_error=?2,
                     consecutive_failures=CASE WHEN ?1='failure'
                       THEN consecutive_failures+1 ELSE 0 END,
                     updated_at={NOW}
                     WHERE id=1"
                ),
                params![outcome, stored_error],
            )
            .map_err(|e| e.to_string())?;
        self.proactive_scheduler_status()
    }

    // Seed bootstrap state lives with schema 5 but is consumed by archive
    // sync. True means the initial bounded seed (or a completed historical
    // import) already established the frontier; it is not history_complete.
    pub(crate) fn seed_complete(&self, id: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT seed_complete FROM sources WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|_| "등록된 정보원을 찾을 수 없습니다.".into())
    }

    pub(crate) fn mark_seed_complete(&self, id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .execute("UPDATE sources SET seed_complete=1 WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("등록된 정보원을 찾을 수 없습니다.".into());
        }
        Ok(())
    }
}
