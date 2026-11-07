//! Persisted per-source scheduler tasks and the scheduler's own run status.
use super::*;

pub(super) fn valid_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_TASK_ID_CHARS
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub(super) fn task_row(row: &Row<'_>) -> rusqlite::Result<ProactiveTask> {
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

pub(super) fn task_by_id(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<ProactiveTask>, String> {
    connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM proactive_tasks WHERE task_id=?1"),
            [task_id],
            task_row,
        )
        .optional()
        .map_err(|e| e.to_string())
}

impl Store {
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
}
