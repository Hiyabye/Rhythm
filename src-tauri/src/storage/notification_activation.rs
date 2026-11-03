//! Durable, one-use Windows notification activation capabilities.
use super::Store;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_TOKENS: i64 = 1024;
const MAX_EVENTS: usize = 3;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ActivatedBatch {
    pub document_id: i64,
    pub revision_id: i64,
    pub attempts: Vec<(i64, u32)>,
}

pub(crate) fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

impl Store {
    /// Save a random token before OS handoff; a failed handoff revokes it.
    pub(crate) fn notification_token_create(
        &self,
        token: &str,
        attempts: &[(i64, u32)],
    ) -> Result<(), String> {
        if !valid_token(token) || attempts.is_empty() || attempts.len() > MAX_EVENTS {
            return Err("알림 활성화 정보가 올바르지 않습니다.".into());
        }
        let mut seen = HashSet::new();
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM notification_activation_tokens WHERE expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ','now')", []).map_err(|e| e.to_string())?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM notification_activation_tokens",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if count >= MAX_TOKENS {
            return Err("대기 중인 알림이 너무 많습니다.".into());
        }
        for (id, attempt) in attempts {
            if *id <= 0 || *attempt == 0 || *attempt > 3 || !seen.insert(*id) {
                return Err("알림 전달 정보가 올바르지 않습니다.".into());
            }
            let valid: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM proactive_events WHERE id=?1 AND attempt_count=?2 AND state='claimed')", params![id, attempt], |r| r.get(0)).map_err(|e| e.to_string())?;
            if !valid {
                return Err("알림 전달 상태가 이미 변경되었습니다.".into());
            }
        }
        let data = serde_json::to_string(attempts).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO notification_activation_tokens(token_hash,attempts_json,expires_at) VALUES(?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now','+7 days'))", params![digest(token), data]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn notification_token_revoke(&self, token: &str) -> Result<(), String> {
        if !valid_token(token) {
            return Ok(());
        }
        self.connection
            .execute(
                "DELETE FROM notification_activation_tokens WHERE token_hash=?1",
                [digest(token)],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Consume and mark every event delivered in one transaction. Missing, stale,
    /// expired or repeated activations cannot mark a different attempt delivered.
    pub(crate) fn notification_token_consume(
        &self,
        token: &str,
    ) -> Result<Option<ActivatedBatch>, String> {
        if !valid_token(token) {
            return Ok(None);
        }
        let hash = digest(token);
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let data: Option<String> = tx.query_row("SELECT attempts_json FROM notification_activation_tokens WHERE token_hash=?1 AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now')", [&hash], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        let Some(data) = data else {
            return Ok(None);
        };
        let attempts: Vec<(i64, u32)> = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        if attempts.is_empty() || attempts.len() > MAX_EVENTS {
            return Err("알림 활성화 정보가 올바르지 않습니다.".into());
        }
        let mut seen = HashSet::new();
        let mut first = None;
        for (id, attempt) in &attempts {
            if *id <= 0 || *attempt == 0 || *attempt > 3 || !seen.insert(*id) {
                return Err("알림 활성화 정보가 올바르지 않습니다.".into());
            }
            let event: Option<(i64,i64)> = tx.query_row("SELECT document_id,revision_id FROM proactive_events WHERE id=?1 AND attempt_count=?2 AND state IN ('claimed','attempted')", params![id, attempt], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e| e.to_string())?;
            let Some(event) = event else {
                return Ok(None);
            };
            if first.is_none() {
                first = Some(event);
            }
        }
        for (id, attempt) in &attempts {
            let changed = tx.execute("UPDATE proactive_events SET state='delivered',delivered_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),deferred_until=NULL,error=NULL,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND attempt_count=?2 AND state IN ('claimed','attempted')", params![id, attempt]).map_err(|e| e.to_string())?;
            if changed != 1 {
                return Err("알림 전달 상태가 이미 변경되었습니다.".into());
            }
        }
        tx.execute(
            "DELETE FROM notification_activation_tokens WHERE token_hash=?1",
            [&hash],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        let (document_id, revision_id) = first.expect("nonempty validated batch");
        Ok(Some(ActivatedBatch {
            document_id,
            revision_id,
            attempts,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_rows(store: &Store) -> Vec<String> {
        let mut statement = store.connection.prepare("SELECT 'token|'||token_hash||'|'||attempts_json||'|'||expires_at FROM notification_activation_tokens UNION ALL SELECT 'event|'||id||'|'||state||'|'||attempt_count||'|'||COALESCE(delivered_at,'') FROM proactive_events ORDER BY 1").unwrap();
        let rows = statement.query_map([], |row| row.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    }

    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.sqlite3")).unwrap();
        // Isolate capability/fencing semantics from archive fixture construction.
        store
            .connection
            .execute_batch(
                "PRAGMA foreign_keys=OFF;
          INSERT INTO proactive_events(id,document_id,revision_id,event_type,state,attempt_count)
          VALUES(1,11,21,'new','claimed',1),(2,12,22,'new','claimed',2);
          PRAGMA foreign_keys=ON;",
            )
            .unwrap();
        (dir, store)
    }

    #[test]
    fn token_consumption_is_durable_single_use_and_delivers_exact_batch() {
        let (dir, store) = fixture();
        let token = "ab".repeat(32);
        store
            .notification_token_create(&token, &[(1, 1), (2, 2)])
            .unwrap();
        let stored: String = store
            .connection
            .query_row(
                "SELECT token_hash FROM notification_activation_tokens",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(stored, token);
        drop(store);
        let store = Store::open(&dir.path().join("test.sqlite3")).unwrap();
        assert_eq!(
            store.notification_token_consume(&token).unwrap(),
            Some(ActivatedBatch {
                document_id: 11,
                revision_id: 21,
                attempts: vec![(1, 1), (2, 2)]
            })
        );
        assert_eq!(store.notification_token_consume(&token).unwrap(), None);
        let states: Vec<String> = store
            .connection
            .prepare("SELECT state FROM proactive_events ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(states, ["delivered", "delivered"]);
    }

    #[test]
    fn expired_wrong_or_stale_capabilities_do_not_deliver_any_event() {
        let (_dir, store) = fixture();
        let token = "cd".repeat(32);
        store
            .notification_token_create(&token, &[(1, 1), (2, 2)])
            .unwrap();
        assert_eq!(
            store.notification_token_consume(&"ef".repeat(32)).unwrap(),
            None
        );
        assert_eq!(
            store
                .notification_token_consume(&(token.clone() + "?x=1"))
                .unwrap(),
            None
        );
        store
            .connection
            .execute("UPDATE proactive_events SET attempt_count=3 WHERE id=2", [])
            .unwrap();
        assert_eq!(store.notification_token_consume(&token).unwrap(), None);
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM proactive_events WHERE state='delivered'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(store.notification_token_consume(&token).unwrap(), None);
        let expired = "12".repeat(32);
        store
            .notification_token_create(&expired, &[(1, 1)])
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE notification_activation_tokens SET expires_at='2000-01-01T00:00:00.000Z'",
                [],
            )
            .unwrap();
        assert_eq!(store.notification_token_consume(&expired).unwrap(), None);
    }

    #[test]
    fn revoke_and_invalid_batches_do_not_activate() {
        let (_dir, store) = fixture();
        let token = "34".repeat(32);
        assert!(store.notification_token_create(&token, &[]).is_err());
        assert!(store
            .notification_token_create(&token, &[(1, 1), (1, 1)])
            .is_err());
        assert!(store.notification_token_create(&token, &[(1, 2)]).is_err());
        store.notification_token_create(&token, &[(1, 1)]).unwrap();
        store.notification_token_revoke(&token).unwrap();
        assert_eq!(store.notification_token_consume(&token).unwrap(), None);
    }

    #[test]
    fn rejected_clicks_leave_events_and_tokens_unchanged() {
        let (_dir, store) = fixture();
        let stale = "45".repeat(32);
        store.notification_token_create(&stale, &[(1, 1)]).unwrap();
        store
            .connection
            .execute("UPDATE proactive_events SET attempt_count=2 WHERE id=1", [])
            .unwrap();
        let before = db_rows(&store);
        assert_eq!(store.notification_token_consume(&stale).unwrap(), None);
        assert_eq!(db_rows(&store), before);
        assert_eq!(
            store.notification_token_consume(&"78".repeat(32)).unwrap(),
            None
        );
        assert_eq!(db_rows(&store), before);
        assert_eq!(store.notification_token_consume("bad-token").unwrap(), None);
        assert_eq!(db_rows(&store), before);
        let expired = "56".repeat(32);
        store
            .notification_token_create(&expired, &[(2, 2)])
            .unwrap();
        store.connection.execute("UPDATE notification_activation_tokens SET expires_at='2000-01-01T00:00:00.000Z' WHERE token_hash=?1", [digest(&expired)]).unwrap();
        let before_expired = db_rows(&store);
        assert_eq!(store.notification_token_consume(&expired).unwrap(), None);
        assert_eq!(db_rows(&store), before_expired);
        let valid = "67".repeat(32);
        store.notification_token_create(&valid, &[(2, 2)]).unwrap();
        assert!(store.notification_token_consume(&valid).unwrap().is_some());
        let before_replay = db_rows(&store);
        assert_eq!(store.notification_token_consume(&valid).unwrap(), None);
        assert_eq!(db_rows(&store), before_replay);
    }
}
