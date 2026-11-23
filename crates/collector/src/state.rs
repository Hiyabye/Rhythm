//! The collector's SQLite state: board checkpoints, the current revision of
//! each notice and skipped notices. Every method takes the connection lock for
//! one statement or transaction only, so no lock is held across network work.
use rhythm_embed::Classified;
use rhythm_sources::{
    dates::{self, Kind, EXTRACTOR_VERSION},
    model::{ArticleInput, ListingEntry},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{collections::HashSet, path::Path, sync::Mutex};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
/// Ordered schema migrations; index `n` upgrades `user_version` n to n+1.
const MIGRATIONS: [&str; 6] = [
    include_str!("../migrations/001_initial.sql"),
    include_str!("../migrations/002_notice_dates.sql"),
    include_str!("../migrations/003_deletions.sql"),
    include_str!("../migrations/004_embeddings.sql"),
    include_str!("../migrations/005_classifications.sql"),
    include_str!("../migrations/006_feed.sql"),
];
/// A second "not found" removes a notice only this long after the first.
const MISSING_CONFIRM: &str = "-1 day";
/// The audit re-fetches a notice about this often (user decision).
const AUDIT_INTERVAL: &str = "-7 days";
/// Notices published this recently are audited on boards without a complete listing.
const AUDIT_WINDOW: &str = "-90 days";
/// A skipped notice is retried at most once a day, this many times per collector version.
const SKIPPED_INTERVAL: &str = "-1 day";
pub const SKIPPED_ATTEMPTS: i64 = 7;

/// A UTC time in the stored form, `modifier` from now (`-1 day`).
fn ago(modifier: &str) -> String {
    format!("strftime('%Y-%m-%dT%H:%M:%fZ','now','{modifier}')")
}

/// One board's checkpoint and the counters of the current attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub source_id: String,
    pub status: String,
    pub next_page: u32,
    pub history_complete: bool,
    pub discovered: u64,
    pub archived: u64,
    pub failed: u64,
    pub last_error: Option<String>,
    pub consecutive_failures: u64,
}

pub type Validators = (Option<String>, Option<String>);

pub struct State {
    connection: Mutex<Connection>,
}

