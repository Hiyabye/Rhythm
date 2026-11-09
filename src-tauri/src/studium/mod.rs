pub(crate) mod commands;
pub(crate) mod controller;
pub(crate) mod model;
pub(crate) mod network;
pub(crate) mod parser;
pub(crate) mod rooms;

#[cfg(test)]
mod live_tests;

pub(crate) async fn cancelled(cancel: &std::sync::atomic::AtomicBool) {
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
