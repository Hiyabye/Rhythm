//! The cancel flag and the single-job slot the controllers share.
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

/// Resolves once `flag` is set, checking it every `poll`.
pub async fn cancelled(flag: &AtomicBool, poll: Duration) {
    while !flag.load(Ordering::Acquire) {
        tokio::time::sleep(poll).await;
    }
}

/// A cancel flag shared between a controller and the job it started.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// The raw flag, for the functions that take `&AtomicBool`.
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }

    /// Whether both tokens are the same flag.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// At most one running job. `Slot` clones share the slot.
#[derive(Clone, Debug, Default)]
pub struct Slot(Arc<Mutex<Option<CancelToken>>>);

/// Holds the slot until dropped, also when the job panics or is aborted.
#[derive(Debug)]
pub struct Running {
    slot: Slot,
    token: CancelToken,
}

/// Why `Slot::begin` did not start a job.
#[derive(Debug, PartialEq, Eq)]
pub enum Busy {
    /// A job is running.
    Running,
    /// The slot's lock was poisoned.
    Unavailable,
}

impl Slot {
    pub fn begin(&self) -> Result<Running, Busy> {
        let mut current = self.0.lock().map_err(|_| Busy::Unavailable)?;
        if current.is_some() {
            return Err(Busy::Running);
        }
        let token = CancelToken::new();
        *current = Some(token.clone());
        Ok(Running {
            slot: self.clone(),
            token,
        })
    }

    /// A poisoned slot counts as running.
    pub fn running(&self) -> bool {
        self.0.lock().map(|c| c.is_some()).unwrap_or(true)
    }

    /// Cancels the running job, if any.
    pub fn cancel(&self) {
        if let Ok(current) = self.0.lock() {
            if let Some(token) = current.as_ref() {
                token.cancel();
            }
        }
    }

    /// Returns once no job is running, polling every 50 ms.
    pub async fn wait(&self) {
        while self.running() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Running {
    pub fn token(&self) -> &CancelToken {
        &self.token
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Ok(mut current) = self.slot.0.lock() {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_begin_fails_until_the_guard_drops() {
        let slot = Slot::default();
        let first = slot.begin().unwrap();
        assert!(slot.running());
        assert_eq!(slot.begin().unwrap_err(), Busy::Running);
        drop(first);
        assert!(!slot.running());
        assert!(slot.begin().is_ok());
    }

    #[test]
    fn cancel_reaches_only_the_running_job() {
        let slot = Slot::default();
        slot.cancel();
        let running = slot.begin().unwrap();
        assert!(
            !running.token().is_cancelled(),
            "a stale cancel must not carry over"
        );
        slot.cancel();
        assert!(running.token().is_cancelled());
        assert!(running.token().flag().load(Ordering::Acquire));
        drop(running);
        slot.cancel();
        assert!(!slot.begin().unwrap().token().is_cancelled());
    }

    #[tokio::test]
    async fn aborted_task_releases_the_slot_and_wakes_waiters() {
        let slot = Slot::default();
        let running = slot.begin().unwrap();
        let task = tokio::spawn(async move {
            let _running = running;
            std::future::pending::<()>().await;
        });
        let waiter = tokio::spawn({
            let slot = slot.clone();
            async move { slot.wait().await }
        });
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!waiter.is_finished());
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("wait returns after the guard drops")
            .unwrap();
        assert!(!slot.running());
    }

    #[tokio::test]
    async fn panicking_task_releases_the_slot() {
        let slot = Slot::default();
        let running = slot.begin().unwrap();
        let task = tokio::spawn(async move {
            let _running = running;
            panic!("job failed");
        });
        assert!(task.await.is_err());
        assert!(!slot.running());
    }

    #[tokio::test]
    async fn wait_returns_at_once_when_idle() {
        tokio::time::timeout(Duration::from_millis(20), Slot::default().wait())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_resolves_after_cancel() {
        let token = CancelToken::new();
        let poll = Duration::from_millis(10);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), cancelled(token.flag(), poll))
                .await
                .is_err()
        );
        let waiter = tokio::spawn({
            let token = token.clone();
            async move { cancelled(token.flag(), poll).await }
        });
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        // Already cancelled: ready without sleeping.
        tokio::time::timeout(
            Duration::from_millis(1),
            cancelled(token.flag(), Duration::from_secs(60)),
        )
        .await
        .unwrap();
        assert!(token.same(&token.clone()) && !token.same(&CancelToken::new()));
    }
}
