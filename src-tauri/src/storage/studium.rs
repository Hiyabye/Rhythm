//! Durable selection only; live seat information never enters SQLite or backup.
use super::Store;
use crate::studium::{model::Preferences, rooms};
use std::collections::HashSet;

impl Store {
    pub fn studium_preferences(&self) -> Result<Preferences, String> {
        let mut statement = self
            .connection
            .prepare("SELECT room_id FROM studium_selected_rooms")
            .map_err(|e| e.to_string())?;
        let selected = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| e.to_string())?;
        let view_room: String = self
            .connection
            .query_row(
                "SELECT view_room FROM studium_preferences WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if selected.is_empty()
            || selected.iter().any(|id| rooms::get(id).is_err())
            || !selected.contains(&view_room)
        {
            return Err("저장된 열람실 설정을 확인해 주세요.".into());
        }
        Ok(Preferences {
            room_ids: rooms::all()
                .into_iter()
                .filter(|room| selected.contains(room.id))
                .map(|room| room.id.to_owned())
                .collect(),
            view_room,
        })
    }

    fn studium_writable(&self) -> Result<(), String> {
        if self.enabled(crate::extensions::STUDIUM)? {
            Ok(())
        } else {
            Err("Studium 확장 기능이 꺼져 있습니다.".into())
        }
    }

    /// Adds targets, switches the view, then removes old targets atomically.
    /// A removed current view falls back to the first selected catalog room.
    pub fn studium_save_preferences(
        &self,
        room_ids: &[String],
        view_room: &str,
    ) -> Result<Preferences, String> {
        self.studium_writable()?;
        let selected: HashSet<&str> = room_ids.iter().map(String::as_str).collect();
        if room_ids.is_empty()
            || room_ids.len() > rooms::all().len()
            || selected.len() != room_ids.len()
            || selected.iter().any(|id| rooms::get(id).is_err())
        {
            return Err("확인할 열람실을 중복 없이 하나 이상 선택해 주세요.".into());
        }
        rooms::get(view_room)?;
        let current = self.studium_preferences()?;
        let view = if selected.contains(view_room) {
            view_room.to_owned()
        } else if view_room == current.view_room {
            rooms::all()
                .into_iter()
                .find(|room| selected.contains(room.id))
                .expect("validated nonempty catalog selection")
                .id
                .to_owned()
        } else {
            return Err("표시할 열람실은 확인할 열람실 중에서 선택해 주세요.".into());
        };
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        for id in room_ids {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO studium_selected_rooms(room_id) VALUES(?1)",
                    [id],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction
            .execute(
                "UPDATE studium_preferences SET view_room=?1 WHERE id=1",
                [&view],
            )
            .map_err(|e| e.to_string())?;
        for id in current.room_ids {
            if !selected.contains(id.as_str()) {
                transaction
                    .execute("DELETE FROM studium_selected_rooms WHERE room_id=?1", [&id])
                    .map_err(|e| e.to_string())?;
            }
        }
        transaction.commit().map_err(|e| e.to_string())?;
        self.studium_preferences()
    }

    /// Changes only the displayed room, never targets or live work.
    pub fn studium_set_view_room(&self, room_id: &str) -> Result<Preferences, String> {
        self.studium_writable()?;
        let current = self.studium_preferences()?;
        if !current.room_ids.iter().any(|id| id == room_id) {
            return Err("표시할 열람실은 확인할 열람실 중에서 선택해 주세요.".into());
        }
        self.connection
            .execute(
                "UPDATE studium_preferences SET view_room=?1 WHERE id=1",
                [room_id],
            )
            .map_err(|e| e.to_string())?;
        self.studium_preferences()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn opened(path: &std::path::Path) -> Store {
        let store = Store::open(path).unwrap();
        store.set_enabled(crate::extensions::STUDIUM, true).unwrap();
        store
    }
    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
    fn schema_twenty_two(path: &std::path::Path) {
        let store = Store::open(path).unwrap();
        store
            .connection
            .execute_batch(
                "DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms;
             ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
             DELETE FROM extension_preferences WHERE extension_id='studium';
             PRAGMA user_version=22;",
            )
            .unwrap();
    }

    #[test]
    fn selection_and_view_survive_reopen_in_catalog_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        let saved = store
            .studium_save_preferences(&ids(&["6", "5", "19"]), "6")
            .unwrap();
        assert_eq!(saved.room_ids, ids(&["19", "5", "6"]));
        assert_eq!(saved.view_room, "6");
        let displayed = store.studium_set_view_room("19").unwrap();
        assert_eq!(displayed.room_ids, saved.room_ids);
        drop(store);
        let restored = Store::open(&path).unwrap().studium_preferences().unwrap();
        assert_eq!(restored.room_ids, saved.room_ids);
        assert_eq!(restored.view_room, "19");
    }

    #[test]
    fn removing_the_view_selects_first_remaining_catalog_room() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        let saved = store
            .studium_save_preferences(&ids(&["6", "19"]), "5")
            .unwrap();
        assert_eq!(saved.room_ids, ids(&["19", "6"]));
        assert_eq!(saved.view_room, "19");
        assert!(store.studium_save_preferences(&ids(&["6"]), "18").is_err());
        assert_eq!(store.studium_preferences().unwrap().view_room, "19");
    }

