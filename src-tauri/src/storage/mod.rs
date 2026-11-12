use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod archive;
pub mod calendar;
pub mod cybercampus;
pub(crate) use archive::search_terms;
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
pub mod studium;
pub(crate) mod studium_alerts;
pub(crate) mod trash;
pub(crate) const SCHEMA_VERSION: i64 = 31;

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
        include_str!("../../migrations/018_cybercampus.sql"),
        include_str!("../../migrations/019_onboarding.sql"),
        include_str!("../../migrations/020_extensions.sql"),
        include_str!("../../migrations/021_source_groups.sql"),
        include_str!("../../migrations/022_calendar.sql"),
        include_str!("../../migrations/023_studium.sql"),
        include_str!("../../migrations/024_clock_format.sql"),
        include_str!("../../migrations/025_trash.sql"),
        include_str!("../../migrations/026_auto_trash.sql"),
        include_str!("../../migrations/027_studium_interval.sql"),
        include_str!("../../migrations/028_studium_favorites.sql"),
        include_str!("../../migrations/029_studium_alerts.sql"),
        include_str!("../../migrations/030_studium_mini.sql"),
        include_str!("../../migrations/031_calendar_todo_notice.sql"),
    ];
    for (index, migration) in migrations.iter().enumerate().skip(version as usize) {
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute_batch(migration)
            .map_err(|e| e.to_string())?;
        // Only a profile created by this open has never seen the setup flow.
        if version == 0 && index == 18 {
            transaction
                .execute("UPDATE settings SET onboarding_completed=0 WHERE id=1", [])
                .map_err(|e| e.to_string())?;
        }
        // A new profile starts blank: the user turns on each feature.
        if version == 0 && index == 19 {
            transaction
                .execute("UPDATE extension_preferences SET enabled=0", [])
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub energy_profile: String,
    pub interests: String,
    /// `12h` or `24h`; only changes how the UI shows times.
    pub clock_format: String,
    /// Months after publication when a notice moves to 휴지통; 0 never.
    pub auto_trash_months: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            energy_profile: "balanced".into(),
            interests: String::new(),
            clock_format: "12h".into(),
            auto_trash_months: 0,
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
        // A seat toast from an earlier process, including one that exited
        // without its shutdown cleanup, must not navigate this session.
        store.studium_alert_invalidate_tokens()?;
        Ok(store)
    }
    pub fn settings(&self) -> Result<Settings, String> {
        self.connection
            .query_row(
                "SELECT theme,energy_profile,interests,clock_format,auto_trash_months FROM settings WHERE id=1",
                [],
                |r| {
                    Ok(Settings {
                        theme: r.get(0)?,
                        energy_profile: r.get(1)?,
                        interests: r.get(2)?,
                        clock_format: r.get(3)?,
                        auto_trash_months: r.get(4)?,
                    })
                },
            )
            .map_err(|e| e.to_string())
    }
    pub fn save(&self, s: &Settings) -> Result<(), String> {
        if !["system", "light", "dark"].contains(&s.theme.as_str())
            || !["eco", "balanced", "performance"].contains(&s.energy_profile.as_str())
            || s.interests.chars().count() > 4000
            || !["12h", "24h"].contains(&s.clock_format.as_str())
            || !trash::AUTO_TRASH_MONTHS.contains(&s.auto_trash_months)
        {
            return Err("설정값을 확인해 주세요.".into());
        }
        self.connection
            .execute(
                "UPDATE settings SET theme=?1,energy_profile=?2,interests=?3,clock_format=?4,auto_trash_months=?5 WHERE id=1",
                params![
                    s.theme,
                    s.energy_profile,
                    s.interests,
                    s.clock_format,
                    s.auto_trash_months
                ],
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
    pub fn onboarding_completed(&self) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT onboarding_completed FROM settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn complete_onboarding(&self) -> Result<(), String> {
        self.connection
            .execute("UPDATE settings SET onboarding_completed=1 WHERE id=1", [])
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
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        transaction.execute("INSERT INTO extension_preferences(extension_id,enabled) VALUES(?1,?2) ON CONFLICT(extension_id) DO UPDATE SET enabled=excluded.enabled",params![id,enabled]).map_err(|e|e.to_string())?;
        if !enabled
            && matches!(
                id,
                crate::extensions::STUDIUM | crate::extensions::PROACTIVE
            )
        {
            transaction
                .execute("DELETE FROM studium_alert_tokens", [])
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
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
                clock_format: "24h".into(),
                auto_trash_months: 6,
            })
            .unwrap();
            db.set_enabled("sogang_notices", true).unwrap();
            db.set_enabled("sogang_notices", false).unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.settings().unwrap().theme, "dark");
        assert_eq!(db.settings().unwrap().energy_profile, "eco");
        assert_eq!(db.settings().unwrap().interests, "보안, 장학금");
        assert_eq!(db.settings().unwrap().clock_format, "24h");
        assert_eq!(db.settings().unwrap().auto_trash_months, 6);
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
                ..Settings::default()
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
        let mut s = db.settings().unwrap();
        s.clock_format = "12".into();
        assert!(db.save(&s).is_err());
        assert_eq!(db.settings().unwrap().clock_format, "12h");
        for months in [2, 36] {
            let mut s = db.settings().unwrap();
            s.auto_trash_months = months;
            assert!(db.save(&s).is_err());
        }
        assert_eq!(db.settings().unwrap().auto_trash_months, 0);
    }
    #[test]
    fn schema_twenty_five_upgrade_keeps_settings_and_never_trashes_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        Store::open(&path)
            .unwrap()
            .connection
            .execute_batch(
                "DROP TRIGGER calendar_todos_document_deleted; DROP INDEX calendar_todos_document; ALTER TABLE calendar_todos DROP COLUMN document_id; DROP TABLE studium_mini_geometry;
                 DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules;
                 DROP TABLE studium_favorite_seats;
                 ALTER TABLE studium_preferences DROP COLUMN interval_minutes;
                 ALTER TABLE settings DROP COLUMN auto_trash_months;
                 UPDATE settings SET theme='dark',clock_format='24h';
                 PRAGMA user_version=25;",
            )
            .unwrap();
        let db = Store::open(&path).unwrap();
        assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
        let settings = db.settings().unwrap();
        assert_eq!(
            (
                settings.theme.as_str(),
                settings.clock_format.as_str(),
                settings.auto_trash_months
            ),
            ("dark", "24h", 0)
        );
    }
    #[test]
    fn failed_auto_trash_migration_keeps_schema_twenty_five() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        // The column survives the downgrade, so the 026 ALTER fails mid-open.
        Store::open(&path)
            .unwrap()
            .connection
            .execute_batch("UPDATE settings SET auto_trash_months=3; PRAGMA user_version=25;")
            .unwrap();
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        let (version, months): (i64, u32) = connection
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version),auto_trash_months FROM settings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((version, months), (25, 3));
    }
    #[test]
    fn a_new_profile_shows_twelve_hour_times() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(&dir.path().join("db")).unwrap();
        assert_eq!(db.settings().unwrap().clock_format, "12h");
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
    fn a_new_profile_needs_setup_once_and_keeps_completion_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let db = Store::open(&path).unwrap();
            assert!(!db.onboarding_completed().unwrap());
        }
        let db = Store::open(&path).unwrap();
        assert!(!db.onboarding_completed().unwrap());
        db.complete_onboarding().unwrap();
        db.complete_onboarding().unwrap();
        assert!(Store::open(&path).unwrap().onboarding_completed().unwrap());
    }
    #[test]
    fn schema_eighteen_upgrade_marks_setup_completed() {
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
                include_str!("../../migrations/010_update_check.sql"),
                include_str!("../../migrations/011_skipped_articles.sql"),
                include_str!("../../migrations/012_resource_retry.sql"),
                include_str!("../../migrations/013_ai_projector.sql"),
                include_str!("../../migrations/014_ai_backlog_mode.sql"),
                include_str!("../../migrations/015_ai_embedding.sql"),
                include_str!("../../migrations/016_remote_engine.sql"),
                include_str!("../../migrations/017_retire_eng_newsletter.sql"),
                include_str!("../../migrations/018_cybercampus.sql"),
            ] {
                connection.execute_batch(migration).unwrap();
            }
            connection
                .execute_batch("UPDATE settings SET theme='dark',update_check=0")
                .unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
        assert!(db.onboarding_completed().unwrap());
        assert_eq!(db.settings().unwrap().theme, "dark");
        assert!(!db.update_check_enabled().unwrap());
    }
    #[test]
    fn schema_nineteen_upgrade_keeps_the_features_a_profile_had() {
        // (notices enabled, cybercampus subscribed) -> cybercampus extension
        for (notices, subscribed, cybercampus) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
            (false, false, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            {
                let mut connection = Connection::open(&path).unwrap();
                migrate(&mut connection, 0).unwrap();
                // Rebuild a schema-19 profile: drop what 020 added.
                connection
                    .execute_batch(&format!(
                        "DELETE FROM extension_preferences;
                         INSERT INTO extension_preferences VALUES('sogang_notices',{notices});
                         INSERT INTO sources(id,name,organization,description,url,family,supported,subscribed)
                           VALUES('cybercampus','사이버캠퍼스','서강대학교','','https://eclass.sogang.ac.kr/','cybercampus',1,{subscribed}),
                                 ('sogang-academic','학사','서강대학교','','https://www.sogang.ac.kr/','sogang',1,1);
                         INSERT INTO source_extensions VALUES('cybercampus','sogang_notices'),('sogang-academic','sogang_notices');
                         ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TRIGGER calendar_todos_document_deleted; DROP TABLE calendar_todos; DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices; ALTER TABLE settings DROP COLUMN auto_trash_months;
                         PRAGMA user_version=19;",
                        notices = notices as i32,
                        subscribed = subscribed as i32,
                    ))
                    .unwrap();
            }
            let db = Store::open(&path).unwrap();
            assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
            assert_eq!(db.enabled("sogang_notices").unwrap(), notices);
            assert_eq!(db.enabled("cybercampus").unwrap(), cybercampus);
            for id in [
                "search",
                "saved",
                "recommendations",
                "week",
                "ai",
                "proactive",
            ] {
                assert!(db.enabled(id).unwrap(), "{id}");
            }
            let owner: String = db
                .connection
                .query_row(
                    "SELECT extension_id FROM source_extensions WHERE source_id='cybercampus'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(owner, "cybercampus");
        }
    }
    #[test]
    fn schema_twenty_three_upgrade_keeps_settings_and_shows_twelve_hour_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        Store::open(&path)
            .unwrap()
            .connection
            .execute_batch(
                "DROP TRIGGER calendar_todos_document_deleted; DROP INDEX calendar_todos_document; ALTER TABLE calendar_todos DROP COLUMN document_id; DROP TABLE studium_mini_geometry;
                 DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules;
                 DROP TABLE studium_favorite_seats;
                 ALTER TABLE studium_preferences DROP COLUMN interval_minutes;
                 ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices; ALTER TABLE settings DROP COLUMN auto_trash_months;
                 UPDATE settings SET theme='dark',energy_profile='eco',interests='장학금';
                 PRAGMA user_version=23;",
            )
            .unwrap();
        let db = Store::open(&path).unwrap();
        assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
        let settings = db.settings().unwrap();
        assert_eq!(
            (
                settings.theme.as_str(),
                settings.energy_profile.as_str(),
                settings.interests.as_str(),
                settings.clock_format.as_str()
            ),
            ("dark", "eco", "장학금", "12h")
        );
    }
    #[test]
    fn failed_clock_format_migration_keeps_schema_twenty_three() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        // The column survives the downgrade, so the 024 ALTER fails mid-open.
        Store::open(&path)
            .unwrap()
            .connection
            .execute_batch(
                "UPDATE settings SET theme='dark',clock_format='24h'; PRAGMA user_version=23;",
            )
            .unwrap();
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        let (version, theme, clock): (i64, String, String) = connection
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version),theme,clock_format FROM settings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (version, theme.as_str(), clock.as_str()),
            (23, "dark", "24h")
        );
    }
    #[test]
    fn schema_twenty_one_upgrade_adds_calendar_off_and_keeps_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        {
            let db = Store::open(&path).unwrap();
            db.set_enabled("week", true).unwrap();
            db.set_enabled("search", true).unwrap();
            db.connection
                .execute_batch(
                    "DROP TRIGGER calendar_todos_document_deleted; DROP TABLE calendar_todos; DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices; ALTER TABLE settings DROP COLUMN auto_trash_months;
                     DELETE FROM extension_preferences WHERE extension_id='calendar';
                     PRAGMA user_version=21;",
                )
                .unwrap();
        }
        let db = Store::open(&path).unwrap();
        assert_eq!(db.info().unwrap().schema_version, SCHEMA_VERSION);
        assert!(db.enabled("week").unwrap() && db.enabled("search").unwrap());
        assert!(!db.enabled("ai").unwrap());
        assert!(!db.enabled("calendar").unwrap());
        assert!(db
            .calendar_todos("2026-11-01", "2026-11-30")
            .unwrap()
            .is_empty());
    }
    #[test]
    fn new_profile_starts_with_every_extension_off() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(&dir.path().join("db")).unwrap();
        let enabled: i64 = db
            .connection
            .query_row(
                "SELECT COUNT(*) FROM extension_preferences WHERE enabled=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(enabled, 0);
        assert!(!db.enabled("search").unwrap());
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
