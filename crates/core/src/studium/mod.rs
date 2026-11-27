pub mod controller;
pub mod jobs;
pub mod model;
pub mod network;
pub mod notification;
pub mod parser;
pub mod rooms;

pub async fn cancelled(cancel: &std::sync::atomic::AtomicBool) {
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
