//! Explicitly invoked, non-shipping helper for the isolated native Windows
//! notification acceptance test. It never sends a notification itself.
use crate::{
    archive::model::{ArticleInput, ResourceInput},
    core::lock_data_dir,
    storage::{proactive::NotificationSensitivity, Store},
};
use rusqlite::{params, Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

const TITLE: &str = "[전공] 2026학년도 2학기 전공 신청 및 변경 안내";
const RETRY_REASON: &str = "native-validation-unclicked-retry";
const ORIGINAL_HASHES: [&str; 10] = [
    "7d3d71c8eab8a2bf68f7f8dafac3b13af580b6f788a067d89c4ef52f969713ff",
    "39fdbe28cda1c12295a4ffb61ffc4b844857658309b778ea26981a821b9f1543",
    "9b2a457461c4d07941af433c950f99739088959a1e78c4d9fdeb7e8b76aee7f5",
    "417f9db4dbb170be65b8dc256754d76349d4ca55f5eac3e81c41f1cda75327f7",
    "234ed439f8045cf9dd24f3b6b0252edd1dd6738ddfbe0cb232ec9b3d48d294df",
    "0c3261b3eb8e45891f23ebcb28a1f6390be2b3953450c52f0734c2a15247452b",
    "eaaf8864341c7f3203fd0e04856d6a79961add081b390bcc7ee38189405d35db",
    "20c56930a9f599b498ca80e80b5b53c194eb0aa8d98d766f6fd57f93af005d22",
    "e92f4bd0a780bb0ad35bf5caa91d39a4bba050477f12d816f608e2480fc200d4",
    "2f9af2bb3e84490fb91f92340eef7a90a759e85e2648306024b24041833020f6",
];

fn marker(stage: u32) -> String {
    format!("[Rhythm Windows 알림 검증 {stage}]")
}

// The production suppression API intentionally refuses terminal `attempted`
// events. This isolated test-only transaction revokes only the exact capability
// for the last verified test event and never changes its delivery ledger.
fn suppress_unclicked_test_event(
    connection: &mut Connection,
    event_id: i64,
    attempt: u32,
) -> Result<(), String> {
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let tokens: Vec<(String, String)> = {
        let mut statement = tx
            .prepare("SELECT token_hash, attempts_json FROM notification_activation_tokens")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    if tokens.len() > 1 {
        return Err("unexpected activation tokens in isolated fixture".into());
    }
    for (_, json) in &tokens {
        let attempts: Vec<(i64, u32)> = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if attempts != [(event_id, attempt)] {
            return Err("activation token is not solely for the verified test event".into());
        }
    }
    let changed = tx.execute(
        "UPDATE proactive_events SET state='suppressed',reason=?1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?2 AND state='attempted' AND attempt_count=?3",
        params![RETRY_REASON, event_id, attempt],
    ).map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("verified unclicked event changed before retry".into());
    }
    for (hash, json) in tokens {
        let changed = tx.execute(
            "DELETE FROM notification_activation_tokens WHERE token_hash=?1 AND attempts_json=?2",
            params![hash, json],
        ).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("test activation token changed before retry".into());
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

#[test]
#[ignore = "explicitly authorized, isolated Windows native acceptance test only"]
fn prepare_one_native_notification_revision() {
    let stage: u32 = env::var("RHYTHM_NATIVE_TEST_STAGE")
        .expect("set RHYTHM_NATIVE_TEST_STAGE to 1 through 8")
        .parse()
        .expect("invalid test stage");
    assert!((1..=8).contains(&stage), "invalid test stage");
    let retry_unclicked = match env::var("RHYTHM_NATIVE_TEST_RETRY_UNCLICKED") {
        Ok(value) if value == "1" => true,
        Err(env::VarError::NotPresent) => false,
        _ => panic!("RHYTHM_NATIVE_TEST_RETRY_UNCLICKED must be unset or 1"),
    };
    assert!(
        !retry_unclicked || stage > 1,
        "stage 1 has no previous attempt"
    );
    let supplied = PathBuf::from(env::var_os("RHYTHM_NATIVE_TEST_DATA").expect("set test path"));
    assert!(supplied.is_absolute(), "test path must be absolute");
    let expected = PathBuf::from(env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"))
        .join("app.rhythm.windows");
    let root = supplied
        .canonicalize()
        .expect("test profile must already exist");
    assert_eq!(
        root,
        expected
            .canonicalize()
            .expect("expected profile must exist")
    );
    let sentinel = env::var("RHYTHM_NATIVE_TEST_SENTINEL").expect("set sentinel");
    assert!(sentinel.len() >= 32 && sentinel.trim() == sentinel);
    let sentinel_path = root.join(".rhythm-native-validation");
    let metadata = fs::symlink_metadata(&sentinel_path).expect("missing sentinel");
    assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_to_string(&sentinel_path).expect("read sentinel"),
        sentinel
    );
    assert!(
        root.join("rhythm.lock").is_file(),
        "profile lock must already exist"
    );
    let _lock = lock_data_dir(&root).expect("close Rhythm completely before preparing");
    let db_path = root.join("rhythm.sqlite3");
    let db_metadata = fs::symlink_metadata(&db_path).expect("fixture database missing");
    assert!(db_metadata.is_file() && !db_metadata.file_type().is_symlink());
    assert_eq!(
        db_path.canonicalize().unwrap().parent(),
        Some(root.as_path())
    );
    let read = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open fixture read-only");
    let count = |sql: &str| -> i64 { read.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM documents"), 10);
    assert_eq!(
        count("SELECT COUNT(*) FROM document_revisions"),
        10 + i64::from(stage - 1)
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM proactive_events"),
        i64::from(stage - 1)
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM proactive_dedupe"),
        i64::from(stage - 1)
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM proactive_delivery_attempts"),
        i64::from(stage - 1)
    );
    for (index, expected_hash) in ORIGINAL_HASHES.iter().enumerate() {
        let id = index as i64 + 1;
        let (document_id, hash): (i64, String) = read
            .query_row(
                "SELECT document_id,content_hash FROM document_revisions WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("missing original revision");
        assert_eq!(document_id, id);
        assert_eq!(&hash, expected_hash, "original fixture differs from backup");
    }
    let (source, external, title, url, subscribed, supported, extension): (
        String,
        String,
        String,
        String,
        bool,
        bool,
        bool,
    ) = read
        .query_row(
            "SELECT d.source_id,d.external_id,r.title,r.canonical_url,s.subscribed,s.supported,
                EXISTS(SELECT 1 FROM source_extensions se JOIN extension_preferences ep
                       ON ep.extension_id=se.extension_id
                       WHERE se.source_id=s.id AND ep.enabled=1)
         FROM documents d JOIN document_revisions r ON r.document_id=d.id AND r.id=10
         JOIN sources s ON s.id=d.source_id WHERE d.id=10",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("missing original document 10/revision 10");
    assert_eq!(source, "computing-academic");
    assert_eq!(external, "970");
    assert_eq!(title, TITLE);
    assert_eq!(
        url,
        "https://computing.sogang.ac.kr/ko/community/academicNotice/detail/970"
    );
    assert!(
        subscribed && supported && extension,
        "fixture source must already be active"
    );
    let original: (String, String, String) = read.query_row(
        "SELECT body_html,body_text,original FROM document_revisions WHERE id=10 AND document_id=10",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    assert_eq!(
        hash(&original.0),
        "8686b0619bd18ec2e9d2e374d77f4dbad7eb67f57c4b2e89cae0d73d7e469cc3"
    );
    assert_eq!(
        hash(&original.1),
        "6652d37e92f1ac4747aea77f01cbddbc77e42af72bf297455111c151b1807706"
    );
    assert_eq!(
        hash(&original.2),
        "9cd45b914ffccaf81d1c1c8f0213b1329c33d8319beb7fe1a2c629eeb435d6dd"
    );
    let mut previous = read
        .prepare(
            "SELECT r.body_text,e.state,e.id,e.attempt_count,e.reason,r.id FROM document_revisions r
         JOIN proactive_events e ON e.document_id=r.document_id AND e.revision_id=r.id
         WHERE r.document_id=10 AND r.id>10 AND e.event_type='revision' ORDER BY r.id",
        )
        .unwrap();
    let rows = previous
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), (stage - 1) as usize);
    let mut expected_body = original.1;
    for (index, (body, state, _, attempt, reason, revision)) in rows.iter().enumerate() {
        expected_body.push('\n');
        expected_body.push_str(&marker(index as u32 + 1));
        assert_eq!(
            body, &expected_body,
            "previous test revision is inconsistent"
        );
        assert_eq!(*revision, 11 + index as i64, "unexpected revision identity");
        assert_eq!(*attempt, 1, "unexpected retry count for a test event");
        let valid_previous = state == "delivered"
            || (state == "suppressed" && reason.as_deref() == Some(RETRY_REASON))
            || (retry_unclicked && index + 1 == rows.len() && state == "attempted");
        assert!(
            valid_previous,
            "previous toast must be clicked or explicitly retried"
        );
        let ledger: i64 = read.query_row(
            "SELECT COUNT(*) FROM proactive_delivery_attempts WHERE document_id=10 AND revision_id=?1 AND event_type='revision' AND attempt_number=1",
            [revision], |row| row.get(0),
        ).unwrap();
        assert_eq!(ledger, 1, "previous test delivery ledger does not match");
    }
    if retry_unclicked {
        assert_eq!(
            rows.last().map(|row| row.1.as_str()),
            Some("attempted"),
            "retry requires the most recent event to be attempted and unclicked"
        );
    } else {
        assert_eq!(
            count("SELECT COUNT(*) FROM notification_activation_tokens"),
            0
        );
    }
    drop(previous);
    drop(read);

    let store = Store::open(&db_path).expect("open validated test profile");
    let score = store
        .personalization_notification_score(10)
        .unwrap()
        .expect("document score");
    assert!(!score.negative_feedback && score.negative_interest_matches.is_empty());
    let mut settings = store.proactive_settings().unwrap();
    if stage == 1 {
        settings.scheduler_enabled = false;
        settings.notifications_enabled = true;
        settings.notification_sensitivity = NotificationSensitivity::All;
        settings.quiet_hours.enabled = false;
        store.proactive_save_settings(&settings).unwrap();
        assert_eq!(store.proactive_establish_baseline().unwrap(), 10);
    } else {
        assert!(!settings.scheduler_enabled && settings.notifications_enabled);
        assert_eq!(
            settings.notification_sensitivity,
            NotificationSensitivity::All
        );
        assert!(!settings.quiet_hours.enabled);
        assert_eq!(store.proactive_baseline().unwrap(), 10);
    }
    assert!(store.proactive_candidates(100).unwrap().is_empty());
    if retry_unclicked {
        let (_, _, event_id, attempt, _, _) = rows.last().expect("verified previous event");
        let mut connection =
            Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .expect("open isolated test profile for bounded retry");
        suppress_unclicked_test_event(&mut connection, *event_id, *attempt)
            .expect("could not safely revoke the unclicked test activation");
    }
    let detail = store.document(10, None).unwrap();
    let label = marker(stage);
    let article = ArticleInput {
        external_id: external,
        canonical_url: detail.canonical_url,
        title: detail.item.title,
        author: detail.item.author,
        category: detail.item.category,
        published_at: detail.item.published_at,
        modified_at: None,
        original: store.latest_original(10).unwrap(),
        unsupported_body: false,
        body_html: format!("{}<p>{label}</p>", detail.body_html),
        body_text: format!("{}\n{label}", detail.body_text),
        resources: detail
            .resources
            .into_iter()
            .map(|r| ResourceInput {
                url: r.url,
                filename: r.filename,
                kind: r.kind,
            })
            .collect(),
        etag: None,
        last_modified: None,
    };
    let (id, changed) = store
        .upsert_article(&source, &article, detail.item.historical)
        .unwrap();
    assert_eq!(id, 10);
    assert!(changed);
    let candidates = store.proactive_candidates(100).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].document_id, 10);
    assert_eq!(store.document(10, Some(10)).unwrap().item.title, TITLE);
    println!(
        "Prepared stage {stage}: document 10, revision {}",
        candidates[0].revision_id
    );
}

#[test]
fn explicit_unclicked_retry_revokes_only_its_token_and_keeps_ledger() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE proactive_events(id INTEGER PRIMARY KEY,state TEXT,reason TEXT,attempt_count INTEGER,updated_at TEXT);
         CREATE TABLE notification_activation_tokens(token_hash TEXT PRIMARY KEY,attempts_json TEXT);
         CREATE TABLE proactive_delivery_attempts(event_id INTEGER,attempt_number INTEGER);
         INSERT INTO proactive_events VALUES(7,'attempted',NULL,1,'old');
         INSERT INTO proactive_delivery_attempts VALUES(7,1);
         INSERT INTO notification_activation_tokens VALUES('test-hash','[[7,1]]');"
    ).unwrap();
    suppress_unclicked_test_event(&mut connection, 7, 1).unwrap();
    let (state, reason): (String, String) = connection
        .query_row(
            "SELECT state,reason FROM proactive_events WHERE id=7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), reason.as_str()),
        ("suppressed", RETRY_REASON)
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM notification_activation_tokens",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM proactive_delivery_attempts",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn unclicked_retry_rejects_unrelated_token_without_changes() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE proactive_events(id INTEGER PRIMARY KEY,state TEXT,reason TEXT,attempt_count INTEGER,updated_at TEXT);
         CREATE TABLE notification_activation_tokens(token_hash TEXT PRIMARY KEY,attempts_json TEXT);
         INSERT INTO proactive_events VALUES(7,'attempted',NULL,1,'old');
         INSERT INTO notification_activation_tokens VALUES('other-hash','[[8,1]]');"
    ).unwrap();
    assert!(suppress_unclicked_test_event(&mut connection, 7, 1).is_err());
    let state: String = connection
        .query_row("SELECT state FROM proactive_events WHERE id=7", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(state, "attempted");
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM notification_activation_tokens",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
