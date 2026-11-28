use super::*;
use crate::r2::Object;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, UNIX_EPOCH},
};

const PREFIX: &str = "state/";
const BOARD: &str = "sogang-academic";

/// An in-memory bucket that records each object's SHA-256 like R2's metadata.
#[derive(Default)]
struct Bucket {
    objects: Mutex<HashMap<String, Object>>,
    writes: Mutex<Vec<String>>,
}
impl ObjectStore for Bucket {
    async fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }
    async fn get(&self, key: &str) -> Result<Object, String> {
        let objects = self.objects.lock().unwrap();
        let object = objects.get(key).ok_or("NoSuchKey")?;
        Ok(Object {
            bytes: object.bytes.clone(),
            sha256: object.sha256.clone(),
        })
    }
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.writes.lock().unwrap().push(format!("put {key}"));
        let sha256 = Some(sha256_hex(&bytes));
        self.objects
            .lock()
            .unwrap()
            .insert(key.into(), Object { bytes, sha256 });
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push(format!("delete {key}"));
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
}
impl Bucket {
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.objects.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }
    fn writes(&self) -> usize {
        self.writes.lock().unwrap().len()
    }
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_763_337_600 + seconds)
}

struct Run {
    dir: tempfile::TempDir,
}
impl Run {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }
}

