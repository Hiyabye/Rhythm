//! The feed's side of the state (migration 006): each notice's current
//! records next to what it last published, the records of uncompacted run
//! segments and the published files. The publisher (`publish.rs`) decides;
//! this only reads and commits.
use super::State;
use rhythm_feed::{
    manifest::{BoardHealth, CalendarHealth},
    record::{
        BodyState, Classification, DateKind, Embedding, Event, MealWeek, Notice, NoticeDate,
        Resource,
    },
};
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;

/// What a notice last published.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Published {
    pub notice_seq: Option<u64>,
    pub notice_digest: Option<String>,
    pub embedding_seq: Option<u64>,
    pub embedding_digest: Option<String>,
    pub classification_seq: Option<u64>,
    pub classification_digest: Option<String>,
    pub removed_seq: Option<u64>,
}

/// A notice as the feed would publish it now.
pub struct Scanned {
    pub notice_id: i64,
    pub notice: Notice,
    pub removed_at: Option<String>,
    /// The vector of the current revision, f16 as stored.
    pub embedding: Option<(Embedding, Vec<u8>)>,
    /// The classification of that vector.
    pub classification: Option<Classification>,
    pub published: Published,
}

/// What a calendar event last published.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishedEvent {
    pub event_seq: Option<u64>,
    pub event_digest: Option<String>,
    pub removed_seq: Option<u64>,
}

/// A calendar event as the feed would publish it now.
pub struct ScannedEvent {
    pub event: Event,
    pub removed_at: Option<String>,
    pub published: PublishedEvent,
}

/// A menu week as the feed would publish it now, with the seq and digest it
/// last published.
pub struct ScannedMeal {
    pub week: MealWeek,
    pub published: Option<(u64, String)>,
}

/// One published records file and its vectors file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub records_path: String,
    pub role: String,
    pub day: String,
    pub from_seq: u64,
    pub to_seq: u64,
    pub records_sha256: String,
    pub records_size: u64,
    pub vectors: Option<(String, String, u64)>,
}

/// A run record kept for compaction: seq, record JSON and int8 vector.
pub type StoredRecord = (u64, String, Option<Vec<u8>>);

/// A run's changes, committed in one transaction after its files are uploaded.
#[derive(Default)]
pub struct Commit {
    pub next_seq: u64,
    pub published: Vec<(i64, Published)>,
    /// By `(calendar_id, uid)`.
    pub published_events: Vec<((String, String), PublishedEvent)>,
    /// By week: the seq and digest last published.
    pub published_meals: Vec<(String, u64, String)>,
    /// New run records kept until their day is compacted.
    pub records: Vec<StoredRecord>,
    /// Compacted records up to this seq are dropped.
    pub compacted_through: Option<u64>,
    pub files: Vec<FileRow>,
    pub retire: Vec<String>,
    /// A snapshot whose content did not change gets the new day.
    pub redate: Option<(String, String)>,
}

const FILE_COLUMNS: &str = "records_path,role,day,from_seq,to_seq,records_sha256,records_size,
     vectors_path,vectors_sha256,vectors_size";

/// Seqs and sizes are stored as SQLite integers; they stay far below `i64::MAX`.
fn int(row: &rusqlite::Row, index: usize) -> rusqlite::Result<u64> {
    Ok(row.get::<_, i64>(index)? as u64)
}
fn opt(row: &rusqlite::Row, index: usize) -> rusqlite::Result<Option<u64>> {
    Ok(row.get::<_, Option<i64>>(index)?.map(|value| value as u64))
}
fn sql(value: Option<u64>) -> Option<i64> {
    value.map(|value| value as i64)
}

fn file_row(row: &rusqlite::Row) -> rusqlite::Result<FileRow> {
    let vectors_path: Option<String> = row.get(7)?;
    Ok(FileRow {
        records_path: row.get(0)?,
        role: row.get(1)?,
        day: row.get(2)?,
        from_seq: int(row, 3)?,
        to_seq: int(row, 4)?,
        records_sha256: row.get(5)?,
        records_size: int(row, 6)?,
        vectors: match vectors_path {
            Some(path) => Some((path, row.get(8)?, int(row, 9)?)),
            None => None,
        },
    })
}