impl State {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::from_connection(Connection::open(path).map_err(|e| e.to_string())?)
    }
    pub fn open_in_memory() -> Result<Self, String> {
        Self::from_connection(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, String> {
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        // A state written by a newer collector must not be rewritten by an older one.
        if version < 0 || version as usize > MIGRATIONS.len() {
            return Err(format!("unsupported collector state version {version}"));
        }
        for migration in &MIGRATIONS[version as usize..] {
            let transaction = connection.transaction().map_err(|e| e.to_string())?;
            transaction
                .execute_batch(migration)
                .map_err(|e| e.to_string())?;
            transaction.commit().map_err(|e| e.to_string())?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    pub(crate) fn with<T>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "collector state is unavailable".to_string())?;
        operation(&connection).map_err(|e| e.to_string())
    }

    /// Writes a compact, consistent copy of the state to a new file and checks
    /// it, so a damaged copy never becomes the next authoritative version.
    pub fn vacuum_into(&self, path: &Path) -> Result<(), String> {
        let target = path.to_str().ok_or("state snapshot path is not UTF-8")?;
        self.with(|c| c.execute("VACUUM INTO ?1", [target]))?;
        let check: String = Connection::open(path)
            .and_then(|copy| copy.query_row("PRAGMA quick_check", [], |row| row.get(0)))
            .map_err(|e| e.to_string())?;
        if check != "ok" {
            return Err(format!(
                "state snapshot failed its integrity check: {check}"
            ));
        }
        Ok(())
    }

    /// The board's row, created on first use.
    pub fn board(&self, source_id: &str) -> Result<Board, String> {
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO boards(source_id) VALUES(?1)",
                [source_id],
            )?;
            c.query_row(
                "SELECT source_id,status,next_page,history_complete,discovered,archived,failed,
                        last_error,consecutive_failures
                 FROM boards WHERE source_id=?1",
                [source_id],
                |row| {
                    Ok(Board {
                        source_id: row.get(0)?,
                        status: row.get(1)?,
                        next_page: row.get(2)?,
                        history_complete: row.get(3)?,
                        discovered: row.get::<_, i64>(4)? as u64,
                        archived: row.get::<_, i64>(5)? as u64,
                        failed: row.get::<_, i64>(6)? as u64,
                        last_error: row.get(7)?,
                        consecutive_failures: row.get::<_, i64>(8)? as u64,
                    })
                },
            )
        })
    }
    /// Stores the board's checkpoint and counters. `success` records a completed
    /// pass; a failed one counts toward the board's health.
    pub fn progress(
        &self,
        board: &Board,
        status: &str,
        error: Option<&str>,
        success: bool,
    ) -> Result<(), String> {
        let counters = [board.discovered, board.archived, board.failed];
        if board.next_page == 0 || counters.iter().any(|n| *n > i64::MAX as u64) {
            return Err("invalid board progress".into());
        }
        self.with(|c| {
            c.execute(
                &format!(
                    "UPDATE boards SET status=?2,next_page=?3,history_complete=?4,discovered=?5,
                       archived=?6,failed=?7,last_error=?8,
                       last_attempt_at=CASE WHEN ?2='running' AND status<>'running' THEN {NOW}
                                       ELSE last_attempt_at END,
                       last_success_at=CASE WHEN ?9 THEN {NOW} ELSE last_success_at END,
                       consecutive_failures=CASE WHEN ?9 THEN 0
                         WHEN ?2='failed' THEN consecutive_failures+1
                         ELSE consecutive_failures END
                     WHERE source_id=?1"
                ),
                params![
                    board.source_id,
                    status,
                    board.next_page,
                    board.history_complete,
                    board.discovered as i64,
                    board.archived as i64,
                    board.failed as i64,
                    error,
                    success
                ],
            )
        })
        .and_then(|changed| match changed {
            1 => Ok(()),
            _ => Err(format!("unknown board {}", board.source_id)),
        })
    }
    /// False for a board the state has never seen.
    pub fn seed_complete(&self, source_id: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT seed_complete FROM boards WHERE source_id=?1",
                [source_id],
                |row| row.get(0),
            )
            .optional()
        })
        .map(|seeded| seeded.unwrap_or(false))
    }
    pub fn mark_seed_complete(&self, source_id: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "UPDATE boards SET seed_complete=1 WHERE source_id=?1",
                [source_id],
            )
        })
        .map(drop)
    }

    pub fn notice_id(&self, source_id: &str, external_id: &str) -> Result<Option<i64>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT id FROM notices WHERE source_id=?1 AND external_id=?2",
                params![source_id, external_id],
                |row| row.get(0),
            )
            .optional()
        })
    }
    pub fn validators(
        &self,
        source_id: &str,
        external_id: &str,
    ) -> Result<Option<Validators>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT etag,last_modified FROM notices WHERE source_id=?1 AND external_id=?2",
                params![source_id, external_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        })
    }
    /// True when the stored notice already carries the listing's own change time.
    pub fn listing_unchanged(
        &self,
        source_id: &str,
        external_id: &str,
        modified_at: &str,
    ) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                // A removed notice listed again is fetched, which restores it.
                "SELECT modified_at FROM notices
                 WHERE source_id=?1 AND external_id=?2 AND removed_at IS NULL",
                params![source_id, external_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
        })
        .map(|stored| stored.flatten().is_some_and(|value| value == modified_at))
    }
    /// True when the stored notice already carries the title its list row
    /// shows (k2web, whose articles have no change marker; #80 step 3). A
    /// notice reported missing or removed is fetched again instead.
    pub fn listed_title_unchanged(
        &self,
        source_id: &str,
        external_id: &str,
        title: &str,
    ) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT title FROM notices WHERE source_id=?1 AND external_id=?2
                 AND missing_since IS NULL AND removed_at IS NULL",
                params![source_id, external_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
        })
        .map(|stored| stored.is_some_and(|value| value == title))
    }
    pub fn checked(&self, source_id: &str, external_id: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                &format!(
                    "UPDATE notices SET checked_at={NOW},missing_since=NULL,removed_at=NULL
                     WHERE source_id=?1 AND external_id=?2"
                ),
                params![source_id, external_id],
            )
        })
        .map(drop)
    }
    /// Readable text or an image, as the app's reader would show it.
    pub fn has_readable_content(&self, notice_id: i64) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT body_text<>'' OR EXISTS(
                    SELECT 1 FROM json_each(resources) WHERE json_extract(value,'$.kind')='image'
                 ) FROM notices WHERE id=?1",
                [notice_id],
                |row| row.get(0),
            )
        })
    }
    /// Stores the article as the notice's current revision. Returns its id and
    /// whether it is new; an unchanged revision only refreshes its validators.
    pub fn upsert(
        &self,
        source_id: &str,
        article: &ArticleInput,
        historical: bool,
    ) -> Result<(i64, bool), String> {
        validate(source_id, article)?;
        let hash = revision_hash(article)?;
        let resources = resources_json(article)?;
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            let existing: Option<(i64, String)> = transaction
                .query_row(
                    "SELECT id,revision_hash FROM notices WHERE source_id=?1 AND external_id=?2",
                    params![source_id, article.external_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let inserted = existing.is_none();
            let id = match existing {
                Some((id, old)) if old == hash => {
                    transaction.execute(
                        &format!(
                            "UPDATE notices SET checked_at={NOW},etag=?2,last_modified=?3,modified_at=?4,
                               missing_since=NULL,removed_at=NULL
                             WHERE id=?1"
                        ),
                        params![id, article.etag, article.last_modified, article.modified_at],
                    )?;
                    id
                }
                Some((id, _)) => {
                    transaction.execute(
                        &format!(
                            "UPDATE notices SET canonical_url=?2,title=?3,author=?4,category=?5,
                               published_at=?6,modified_at=?7,body_state=?8,body_text=?9,
                               resources=?10,revision_hash=?11,etag=?12,last_modified=?13,
                               revised_at={NOW},checked_at={NOW},missing_since=NULL,removed_at=NULL
                             WHERE id=?1"
                        ),
                        params![
                            id,
                            article.canonical_url,
                            article.title,
                            article.author,
                            article.category,
                            article.published_at,
                            article.modified_at,
                            body_state(article),
                            article.body_text,
                            resources,
                            hash,
                            article.etag,
                            article.last_modified
                        ],
                    )?;
                    replace_dates(&transaction, id)?;
                    id
                }
                None => {
                    transaction.execute(
                        &format!(
                            "INSERT INTO notices(source_id,external_id,canonical_url,title,author,
                               category,published_at,modified_at,body_state,body_text,resources,
                               revision_hash,etag,last_modified,historical,first_seen_at,
                               revised_at,checked_at)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,{NOW},{NOW},{NOW})"
                        ),
                        params![
                            source_id,
                            article.external_id,
                            article.canonical_url,
                            article.title,
                            article.author,
                            article.category,
                            article.published_at,
                            article.modified_at,
                            body_state(article),
                            article.body_text,
                            resources,
                            hash,
                            article.etag,
                            article.last_modified,
                            historical
                        ],
                    )?;
                    let id = transaction.last_insert_rowid();
                    replace_dates(&transaction, id)?;
                    id
                }
            };
            transaction.commit()?;
            Ok((id, inserted))
        })
    }
    /// Extracts the dates of every notice not yet extracted by the current
    /// extractor version (notices stored before migration 002 or before a
    /// version bump) and returns how many it extracted.
    pub fn refresh_dates(&self) -> Result<usize, String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            let stale: Vec<i64> = transaction
                .prepare("SELECT id FROM notices WHERE dates_version IS NOT ?1")?
                .query_map([EXTRACTOR_VERSION], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in &stale {
                replace_dates(&transaction, *id)?;
            }
            transaction.commit()?;
            Ok(stale.len())
        })
    }
    pub fn record_skipped(
        &self,
        source_id: &str,
        entry: &ListingEntry,
        historical: bool,
        reason: &str,
        response_sha256: &str,
    ) -> Result<(), String> {
        if !valid_id(&entry.external_id)
            || reason.trim().is_empty()
            || reason.len() > 1024
            || response_sha256.len() != 64
        {
            return Err("invalid skipped notice".into());
        }
        self.with(|c| {
            c.execute(
                &format!(
                    "INSERT INTO skipped(source_id,external_id,url,historical,reason,response_sha256,
                       recorded_at,attempts,collector_version)
                     VALUES(?1,?2,?3,?4,?5,?6,{NOW},1,?7)
                     ON CONFLICT(source_id,external_id) DO UPDATE SET
                       url=excluded.url,reason=excluded.reason,
                       response_sha256=excluded.response_sha256,recorded_at=excluded.recorded_at,
                       attempts=CASE WHEN collector_version=excluded.collector_version
                         THEN attempts+1 ELSE 1 END,
                       collector_version=excluded.collector_version"
                ),
                params![
                    source_id,
                    entry.external_id,
                    entry.url,
                    historical,
                    reason,
                    response_sha256,
                    crate::VERSION
                ],
            )
        })
        .map(drop)
    }
    pub fn clear_skipped(&self, source_id: &str, external_id: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "DELETE FROM skipped WHERE source_id=?1 AND external_id=?2",
                params![source_id, external_id],
            )
        })
        .map(drop)
    }

    /// Skipped notices due for a retry: last tried a day ago or more, with
    /// attempts left in this collector version. Returns each listing entry and
    /// whether it was found by the history import.
    pub fn skipped_due(
        &self,
        source_id: &str,
        limit: usize,
    ) -> Result<Vec<(ListingEntry, bool)>, String> {
        self.with(|c| {
            c.prepare(&format!(
                "SELECT external_id,url,historical FROM skipped
                 WHERE source_id=?1 AND recorded_at<={}
                   AND (attempts<?2 OR collector_version<>?3)
                 ORDER BY recorded_at LIMIT ?4",
                ago(SKIPPED_INTERVAL)
            ))?
            .query_map(
                params![source_id, SKIPPED_ATTEMPTS, crate::VERSION, limit as i64],
                |row| Ok((entry(row.get(0)?, row.get(1)?), row.get(2)?)),
            )?
            .collect()
        })
    }

    /// Notices for the deletion audit, at most `limit`. A notice reported
    /// missing is re-checked once a day has passed; others about weekly, the
    /// longest unchecked first. `listed` is the board's complete current
    /// listing (the mechanical sitemap): then only notices absent from it are
    /// audited, at any age. Otherwise notices published within 90 days are.
    pub fn audit_candidates(
        &self,
        source_id: &str,
        listed: Option<&HashSet<String>>,
        limit: usize,
    ) -> Result<Vec<ListingEntry>, String> {
        let rows: Vec<(ListingEntry, bool)> = self.with(|c| {
            c.prepare(&format!(
                "SELECT external_id,canonical_url,missing_since IS NOT NULL FROM notices
                 WHERE source_id=?1 AND removed_at IS NULL
                   AND (missing_since<={confirm}
                     OR (missing_since IS NULL AND checked_at<={interval}
                       AND (?2 OR published_at>=date('now','{window}'))))
                 ORDER BY missing_since IS NULL, missing_since, checked_at",
                confirm = ago(MISSING_CONFIRM),
                interval = ago(AUDIT_INTERVAL),
                window = AUDIT_WINDOW,
            ))?
            .query_map(params![source_id, listed.is_some()], |row| {
                Ok((entry(row.get(0)?, row.get(1)?), row.get(2)?))
            })?
            .collect()
        })?;
        Ok(rows
            .into_iter()
            // A listed notice is present; one reported missing is re-checked
            // anyway so its report is cleared or confirmed.
            .filter(|(entry, missing)| {
                *missing || listed.is_none_or(|listed| !listed.contains(&entry.external_id))
            })
            .map(|(entry, _)| entry)
            .take(limit)
            .collect())
    }

    /// Records a definitive "not found" from the source. The first since the
    /// notice was last fetched marks it missing; one at least a day later
    /// removes it. Returns whether this answer removed it.
    pub fn mark_missing(&self, source_id: &str, external_id: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                &format!(
                    "UPDATE notices SET
                       removed_at=CASE WHEN missing_since<={} THEN {NOW} END,
                       missing_since=COALESCE(missing_since,{NOW})
                     WHERE source_id=?1 AND external_id=?2 AND removed_at IS NULL
                     RETURNING removed_at IS NOT NULL",
                    ago(MISSING_CONFIRM)
                ),
                params![source_id, external_id],
                |row| row.get(0),
            )
            .optional()
        })
        .map(|removed| removed.unwrap_or(false))
    }
}