/// One run: download, mark the board seeded, upload.
async fn seeded_run(bucket: &Bucket, run: &Run, name: &str, init: bool, now: SystemTime) -> String {
    let path = run.path(name);
    let base = download(bucket, PREFIX, &path, init).await.unwrap();
    let state = State::open(&path).unwrap();
    state.board(BOARD).unwrap();
    state.mark_seed_complete(BOARD).unwrap();
    upload(
        bucket,
        PREFIX,
        &state,
        base.as_deref(),
        now,
        &run.path(&format!("{name}.upload")),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_new_state_round_trips_through_the_bucket() {
    let bucket = Bucket::default();
    let run = Run::new();
    let key = seeded_run(&bucket, &run, "first.sqlite3", true, at(0)).await;
    assert_eq!(key, "state/20251117T000000Z.sqlite3.zst");
    assert!(!run.path("first.sqlite3.upload").exists());
    let copy = run.path("second.sqlite3");
    assert_eq!(
        download(&bucket, PREFIX, &copy, false).await.unwrap(),
        Some(key)
    );
    assert!(State::open(&copy).unwrap().seed_complete(BOARD).unwrap());
}

#[tokio::test]
async fn an_empty_prefix_needs_init_and_init_never_replaces_a_state() {
    let bucket = Bucket::default();
    let run = Run::new();
    let error = download(&bucket, PREFIX, &run.path("a.sqlite3"), false)
        .await
        .unwrap_err();
    assert!(error.contains("--init"), "{error}");
    assert!(!run.path("a.sqlite3").exists());
    seeded_run(&bucket, &run, "b.sqlite3", true, at(0)).await;
    let error = download(&bucket, PREFIX, &run.path("c.sqlite3"), true)
        .await
        .unwrap_err();
    assert!(error.contains("--init refused"), "{error}");
    // Another history under another prefix is separate.
    assert_eq!(
        download(&bucket, "test/", &run.path("d.sqlite3"), true)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_run_that_fails_before_upload_leaves_the_previous_state() {
    let bucket = Bucket::default();
    let run = Run::new();
    let key = seeded_run(&bucket, &run, "first.sqlite3", true, at(0)).await;
    let writes = bucket.writes();
    // The failed run downloads and changes its copy, then stops before uploading.
    let failed = run.path("failed.sqlite3");
    download(&bucket, PREFIX, &failed, false).await.unwrap();
    State::open(&failed).unwrap().board("me-general").unwrap();
    assert_eq!(bucket.writes(), writes);
    let next = run.path("next.sqlite3");
    assert_eq!(
        download(&bucket, PREFIX, &next, false).await.unwrap(),
        Some(key)
    );
    let state = State::open(&next).unwrap();
    assert!(state.seed_complete(BOARD).unwrap());
    assert!(!state.seed_complete("me-general").unwrap_or(false));
}

#[tokio::test]
async fn the_newest_version_is_read_and_a_damaged_one_is_refused() {
    let bucket = Bucket::default();
    let run = Run::new();
    seeded_run(&bucket, &run, "a.sqlite3", true, at(0)).await;
    let newest = seeded_run(&bucket, &run, "b.sqlite3", false, at(3_600)).await;
    // Unrelated keys under the prefix are not versions.
    bucket.put("state/notes.txt", b"x".to_vec()).await.unwrap();
    assert_eq!(
        download(&bucket, PREFIX, &run.path("c.sqlite3"), false)
            .await
            .unwrap(),
        Some(newest.clone())
    );
    bucket
        .objects
        .lock()
        .unwrap()
        .get_mut(&newest)
        .unwrap()
        .bytes[20] ^= 1;
    let error = download(&bucket, PREFIX, &run.path("d.sqlite3"), false)
        .await
        .unwrap_err();
    assert!(error.contains("SHA-256"), "{error}");
    assert!(!run.path("d.sqlite3").exists());
    // Undo that damage and change only the zstd frame checksum (the last four
    // bytes), stored with matching metadata: the frame checksum refuses it.
    let mut bytes = bucket
        .objects
        .lock()
        .unwrap()
        .remove(&newest)
        .unwrap()
        .bytes;
    bytes[20] ^= 1;
    *bytes.last_mut().unwrap() ^= 1;
    bucket.put(&newest, bytes).await.unwrap();
    let error = download(&bucket, PREFIX, &run.path("e.sqlite3"), false)
        .await
        .unwrap_err();
    assert!(error.contains("checksum"), "{error}");
    assert!(!run.path("e.sqlite3").exists());
}

#[tokio::test]
async fn an_existing_local_file_is_never_used() {
    let bucket = Bucket::default();
    let run = Run::new();
    seeded_run(&bucket, &run, "a.sqlite3", true, at(0)).await;
    fs::write(run.path("stale.sqlite3"), b"old").unwrap();
    let error = download(&bucket, PREFIX, &run.path("stale.sqlite3"), false)
        .await
        .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(fs::read(run.path("stale.sqlite3")).unwrap(), b"old");
}

#[tokio::test]
async fn an_upload_is_refused_when_another_run_uploaded_or_the_clock_is_behind() {
    let bucket = Bucket::default();
    let run = Run::new();
    seeded_run(&bucket, &run, "a.sqlite3", true, at(0)).await;
    let path = run.path("b.sqlite3");
    let base = download(&bucket, PREFIX, &path, false).await.unwrap();
    let state = State::open(&path).unwrap();
    // Another run uploads in the meantime.
    seeded_run(&bucket, &run, "other.sqlite3", false, at(60)).await;
    let before = bucket.keys();
    let error = upload(
        &bucket,
        PREFIX,
        &state,
        base.as_deref(),
        at(120),
        &run.path("b.upload"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("changed in R2"), "{error}");
    assert_eq!(bucket.keys(), before);

    let path = run.path("c.sqlite3");
    let base = download(&bucket, PREFIX, &path, false).await.unwrap();
    let state = State::open(&path).unwrap();
    let error = upload(
        &bucket,
        PREFIX,
        &state,
        base.as_deref(),
        at(60),
        &run.path("c.upload"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("clock"), "{error}");
    assert_eq!(bucket.keys(), before);
}

#[tokio::test]
async fn pruning_keeps_the_newest_versions() {
    let bucket = Bucket::default();
    let run = Run::new();
    for hour in 0..KEEP as u64 + 2 {
        seeded_run(
            &bucket,
            &run,
            &format!("{hour}.sqlite3"),
            hour == 0,
            at(hour * 3_600),
        )
        .await;
    }
    bucket.put("state/notes.txt", b"x".to_vec()).await.unwrap();
    let all = bucket.keys();
    assert_eq!(prune(&bucket, PREFIX).await.unwrap(), 2);
    let kept = bucket.keys();
    assert_eq!(kept.len(), KEEP + 1);
    assert!(kept.contains(&"state/notes.txt".to_string()));
    assert!(!kept.contains(&all[0]) && !kept.contains(&all[1]));
    assert_eq!(prune(&bucket, PREFIX).await.unwrap(), 0);
}

#[test]
fn prefixes_are_plain_directories() {
    assert!(validate_prefix("state/").is_ok());
    assert!(validate_prefix("test-1/state/").is_ok());
    for prefix in ["", "state", "/state/", "state//", "State/", "a b/"] {
        assert!(validate_prefix(prefix).is_err(), "{prefix}");
    }
}

/// A zstd frame of `size` zero bytes with the frame checksum, built in chunks
/// so the test never holds the whole state in memory.
fn zeros(size: u64) -> Vec<u8> {
    let mut encoder = zstd::Encoder::new(Vec::new(), 1).unwrap();
    encoder.include_checksum(true).unwrap();
    io::copy(&mut io::repeat(0).take(size), &mut encoder).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn a_state_over_512_mib_downloads() {
    // Production stopped on 2026-11-27 when the state passed 512 MiB.
    let size = 512 * 1024 * 1024 + 1;
    let bucket = Bucket::default();
    bucket
        .put("state/20251117T000000Z.sqlite3.zst", zeros(size))
        .await
        .unwrap();
    let run = Run::new();
    let path = run.path("large.sqlite3");
    download(&bucket, PREFIX, &path, false).await.unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), size);
}

#[test]
fn a_state_over_the_limit_is_refused() {
    let mut out = Vec::new();
    decompress_into(&zeros(100), &mut out, 100).unwrap();
    assert_eq!(out.len(), 100);
    let error = decompress_into(&zeros(101), &mut Vec::new(), 100).unwrap_err();
    assert!(error.contains("too large"), "{error}");
}

#[test]
fn a_compressed_state_the_next_run_could_not_read_is_refused() {
    let state = b"collector state";
    let size = compress(state, usize::MAX).unwrap().len();
    assert_eq!(compress(state, size).unwrap().len(), size);
    let error = compress(state, size - 1).unwrap_err();
    assert!(error.contains("not uploading"), "{error}");
}
