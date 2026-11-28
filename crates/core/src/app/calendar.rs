//! The 캘린더 page: todos, notice dates for a range, its two switches, the
//! `.ics` export and a clicked reminder's day. The page belongs to the
//! `calendar` extension, and the week pane (`week`) shares the notice dates.

use super::App;
use crate::{
    calendar_export::{self, CalendarExport},
    extensions::{CALENDAR, WEEK},
    storage::{
        calendar::{CalendarTodo, TodoInput},
        notice_dates::CalendarNoticeDates,
        with_store,
    },
};
use std::path::Path;

pub struct Calendar<'a>(pub(super) &'a App);

impl Calendar<'_> {
    /// Todos dated `from` through `to`, at most 62 days.
    pub async fn todos(&self, from: &str, to: &str) -> Result<Vec<CalendarTodo>, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            s.calendar_on()?;
            s.calendar_todos(from, to)
        })
    }

    /// Dates found in notice text for the range: saved notices' dates, plus
    /// every notice's deadlines when the user turned that on, and the academic
    /// calendar while it is shown. The week pane reads them too, so either
    /// extension opens this.
    pub async fn notice_dates(&self, from: &str, to: &str) -> Result<CalendarNoticeDates, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            if !s.enabled(CALENDAR)? && !s.enabled(WEEK)? {
                return Err("캘린더와 앞으로 7일 확장 기능이 모두 꺼져 있습니다.".into());
            }
            s.calendar_notice_dates(from, to)
        })
    }

    pub async fn set_all_deadlines(&self, enabled: bool) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            s.calendar_on()?;
            s.set_calendar_all_deadlines(enabled)
        })
    }

    /// Shows or hides the academic calendar (학사 일정) layer (#319).
    pub async fn set_academic(&self, enabled: bool) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            s.calendar_on()?;
            s.set_calendar_academic(enabled)
        })
    }

    // The store refuses every todo write while the extension is off.
    pub async fn create_todo(&self, todo: &TodoInput) -> Result<CalendarTodo, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.calendar_create_todo(todo))
    }

    pub async fn update_todo(&self, id: i64, todo: &TodoInput) -> Result<CalendarTodo, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.calendar_update_todo(id, todo))
    }

    pub async fn set_todo_done(&self, id: i64, done: bool) -> Result<CalendarTodo, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.calendar_set_todo_done(id, done))
    }

    pub async fn delete_todo(&self, id: i64) -> Result<(), String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.calendar_delete_todo(id))
    }

    /// Writes the `.ics` file to the path the user picked in the shell's save
    /// dialog.
    pub async fn export(&self, destination: &Path) -> Result<CalendarExport, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| s.calendar_on())?;
        calendar_export::export(&self.0.store, destination).await
    }

    /// One-shot fallback for a reminder click that arrived before the UI
    /// subscribed to [`Event::ReminderClicked`](crate::shell::Event): the
    /// todo's day while the calendar is still on.
    pub async fn reminder_activation(&self) -> Result<Option<String>, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        with_store(&app.store, |s| {
            let day = app.proactive.take_reminder_activation()?;
            Ok(day.filter(|_| s.enabled(CALENDAR).unwrap_or(false)))
        })
    }
}