/// A notice whose current revision needs a vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unembedded {
    pub id: i64,
    pub revision_hash: String,
    pub title: String,
    pub body_text: String,
}

/// A notice whose current vector has no classification yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Unclassified {
    pub id: i64,
    pub revision_hash: String,
    pub vector: Vec<u8>,
}

impl State {
    /// Notices without a vector of their current revision in `scope`.
    /// `fresh` selects those embedded every run: new notices (not found by the
    /// history import) and revisions of notices already embedded in this
    /// scope. Otherwise the backfill order: never-embedded notices newest
    /// first, then vectors of another scope (#166 D15). Removed notices are
    /// never embedded.
    pub fn unembedded(
        &self,
        scope: &str,
        fresh: bool,
        limit: usize,
    ) -> Result<Vec<Unembedded>, String> {
        let (filter, order) = if fresh {
            (
                "(e.notice_id IS NULL AND n.historical=0)
                 OR (e.scope=?1 AND e.revision_hash<>n.revision_hash)",
                "n.id",
            )
        } else {
            (
                "e.notice_id IS NULL OR e.scope<>?1 OR e.revision_hash<>n.revision_hash",
                "e.notice_id IS NOT NULL, n.published_at DESC, n.id DESC",
            )
        };
        self.with(|c| {
            c.prepare(&format!(
                "SELECT n.id,n.revision_hash,n.title,n.body_text FROM notices n
                 LEFT JOIN embeddings e ON e.notice_id=n.id
                 WHERE n.removed_at IS NULL AND ({filter}) ORDER BY {order} LIMIT ?2"
            ))?
            .query_map(params![scope, limit as i64], |row| {
                Ok(Unembedded {
                    id: row.get(0)?,
                    revision_hash: row.get(1)?,
                    title: row.get(2)?,
                    body_text: row.get(3)?,
                })
            })?
            .collect()
        })
    }

