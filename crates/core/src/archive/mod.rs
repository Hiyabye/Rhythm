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
/// Where a stored resource path points, only when it is a file directly in
/// `resources/`. Stored paths are generated that way; a database from a
/// restored backup must not be able to name a file anywhere else.
fn resource_file(root: &std::path::Path, stored: &str) -> Option<std::path::PathBuf> {
    let name = stored.strip_prefix("resources/")?;
    let plain =
        !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0']);
    plain.then(|| root.join("resources").join(name))
}
/// Deletes resource files no row references any more. The rows are gone, so
/// a file left behind is only unused disk space.
pub async fn remove_orphans<P: AsRef<str>>(
    root: &std::path::Path,
    paths: impl IntoIterator<Item = P>,
) {
    for path in paths {
        if let Some(file) = resource_file(root, path.as_ref()) {
            let _ = tokio::fs::remove_file(file).await;
        }
    }
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
        // A closed gate (a backup or update at launch) only postpones the
        // pass; it does not cost the hour.
        let Ok(lease) = gate.shared().await else {
            tokio::time::sleep(AUTO_TRASH_RETRY).await;
            continue;
        };
        if let Err(error) = with_store(&store, |s| s.auto_trash_old()) {
            eprintln!("Archive: 오래된 공지를 휴지통으로 옮기지 못했습니다: {error}");
        }
        drop(lease);
        tokio::time::sleep(AUTO_TRASH_WAKE).await;
    }
}
const AUTO_TRASH_WAKE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const AUTO_TRASH_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn orphan_cleanup_follows_a_stored_path_only_inside_resources() {
        let outside = tempfile::tempdir().unwrap();
        let root = outside.path().join("data");
        std::fs::create_dir_all(root.join("resources/nested")).unwrap();
        let kept = [
            outside.path().join("victim"),
            root.join("rhythm.sqlite3"),
            root.join("resources/nested/file.pdf"),
        ];
        for path in &kept {
            std::fs::write(path, b"keep").unwrap();
        }
        let orphan = root.join("resources/abc.pdf");
        std::fs::write(&orphan, b"orphan").unwrap();
        let absolute = kept[0].to_string_lossy().into_owned();
        remove_orphans(
            &root,
            [
                "resources/abc.pdf",
                "resources/../../victim",
                "../victim",
                absolute.as_str(),
                "rhythm.sqlite3",
                "resources/nested/file.pdf",
                "resources/..",
                "resources/",
            ],
        )
        .await;
        assert!(!orphan.exists());
        for path in &kept {
            assert!(path.exists(), "{}", path.display());
        }
    }
}
