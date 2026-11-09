//! Courses and deadlines of the Cybercampus connector. Notices are ordinary
//! archive documents; these rows mirror what eClass currently shows and are
//! replaced per course on every successful sync.
use super::Store;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CourseInput {
    pub key: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskInput {
    pub category: String,
    pub item_id: String,
    pub label: String,
    pub title: String,
    pub week: Option<u32>,
    pub opens_at: Option<String>,
    pub due_at: Option<String>,
    pub late_due_at: Option<String>,
    pub status: Option<String>,
    pub submitted_at: Option<String>,
    pub pending: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CybercampusTask {
    pub course_key: String,
    pub course_name: String,
    pub category: String,
    pub item_id: String,
    pub label: String,
    pub title: String,
    pub week: Option<u32>,
    pub opens_at: Option<String>,
    pub due_at: Option<String>,
    pub late_due_at: Option<String>,
    pub status: Option<String>,
    pub submitted_at: Option<String>,
    pub pending: bool,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CybercampusCourse {
    pub course_key: String,
    pub name: String,
    pub year: i32,
    pub term: String,
}

fn short(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}
fn optional(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|value| value.len() <= 64)
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Store {
    /// Makes `courses` the course list of the synced term. Courses that are no
    /// longer listed lose their deadlines; their archived notices stay.
    pub(crate) fn cybercampus_set_courses(
        &self,
        year: i32,
        term: &str,
        courses: &[CourseInput],
    ) -> Result<(), String> {
        if courses.len() > 64
            || !identifier(term)
            || courses
                .iter()
                .any(|course| !identifier(&course.key) || !short(&course.name, 512))
        {
            return Err("사이버캠퍼스 강좌 목록을 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let keys: Vec<&str> = courses.iter().map(|course| course.key.as_str()).collect();
        let stale: Vec<String> = {
            let mut statement = transaction
                .prepare("SELECT course_key FROM cybercampus_courses")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows.into_iter()
                .filter(|key| !keys.contains(&key.as_str()))
                .collect()
        };
        for key in stale {
            transaction
                .execute("DELETE FROM cybercampus_tasks WHERE course_key=?1", [&key])
                .map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "DELETE FROM cybercampus_courses WHERE course_key=?1",
                    [&key],
                )
                .map_err(|e| e.to_string())?;
        }
        for course in courses {
            transaction
                .execute(
                    "INSERT INTO cybercampus_courses(course_key,name,year,term) VALUES(?1,?2,?3,?4)
                     ON CONFLICT(course_key) DO UPDATE SET name=excluded.name,year=excluded.year,
                       term=excluded.term,synced_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                    params![course.key, course.name.trim(), year, term],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }
    pub(crate) fn cybercampus_courses(&self) -> Result<Vec<CybercampusCourse>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT course_key,name,year,term FROM cybercampus_courses ORDER BY name,course_key")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(CybercampusCourse {
                    course_key: row.get(0)?,
                    name: row.get(1)?,
                    year: row.get(2)?,
                    term: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }
    /// Replaces one course's deadlines with the current eClass state.
    pub(crate) fn cybercampus_replace_tasks(
        &self,
        course_key: &str,
        tasks: &[TaskInput],
    ) -> Result<(), String> {
        if tasks.len() > 512
            || tasks.iter().any(|task| {
                !identifier(&task.category)
                    || !identifier(&task.item_id)
                    || task.label.len() > 64
                    || !short(&task.title, 2048)
                    || task.week.is_some_and(|week| week > 60)
                    || ![
                        &task.opens_at,
                        &task.due_at,
                        &task.late_due_at,
                        &task.status,
                        &task.submitted_at,
                    ]
                    .into_iter()
                    .all(optional)
            })
        {
            return Err("사이버캠퍼스 과제 정보를 확인해 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let known: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM cybercampus_courses WHERE course_key=?1",
                [course_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if known.is_none() {
            return Err("등록되지 않은 사이버캠퍼스 강좌입니다.".into());
        }
        transaction
            .execute(
                "DELETE FROM cybercampus_tasks WHERE course_key=?1",
                [course_key],
            )
            .map_err(|e| e.to_string())?;
        for task in tasks {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO cybercampus_tasks(course_key,category,item_id,label,title,week,
                       opens_at,due_at,late_due_at,status,submitted_at,pending)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        course_key,
                        task.category,
                        task.item_id,
                        task.label,
                        task.title.trim(),
                        task.week,
                        task.opens_at,
                        task.due_at,
                        task.late_due_at,
                        task.status,
                        task.submitted_at,
                        task.pending
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }
    /// Stored rows of one course, so a sync can skip pages it already read.
    pub(crate) fn cybercampus_stored_tasks(
        &self,
        course_key: &str,
    ) -> Result<HashMap<(String, String), TaskInput>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT category,item_id,label,title,week,opens_at,due_at,late_due_at,status,submitted_at,pending
                 FROM cybercampus_tasks WHERE course_key=?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([course_key], |row| {
                Ok(TaskInput {
                    category: row.get(0)?,
                    item_id: row.get(1)?,
                    label: row.get(2)?,
                    title: row.get(3)?,
                    week: row.get(4)?,
                    opens_at: row.get(5)?,
                    due_at: row.get(6)?,
                    late_due_at: row.get(7)?,
                    status: row.get(8)?,
                    submitted_at: row.get(9)?,
                    pending: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|task| ((task.category.clone(), task.item_id.clone()), task))
            .collect())
    }
    /// Unfinished items with a deadline first (soonest first), then the rest.
    pub(crate) fn cybercampus_tasks(&self) -> Result<Vec<CybercampusTask>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT t.course_key,c.name,t.category,t.item_id,t.label,t.title,t.week,t.opens_at,
                   t.due_at,t.late_due_at,t.status,t.submitted_at,t.pending,t.updated_at
                 FROM cybercampus_tasks t JOIN cybercampus_courses c ON c.course_key=t.course_key
                 ORDER BY t.due_at IS NULL, t.due_at, c.name, t.title
                 LIMIT 1000",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(CybercampusTask {
                    course_key: row.get(0)?,
                    course_name: row.get(1)?,
                    category: row.get(2)?,
                    item_id: row.get(3)?,
                    label: row.get(4)?,
                    title: row.get(5)?,
                    week: row.get(6)?,
                    opens_at: row.get(7)?,
                    due_at: row.get(8)?,
                    late_due_at: row.get(9)?,
                    status: row.get(10)?,
                    submitted_at: row.get(11)?,
                    pending: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn task(item: &str, due: Option<&str>) -> TaskInput {
        TaskInput {
            category: "report".into(),
            item_id: item.into(),
            label: "과제".into(),
            title: format!("과제 {item}"),
            due_at: due.map(str::to_owned),
            ..TaskInput::default()
        }
    }
    fn course(key: &str, name: &str) -> CourseInput {
        CourseInput {
            key: key.into(),
            name: name.into(),
        }
    }

    #[test]
    fn schema_seventeen_upgrades_and_courses_replace_their_deadlines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        {
            let mut connection = Connection::open(&path).unwrap();
            super::super::migrate(&mut connection, 0).unwrap();
            connection
                .execute_batch(
                    "DROP TABLE cybercampus_tasks; DROP TABLE cybercampus_courses;
                     ALTER TABLE settings DROP COLUMN onboarding_completed;
                     ALTER TABLE sources DROP COLUMN group_id; ALTER TABLE sources DROP COLUMN selected; DROP TABLE source_groups; DROP TABLE calendar_todos; DROP TABLE studium_preferences; DROP TABLE studium_selected_rooms; ALTER TABLE settings DROP COLUMN clock_format; DROP TABLE trashed_documents; DROP TABLE deleted_notices;
                     PRAGMA user_version=17;",
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.info().unwrap().schema_version,
            super::super::SCHEMA_VERSION
        );
        store
            .cybercampus_set_courses(
                2026,
                "3",
                &[course("K1", "운영체제"), course("K2", "자료구조")],
            )
            .unwrap();
        store
            .cybercampus_replace_tasks(
                "K1",
                &[task("2", Some("2026-11-09T23:59")), task("1", None)],
            )
            .unwrap();
        store
            .cybercampus_replace_tasks("K2", &[task("3", Some("2026-11-01T10:00"))])
            .unwrap();
        let ids = |store: &Store| {
            store
                .cybercampus_tasks()
                .unwrap()
                .into_iter()
                .map(|t| t.item_id)
                .collect::<Vec<_>>()
        };
        // Soonest deadline first; undated items last.
        assert_eq!(ids(&store), ["3", "2", "1"]);
        // A sync replaces a course's rows with what eClass shows now.
        store
            .cybercampus_replace_tasks("K1", &[task("2", Some("2026-11-09T23:59"))])
            .unwrap();
        assert_eq!(ids(&store), ["3", "2"]);
        // A course that left the term list takes its deadlines with it.
        store
            .cybercampus_set_courses(2026, "3", &[course("K1", "운영체제")])
            .unwrap();
        assert_eq!(ids(&store), ["2"]);
        assert_eq!(store.cybercampus_courses().unwrap().len(), 1);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(ids(&reopened), ["2"]);
        assert!(reopened.cybercampus_stored_tasks("K1").unwrap().len() == 1);
    }

    #[test]
    fn invalid_rows_are_rejected_without_partial_writes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("db")).unwrap();
        assert!(store
            .cybercampus_set_courses(2026, "3", &[course("K 1", "이름")])
            .is_err());
        assert!(store
            .cybercampus_replace_tasks("unknown", &[task("1", None)])
            .is_err());
        store
            .cybercampus_set_courses(2026, "3", &[course("K1", "운영체제")])
            .unwrap();
        store
            .cybercampus_replace_tasks("K1", &[task("1", None)])
            .unwrap();
        let mut bad = task("2", None);
        bad.title = String::new();
        assert!(store
            .cybercampus_replace_tasks("K1", &[task("3", None), bad])
            .is_err());
        assert_eq!(store.cybercampus_tasks().unwrap().len(), 1);
    }
}