fn date_kind(kind: &str) -> rusqlite::Result<DateKind> {
    match kind {
        "deadline" => Ok(DateKind::Deadline),
        "start" => Ok(DateKind::Start),
        "date" => Ok(DateKind::Date),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

impl State {
    pub fn feed_next_seq(&self) -> Result<u64, String> {
        self.with(|c| c.query_row("SELECT next_seq FROM feed", [], |row| int(row, 0)))
    }

    /// Forgets what was published, so the next publish assigns every notice a
    /// seq from `next_seq` (`--reset-feed`). Every known file and the live
    /// manifest's `listed` files (`(path, vectors path)`) are retired now, so
    /// a client still on the old manifest has a day to download them.
    pub fn feed_restart(
        &self,
        next_seq: u64,
        listed: &[(String, Option<String>)],
        now: &str,
    ) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            transaction.execute_batch(
                "DELETE FROM feed_records; DELETE FROM feed_notices; DELETE FROM feed_events;
                 DELETE FROM feed_meals;",
            )?;
            transaction.execute(
                "UPDATE feed_files SET retired_at=?1 WHERE retired_at IS NULL",
                [now],
            )?;
            // Only paths matter for a retired file; size and hash are never read.
            for (path, vectors) in listed {
                let (hash, size) = match vectors {
                    Some(_) => (Some(""), Some(0)),
                    None => (None, None),
                };
                transaction.execute(
                    &format!(
                        "INSERT OR IGNORE INTO feed_files({FILE_COLUMNS},retired_at)
                         VALUES(?1,'run',substr(?5,1,10),0,0,'',0,?2,?3,?4,?5)"
                    ),
                    params![path, vectors, hash, size, now],
                )?;
            }
            transaction.execute("UPDATE feed SET next_seq=?1", [next_seq as i64])?;
            transaction.commit()
        })
    }

    /// Every notice with its current records and what it last published, by id.
    pub fn feed_scan(&self) -> Result<Vec<Scanned>, String> {
        self.with(|c| {
            let mut dates: HashMap<i64, Vec<NoticeDate>> = HashMap::new();
            let mut statement = c.prepare(
                "SELECT notice_id,day,time,end_day,end_time,label,kind FROM notice_dates
                 ORDER BY notice_id,ordinal",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                dates.entry(row.get(0)?).or_default().push(NoticeDate {
                    day: row.get(1)?,
                    time: row.get(2)?,
                    end_day: row.get(3)?,
                    end_time: row.get(4)?,
                    label: row.get(5)?,
                    kind: date_kind(&row.get::<_, String>(6)?)?,
                });
            }
            let mut statement = c.prepare(
                "SELECT n.id,n.source_id,n.external_id,n.revision_hash,n.canonical_url,n.title,
                   n.author,n.category,n.published_at,n.modified_at,n.body_state,n.body_text,
                   n.resources,COALESCE(n.dates_version,0),n.historical,n.first_seen_at,
                   n.revised_at,n.removed_at,
                   e.scope,e.vector,
                   k.category,k.score,k.margin,k.low_confidence,
                   p.notice_seq,p.notice_digest,p.embedding_seq,p.embedding_digest,
                   p.classification_seq,p.classification_digest,p.removed_seq
                 FROM notices n
                 LEFT JOIN embeddings e ON e.notice_id=n.id AND e.revision_hash=n.revision_hash
                 LEFT JOIN classifications k ON k.notice_id=n.id
                   AND k.revision_hash=n.revision_hash AND k.scope=e.scope
                 LEFT JOIN feed_notices p ON p.notice_id=n.id
                 ORDER BY n.id",
            )?;
            let mut rows = statement.query([])?;
            let mut scanned = Vec::new();
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let resources: String = row.get(12)?;
                let body_text: String = row.get(11)?;
                let resources: Vec<Resource> = serde_json::from_str(&resources)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                let notice = Notice {
                    source_id: row.get(1)?,
                    external_id: row.get(2)?,
                    revision_hash: row.get(3)?,
                    canonical_url: row.get(4)?,
                    title: row.get(5)?,
                    author: row.get(6)?,
                    category: row.get(7)?,
                    published_at: row.get(8)?,
                    modified_at: row.get(9)?,
                    body_state: match row.get::<_, String>(10)?.as_str() {
                        "normal" => BodyState::Normal,
                        _ => BodyState::Unsupported,
                    },
                    excerpt: rhythm_feed::excerpt(&body_text),
                    body_hash: Some(rhythm_feed::body_hash(&body_text)),
                    resources,
                    dates: dates.remove(&id).unwrap_or_default(),
                    dates_version: row.get(13)?,
                    historical: row.get(14)?,
                    first_seen_at: row.get(15)?,
                    revised_at: row.get(16)?,
                };
                let scope: Option<String> = row.get(18)?;
                let embedding = match scope {
                    Some(scope) => Some((
                        Embedding {
                            source_id: notice.source_id.clone(),
                            external_id: notice.external_id.clone(),
                            revision_hash: notice.revision_hash.clone(),
                            scope,
                            vector: 0,
                        },
                        row.get(19)?,
                    )),
                    None => None,
                };
                let category: Option<String> = row.get(20)?;
                let classification = match (category, &embedding) {
                    (Some(category), Some((embedding, _))) => Some(Classification {
                        source_id: notice.source_id.clone(),
                        external_id: notice.external_id.clone(),
                        revision_hash: notice.revision_hash.clone(),
                        scope: embedding.scope.clone(),
                        category,
                        score: row.get::<_, f64>(21)? as f32,
                        margin: row.get::<_, f64>(22)? as f32,
                        low_confidence: row.get(23)?,
                    }),
                    _ => None,
                };
                scanned.push(Scanned {
                    notice_id: id,
                    notice,
                    removed_at: row.get(17)?,
                    embedding,
                    classification,
                    published: Published {
                        notice_seq: opt(row, 24)?,
                        notice_digest: row.get(25)?,
                        embedding_seq: opt(row, 26)?,
                        embedding_digest: row.get(27)?,
                        classification_seq: opt(row, 28)?,
                        classification_digest: row.get(29)?,
                        removed_seq: opt(row, 30)?,
                    },
                });
            }
            Ok(scanned)
        })
    }

    /// Every calendar event with what it last published, by calendar and UID.
    pub fn feed_event_scan(&self) -> Result<Vec<ScannedEvent>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT e.calendar_id,e.uid,e.title,e.start_day,e.end_day,e.removed_at,
                   p.event_seq,p.event_digest,p.removed_seq
                 FROM calendar_events e
                 LEFT JOIN feed_events p ON p.calendar_id=e.calendar_id AND p.uid=e.uid
                 ORDER BY e.calendar_id,e.start_day,e.uid",
            )?
            .query_map([], |row| {
                Ok(ScannedEvent {
                    event: Event {
                        calendar_id: row.get(0)?,
                        uid: row.get(1)?,
                        title: row.get(2)?,
                        start_day: row.get(3)?,
                        end_day: row.get(4)?,
                    },
                    removed_at: row.get(5)?,
                    published: PublishedEvent {
                        event_seq: opt(row, 6)?,
                        event_digest: row.get(7)?,
                        removed_seq: opt(row, 8)?,
                    },
                })
            })?
            .collect()
        })
    }

    /// The menu weeks a client still shows at `now` (RFC 3339 UTC): those that
    /// ended at most 7 days before today in Korea (docs/FEED.md).
    pub fn feed_meal_scan(&self, now: &str) -> Result<Vec<ScannedMeal>, String> {
        let rows: Vec<(String, Option<u64>, Option<String>)> = self.with(|c| {
            c.prepare(
                "SELECT w.record,p.meal_seq,p.meal_digest FROM meal_weeks w
                 LEFT JOIN feed_meals p ON p.week=w.week
                 WHERE w.week>=date(?1,'+9 hours','-13 days') ORDER BY w.week",
            )?
            .query_map([now], |row| Ok((row.get(0)?, opt(row, 1)?, row.get(2)?)))?
            .collect()
        })?;
        rows.into_iter()
            .map(|(record, seq, digest)| {
                Ok(ScannedMeal {
                    week: serde_json::from_str(&record).map_err(|e| e.to_string())?,
                    published: seq.zip(digest),
                })
            })
            .collect()
    }

    /// Calendar health for the manifest.
    pub fn feed_calendars(&self) -> Result<Vec<CalendarHealth>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT calendar_id,last_success_at,consecutive_failures
                 FROM calendars ORDER BY calendar_id",
            )?
            .query_map([], |row| {
                Ok(CalendarHealth {
                    calendar_id: row.get(0)?,
                    last_success_at: row.get(1)?,
                    consecutive_failures: row.get(2)?,
                })
            })?
            .collect()
        })
    }

    /// Files the manifest lists (`listed`) or every file the state knows.
    pub fn feed_files(&self, listed: bool) -> Result<Vec<FileRow>, String> {
        let filter = if listed {
            "WHERE retired_at IS NULL"
        } else {
            ""
        };
        self.with(|c| {
            c.prepare(&format!(
                "SELECT {FILE_COLUMNS} FROM feed_files {filter} ORDER BY from_seq,role"
            ))?
            .query_map([], file_row)?
            .collect()
        })
    }

    /// Files retired at least a day before `now`, which may be deleted.
    pub fn feed_expired(&self, now: &str) -> Result<Vec<FileRow>, String> {
        self.with(|c| {
            c.prepare(&format!(
                "SELECT {FILE_COLUMNS} FROM feed_files
                 WHERE retired_at <= strftime('%Y-%m-%dT%H:%M:%fZ',?1,'-1 day')"
            ))?
            .query_map([now], file_row)?
            .collect()
        })
    }

    pub fn feed_forget(&self, records_path: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "DELETE FROM feed_files WHERE records_path=?1",
                [records_path],
            )
        })
        .map(drop)
    }

    /// The stored run records from `from` to `to`.
    pub fn feed_records(&self, from: u64, to: u64) -> Result<Vec<StoredRecord>, String> {
        self.with(|c| {
            c.prepare("SELECT seq,record,vector FROM feed_records WHERE seq BETWEEN ?1 AND ?2 ORDER BY seq")?
                .query_map(params![from as i64, to as i64], |row| {
                    Ok((int(row, 0)?, row.get(1)?, row.get(2)?))
                })?
                .collect()
        })
    }

    /// Board health for the manifest.
    pub fn feed_boards(&self) -> Result<Vec<BoardHealth>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT source_id,last_success_at,consecutive_failures,history_complete
                 FROM boards ORDER BY source_id",
            )?
            .query_map([], |row| {
                Ok(BoardHealth {
                    source_id: row.get(0)?,
                    last_success_at: row.get(1)?,
                    consecutive_failures: row.get(2)?,
                    history_complete: row.get(3)?,
                })
            })?
            .collect()
        })
    }

    /// The latest drift check's state.
    pub fn canary_latest(&self) -> Result<Option<String>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT state FROM canary_checks ORDER BY day DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
        })
    }

    /// Commits a run's publish. Daily segments older than 30 days are retired
    /// here too, so the manifest written after the state upload drops them.
    pub fn feed_commit(&self, commit: &Commit, now: &str) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            transaction.execute("UPDATE feed SET next_seq=?1", [commit.next_seq as i64])?;
            for (notice_id, p) in &commit.published {
                transaction.execute(
                    "INSERT INTO feed_notices(notice_id,notice_seq,notice_digest,embedding_seq,
                       embedding_digest,classification_seq,classification_digest,removed_seq)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                     ON CONFLICT(notice_id) DO UPDATE SET notice_seq=excluded.notice_seq,
                       notice_digest=excluded.notice_digest,embedding_seq=excluded.embedding_seq,
                       embedding_digest=excluded.embedding_digest,
                       classification_seq=excluded.classification_seq,
                       classification_digest=excluded.classification_digest,
                       removed_seq=excluded.removed_seq",
                    params![
                        notice_id,
                        sql(p.notice_seq),
                        p.notice_digest,
                        sql(p.embedding_seq),
                        p.embedding_digest,
                        sql(p.classification_seq),
                        p.classification_digest,
                        sql(p.removed_seq)
                    ],
                )?;
            }
            for ((calendar_id, uid), p) in &commit.published_events {
                transaction.execute(
                    "INSERT INTO feed_events(calendar_id,uid,event_seq,event_digest,removed_seq)
                     VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(calendar_id,uid) DO UPDATE SET event_seq=excluded.event_seq,
                       event_digest=excluded.event_digest,removed_seq=excluded.removed_seq",
                    params![
                        calendar_id,
                        uid,
                        sql(p.event_seq),
                        p.event_digest,
                        sql(p.removed_seq)
                    ],
                )?;
            }
            for (week, seq, digest) in &commit.published_meals {
                transaction.execute(
                    "INSERT INTO feed_meals(week,meal_seq,meal_digest) VALUES(?1,?2,?3)
                     ON CONFLICT(week) DO UPDATE SET meal_seq=excluded.meal_seq,
                       meal_digest=excluded.meal_digest",
                    params![week, *seq as i64, digest],
                )?;
            }
            for (seq, record, vector) in &commit.records {
                transaction.execute(
                    "INSERT INTO feed_records(seq,record,vector) VALUES(?1,?2,?3)",
                    params![*seq as i64, record, vector],
                )?;
            }
            if let Some(through) = commit.compacted_through {
                transaction.execute("DELETE FROM feed_records WHERE seq<=?1", [through as i64])?;
            }
            for path in &commit.retire {
                transaction.execute(
                    "UPDATE feed_files SET retired_at=?2 WHERE records_path=?1",
                    params![path, now],
                )?;
            }
            if let Some((path, day)) = &commit.redate {
                transaction.execute(
                    "UPDATE feed_files SET day=?2 WHERE records_path=?1",
                    params![path, day],
                )?;
            }
            for file in &commit.files {
                let (vectors_path, vectors_sha256, vectors_size) = match &file.vectors {
                    Some((path, sha256, size)) => (Some(path), Some(sha256), Some(*size as i64)),
                    None => (None, None, None),
                };
                transaction.execute(
                    &format!("INSERT INTO feed_files({FILE_COLUMNS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"),
                    params![
                        file.records_path,
                        file.role,
                        file.day,
                        file.from_seq as i64,
                        file.to_seq as i64,
                        file.records_sha256,
                        file.records_size as i64,
                        vectors_path,
                        vectors_sha256,
                        vectors_size
                    ],
                )?;
            }
            transaction.execute(
                "UPDATE feed_files SET retired_at=?1
                 WHERE role='daily' AND retired_at IS NULL AND day<date(?1,'-30 days')",
                [now],
            )?;
            transaction.commit()
        })
    }
}