    /// Stores one batch of vectors with the Neurons it used, in one transaction.
    pub fn store_embeddings(
        &self,
        scope: &str,
        day: &str,
        vectors: &[(&Unembedded, Vec<u8>)],
        neurons: f64,
    ) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            for (notice, vector) in vectors {
                transaction.execute(
                    &format!(
                        "INSERT INTO embeddings(notice_id,revision_hash,scope,vector,embedded_at)
                         VALUES(?1,?2,?3,?4,{NOW})
                         ON CONFLICT(notice_id) DO UPDATE SET revision_hash=excluded.revision_hash,
                           scope=excluded.scope,vector=excluded.vector,embedded_at=excluded.embedded_at"
                    ),
                    params![notice.id, notice.revision_hash, scope, vector],
                )?;
            }
            add_neurons(&transaction, day, neurons)?;
            transaction.commit()
        })
    }

    pub fn add_neurons(&self, day: &str, neurons: f64) -> Result<(), String> {
        self.with(|c| add_neurons(c, day, neurons))
    }
    /// Neurons used on the UTC day.
    pub fn neurons(&self, day: &str) -> Result<f64, String> {
        self.with(|c| {
            c.query_row("SELECT neurons FROM ai_usage WHERE day=?1", [day], |row| {
                row.get(0)
            })
            .optional()
        })
        .map(Option::unwrap_or_default)
    }
    pub fn record_ai_error(&self, day: &str, error: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "INSERT INTO ai_usage(day,last_error) VALUES(?1,?2)
                 ON CONFLICT(day) DO UPDATE SET last_error=excluded.last_error",
                params![day, error],
            )
        })
        .map(drop)
    }

    /// The canary reference: each text's SHA-256 and f16 vector, by ordinal.
    pub fn canary_reference(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        self.with(|c| {
            c.prepare("SELECT text_sha256,vector FROM canary ORDER BY ordinal")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
    }
    /// The state of the day's canary check, if one ran.
    pub fn canary_check(&self, day: &str) -> Result<Option<String>, String> {
        self.with(|c| {
            c.query_row(
                "SELECT state FROM canary_checks WHERE day=?1",
                [day],
                |row| row.get(0),
            )
            .optional()
        })
    }
    /// Whether the latest check stopped new vectors. It stays so, since no
    /// later check runs, until `reset_canary`.
    pub fn canary_stopped(&self) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT state='stopped' FROM canary_checks ORDER BY day DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
        })
        .map(|stopped| stopped.unwrap_or(false))
    }
    /// Records the day's check; `reference` replaces the stored vectors.
    pub fn record_canary(
        &self,
        day: &str,
        state: &str,
        min_cosine: Option<f64>,
        reference: Option<&[(String, Vec<u8>)]>,
    ) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            if let Some(reference) = reference {
                transaction.execute("DELETE FROM canary", [])?;
                for (ordinal, (sha256, vector)) in reference.iter().enumerate() {
                    transaction.execute(
                        &format!(
                            "INSERT INTO canary(ordinal,text_sha256,vector,recorded_at)
                             VALUES(?1,?2,?3,{NOW})"
                        ),
                        params![ordinal as i64, sha256, vector],
                    )?;
                }
            }
            transaction.execute(
                &format!(
                    "INSERT INTO canary_checks(day,state,min_cosine,checked_at) VALUES(?1,?2,?3,{NOW})
                     ON CONFLICT(day) DO UPDATE SET state=excluded.state,
                       min_cosine=excluded.min_cosine,checked_at=excluded.checked_at"
                ),
                params![day, state, min_cosine],
            )?;
            transaction.commit()
        })
    }
    /// The maintainer's decision after a drift stop: forget the reference and
    /// the checks, so the next run records a new reference.
    /// The cached prototypes go too: they were embedded by the model the
    /// reset no longer trusts.
    pub fn reset_canary(&self) -> Result<(), String> {
        self.with(|c| {
            c.execute_batch(
                "DELETE FROM canary; DELETE FROM canary_checks; DELETE FROM prototypes;",
            )
        })
    }

    /// The f16 prototype vectors of `scope`, by ordinal.
    pub fn prototypes(&self, scope: &str) -> Result<Vec<Vec<u8>>, String> {
        self.with(|c| {
            c.prepare("SELECT vector FROM prototypes WHERE scope=?1 ORDER BY ordinal")?
                .query_map([scope], |row| row.get(0))?
                .collect()
        })
    }
    /// Replaces every cached prototype with those of `scope`.
    pub fn store_prototypes(&self, scope: &str, vectors: &[Vec<u8>]) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            transaction.execute("DELETE FROM prototypes", [])?;
            for (ordinal, vector) in vectors.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO prototypes(scope,ordinal,vector) VALUES(?1,?2,?3)",
                    params![scope, ordinal as i64, vector],
                )?;
            }
            transaction.commit()
        })
    }

    /// Current vectors in `scope` whose classification is missing or was made
    /// from another vector. Removed notices are skipped.
    pub fn unclassified(&self, scope: &str, limit: usize) -> Result<Vec<Unclassified>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT n.id,n.revision_hash,e.vector FROM notices n
                 JOIN embeddings e ON e.notice_id=n.id
                 LEFT JOIN classifications k ON k.notice_id=n.id
                 WHERE n.removed_at IS NULL AND e.scope=?1 AND e.revision_hash=n.revision_hash
                   AND (k.notice_id IS NULL OR k.scope<>?1 OR k.revision_hash<>n.revision_hash)
                 ORDER BY n.id LIMIT ?2",
            )?
            .query_map(params![scope, limit as i64], |row| {
                Ok(Unclassified {
                    id: row.get(0)?,
                    revision_hash: row.get(1)?,
                    vector: row.get(2)?,
                })
            })?
            .collect()
        })
    }
    pub fn store_classifications(
        &self,
        scope: &str,
        rows: &[(&Unclassified, Classified)],
    ) -> Result<(), String> {
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            for (notice, classified) in rows {
                transaction.execute(
                    &format!(
                        "INSERT INTO classifications(notice_id,revision_hash,scope,category,score,
                           margin,low_confidence,classified_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,{NOW})
                         ON CONFLICT(notice_id) DO UPDATE SET revision_hash=excluded.revision_hash,
                           scope=excluded.scope,category=excluded.category,score=excluded.score,
                           margin=excluded.margin,low_confidence=excluded.low_confidence,
                           classified_at=excluded.classified_at"
                    ),
                    params![
                        notice.id,
                        notice.revision_hash,
                        scope,
                        classified.category,
                        f64::from(classified.score),
                        f64::from(classified.margin),
                        classified.low_confidence
                    ],
                )?;
            }
            transaction.commit()
        })
    }
}

