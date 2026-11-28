//! Backup and restore in 데이터 및 정보 (core, never gated); the logic is
//! [`crate::backup`].

use super::App;
use crate::backup::{self, BackupPreview, RestoreOutcome};
use std::path::PathBuf;

pub struct Backup<'a>(pub(super) &'a App);

impl Backup<'_> {
    /// Why restore cannot run on this data directory's volume, if it cannot.
    pub fn restore_support(&self) -> Result<(), String> {
        backup::restore_support(&self.0.data_root)
    }

    /// Cancels the backup, inspection or restore registered as `operation_id`.
    pub fn cancel(&self, operation_id: &str) -> Result<bool, String> {
        backup::cancel(operation_id)
    }

    /// Writes a backup file to `destination` while every other job waits.
    pub async fn create(
        &self,
        destination: PathBuf,
        operation_id: String,
    ) -> Result<BackupPreview, String> {
        let app = self.0;
        let operation = backup::begin(operation_id)?;
        let maintenance = app.exclusive_maintenance().await?;
        let root = app.data_root.clone();
        let store = app.store.clone();
        tokio::task::spawn_blocking(move || {
            let _maintenance = maintenance;
            operation.create(&store, &root, &destination)
        })
        .await
        .map_err(|_| "백업 작업을 마치지 못했습니다.".to_owned())?
    }

    /// Checks a backup file without touching the data directory.
    pub async fn inspect(
        &self,
        source: PathBuf,
        operation_id: String,
    ) -> Result<BackupPreview, String> {
        let operation = backup::begin(operation_id)?;
        tokio::task::spawn_blocking(move || operation.inspect(&source))
            .await
            .map_err(|_| "백업 파일을 검사하지 못했습니다.".to_owned())?
    }

    /// The restore this launch finalized, if any; it stays available for the
    /// whole session so a remounted window still reports it.
    pub fn restore_outcome(&self) -> Option<RestoreOutcome> {
        self.0.restore_outcome.clone()
    }

    /// Stages the restore of `source`. Once it is staged (or a failed step left
    /// its journal) maintenance stays closed and `exit` runs: the shell must
    /// then quit without relaunching, and the next launch finishes the restore
    /// (see [`backup::Operation::restore`]).
    pub async fn restore(
        &self,
        source: PathBuf,
        confirmed: bool,
        operation_id: String,
        exit: impl FnOnce() + Send + 'static,
    ) -> Result<(), String> {
        if !confirmed {
            return Err("복원을 시작하려면 확인이 필요합니다.".into());
        }
        let app = self.0;
        let operation = backup::begin(operation_id)?;
        backup::restore_support(&app.data_root)?;
        let maintenance = app.exclusive_maintenance().await?;
        let root = app.data_root.clone();
        tokio::task::spawn_blocking(move || {
            operation.restore(&root, &source, move || {
                // Keep maintenance closed until exit (see `Operation::restore`).
                maintenance.hold_until_exit();
                exit();
            })
        })
        .await
        .map_err(|_| "복원 준비를 마치지 못했습니다.".to_owned())?
    }
}
