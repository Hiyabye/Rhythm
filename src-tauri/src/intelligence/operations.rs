//! Transient command-owned work. No source text, generated text or job state is retained.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

pub const CANCELLED: &str = "AI 작업을 취소했습니다.";
pub const EXITING: &str = "앱을 종료하고 있습니다.";

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub kind: String,
    pub phase: String,
    pub elapsed_ms: u64,
    pub cancelling: bool,
}
struct Active {
    kind: String,
    phase: String,
    started: Instant,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Default)]
pub struct Controller {
    active: Arc<Mutex<Option<Active>>>,
    activity: Arc<AtomicU64>,
}
impl Controller {
    pub fn begin(&self, kind: &str) -> Result<Operation, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "AI 작업 상태를 확인하지 못했습니다.")?;
        if active.is_some() {
            return Err("진행 중인 AI 작업을 먼저 취소하거나 완료해 주세요.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.activity.fetch_add(1, Ordering::Release);
        *active = Some(Active {
            kind: kind.into(),
            phase: "verifying".into(),
            started: Instant::now(),
            cancel: cancel.clone(),
        });
        Ok(Operation {
            controller: self.clone(),
            cancel,
        })
    }
    /// Retain an observation of work that began and finished between idle polls.
    pub fn activity_generation(&self) -> u64 {
        self.activity.load(Ordering::Acquire)
    }
    pub fn status(&self) -> Option<Status> {
        let active = self.active.lock().ok()?;
        let active = active.as_ref()?;
        Some(Status {
            kind: active.kind.clone(),
            phase: active.phase.clone(),
            elapsed_ms: active.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            cancelling: active.cancel.load(Ordering::Acquire),
        })
    }
    pub fn cancel(&self) {
        if let Ok(active) = self.active.lock() {
            if let Some(active) = active.as_ref() {
                active.cancel.store(true, Ordering::Release);
            }
        }
    }
}
/// Held until blocking verification has stopped or outstanding IPC has been reaped.
/// Dropping only the async future is not a safe cancellation boundary.
pub struct Operation {
    controller: Controller,
    pub cancel: Arc<AtomicBool>,
}
impl Operation {
    pub fn phase(&self, phase: &str) {
        if let Ok(mut active) = self.controller.active.lock() {
            if let Some(active) = active
                .as_mut()
                .filter(|a| Arc::ptr_eq(&a.cancel, &self.cancel))
            {
                active.phase = phase.into();
            }
        }
    }
    pub fn check(&self, shutdown: &AtomicBool) -> Result<(), String> {
        if shutdown.load(Ordering::Acquire) {
            self.cancel.store(true, Ordering::Release);
            Err(EXITING.into())
        } else if self.cancel.load(Ordering::Acquire) {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
    pub async fn interrupted(&self, shutdown: &AtomicBool) -> String {
        loop {
            if let Err(error) = self.check(shutdown) {
                return error;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        // Also stop a blocking verifier if its owning command is abandoned.
        self.cancel.store(true, Ordering::Release);
        if let Ok(mut active) = self.controller.active.lock() {
            if active
                .as_ref()
                .is_some_and(|a| Arc::ptr_eq(&a.cancel, &self.cancel))
            {
                *active = None;
                self.controller.activity.fetch_add(1, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn status_survives_observer_remount_and_cancel_until_owner_finishes() {
        let controller = Controller::default();
        let operation = controller.begin("setup").unwrap();
        operation.phase("loading");
        let observer = controller.clone();
        let status = observer.status().unwrap();
        assert_eq!(
            (status.kind.as_str(), status.phase.as_str()),
            ("setup", "loading")
        );
        assert!(controller.begin("load").is_err());
        observer.cancel();
        assert!(controller.status().unwrap().cancelling);
        assert_eq!(
            operation.interrupted(&AtomicBool::new(false)).await,
            CANCELLED
        );
        drop(operation);
        assert!(observer.status().is_none());
        let next = controller.begin("load").unwrap();
        assert!(!controller.status().unwrap().cancelling);
        drop(next);
    }
    #[test]
    fn short_completed_operations_remain_observable_between_idle_polls() {
        let controller = Controller::default();
        let observer = controller.clone();
        let before = observer.activity_generation();
        assert!(observer.status().is_none());
        drop(controller.begin("load").unwrap());
        assert!(observer.status().is_none());
        assert_ne!(observer.activity_generation(), before);
        let after = observer.activity_generation();
        assert_eq!(controller.activity_generation(), after);
    }
    #[tokio::test]
    async fn shutdown_sets_the_same_hash_cancellation_token() {
        let controller = Controller::default();
        let operation = controller.begin("load").unwrap();
        let token = operation.cancel.clone();
        assert_eq!(operation.interrupted(&AtomicBool::new(true)).await, EXITING);
        assert!(token.load(Ordering::Acquire));
        drop(operation);
        assert!(controller.status().is_none());
    }
}
