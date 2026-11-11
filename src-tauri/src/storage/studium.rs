//! Durable preferences only; live seat information and automatic running state
//! never enter SQLite or backup.
use super::Store;
use crate::studium::{model::Preferences, rooms};
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, HashSet};

/// Position uses physical screen coordinates; window size uses logical pixels.
/// Open/always-on-top state and seat data are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MiniGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MiniGeometry {
    fn validate(&self) -> Result<(), String> {
        if !(-100_000..=100_000).contains(&self.x)
            || !(-100_000..=100_000).contains(&self.y)
            || !(300..=1200).contains(&self.width)
            || !(260..=1400).contains(&self.height)
        {
            return Err("저장할 미니 창의 위치와 크기를 확인해 주세요.".into());
        }
        Ok(())
    }
}

impl Store {
    pub(crate) fn studium_mini_geometry(&self) -> Result<Option<MiniGeometry>, String> {
        let geometry: Option<MiniGeometry> = self
            .connection
            .query_row(
                "SELECT x,y,width,height FROM studium_mini_geometry WHERE id=1",
                [],
                |row| {
                    Ok(MiniGeometry {
                        x: row.get(0)?,
                        y: row.get(1)?,
                        width: row.get(2)?,
                        height: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(geometry) = &geometry {
            geometry.validate()?;
        }
        Ok(geometry)
    }

    /// Native close/cleanup captures geometry under admission before calling
    /// this internal method. It must also work just after extension disable.
    /// No raw geometry write command is exposed to a WebView.
    pub(crate) fn studium_save_mini_geometry(&self, geometry: &MiniGeometry) -> Result<(), String> {
        geometry.validate()?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        transaction.execute(
            "INSERT INTO studium_mini_geometry(id,x,y,width,height) VALUES(1,?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET x=excluded.x,y=excluded.y,width=excluded.width,height=excluded.height",
            rusqlite::params![geometry.x, geometry.y, geometry.width, geometry.height],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    /// Includes every fixed room, including unselected rooms and empty lists.
    pub fn studium_favorites(&self) -> Result<BTreeMap<String, Vec<u32>>, String> {
        let mut favorites: BTreeMap<String, Vec<u32>> = rooms::all()
            .into_iter()
            .map(|room| (room.id.to_owned(), Vec::new()))
            .collect();
        let mut statement = self
            .connection
            .prepare("SELECT room_id,seat_number FROM studium_favorite_seats ORDER BY room_id,seat_number")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (room_id, number) = row.map_err(|e| e.to_string())?;
            let room = rooms::get(&room_id)?;
            let seats = favorites
                .get_mut(&room_id)
                .expect("fixed catalog room initialized above");
            if number == 0 || seats.last().is_some_and(|previous| *previous >= number) {
                return Err("저장된 선호 좌석 설정을 확인해 주세요.".into());
            }
            // A seat beyond a reduced catalog capacity cannot exist. Skip it
            // instead of failing the whole page; a changed save replaces the row.
            if number <= room.capacity {
                seats.push(number);
            }
        }
        Ok(favorites)
    }

    /// Replaces one room's favorites atomically, retaining every other room.
    pub fn studium_save_favorites(
        &self,
        room_id: &str,
        seat_numbers: &[u32],
    ) -> Result<BTreeMap<String, Vec<u32>>, String> {
        self.studium_writable()?;
        let room = rooms::get(room_id)?;
        if seat_numbers.len() > room.capacity as usize
            || seat_numbers.iter().copied().collect::<HashSet<_>>().len() != seat_numbers.len()
            || seat_numbers
                .iter()
                .any(|number| *number == 0 || *number > room.capacity)
        {
            return Err(format!(
                "선호 좌석은 1번부터 {}번까지 중복 없이 입력해 주세요.",
                room.capacity
            ));
        }
        let current = self.studium_favorites()?;
        let mut sorted = seat_numbers.to_vec();
        sorted.sort_unstable();
        if current[room_id] == sorted {
            return Ok(current);
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "DELETE FROM studium_favorite_seats WHERE room_id=?1",
                [room_id],
            )
            .map_err(|e| e.to_string())?;
        for number in seat_numbers {
            transaction
                .execute(
                    "INSERT INTO studium_favorite_seats(room_id,seat_number) VALUES(?1,?2)",
                    rusqlite::params![room_id, number],
                )
                .map_err(|e| e.to_string())?;
        }
        // Favorite changes alter preferred-seat eligibility, so pending
        // capabilities/candidates from the preceding rule version are stale.
        transaction
            .execute(
                "UPDATE studium_alert_rules SET version=version+1,armed=1 WHERE room_id=?1",
                [room_id],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "DELETE FROM studium_alert_tokens WHERE room_id=?1",
                [room_id],
            )
            .map_err(|e| e.to_string())?;
        let favorites = self.studium_favorites()?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(favorites)
    }

    pub fn studium_auto_interval(&self) -> Result<u32, String> {
        let interval: u32 = self
            .connection
            .query_row(
                "SELECT interval_minutes FROM studium_preferences WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !(2..=60).contains(&interval) {
            return Err("저장된 자동 확인 간격을 확인해 주세요.".into());
        }
        Ok(interval)
    }

    /// Saves the interval without enabling automatic work or changing targets.
    pub fn studium_save_auto_interval(&self, interval_minutes: u32) -> Result<u32, String> {
        self.studium_writable()?;
        if !(2..=60).contains(&interval_minutes) {
            return Err("자동 확인 간격은 2분에서 60분 사이로 정해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE studium_preferences SET interval_minutes=?1 WHERE id=1",
                [interval_minutes],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("저장된 열람실 설정을 확인해 주세요.".into());
        }
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(interval_minutes)
    }

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
            Err("열람실 확장 기능이 꺼져 있습니다.".into())
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
                    .execute("DELETE FROM studium_alert_tokens WHERE room_id=?1", [&id])
                    .map_err(|e| e.to_string())?;
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
                "DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices; ALTER TABLE settings DROP COLUMN auto_trash_months;
             DELETE FROM extension_preferences WHERE extension_id='studium';
             PRAGMA user_version=22;",
            )
            .unwrap();
    }

    fn geometry() -> MiniGeometry {
        MiniGeometry {
            x: -500,
            y: 200,
            width: 380,
            height: 480,
        }
    }

    #[test]
    fn mini_geometry_starts_absent_and_survives_disabled_cleanup_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        assert_eq!(store.studium_mini_geometry().unwrap(), None);
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        // Internal close cleanup follows extension disable and preserves the
        // user's last window geometry without turning the extension back on.
        store.studium_save_mini_geometry(&geometry()).unwrap();
        assert_eq!(store.studium_mini_geometry().unwrap(), Some(geometry()));
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert!(!reopened.enabled(crate::extensions::STUDIUM).unwrap());
        assert_eq!(reopened.studium_mini_geometry().unwrap(), Some(geometry()));
    }

    #[test]
    fn mini_geometry_bounds_and_single_row_checks_preserve_previous_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        for valid in [
            MiniGeometry {
                x: -100_000,
                y: 100_000,
                width: 300,
                height: 260,
            },
            MiniGeometry {
                x: 100_000,
                y: -100_000,
                width: 1200,
                height: 1400,
            },
            geometry(),
        ] {
            store.studium_save_mini_geometry(&valid).unwrap();
            assert_eq!(store.studium_mini_geometry().unwrap(), Some(valid));
        }
        for invalid in [
            MiniGeometry {
                x: -100_001,
                ..geometry()
            },
            MiniGeometry {
                x: 100_001,
                ..geometry()
            },
            MiniGeometry {
                y: i32::MIN,
                ..geometry()
            },
            MiniGeometry {
                y: i32::MAX,
                ..geometry()
            },
            MiniGeometry {
                width: 299,
                ..geometry()
            },
            MiniGeometry {
                width: 1201,
                ..geometry()
            },
            MiniGeometry {
                height: 259,
                ..geometry()
            },
            MiniGeometry {
                height: 1401,
                ..geometry()
            },
        ] {
            assert!(store.studium_save_mini_geometry(&invalid).is_err());
            assert_eq!(store.studium_mini_geometry().unwrap(), Some(geometry()));
        }
        for sql in [
            "INSERT INTO studium_mini_geometry VALUES(2,0,0,380,480)",
            "UPDATE studium_mini_geometry SET x=100001",
            "UPDATE studium_mini_geometry SET y=-100001",
            "UPDATE studium_mini_geometry SET width=299",
            "UPDATE studium_mini_geometry SET height=1401",
            "UPDATE studium_mini_geometry SET x=0.5",
        ] {
            assert!(store.connection.execute(sql, []).is_err());
            assert_eq!(store.studium_mini_geometry().unwrap(), Some(geometry()));
        }
    }

    #[test]
    fn failed_geometry_upsert_keeps_the_previous_row_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.studium_save_mini_geometry(&geometry()).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_mini_update BEFORE UPDATE ON studium_mini_geometry
             BEGIN SELECT RAISE(ABORT,'synthetic geometry write failure'); END;",
            )
            .unwrap();
        assert!(store
            .studium_save_mini_geometry(&MiniGeometry {
                x: 700,
                ..geometry()
            })
            .is_err());
        assert_eq!(store.studium_mini_geometry().unwrap(), Some(geometry()));
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().studium_mini_geometry().unwrap(),
            Some(geometry())
        );
    }

    #[test]
    fn schema_twenty_nine_upgrade_preserves_room_interval_favorite_and_alert_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        let preferences = store
            .studium_save_preferences(&ids(&["5", "6"]), "6")
            .unwrap();
        store.studium_save_auto_interval(14).unwrap();
        let favorites = store.studium_save_favorites("6", &[1, 311]).unwrap();
        let rules = store
            .studium_save_alert_rule(
                "6",
                &crate::studium::model::AlertRuleInput {
                    enabled: true,
                    kind: "remaining".into(),
                    minimum: 5,
                },
            )
            .unwrap();
        let mut settings = store.settings().unwrap();
        settings.clock_format = "24h".into();
        settings.auto_trash_months = 12;
        store.save(&settings).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE studium_mini_geometry; PRAGMA user_version=29;")
            .unwrap();
        drop(store);
        let upgraded = Store::open(&path).unwrap();
        assert_eq!(
            upgraded.info().unwrap().schema_version,
            super::super::SCHEMA_VERSION
        );
        assert_eq!(upgraded.studium_mini_geometry().unwrap(), None);
        assert_eq!(upgraded.studium_preferences().unwrap(), preferences);
        assert_eq!(upgraded.studium_auto_interval().unwrap(), 14);
        assert_eq!(upgraded.studium_favorites().unwrap(), favorites);
        assert_eq!(upgraded.studium_alert_rules().unwrap(), rules);
        assert!(upgraded.enabled(crate::extensions::STUDIUM).unwrap());
        assert_eq!(upgraded.settings().unwrap().clock_format, "24h");
        assert_eq!(upgraded.settings().unwrap().auto_trash_months, 12);
    }

    #[test]
    fn geometry_migration_failure_preserves_schema_twenty_nine_and_existing_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.connection.execute_batch(
            "DROP TABLE studium_mini_geometry; CREATE TABLE studium_mini_geometry(sentinel TEXT);
             INSERT INTO studium_mini_geometry VALUES('keep'); PRAGMA user_version=29;",
        ).unwrap();
        drop(store);
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            29
        );
        assert_eq!(
            connection
                .query_row("SELECT sentinel FROM studium_mini_geometry", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "keep"
        );
    }

    #[test]
    fn favorites_beyond_a_reduced_capacity_are_skipped_without_failing_the_page() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        store.studium_save_favorites("5", &[7]).unwrap();
        let capacity = rooms::get("5").unwrap().capacity;
        // Models a row saved under a larger capacity than the current catalog.
        store
            .connection
            .execute_batch(&format!(
                "PRAGMA ignore_check_constraints=ON;
                 INSERT INTO studium_favorite_seats(room_id,seat_number) VALUES('5',{});
                 PRAGMA ignore_check_constraints=OFF;",
                capacity + 1
            ))
            .unwrap();
        assert_eq!(store.studium_favorites().unwrap()["5"], [7]);
        assert_eq!(
            store.studium_save_favorites("5", &[7, 8]).unwrap()["5"],
            [7, 8]
        );
        let stored: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM studium_favorite_seats WHERE room_id='5'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, 2);
    }

    #[test]
    fn favorites_include_every_room_sort_numbers_and_allow_clearing_one_room() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        let empty = store.studium_favorites().unwrap();
        assert_eq!(empty.len(), rooms::all().len());
        assert!(empty.values().all(Vec::is_empty));
        for room in rooms::all() {
            let saved = store
                .studium_save_favorites(room.id, &[room.capacity, 1])
                .unwrap();
            assert_eq!(saved[room.id], [1, room.capacity]);
            // The migration's per-room CHECK stays aligned with the catalog.
            assert!(store
                .connection
                .execute(
                    "INSERT INTO studium_favorite_seats(room_id,seat_number) VALUES(?1,?2)",
                    rusqlite::params![room.id, room.capacity + 1],
                )
                .is_err());
        }
        let full_room: Vec<u32> = (1..=rooms::get("6").unwrap().capacity).collect();
        assert_eq!(
            store.studium_save_favorites("6", &full_room).unwrap()["6"],
            full_room
        );
        let before = store.studium_favorites().unwrap();
        let cleared = store.studium_save_favorites("5", &[]).unwrap();
        assert!(cleared["5"].is_empty());
        for (id, seats) in before.iter().filter(|(id, _)| id.as_str() != "5") {
            assert_eq!(&cleared[id], seats);
        }
    }

    #[test]
    fn invalid_favorites_and_disabled_writes_preserve_all_rooms() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        let before = store.studium_save_favorites("5", &[4, 7]).unwrap();
        for numbers in [vec![0], vec![103], vec![7, 7], vec![1; 103], vec![u32::MAX]] {
            assert!(store.studium_save_favorites("5", &numbers).is_err());
            assert_eq!(store.studium_favorites().unwrap(), before);
        }
        assert!(store.studium_save_favorites("unknown", &[1]).is_err());
        assert!(store.studium_save_favorites("unknown", &[]).is_err());
        assert!(store
            .connection
            .execute(
                "INSERT INTO studium_favorite_seats(room_id,seat_number) VALUES('5',4)",
                [],
            )
            .is_err());
        assert!(store
            .connection
            .execute(
                "INSERT INTO studium_favorite_seats(room_id,seat_number) VALUES('unknown',1)",
                [],
            )
            .is_err());
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        assert!(store.studium_save_favorites("5", &[]).is_err());
        assert_eq!(store.studium_favorites().unwrap(), before);
    }

    #[test]
    fn favorites_survive_deselection_and_reopen_as_preferences_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store
            .studium_save_preferences(&ids(&["5", "6"]), "6")
            .unwrap();
        let saved = store.studium_save_favorites("6", &[311, 20]).unwrap();
        store.studium_save_preferences(&ids(&["5"]), "6").unwrap();
        assert_eq!(store.studium_favorites().unwrap(), saved);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened.studium_preferences().unwrap().room_ids,
            ids(&["5"])
        );
        assert_eq!(reopened.studium_favorites().unwrap(), saved);
    }

    #[test]
    fn favorite_insert_failure_rolls_back_delete_and_partial_inserts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.studium_save_favorites("6", &[1, 3]).unwrap();
        let saved = store.studium_save_favorites("5", &[2, 8]).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_favorite_insert BEFORE INSERT ON studium_favorite_seats
             WHEN NEW.seat_number=9
             BEGIN SELECT RAISE(ABORT,'synthetic favorite write failure'); END;",
            )
            .unwrap();
        assert!(store.studium_save_favorites("5", &[3, 9]).is_err());
        assert_eq!(store.studium_favorites().unwrap(), saved);
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().studium_favorites().unwrap(),
            saved
        );
    }

    #[test]
    fn populated_schema_twenty_seven_upgrade_keeps_interval_rooms_and_extension_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        let preferences = store
            .studium_save_preferences(&ids(&["5", "6"]), "6")
            .unwrap();
        store.studium_save_auto_interval(18).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats; PRAGMA user_version=27;")
            .unwrap();
        drop(store);
        let upgraded = Store::open(&path).unwrap();
        assert_eq!(
            upgraded.info().unwrap().schema_version,
            super::super::SCHEMA_VERSION
        );
        assert_eq!(upgraded.studium_auto_interval().unwrap(), 18);
        assert_eq!(upgraded.studium_preferences().unwrap(), preferences);
        assert!(upgraded.enabled(crate::extensions::STUDIUM).unwrap());
        assert_eq!(upgraded.studium_favorites().unwrap().len(), 7);
        assert!(upgraded
            .studium_favorites()
            .unwrap()
            .values()
            .all(Vec::is_empty));
    }

    #[test]
    fn favorite_migration_failure_keeps_schema_twenty_seven_and_existing_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.studium_save_auto_interval(8).unwrap();
        store
            .connection
            .execute_batch(
                "DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats;
             CREATE TABLE studium_favorite_seats(sentinel TEXT);
             INSERT INTO studium_favorite_seats VALUES('keep');
             PRAGMA user_version=27;",
            )
            .unwrap();
        drop(store);
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            27
        );
        assert_eq!(
            connection
                .query_row("SELECT sentinel FROM studium_favorite_seats", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "keep"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT interval_minutes FROM studium_preferences WHERE id=1",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            8
        );
    }

    #[test]
    fn auto_interval_defaults_to_five_and_survives_reopen_without_changing_rooms() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        assert_eq!(store.studium_auto_interval().unwrap(), 5);
        let preferences = store
            .studium_save_preferences(&ids(&["5", "6"]), "6")
            .unwrap();
        for interval in [2, 60, 5, 13] {
            assert_eq!(
                store.studium_save_auto_interval(interval).unwrap(),
                interval
            );
            assert_eq!(store.studium_auto_interval().unwrap(), interval);
            assert_eq!(store.studium_preferences().unwrap(), preferences);
        }
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.studium_auto_interval().unwrap(), 13);
        assert_eq!(reopened.studium_preferences().unwrap(), preferences);
    }

    #[test]
    fn invalid_or_disabled_interval_writes_preserve_the_saved_value() {
        let directory = tempfile::tempdir().unwrap();
        let store = opened(&directory.path().join("db"));
        store.studium_save_auto_interval(11).unwrap();
        for invalid in [0, 1, 61, u32::MAX] {
            assert!(store.studium_save_auto_interval(invalid).is_err());
            assert_eq!(store.studium_auto_interval().unwrap(), 11);
        }
        store
            .set_enabled(crate::extensions::STUDIUM, false)
            .unwrap();
        assert!(store.studium_save_auto_interval(8).is_err());
        assert_eq!(store.studium_auto_interval().unwrap(), 11);
    }

    #[test]
    fn auto_interval_write_failure_keeps_the_previous_value_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.studium_save_auto_interval(9).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_interval_write BEFORE UPDATE OF interval_minutes ON studium_preferences
                 BEGIN SELECT RAISE(ABORT,'synthetic interval failure'); END;",
            )
            .unwrap();
        assert!(store.studium_save_auto_interval(12).is_err());
        assert_eq!(store.studium_auto_interval().unwrap(), 9);
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().studium_auto_interval().unwrap(),
            9
        );
    }

    #[test]
    fn populated_schema_twenty_six_upgrade_adds_only_interval_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        let preferences = store
            .studium_save_preferences(&ids(&["5", "6"]), "6")
            .unwrap();
        store
            .connection
            .execute_batch(
                "UPDATE settings SET theme='dark',clock_format='24h',auto_trash_months=3;
                 DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats;
                 ALTER TABLE studium_preferences DROP COLUMN interval_minutes;
                 PRAGMA user_version=26;",
            )
            .unwrap();
        drop(store);
        let upgraded = Store::open(&path).unwrap();
        assert_eq!(
            upgraded.info().unwrap().schema_version,
            super::super::SCHEMA_VERSION
        );
        assert_eq!(upgraded.studium_auto_interval().unwrap(), 5);
        assert_eq!(upgraded.studium_preferences().unwrap(), preferences);
        assert!(upgraded.enabled(crate::extensions::STUDIUM).unwrap());
        assert_eq!(upgraded.settings().unwrap().theme, "dark");
        assert_eq!(upgraded.settings().unwrap().clock_format, "24h");
        assert_eq!(upgraded.settings().unwrap().auto_trash_months, 3);
    }

    #[test]
    fn interval_migration_failure_keeps_schema_twenty_six_and_existing_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let store = opened(&path);
        store.studium_save_auto_interval(17).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE studium_mini_geometry; DROP TABLE studium_alert_tokens; DROP TABLE studium_alert_attempts; DROP TABLE studium_alert_rules; DROP TABLE studium_favorite_seats; PRAGMA user_version=26;")
            .unwrap();
        drop(store);
        // An unexpected pre-existing column makes 027 fail instead of silently
        // treating a partially changed profile as a completed migration.
        assert!(Store::open(&path).is_err());
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            26
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT interval_minutes FROM studium_preferences WHERE id=1",
                    [],
                    |row| row.get::<_, u32>(0),
                )
                .unwrap(),
            17
        );
    }

    #[test]
    fn newer_schema_is_rejected_without_changing_journal_or_database_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("newer.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(&format!(
                "PRAGMA journal_mode=DELETE;
                 CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('keep');
                 PRAGMA user_version={};",
                super::super::SCHEMA_VERSION + 1,
            ))
            .unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "delete"
        );
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
    fn sqlite_backup_contains_selection_interval_and_favorites_without_results() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let backup = directory.path().join("backup");
        let store = opened(&path);
        store
            .studium_save_preferences(&ids(&["5", "11"]), "11")
            .unwrap();
        store.studium_save_auto_interval(14).unwrap();
        let favorites = store.studium_save_favorites("18", &[174, 2]).unwrap();
        store.studium_save_mini_geometry(&geometry()).unwrap();
        store.backup_to(&backup).unwrap();
        let restored = Store::open(&backup).unwrap();
        let preferences = restored.studium_preferences().unwrap();
        assert_eq!(preferences.room_ids, ids(&["5", "11"]));
        assert_eq!(preferences.view_room, "11");
        assert_eq!(restored.studium_auto_interval().unwrap(), 14);
        assert_eq!(restored.studium_favorites().unwrap(), favorites);
        assert_eq!(restored.studium_mini_geometry().unwrap(), Some(geometry()));
        assert_eq!(restored.connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'studium_%'", [], |row| row.get::<_, i64>(0),
        ).unwrap(), 7);
        let columns = restored
            .connection
            .prepare("SELECT name FROM pragma_table_info('studium_preferences') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        // No durable automatic-enabled/job/result state is introduced.
        assert_eq!(columns, ["id", "view_room", "interval_minutes"]);
        let favorite_columns = restored
            .connection
            .prepare("SELECT name FROM pragma_table_info('studium_favorite_seats') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(favorite_columns, ["room_id", "seat_number"]);
        let geometry_columns = restored
            .connection
            .prepare("SELECT name FROM pragma_table_info('studium_mini_geometry') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(geometry_columns, ["id", "x", "y", "width", "height"]);
    }
}