fn add_neurons(connection: &Connection, day: &str, neurons: f64) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO ai_usage(day,neurons) VALUES(?1,?2)
             ON CONFLICT(day) DO UPDATE SET neurons=neurons+excluded.neurons",
            params![day, neurons],
        )
        .map(drop)
}

fn entry(external_id: String, url: String) -> ListingEntry {
    ListingEntry {
        external_id,
        url,
        pinned: false,
        modified_at: None,
        title: None,
    }
}

/// Replaces a notice's dates with those of its current title and body.
fn replace_dates(connection: &Connection, id: i64) -> rusqlite::Result<()> {
    let (title, body, published): (String, String, String) = connection.query_row(
        "SELECT title,body_text,published_at FROM notices WHERE id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    connection.execute("DELETE FROM notice_dates WHERE notice_id=?1", [id])?;
    for (ordinal, found) in dates::extract_notice(&title, &body, &published)
        .iter()
        .enumerate()
    {
        let kind = match found.kind {
            Kind::Deadline => "deadline",
            Kind::Start => "start",
            Kind::Date => "date",
        };
        connection.execute(
            "INSERT INTO notice_dates(notice_id,ordinal,day,time,end_day,end_time,label,kind)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                ordinal as i64,
                found.day,
                found.time,
                found.end_day,
                found.end_time,
                found.label,
                kind
            ],
        )?;
    }
    connection.execute(
        "UPDATE notices SET dates_version=?2 WHERE id=?1",
        params![id, EXTRACTOR_VERSION],
    )?;
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn https_url(value: &str) -> bool {
    value.len() <= 8192
        && url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}
/// The same bounds the app applies before archiving; a parser regression must
/// fail here instead of publishing a malformed notice.
fn validate(source_id: &str, article: &ArticleInput) -> Result<(), String> {
    let empty_body = article.body_text.trim().is_empty() && article.body_html.trim().is_empty();
    // The approved metadata-only exception exists for the main site's boards alone.
    let invalid_unsupported_body = article.unsupported_body
        && (rhythm_sources::parser::main_board(source_id).is_none()
            || !empty_body
            || article.resources.iter().any(|r| r.kind == "image"));
    if !valid_id(&article.external_id)
        || article.title.trim().is_empty()
        || article.title.len() > 16_384
        || article.body_text.len() > 8 * 1024 * 1024
        || !https_url(&article.canonical_url)
        || article.published_at.trim().is_empty()
        || article.resources.len() > 256
        || invalid_unsupported_body
        || (empty_body && article.resources.is_empty() && !article.unsupported_body)
    {
        return Err("invalid notice identity, body or size".into());
    }
    for resource in &article.resources {
        // Kept as a reference only; the collector never fetches resources.
        let scheme_ok = url::Url::parse(&resource.url)
            .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some());
        if !scheme_ok
            || resource.url.len() > 8192
            || resource.filename.len() > 4096
            || !["image", "attachment"].contains(&resource.kind.as_str())
        {
            return Err("invalid resource reference".into());
        }
    }
    Ok(())
}
fn body_state(article: &ArticleInput) -> &'static str {
    if article.unsupported_body {
        "unsupported"
    } else {
        "normal"
    }
}
#[derive(Serialize)]
struct Resource<'a> {
    url: &'a str,
    filename: &'a str,
    kind: &'a str,
}
fn resources_json(article: &ArticleInput) -> Result<String, String> {
    let resources: Vec<_> = article
        .resources
        .iter()
        .map(|r| Resource {
            url: &r.url,
            filename: &r.filename,
            kind: &r.kind,
        })
        .collect();
    serde_json::to_string(&resources).map_err(|e| e.to_string())
}
/// Identity of the published revision (`rhythm_feed::revision_hash`). Unlike
/// the app's content hash it leaves out the HTML, which the collector does not
/// keep: a markup-only change is not a new revision.
fn revision_hash(article: &ArticleInput) -> Result<String, String> {
    let resources: Vec<_> = article
        .resources
        .iter()
        .map(|r| (r.url.as_str(), r.filename.as_str(), r.kind.as_str()))
        .collect();
    Ok(rhythm_feed::revision_hash(&rhythm_feed::Revision {
        canonical_url: &article.canonical_url,
        title: &article.title,
        author: article.author.as_deref(),
        category: article.category.as_deref(),
        published_at: &article.published_at,
        body_text: &article.body_text,
        resources: &resources,
    }))
}

