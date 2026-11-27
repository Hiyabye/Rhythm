//! Native notification plumbing shared by the scheduler's notices, todo
//! reminders and seat alerts: the OS authorization check and, on Windows, the
//! WinRT toast sender and its protocol activation.
pub mod permission;
#[cfg(windows)]
pub mod windows;
