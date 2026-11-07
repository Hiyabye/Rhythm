//! Persisted shapes of the proactive settings, events, tasks and scheduler status.
use super::*;

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
    pub(super) fn is_terminal(self) -> bool {
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
