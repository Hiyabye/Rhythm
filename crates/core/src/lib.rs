//! Rhythm's app logic that does not depend on the desktop shell (#341).
//! The Tauri app uses it today; the GPUI app (#339) will use it too.

pub mod archive;
pub mod extensions;
pub mod intelligence;
pub mod operation_gate;
pub mod paths;
pub mod storage;
pub mod studium;
