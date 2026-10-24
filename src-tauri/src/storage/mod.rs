use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod archive;
#[cfg(test)]
mod archive_tests;
const SCHEMA_VERSION: i64 = 2;

fn migrate(connection: &mut Connection, version: i64) -> Result<(), String> {
    let migrations = [
        include_str!("../../migrations/001_foundation.sql"),
        include_str!("../../migrations/002_archive.sql"),
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
    fn invalid_settings_do_not_change_storage() {
        let dir = tempfile::tempdir().unwrap();
        let db = Store::open(&dir.path().join("db")).unwrap();
        let mut s = db.settings().unwrap();
        s.theme = "invalid".into();
        assert!(db.save(&s).is_err());
        assert_eq!(db.settings().unwrap().theme, "system");
    }
    #[test]
    fn future_schema_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA user_version=3")
            .unwrap();
        assert!(Store::open(&path).is_err());
    }
}
