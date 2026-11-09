//! 휴지통. A trashed notice keeps all its content, state and AI results until
//! the user deletes it permanently; only then is the document removed and its
//! `(source_id, external_id)` recorded so no sync archives it again.
use super::Store;
use rusqlite::{params_from_iter, OptionalExtension};

/// SQL condition, for a query whose documents are aliased `d`, that keeps only
/// notices outside 휴지통.
pub(crate) const NOT_TRASHED: &str =
    "NOT EXISTS(SELECT 1 FROM trashed_documents trash WHERE trash.document_id=d.id)";
/// One list selection; a page holds at most 100 notices.
const MAX_SELECTION: usize = 200;

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
}
fn selection(ids: &[i64]) -> Result<(), String> {
    if ids.is_empty() || ids.len() > MAX_SELECTION || ids.iter().any(|id| *id <= 0) {
        return Err("선택한 공지를 확인해 주세요.".into());
    }
    Ok(())
}

impl Store {
    /// Moves notices to 휴지통. Already trashed notices keep their original time.
    pub(crate) fn trash_documents(&self, ids: &[i64]) -> Result<(), String> {
        selection(ids)?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let found: i64 = tx
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM documents WHERE id IN ({})",
                    placeholders(ids.len())
                ),
                params_from_iter(ids),
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut unique = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if found != unique.len() as i64 {
            return Err("보관된 공지를 찾을 수 없습니다.".into());
        }
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO trashed_documents(document_id) SELECT id FROM documents WHERE id IN ({})",
                placeholders(ids.len())
            ),
            params_from_iter(ids),
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        // Saved or interested notices in 휴지통 no longer teach topics.
        self.recompute_learned()
    }
    pub(crate) fn restore_documents(&self, ids: &[i64]) -> Result<(), String> {
        selection(ids)?;
        self.connection
            .execute(
                &format!(
                    "DELETE FROM trashed_documents WHERE document_id IN ({})",
                    placeholders(ids.len())
                ),
                params_from_iter(ids),
            )
            .map_err(|e| e.to_string())?;
        self.recompute_learned()
    }
    /// Permanently deletes the given trashed notices, or all of 휴지통 when
    /// `ids` is `None`. Returns the downloaded files no remaining resource
    /// uses; the caller removes them after the store lock is released.
    pub(crate) fn purge_trashed(&self, ids: Option<&[i64]>) -> Result<Vec<String>, String> {
        if let Some(ids) = ids {
            selection(ids)?;
        }
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let targets: Vec<i64> = match ids {
            Some(ids) => {
                let mut statement = tx
                    .prepare(&format!(
                        "SELECT document_id FROM trashed_documents WHERE document_id IN ({})",
                        placeholders(ids.len())
                    ))
                    .map_err(|e| e.to_string())?;
                let rows = statement
                    .query_map(params_from_iter(ids), |row| row.get(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                let mut unique = ids.to_vec();
                unique.sort_unstable();
                unique.dedup();
                if rows.len() != unique.len() {
                    return Err("휴지통에 있는 공지만 영구 삭제할 수 있습니다.".into());
                }
                rows
            }
            None => {
                let mut statement = tx
                    .prepare("SELECT document_id FROM trashed_documents")
                    .map_err(|e| e.to_string())?;
                let rows = statement
                    .query_map([], |row| row.get(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                rows
            }
        };
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let list = placeholders(targets.len());
        // A running or queued analysis would lose the notice it writes to.
        let busy: Option<i64> = tx
            .query_row(
                &format!(
                    "SELECT 1 FROM ai_jobs WHERE document_id IN ({list}) AND status IN ('pending','running') LIMIT 1"
                ),
                params_from_iter(&targets),
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if busy.is_some() {
            return Err("AI 분석이 진행 중인 공지가 있어 영구 삭제하지 않았습니다. 분석이 끝난 뒤 다시 시도해 주세요.".into());
        }
        let paths: Vec<String> = {
            let mut statement = tx
                .prepare(&format!(
                    "SELECT DISTINCT local_path FROM resources WHERE document_id IN ({list}) AND local_path IS NOT NULL"
                ))
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params_from_iter(&targets), |row| row.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        tx.execute(
            &format!(
                "INSERT OR IGNORE INTO deleted_notices(source_id,external_id)
                 SELECT source_id,external_id FROM documents WHERE id IN ({list})"
            ),
            params_from_iter(&targets),
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            &format!(
                "DELETE FROM skipped_articles WHERE EXISTS(
                   SELECT 1 FROM documents d WHERE d.id IN ({list})
                     AND d.source_id=skipped_articles.source_id AND d.external_id=skipped_articles.external_id)"
            ),
            params_from_iter(&targets),
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            &format!("DELETE FROM documents WHERE id IN ({list})"),
            params_from_iter(&targets),
        )
        .map_err(|e| e.to_string())?;
        // Files are content-addressed, so another notice may share one.
        let mut orphaned = Vec::new();
        for path in paths {
            let used: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM resources WHERE local_path=?1)",
                    [&path],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !used {
                orphaned.push(path);
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.recompute_learned()?;
        Ok(orphaned)
    }
    /// Whether the user permanently deleted this notice.
    pub(crate) fn notice_deleted(&self, source: &str, external: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deleted_notices WHERE source_id=?1 AND external_id=?2)",
                [source, external],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
    }
}
