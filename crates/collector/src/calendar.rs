//! The academic calendar step of a run (#319): one robots-checked fetch of
//! the school's published ICS, parsed and applied to the state. A failure is
//! recorded in the calendar's health and leaves its events as they were; it
//! never fails the run.
use crate::state::{CalendarApplied, State};
use rhythm_sources::{
    calendar::{self, Parsed, ACADEMIC_CALENDAR_ID, ACADEMIC_CALENDAR_URL},
    robots::Robots,
    transport::{Access, Fetch, Transport},
};
use std::{sync::atomic::AtomicBool, time::Duration};
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub events: usize,
    /// Events the parser left out (recurring, cancelled or malformed).
    pub skipped: usize,
    pub applied: CalendarApplied,
}

pub async fn run<T: Transport>(
    state: &State,
    transport: &T,
    now: &str,
    cancel: &AtomicBool,
) -> Result<Report, String> {
    match fetch(transport, cancel).await {
        Ok(parsed) => {
            let applied = state.calendar_apply(ACADEMIC_CALENDAR_ID, &parsed.events, now)?;
            Ok(Report {
                events: parsed.events.len(),
                skipped: parsed.skipped,
                applied,
            })
        }
        Err(error) => {
            state.calendar_failed(ACADEMIC_CALENDAR_ID, &error)?;
            Err(error)
        }
    }
}

async fn fetch<T: Transport>(transport: &T, cancel: &AtomicBool) -> Result<Parsed, String> {
    let url = Url::parse(ACADEMIC_CALENDAR_URL).map_err(|e| e.to_string())?;
    // The host has no robots.txt; the transport reads that as no rules for
    // this access only (user decision, #319). A real policy is still obeyed.
    let robots = Robots::load(transport, &url, cancel, Access::Calendar).await?;
    if !robots.permits(&url) {
        return Err("robots.txt disallows the calendar".into());
    }
    if robots.delay > 0.0 {
        tokio::time::sleep(Duration::from_secs_f64(robots.delay)).await;
    }
    match transport
        .fetch(&url, None, calendar::MAX_BYTES, cancel, Access::Calendar)
        .await?
    {
        Fetch::Body { bytes, mime, .. } if mime.as_deref() == Some("text/calendar") => {
            calendar::parse(&bytes)
        }
        Fetch::Body { mime, .. } => Err(format!("the calendar answered with {mime:?}")),
        Fetch::Redirect(target) => Err(format!("the calendar redirected to {target}")),
        Fetch::NotModified => Err("the calendar answered 304 to an unconditional request".into()),
    }
}

/// The run log line.
pub fn summary(result: &Result<Report, String>) -> String {
    match result {
        Ok(report) => format!(
            "{ACADEMIC_CALENDAR_ID}: ok, {} events ({} new, {} changed, {} missing, {} removed, {} skipped)",
            report.events,
            report.applied.added,
            report.applied.changed,
            report.applied.missing,
            report.applied.removed,
            report.skipped
        ),
        Err(error) => format!("{ACADEMIC_CALENDAR_ID}: failed: {error}"),
    }
}

#[cfg(test)]
mod tests;
