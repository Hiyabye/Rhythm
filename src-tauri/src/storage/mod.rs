use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod archive;
#[cfg(test)]
mod archive_tests;
pub mod intelligence;
#[cfg(test)]
mod intelligence_tests;
#[cfg(any(windows, test))]
pub(crate) mod notification_activation;
pub mod personalization;
#[cfg(test)]
mod personalization_tests;
pub mod proactive;
#[cfg(test)]
mod proactive_tests;
pub(crate) const SCHEMA_VERSION: i64 = 17;

fn migrate(connection: &mut Connection, version: i64) -> Result<(), String> {
    let migrations = [
        include_str!("../../migrations/001_foundation.sql"),
        include_str!("../../migrations/002_archive.sql"),
        include_str!("../../migrations/003_intelligence.sql"),
        include_str!("../../migrations/004_personalization.sql"),
        include_str!("../../migrations/005_proactive.sql"),
        include_str!("../../migrations/006_notification_attempts.sql"),
        include_str!("../../migrations/007_notification_activation.sql"),
        include_str!("../../migrations/008_ai_execution_mode.sql"),
        include_str!("../../migrations/009_ai_model_directory.sql"),
        include_str!("../../migrations/010_update_check.sql"),
        include_str!("../../migrations/011_skipped_articles.sql"),
        include_str!("../../migrations/012_resource_retry.sql"),
        include_str!("../../migrations/013_ai_projector.sql"),
        include_str!("../../migrations/014_ai_backlog_mode.sql"),
        include_str!("../../migrations/015_ai_embedding.sql"),
        include_str!("../../migrations/016_remote_engine.sql"),
        include_str!("../../migrations/017_retire_eng_newsletter.sql"),
    ];
    for migration in migrations.iter().skip(version as usize) {
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute_batch(migration)
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub energy_profile: String,
    pub interests: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            energy_profile: "balanced".into(),
            interests: String::new(),
        }
    }
}
#[derive(Serialize)]
pub struct StorageInfo {
    pub path: String,
    pub bytes: u64,
    pub schema_version: i64,
}
pub struct Store {
    connection: Connection,
    path: PathBuf,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > SCHEMA_VERSION {
            return Err(
                "더 새로운 Rhythm에서 만든 데이터베이스입니다. 앱을 업데이트해 주세요.".into(),
            );
        }
        connection
            .execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")
            .map_err(|e| e.to_string())?;
        migrate(&mut connection, version)?;
        let store = Self {
            connection,
            path: path.into(),
        };
        store.reset_interrupted()?;
        store.reset_ai_interrupted()?;
        Ok(store)
    }
    pub fn settings(&self) -> Result<Settings, String> {
        self.connection
            .query_row(
                "SELECT theme,energy_profile,interests FROM settings WHERE id=1",
                [],
                |r| {
                    Ok(Settings {
                        theme: r.get(0)?,
                        energy_profile: r.get(1)?,
                        interests: r.get(2)?,
                    })
                },
            )
            .map_err(|e| e.to_string())
    }
    pub fn save(&self, s: &Settings) -> Result<(), String> {
        if !["system", "light", "dark"].contains(&s.theme.as_str())
            || !["eco", "balanced", "performance"].contains(&s.energy_profile.as_str())
            || s.interests.chars().count() > 4000
        {
            return Err("설정값을 확인해 주세요.".into());
        }
        self.connection
            .execute(
                "UPDATE settings SET theme=?1,energy_profile=?2,interests=?3 WHERE id=1",
                params![s.theme, s.energy_profile, s.interests],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn update_check_enabled(&self) -> Result<bool, String> {
        self.connection
            .query_row("SELECT update_check FROM settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())
    }
    pub fn set_update_check(&self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute("UPDATE settings SET update_check=?1 WHERE id=1", [enabled])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Whether automatic checks are on and the last successful one is at least
    /// a day old. A timestamp in the future (the clock moved back) is also due,
    /// so a clock change cannot postpone checks indefinitely.
    pub fn update_check_due(&self) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT update_check=1 AND (update_checked_at IS NULL
                   OR update_checked_at<=strftime('%Y-%m-%dT%H:%M:%fZ','now','-1 day')
                   OR update_checked_at>strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                 FROM settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn record_update_check(&self) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE settings SET update_checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=1",
                [],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn enabled(&self, id: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT enabled FROM extension_preferences WHERE extension_id=?1",
                [id],
                |r| r.get(0),
            )
            .or_else(|e| {
                if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                    Ok(false)
                } else {
                    Err(e)
                }
            })
            .map_err(|e| e.to_string())
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        self.connection.execute("INSERT INTO extension_preferences(extension_id,enabled) VALUES(?1,?2) ON CONFLICT(extension_id) DO UPDATE SET enabled=excluded.enabled",params![id,enabled]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn info(&self) -> Result<StorageInfo, String> {
        let bytes = std::fs::metadata(&self.path)
            .map_err(|e| e.to_string())?
            .len()
            + std::fs::metadata(format!("{}-wal", self.path.display()))
                .map(|m| m.len())
                .unwrap_or(0);
        Ok(StorageInfo {
            path: self.path.display().to_string(),
            bytes,
            schema_version: self
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|e| e.to_string())?,
        })
    }
    pub(crate) fn backup_to(&self, destination: &Path) -> Result<(), String> {
        let mut target = Connection::open(destination).map_err(|e| e.to_string())?;
        let backup = rusqlite::backup::Backup::new(&self.connection, &mut target)
            .map_err(|e| e.to_string())?;
        // The caller holds the only connection's mutex, so no writer can run
        // between steps; pausing would only extend the time the store is locked.
        backup
            .run_to_completion(i32::MAX, std::time::Duration::ZERO, None)
            .map_err(|e| e.to_string())?;
        drop(backup);
        target
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(crate) fn backup_summary(&self) -> Result<(i64, u64), String> {
        let schema = self
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let documents = self
            .connection
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())? as u64;
        Ok((schema, documents))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_and_restart_preserve_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rhythm.db");
        {
            let db = Store::open(&path).unwrap();
            db.save(&Settings {
                theme: "dark".into(),
                energy_profile: "eco".into(),
                interests: "보안, 장학금".into(),
            })
            .unwrap();
            db.set_enabled("sogang_notices", true).unwrap();
            db.set_enabled("sogang_notices", false).unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.settings().unwrap().theme, "dark");
        assert_eq!(db.settings().unwrap().energy_profile, "eco");
        assert_eq!(db.settings().unwrap().interests, "보안, 장학금");
        assert!(!db.enabled("sogang_notices").unwrap());
        assert!(db
            .connection
            .query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| r
                .get::<_, bool>(0))
            .unwrap());
    }
    #[test]
    fn online_backup_contains_committed_wal_state() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("live.db");
        let snapshot = directory.path().join("snapshot.db");
        let store = Store::open(&live).unwrap();
        store
            .save(&Settings {
                theme: "dark".into(),
                energy_profile: "eco".into(),
                interests: "보안".into(),
            })
            .unwrap();
        store.backup_to(&snapshot).unwrap();
        let copy = Connection::open(&snapshot).unwrap();
        let settings = copy
            .query_row(
                "SELECT theme,energy_profile,interests FROM settings WHERE id=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(settings, ("dark".into(), "eco".into(), "보안".into()));
    }
    #[test]
    fn invalid_settings_do_not_change_storage() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(&dir.path().join("db")).unwrap();
        let mut s = db.settings().unwrap();
        s.theme = "invalid".into();
        assert!(db.save(&s).is_err());
        assert_eq!(db.settings().unwrap().theme, "system");
    }
    #[test]
    fn update_checks_default_on_and_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let db = Store::open(&path).unwrap();
            assert!(db.update_check_enabled().unwrap());
            assert!(db.update_check_due().unwrap());
            db.set_update_check(false).unwrap();
            assert!(!db.update_check_due().unwrap());
        }
        let db = Store::open(&path).unwrap();
        assert!(!db.update_check_enabled().unwrap());
        db.set_update_check(true).unwrap();
        assert!(Store::open(&path).unwrap().update_check_enabled().unwrap());
    }
    #[test]
    fn schema_nine_upgrade_keeps_settings_and_enables_update_checks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let connection = Connection::open(&path).unwrap();
            for migration in [
                include_str!("../../migrations/001_foundation.sql"),
                include_str!("../../migrations/002_archive.sql"),
                include_str!("../../migrations/003_intelligence.sql"),
                include_str!("../../migrations/004_personalization.sql"),
                include_str!("../../migrations/005_proactive.sql"),
                include_str!("../../migrations/006_notification_attempts.sql"),
                include_str!("../../migrations/007_notification_activation.sql"),
                include_str!("../../migrations/008_ai_execution_mode.sql"),
                include_str!("../../migrations/009_ai_model_directory.sql"),
            ] {
                connection.execute_batch(migration).unwrap();
            }
            connection
                .execute_batch(
                    "UPDATE settings SET theme='dark',energy_profile='eco',interests='장학금'",
                )
                .unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
        let settings = db.settings().unwrap();
        assert_eq!(
            (
                settings.theme.as_str(),
                settings.energy_profile.as_str(),
                settings.interests.as_str()
            ),
            ("dark", "eco", "장학금")
        );
        assert!(db.update_check_enabled().unwrap());
        assert!(db.update_check_due().unwrap());
    }
    #[test]
    fn update_check_is_due_daily_and_after_a_clock_change() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(&dir.path().join("db")).unwrap();
        db.record_update_check().unwrap();
        assert!(!db.update_check_due().unwrap());
        let set = |at: &str| {
            db.connection
                .execute(
                    &format!("UPDATE settings SET update_checked_at=strftime('%Y-%m-%dT%H:%M:%fZ','now','{at}')"),
                    [],
                )
                .unwrap();
        };
        set("-23 hours");
        assert!(!db.update_check_due().unwrap());
        set("-25 hours");
        assert!(db.update_check_due().unwrap());
        set("+2 hours");
        assert!(db.update_check_due().unwrap());
    }
    #[test]
    fn future_schema_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version=99")
            .unwrap();
        assert!(Store::open(&path).is_err());
    }
}
