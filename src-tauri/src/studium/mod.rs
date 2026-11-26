pub(crate) mod commands;
pub(crate) mod controller;
pub(crate) use rhythm_core::studium::{model, parser, rooms};
pub(crate) mod network;
pub(crate) mod notification;
pub(crate) mod views;

#[cfg(test)]
mod live_tests;

pub(crate) async fn cancelled(cancel: &std::sync::atomic::AtomicBool) {
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
