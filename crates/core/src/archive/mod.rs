pub mod cybercampus;
pub mod model;
pub mod sync;
#[cfg(windows)]
pub mod windows_reveal;

pub use network::Client;
pub use rhythm_sources::{catalog, dates, parser, pdf, selector, tls, transport as network};

use crate::{
    operation_gate::AdmissionGate,
    storage::{with_store, SharedStore},
};
use model::{SourceDefinition, SourceGroup};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub fn definitions() -> Vec<SourceDefinition> {
    catalog::definitions()
}
pub fn groups() -> Vec<SourceGroup> {
    catalog::groups()
}
/// Whether the app collects this source itself: only Cybercampus. Public
/// boards arrive through the feed (#177, #178).
pub fn app_collected(id: &str) -> bool {
    id == catalog::CYBERCAMPUS_SOURCE_ID
}
/// Moves old notices to 휴지통 at launch and then hourly while the app is open,
/// so a notice that crosses the chosen age moves without a sync or a settings
/// change. Sync jobs and saving settings also run it.
pub async fn run_auto_trash(
    store: SharedStore,
    gate: Arc<AdmissionGate>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        if let Ok(_lease) = gate.shared().await {
            if let Err(error) = with_store(&store, |s| s.auto_trash_old()) {
                eprintln!("Archive: 오래된 공지를 휴지통으로 옮기지 못했습니다: {error}");
            }
        }
        tokio::time::sleep(AUTO_TRASH_WAKE).await;
    }
}
const AUTO_TRASH_WAKE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Extracts dates for notices archived before the dates table existed or by an
/// older extractor, once per launch. New and changed notices are extracted when
/// they are written. Batches release the store between them so commands and
/// sync are not held up.
pub async fn run_notice_dates(
    store: SharedStore,
    gate: Arc<AdmissionGate>,
    shutdown: Arc<AtomicBool>,
) {
    let mut failures = 0;
    while !shutdown.load(Ordering::Acquire) {
        // A closed gate (a backup or update at launch) only pauses the
        // backfill; it resumes once admission reopens.
        let Ok(lease) = gate.shared().await else {
            tokio::time::sleep(NOTICE_DATES_RETRY).await;
            continue;
        };
        match with_store(&store, |s| s.refresh_notice_dates(NOTICE_DATES_BATCH)) {
            Ok(0) => return,
            Ok(_) => failures = 0,
            Err(error) => {
                eprintln!("Archive: 공지 날짜를 정리하지 못했습니다: {error}");
                failures += 1;
                if failures >= NOTICE_DATES_ATTEMPTS {
                    return;
                }
                drop(lease);
                tokio::time::sleep(NOTICE_DATES_RETRY).await;
                continue;
            }
        }
        drop(lease);
        tokio::task::yield_now().await;
    }
}
const NOTICE_DATES_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
const NOTICE_DATES_ATTEMPTS: u32 = 3;
const NOTICE_DATES_BATCH: i64 = 200;