    #[test]
    fn invalid_selections_and_disabled_writes_preserve_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        for values in [
            ids(&[]),
            ids(&["5", "5"]),
            ids(&["invalid"]),
            ids(&["5", "6", "19", "11", "15", "16", "18", "5"]),
        ] {
            assert!(store.studium_save_preferences(&values, "5").is_err());
            assert_eq!(store.studium_preferences().unwrap().room_ids, ids(&["5"]));
        }
        assert!(store.studium_set_view_room("6").is_err());
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        assert!(store.studium_save_preferences(&ids(&["6"]), "6").is_err());
        assert!(store.studium_set_view_room("5").is_err());
        assert_eq!(store.studium_preferences().unwrap().room_ids, ids(&["5"]));
    }

    #[test]
    fn write_failure_rolls_back_added_targets_and_view_together() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_target_removal BEFORE DELETE ON studium_selected_rooms
             BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;",
            )
            .unwrap();
        assert!(store.studium_save_preferences(&ids(&["6"]), "6").is_err());
        let unchanged = store.studium_preferences().unwrap();
        assert_eq!(unchanged.room_ids, ids(&["5"]));
        assert_eq!(unchanged.view_room, "5");
    }

    #[test]
    fn populated_schema_twenty_two_upgrade_preserves_user_data_and_adds_off() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        schema_twenty_two(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "UPDATE settings SET theme='dark';
             UPDATE extension_preferences SET enabled=1 WHERE extension_id='calendar';
             INSERT INTO calendar_todos(title,day,memo) VALUES('보존할 일정','2026-11-09','메모');",
            )
            .unwrap();
        drop(connection);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.info().unwrap().schema_version,
            super::super::SCHEMA_VERSION
        );
        assert!(!store.enabled(crate::extensions::STUDIUM).unwrap());
        assert!(store.enabled(crate::extensions::CALENDAR).unwrap());
        assert_eq!(store.settings().unwrap().theme, "dark");
        assert_eq!(
            store.calendar_todos("2026-11-01", "2026-11-30").unwrap()[0].title,
            "보존할 일정"
        );
        assert_eq!(store.studium_preferences().unwrap().view_room, "5");
    }

    #[test]
    fn migration_failure_keeps_schema_twenty_two_and_existing_sentinel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        schema_twenty_two(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE studium_preferences(sentinel TEXT);
             INSERT INTO studium_preferences VALUES('keep');",
            )
            .unwrap();
        drop(connection);
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            22
        );
        assert_eq!(
            connection
                .query_row("SELECT sentinel FROM studium_preferences", [], |row| row
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
            "keep"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='studium_selected_rooms'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn sqlite_backup_contains_selection_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let backup = directory.path().join("backup");
        let store = opened(&path);
        store
            .studium_save_preferences(&ids(&["5", "11"]), "11")
            .unwrap();
        store.backup_to(&backup).unwrap();
        let restored = Store::open(&backup).unwrap();
        let preferences = restored.studium_preferences().unwrap();
        assert_eq!(preferences.room_ids, ids(&["5", "11"]));
        assert_eq!(preferences.view_room, "11");
        assert_eq!(restored.connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'studium_%'", [], |row| row.get::<_, i64>(0),
        ).unwrap(), 2);
    }
}
