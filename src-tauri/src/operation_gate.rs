//! Shared operation admission and exclusive maintenance quiescence.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// App-wide gate for work that can observe or mutate the live data directory.
/// Every accepted operation holds a shared lease for its complete lifetime.
pub(crate) struct AdmissionGate {
    accepting: AtomicBool,
    lock: Arc<RwLock<()>>,
}

impl Default for AdmissionGate {
    fn default() -> Self {
        Self {
            accepting: AtomicBool::new(true),
            lock: Arc::new(RwLock::new(())),
        }
    }
}

impl AdmissionGate {
    /// Admit work unless maintenance has closed the boundary. The second check
    /// closes the race with an exclusive caller between the first check and
    /// acquiring the lock.
    pub(crate) async fn shared(self: &Arc<Self>) -> Result<AdmissionLease, String> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(
                "백업, 복원 또는 업데이트를 진행 중입니다. 완료 후 다시 시도해 주세요.".into(),
            );
        }
        let guard = self.lock.clone().read_owned().await;
        if !self.accepting.load(Ordering::Acquire) {
            drop(guard);
            return Err(
                "백업, 복원 또는 업데이트를 진행 중입니다. 완료 후 다시 시도해 주세요.".into(),
            );
        }
        Ok(AdmissionLease { _guard: guard })
    }

    pub(crate) fn ensure_open(&self) -> Result<(), String> {
        if self.accepting.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("백업, 복원 또는 업데이트를 진행 중입니다. 완료 후 다시 시도해 주세요.".into())
        }
    }

    /// Close admission before waiting for existing leases to drain. Dropping
    /// the returned guard reopens admission; cancellation while waiting also
    /// reopens it through `Closing`'s drop implementation.
    pub(crate) fn close(self: &Arc<Self>) -> Result<MaintenanceReservation, String> {
        self.accepting
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "다른 백업, 복원 또는 업데이트 작업이 이미 진행 중입니다.".to_string())?;
        Ok(MaintenanceReservation {
            closing: Closing(self.clone()),
        })
    }
}

pub(crate) struct MaintenanceReservation {
    closing: Closing,
}

impl MaintenanceReservation {
    pub(crate) async fn acquire(self) -> Result<MaintenanceGuard, String> {
        let guard = self.closing.0.lock.clone().write_owned().await;
        Ok(MaintenanceGuard {
            _guard: guard,
            _closing: self.closing,
        })
    }
}

pub(crate) struct AdmissionLease {
    _guard: OwnedRwLockReadGuard<()>,
}

struct Closing(Arc<AdmissionGate>);
impl Drop for Closing {
    fn drop(&mut self) {
        self.0.accepting.store(true, Ordering::Release);
    }
}

pub(crate) struct MaintenanceGuard {
    _guard: OwnedRwLockWriteGuard<()>,
    _closing: Closing,
}

#[cfg(test)]
mod tests {
    use super::AdmissionGate;
    use std::sync::Arc;

    #[tokio::test]
    async fn exclusive_closes_admission_and_joins_a_task_owned_lease() {
        let gate = Arc::new(AdmissionGate::default());
        let worker_gate = gate.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let _lease = worker_gate.shared().await.unwrap();
            let _ = started_tx.send(());
            let _ = finish_rx.await;
        });
        started_rx.await.unwrap();

        let exclusive_gate = gate.clone();
        let exclusive =
            tokio::spawn(async move { exclusive_gate.close().unwrap().acquire().await.unwrap() });
        loop {
            match gate.shared().await {
                Ok(lease) => drop(lease),
                Err(_) => break,
            }
            tokio::task::yield_now().await;
        }
        assert!(!exclusive.is_finished());
        let _ = finish_tx.send(());
        worker.await.unwrap();

        let guard = exclusive.await.unwrap();
        assert!(gate.shared().await.is_err());
        drop(guard);
        assert!(gate.shared().await.is_ok());
    }

    #[tokio::test]
    async fn abandoned_maintenance_reservation_reopens_admission() {
        let gate = Arc::new(AdmissionGate::default());
        let reservation = gate.close().unwrap();
        assert!(gate.ensure_open().is_err());
        assert!(gate.shared().await.is_err());
        drop(reservation);
        assert!(gate.ensure_open().is_ok());
        assert!(gate.shared().await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_exclusive_wait_reopens_admission() {
        let gate = Arc::new(AdmissionGate::default());
        let lease = gate.shared().await.unwrap();
        let reservation = gate.close().unwrap();
        let acquire = tokio::spawn(reservation.acquire());
        tokio::task::yield_now().await;
        acquire.abort();
        assert!(matches!(acquire.await, Err(error) if error.is_cancelled()));
        assert!(gate.shared().await.is_ok());
        drop(lease);
    }
}