mod feed;
pub use feed::{Commit, FileRow, Published, Scanned, StoredRecord};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopened_state_keeps_checkpoints_and_rejects_a_newer_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let state = State::open(&path).unwrap();
        let mut board = state.board("sogang-academic").unwrap();
        board.next_page = 4;
        board.history_complete = true;
        state.progress(&board, "completed", None, true).unwrap();
        state.mark_seed_complete("sogang-academic").unwrap();
        drop(state);

        let state = State::open(&path).unwrap();
        let board = state.board("sogang-academic").unwrap();
        assert_eq!((board.next_page, board.history_complete), (4, true));
        assert!(state.seed_complete("sogang-academic").unwrap());
        state
            .with(|c| c.execute_batch("PRAGMA user_version=7;"))
            .unwrap();
        drop(state);
        assert!(State::open(&path).err().unwrap().contains("version 7"));
    }

    #[test]
    fn failed_passes_count_toward_health_until_a_success() {
        let state = State::open_in_memory().unwrap();
        let board = state.board("computing-academic").unwrap();
        state.progress(&board, "failed", Some("x"), false).unwrap();
        state.progress(&board, "failed", Some("x"), false).unwrap();
        assert_eq!(
            state
                .board("computing-academic")
                .unwrap()
                .consecutive_failures,
            2
        );
        state.progress(&board, "completed", None, true).unwrap();
        assert_eq!(
            state
                .board("computing-academic")
                .unwrap()
                .consecutive_failures,
            0
        );
    }

    fn article(title: &str, body: &str) -> ArticleInput {
        ArticleInput {
            external_id: "100".into(),
            canonical_url: "https://www.sogang.ac.kr/ko/detail/100".into(),
            title: title.into(),
            author: None,
            category: None,
            published_at: "2026-11-17".into(),
            modified_at: None,
            original: String::new(),
            unsupported_body: false,
            body_html: format!("<p>{body}</p>"),
            body_text: body.into(),
            resources: Vec::new(),
            etag: None,
            last_modified: None,
        }
    }

    type Row = (String, Option<String>, String);
    fn dates(state: &State, id: i64) -> (Option<i64>, Vec<Row>) {
        state
            .with(|c| {
                let version = c.query_row(
                    "SELECT dates_version FROM notices WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                let rows = c
                    .prepare("SELECT day,time,kind FROM notice_dates WHERE notice_id=?1 ORDER BY ordinal")?
                    .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok((version, rows))
            })
            .unwrap()
    }

    #[test]
    fn dates_follow_the_current_revision_of_title_and_body() {
        let state = State::open_in_memory().unwrap();
        state.board("sogang-academic").unwrap();
        let first = article("장학생 선발 안내", "신청 마감: 2026. 11. 27.(금) 18:00");
        let (id, _) = state.upsert("sogang-academic", &first, false).unwrap();
        let deadline = (
            "2026-11-27".to_string(),
            Some("18:00".to_string()),
            "deadline".to_string(),
        );
        assert_eq!(
            dates(&state, id),
            (Some(EXTRACTOR_VERSION), vec![deadline.clone()])
        );

        // An unchanged revision keeps its dates.
        let checked = ArticleInput {
            etag: Some("\"v2\"".into()),
            ..first.clone()
        };
        state.upsert("sogang-academic", &checked, false).unwrap();
        assert_eq!(dates(&state, id).1, vec![deadline]);

        // A revised body replaces them, and the title alone still counts.
        let revised = article(
            "장학생 선발 안내 (~12/4(금)까지)",
            "선발 결과는 추후 안내합니다.",
        );
        state.upsert("sogang-academic", &revised, false).unwrap();
        assert_eq!(
            dates(&state, id).1,
            vec![("2026-12-04".to_string(), None, "deadline".to_string())]
        );

        // A notice without dates still records the extraction.
        let plain = ArticleInput {
            external_id: "101".into(),
            ..article("도서관 이용 안내", "열람실 좌석을 예약해 주세요.")
        };
        let (id, _) = state.upsert("sogang-academic", &plain, false).unwrap();
        assert_eq!(dates(&state, id), (Some(EXTRACTOR_VERSION), Vec::new()));
    }

    #[test]
    fn an_upgraded_state_backfills_dates_once_per_extractor_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(MIGRATIONS[0]).unwrap();
        old.execute_batch(
            "INSERT INTO boards(source_id) VALUES('sogang-academic');
             INSERT INTO notices(source_id,external_id,canonical_url,title,published_at,
               body_state,body_text,resources,revision_hash,historical,first_seen_at,
               revised_at,checked_at)
             VALUES('sogang-academic','100','https://www.sogang.ac.kr/ko/detail/100',
               '장학생 선발 안내','2026-11-17','normal','신청 마감: 2026. 11. 27.(금) 18:00',
               '[]','x',1,'t','t','t');",
        )
        .unwrap();
        drop(old);

        let state = State::open(&path).unwrap();
        let id = state.notice_id("sogang-academic", "100").unwrap().unwrap();
        assert_eq!(dates(&state, id), (None, Vec::new()));
        assert_eq!(state.refresh_dates().unwrap(), 1);
        assert_eq!(dates(&state, id).1.len(), 1);
        assert_eq!(state.refresh_dates().unwrap(), 0);
        // Rows from another extractor version are extracted again.
        state
            .with(|c| {
                c.execute(
                    "UPDATE notices SET dates_version=?1",
                    [EXTRACTOR_VERSION - 1],
                )
            })
            .unwrap();
        assert_eq!(state.refresh_dates().unwrap(), 1);
        assert_eq!(dates(&state, id).0, Some(EXTRACTOR_VERSION));
    }
}
