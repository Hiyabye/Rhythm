//! The collector's SQLite state: board checkpoints, the current revision of
//! each notice and skipped notices. Every method takes the connection lock for
//! one statement or transaction only, so no lock is held across network work.
use rhythm_sources::model::{ArticleInput, ListingEntry};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
/// Ordered schema migrations; index `n` upgrades `user_version` n to n+1.
const MIGRATIONS: [&str; 1] = [include_str!("../migrations/001_initial.sql")];

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
    pub fn seed_complete(&self, source_id: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT seed_complete FROM boards WHERE source_id=?1",
                [source_id],
                |row| row.get(0),
            )
        })
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
                "SELECT modified_at FROM notices WHERE source_id=?1 AND external_id=?2",
                params![source_id, external_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
        })
        .map(|stored| stored.flatten().is_some_and(|value| value == modified_at))
    }
    pub fn checked(&self, source_id: &str, external_id: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                &format!(
                    "UPDATE notices SET checked_at={NOW} WHERE source_id=?1 AND external_id=?2"
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
                            "UPDATE notices SET checked_at={NOW},etag=?2,last_modified=?3,modified_at=?4
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
                               revised_at={NOW},checked_at={NOW}
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
                    transaction.last_insert_rowid()
                }
            };
            transaction.commit()?;
            Ok((id, inserted))
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
                    "INSERT INTO skipped(source_id,external_id,url,historical,reason,response_sha256,recorded_at)
                     VALUES(?1,?2,?3,?4,?5,?6,{NOW})
                     ON CONFLICT(source_id,external_id) DO UPDATE SET
                       url=excluded.url,reason=excluded.reason,
                       response_sha256=excluded.response_sha256,recorded_at=excluded.recorded_at"
                ),
                params![source_id, entry.external_id, entry.url, historical, reason, response_sha256],
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
    // The approved metadata-only exception exists for the main board alone.
    let invalid_unsupported_body = article.unsupported_body
        && (source_id != "sogang-academic"
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
/// Identity of the published revision. Unlike the app's content hash it leaves
/// out the HTML, which the collector does not keep: a markup-only change is not
/// a new revision.
fn revision_hash(article: &ArticleInput) -> Result<String, String> {
    let resources: Vec<_> = article
        .resources
        .iter()
        .map(|r| (&r.url, &r.filename, &r.kind))
        .collect();
    let encoded = serde_json::to_vec(&(
        &article.canonical_url,
        &article.title,
        &article.author,
        &article.category,
        &article.published_at,
        &article.body_text,
        resources,
    ))
    .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

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
            .with(|c| c.execute_batch("PRAGMA user_version=2;"))
            .unwrap();
        drop(state);
        assert!(State::open(&path).err().unwrap().contains("version 2"));
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
}
