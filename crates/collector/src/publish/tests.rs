use super::*;
use crate::r2::Object;
use rhythm_feed::{
    manifest::verify,
    record::{decode_segment, decode_snapshot, Classification, Embedding, Entry, Event, Notice},
    test_keys::{other_signer, signer},
};
use rhythm_sources::model::ArticleInput;
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

const PREFIX: &str = "feed/";
const BOARD: &str = "sogang-academic";
const SCOPE: &str = "model#hash";

#[derive(Default)]
struct Bucket {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
}
impl ObjectStore for Bucket {
    async fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        let objects = self.objects.lock().unwrap();
        Ok(objects
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
    async fn get(&self, key: &str) -> Result<Object, String> {
        let objects = self.objects.lock().unwrap();
        let bytes = objects.get(key).ok_or("NoSuchKey")?.clone();
        Ok(Object {
            bytes,
            sha256: None,
        })
    }
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.objects.lock().unwrap().insert(key.into(), bytes);
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<(), String> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }
}
impl Bucket {
    fn keys(&self) -> Vec<String> {
        self.objects.lock().unwrap().keys().cloned().collect()
    }
    fn bytes(&self, path: &str) -> Vec<u8> {
        self.objects.lock().unwrap()[&format!("{PREFIX}{path}")].clone()
    }
}

fn article(id: u32, body: &str) -> ArticleInput {
    ArticleInput {
        external_id: id.to_string(),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        title: format!("공지 {id}"),
        author: None,
        category: Some("학사".into()),
        published_at: "2026-11-02".into(),
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
fn add(state: &State, id: u32, body: &str) {
    state.board(BOARD).unwrap();
    state.upsert(BOARD, &article(id, body), false).unwrap();
}
fn notice_id(state: &State, id: u32) -> i64 {
    state.notice_id(BOARD, &id.to_string()).unwrap().unwrap()
}
fn direction(seed: u32) -> Vec<f32> {
    let values: Vec<f32> = (0..DIMENSIONS as u32)
        .map(|i| ((i * 7919 + seed * 104_729) % 2003) as f32 - 1001.0)
        .collect();
    let norm = values.iter().map(|x| x * x).sum::<f32>().sqrt();
    values.iter().map(|x| x / norm).collect()
}
/// Stores a vector of the notice's current revision, and with `category` its
/// classification.
fn embed(state: &State, id: u32, seed: u32, category: Option<&str>) {
    let notice = notice_id(state, id);
    let vector = embedding::encode(&direction(seed));
    state
        .with(|c| {
            c.execute(
                "INSERT INTO embeddings(notice_id,revision_hash,scope,vector,embedded_at)
                 SELECT id,revision_hash,?2,?3,'2026-11-18T00:00:00.000Z' FROM notices WHERE id=?1
                 ON CONFLICT(notice_id) DO UPDATE SET revision_hash=excluded.revision_hash,
                   vector=excluded.vector",
                rusqlite::params![notice, SCOPE, vector],
            )?;
            if let Some(category) = category {
                c.execute(
                    "INSERT INTO classifications(notice_id,revision_hash,scope,category,score,margin,
                       low_confidence,classified_at)
                     SELECT id,revision_hash,?2,?3,0.75,0.125,0,'2026-11-18T00:00:00.000Z'
                     FROM notices WHERE id=?1
                     ON CONFLICT(notice_id) DO UPDATE SET revision_hash=excluded.revision_hash,
                       category=excluded.category",
                    rusqlite::params![notice, SCOPE, category],
                )?;
            }
            Ok(())
        })
        .unwrap();
}
fn remove(state: &State, id: u32, removed: bool) {
    let value = removed.then_some("2026-11-18T01:00:00.000Z");
    let notice = notice_id(state, id);
    state
        .with(|c| {
            c.execute(
                "UPDATE notices SET removed_at=?2 WHERE id=?1",
                rusqlite::params![notice, value],
            )
        })
        .unwrap();
}

/// One run's publish at `now`, as `main` orders it.
async fn run(state: &State, bucket: &Bucket, signer: &Signer, now: &str) -> Report {
    let report = prepare(state, bucket, PREFIX, signer, now, false)
        .await
        .unwrap();
    finish(state, bucket, PREFIX, signer, now).await.unwrap();
    report
}

type Key = (String, String);

/// A client written from docs/FEED.md, independent of the publisher.
#[derive(Default)]
struct Client {
    cursor: Option<u64>,
    generated_at: String,
    notices: BTreeMap<Key, Notice>,
    embeddings: BTreeMap<Key, (Embedding, Vec<f32>)>,
    classifications: BTreeMap<Key, Classification>,
    /// By `(calendar_id, uid)` (format 1.2).
    events: BTreeMap<Key, Event>,
    /// Records applied, to show which path a sync took.
    applied: usize,
    from_snapshot: bool,
}
impl Client {
    fn sync(&mut self, bucket: &Bucket, trusted: [u8; 32]) {
        let manifest = verify(
            &bucket.bytes("manifest.json"),
            &bucket.bytes("manifest.json.sig"),
            &[trusted],
        )
        .unwrap();
        assert!(manifest.generated_at > self.generated_at, "no rollback");
        self.generated_at = manifest.generated_at.clone();
        self.applied = 0;
        let newest = manifest
            .segments
            .last()
            .map_or(manifest.snapshot.through_seq, |s| s.to_seq);
        let resume = self.cursor.filter(|cursor| {
            manifest
                .segments
                .iter()
                .any(|s| s.from_seq <= cursor + 1 && *cursor < s.to_seq)
                || *cursor == newest
        });
        self.from_snapshot = resume.is_none();
        if resume.is_none() {
            let snapshot = &manifest.snapshot;
            let entries =
                decode_snapshot(&fetch(bucket, &snapshot.records), snapshot.through_seq).unwrap();
            let vectors = snapshot.vectors.as_ref().map(|v| fetch(bucket, v));
            let keep: Vec<Key> = entries
                .iter()
                .filter_map(|e| match &e.record {
                    Some(Record::Notice(n)) => Some((n.source_id.clone(), n.external_id.clone())),
                    _ => None,
                })
                .collect();
            for key in self.notices.keys().cloned().collect::<Vec<_>>() {
                if !keep.contains(&key) {
                    self.drop_notice(&key);
                }
            }
            // Events the snapshot lacks were removed meanwhile, like notices.
            let events: Vec<Key> = entries
                .iter()
                .filter_map(|e| match &e.record {
                    Some(Record::Event(event)) => {
                        Some((event.calendar_id.clone(), event.uid.clone()))
                    }
                    _ => None,
                })
                .collect();
            self.events.retain(|key, _| events.contains(key));
            // The snapshot carries every current vector and category.
            self.embeddings.clear();
            self.classifications.clear();
            self.apply(&entries, vectors.as_deref());
            self.cursor = Some(snapshot.through_seq);
        }
        for segment in &manifest.segments {
            let cursor = self.cursor.unwrap();
            if segment.to_seq <= cursor {
                continue;
            }
            let entries = decode_segment(
                &fetch(bucket, &segment.records),
                segment.from_seq,
                segment.to_seq,
            )
            .unwrap();
            let vectors = segment.vectors.as_ref().map(|v| fetch(bucket, v));
            let fresh: Vec<Entry> = entries.into_iter().filter(|e| e.seq > cursor).collect();
            self.apply(&fresh, vectors.as_deref());
            self.cursor = Some(segment.to_seq);
        }
    }
    fn apply(&mut self, entries: &[Entry], vectors: Option<&[u8]>) {
        let vectors = vectors
            .map(|v| vector::decode(v).unwrap())
            .unwrap_or_default();
        for entry in entries {
            self.applied += 1;
            match entry.record.clone().unwrap() {
                Record::Notice(notice) => {
                    let key = (notice.source_id.clone(), notice.external_id.clone());
                    // A new revision invalidates what was derived from the old one.
                    if self.notices.get(&key).map(|n| &n.revision_hash)
                        != Some(&notice.revision_hash)
                    {
                        self.embeddings.remove(&key);
                        self.classifications.remove(&key);
                    }
                    self.notices.insert(key, *notice);
                }
                Record::Embedding(embedding) => {
                    let key = (embedding.source_id.clone(), embedding.external_id.clone());
                    if self.current(&key, &embedding.revision_hash) {
                        let vector = vectors[embedding.vector as usize].clone();
                        self.embeddings.insert(key, (embedding, vector));
                    }
                }
                Record::Classification(classification) => {
                    let key = (
                        classification.source_id.clone(),
                        classification.external_id.clone(),
                    );
                    if self.current(&key, &classification.revision_hash) {
                        self.classifications.insert(key, classification);
                    }
                }
                Record::Remove(removal) => {
                    self.drop_notice(&(removal.source_id, removal.external_id))
                }
                Record::Event(event) => {
                    self.events
                        .insert((event.calendar_id.clone(), event.uid.clone()), event);
                }
                Record::EventRemove(removal) => {
                    self.events.remove(&(removal.calendar_id, removal.uid));
                }
            }
        }
    }
    fn current(&self, key: &Key, revision: &str) -> bool {
        self.notices
            .get(key)
            .is_some_and(|n| n.revision_hash == revision)
    }
    fn drop_notice(&mut self, key: &Key) {
        self.notices.remove(key);
        self.embeddings.remove(key);
        self.classifications.remove(key);
    }

    /// The mirror equals the state minus bodies.
    fn assert_mirrors(&self, state: &State) {
        let scan = state.feed_scan().unwrap();
        let live: Vec<&Scanned> = scan.iter().filter(|s| s.removed_at.is_none()).collect();
        let key = |n: &Notice| (n.source_id.clone(), n.external_id.clone());
        let notices: BTreeMap<Key, Notice> = live
            .iter()
            .map(|s| (key(&s.notice), s.notice.clone()))
            .collect();
        assert_eq!(self.notices, notices);
        let embedded: Vec<&&Scanned> = live.iter().filter(|s| s.embedding.is_some()).collect();
        assert_eq!(self.embeddings.len(), embedded.len());
        for scanned in embedded {
            let (expected, f16) = scanned.embedding.as_ref().unwrap();
            let (embedding, vector) = &self.embeddings[&key(&scanned.notice)];
            assert_eq!(
                (&embedding.revision_hash, &embedding.scope),
                (&expected.revision_hash, &expected.scope)
            );
            let original = embedding::decode(f16);
            let cosine: f32 = original.iter().zip(vector).map(|(a, b)| a * b).sum();
            assert!(cosine > 0.9998, "{cosine}");
        }
        let classifications: BTreeMap<Key, Classification> = live
            .iter()
            .filter_map(|s| s.classification.clone().map(|c| (key(&s.notice), c)))
            .collect();
        assert_eq!(self.classifications, classifications);
        let events: BTreeMap<Key, Event> = state
            .feed_event_scan()
            .unwrap()
            .into_iter()
            .filter(|s| s.removed_at.is_none())
            .map(|s| ((s.event.calendar_id.clone(), s.event.uid.clone()), s.event))
            .collect();
        assert_eq!(self.events, events);
    }
}
fn fetch(bucket: &Bucket, file: &FileRef) -> Vec<u8> {
    let bytes = bucket.bytes(&file.path);
    file.check(&bytes).unwrap();
    bytes
}

fn manifest_of(bucket: &Bucket, trusted: [u8; 32]) -> Manifest {
    verify(
        &bucket.bytes("manifest.json"),
        &bucket.bytes("manifest.json.sig"),
        &[trusted],
    )
    .unwrap()
}
/// Every key the manifest names, with the manifest and its signature, sorted.
fn listed_keys(manifest: &Manifest) -> Vec<String> {
    let mut keys: Vec<String> = std::iter::once(&manifest.snapshot.records)
        .chain(manifest.snapshot.vectors.iter())
        .chain(
            manifest
                .segments
                .iter()
                .flat_map(|s| std::iter::once(&s.records).chain(s.vectors.iter())),
        )
        .map(|f| format!("{PREFIX}{}", f.path))
        .collect();
    keys.extend([
        format!("{PREFIX}manifest.json"),
        format!("{PREFIX}manifest.json.sig"),
    ]);
    keys.sort();
    keys
}

#[tokio::test]
async fn a_client_mirror_equals_the_state_minus_bodies() {
    let state = State::open_in_memory().unwrap();
    let bucket = Bucket::default();
    let signer = signer();
    let key = signer.public_key();
    for id in 1..=3 {
        add(&state, id, &format!("본문 {id} {}", "가".repeat(400)));
    }
    embed(&state, 1, 1, Some("학사"));
    embed(&state, 2, 2, None);

    let report = run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    // Three notices, two vectors, one category.
    assert_eq!(report.seqs, Some((1, 6)));
    assert!(report.snapshot);
    let mut first = Client::default();
    first.sync(&bucket, key);
    assert!(first.from_snapshot);
    first.assert_mirrors(&state);
    // The excerpt stands in for the body.
    let notice = &first.notices[&(BOARD.to_string(), "1".to_string())];
    assert_eq!(notice.excerpt.chars().count(), rhythm_feed::EXCERPT_CHARS);
    // The body hash covers the whole body, beyond the excerpt (format 1.1).
    assert_eq!(
        notice.body_hash.as_deref(),
        Some(crate::sha256_hex(format!("본문 1 {}", "가".repeat(400))).as_str())
    );

    // A revision with its new vector, a removal and a new notice.
    add(&state, 1, "고친 본문");
    embed(&state, 1, 11, Some("장학"));
    remove(&state, 2, true);
    add(&state, 4, "새 공지");
    let report = run(&state, &bucket, &signer, "2026-11-18T04:00:00.000Z").await;
    assert_eq!(
        report.seqs,
        Some((7, 11)),
        "notice 1 + vector + category, removal, notice 4"
    );
    assert!(!report.snapshot, "one snapshot a day");
    first.sync(&bucket, key);
    assert!(!first.from_snapshot);
    assert_eq!(first.applied, 5);
    first.assert_mirrors(&state);
    let mut fresh = Client::default();
    fresh.sync(&bucket, key);
    fresh.assert_mirrors(&state);

    // Nothing changed: no segment, but a newer manifest.
    let report = run(&state, &bucket, &signer, "2026-11-18T05:00:00.000Z").await;
    assert_eq!(report.seqs, None);
    first.sync(&bucket, key);
    assert_eq!(first.applied, 0);

    // A restored notice is published again in full.
    remove(&state, 2, false);
    let report = run(&state, &bucket, &signer, "2026-11-18T06:00:00.000Z").await;
    assert_eq!(report.seqs, Some((12, 13)), "notice 2 and its vector");
    first.sync(&bucket, key);
    first.assert_mirrors(&state);
}

#[tokio::test]
async fn days_are_compacted_without_gaps_and_old_files_deleted() {
    let state = State::open_in_memory().unwrap();
    let bucket = Bucket::default();
    let signer = signer();
    let key = signer.public_key();
    add(&state, 1, "첫 공지");
    run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    let mut early = Client::default();
    early.sync(&bucket, key);
    add(&state, 2, "둘째 공지");
    embed(&state, 2, 2, Some("학사"));
    run(&state, &bucket, &signer, "2026-11-18T09:00:00.000Z").await;
    let mut midday = Client::default();
    midday.sync(&bucket, key);
    add(&state, 3, "셋째 공지");
    run(&state, &bucket, &signer, "2026-11-18T15:00:00.000Z").await;
    let manifest = manifest_of(&bucket, key);
    assert_eq!(manifest.segments.len(), 3);
    let day_one_runs: Vec<String> = manifest
        .segments
        .iter()
        .map(|s| s.records.path.clone())
        .collect();

    // The next UTC day: one daily segment for the same seqs, and a new snapshot.
    add(&state, 4, "다음 날 공지");
    let report = run(&state, &bucket, &signer, "2026-11-19T00:30:00.000Z").await;
    assert_eq!((report.compacted_days, report.snapshot), (1, true));
    let manifest = manifest_of(&bucket, key);
    let ranges: Vec<(u64, u64)> = manifest
        .segments
        .iter()
        .map(|s| (s.from_seq, s.to_seq))
        .collect();
    assert_eq!(ranges, [(1, 5), (6, 6)]);
    assert!(manifest.segments[0]
        .records
        .path
        .starts_with("segments/day-1-5-"));
    assert_eq!(manifest.snapshot.through_seq, 6);
    // Retired run files stay a day for clients still on the old manifest.
    for path in &day_one_runs {
        assert!(bucket.keys().contains(&format!("{PREFIX}{path}")), "{path}");
    }
    // Clients resume from inside the daily segment, or from its end.
    early.sync(&bucket, key);
    assert!(!early.from_snapshot);
    // Notice 2 with its vector and category, notice 3, then notice 4.
    assert_eq!(early.applied, 5);
    early.assert_mirrors(&state);
    midday.sync(&bucket, key);
    midday.assert_mirrors(&state);

    // A day after retirement the run files and the old snapshot are gone.
    let report = run(&state, &bucket, &signer, "2026-11-20T01:00:00.000Z").await;
    // Three run segments, one vectors file and the first snapshot.
    assert_eq!(report.deleted, 5);
    let keys = bucket.keys();
    for path in &day_one_runs {
        assert!(!keys.contains(&format!("{PREFIX}{path}")), "{path}");
    }
    // Left besides the listed files: only day two's run segment, compacted
    // and retired by this run. An unchanged snapshot is the same file.
    let listed = listed_keys(&manifest_of(&bucket, key));
    let extra: Vec<&String> = keys.iter().filter(|k| !listed.contains(k)).collect();
    assert_eq!(extra.len(), 1, "{extra:?}");
    assert!(extra[0].starts_with("feed/segments/run-6-6-"));

    // Daily segments older than 30 days leave the manifest.
    run(&state, &bucket, &signer, "2026-12-20T01:00:00.000Z").await;
    let manifest = manifest_of(&bucket, key);
    assert!(manifest.segments.is_empty(), "{:?}", manifest.segments);
    let mut late = Client::default();
    late.sync(&bucket, key);
    late.assert_mirrors(&state);
}

#[tokio::test]
async fn a_state_behind_the_published_feed_is_refused() {
    let bucket = Bucket::default();
    let signer = signer();
    let state = State::open_in_memory().unwrap();
    add(&state, 1, "공지");
    run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    // A rolled-back state would assign seq 1 again.
    let behind = State::open_in_memory().unwrap();
    add(&behind, 2, "다른 공지");
    let error = prepare(
        &behind,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T04:00:00.000Z",
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("reuse seqs"), "{error}");
    // So would a clock behind the live manifest, and another key.
    let error = prepare(
        &state,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T03:00:00.000Z",
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("clock"), "{error}");
    let error = prepare(
        &state,
        &bucket,
        PREFIX,
        &other_signer(),
        "2026-11-18T05:00:00.000Z",
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("does not verify"), "{error}");
}

#[tokio::test]
async fn a_lost_state_upload_reuses_only_seqs_no_client_saw() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite3");
    let bucket = Bucket::default();
    let signer = signer();
    let key = signer.public_key();
    let state = State::open(&path).unwrap();
    add(&state, 1, "공지");
    run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    let mut client = Client::default();
    client.sync(&bucket, key);
    let saved = directory.path().join("saved.sqlite3");
    state.vacuum_into(&saved).unwrap();

    // This run's files are uploaded, but its state never is.
    add(&state, 2, "올라가지 못한 공지");
    prepare(
        &state,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T04:00:00.000Z",
        false,
    )
    .await
    .unwrap();
    drop(state);

    // The next run starts from the saved state: seq 2 is assigned again.
    let state = State::open(&saved).unwrap();
    add(&state, 3, "다음 공지");
    let report = run(&state, &bucket, &signer, "2026-11-18T05:00:00.000Z").await;
    assert_eq!(report.seqs, Some((2, 2)));
    assert_eq!(report.deleted, 1, "the orphaned run segment");
    assert_eq!(bucket.keys(), listed_keys(&manifest_of(&bucket, key)));
    client.sync(&bucket, key);
    client.assert_mirrors(&state);
}

#[tokio::test]
async fn a_restart_brings_every_client_to_the_rolled_back_state() {
    let directory = tempfile::tempdir().unwrap();
    let bucket = Bucket::default();
    let signer = signer();
    let key = signer.public_key();
    let state = State::open(&directory.path().join("state.sqlite3")).unwrap();
    add(&state, 1, "공지");
    add(&state, 4, "지울 공지");
    run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    let mut early = Client::default();
    early.sync(&bucket, key);
    let saved = directory.path().join("saved.sqlite3");
    state.vacuum_into(&saved).unwrap();

    // Published after the saved state: a vector of the same revision, a new
    // notice and a removal.
    embed(&state, 1, 1, Some("학사"));
    add(&state, 2, "되돌릴 공지");
    embed(&state, 2, 2, Some("학사"));
    remove(&state, 4, true);
    run(&state, &bucket, &signer, "2026-11-18T04:00:00.000Z").await;
    let mut late = Client::default();
    late.sync(&bucket, key);
    let old = manifest_of(&bucket, key);
    assert_eq!(late.cursor, Some(8));
    drop(state);

    // The rolled-back state, after a crawl of its own.
    let state = State::open(&saved).unwrap();
    add(&state, 3, "다음 공지");
    let error = prepare(
        &state,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T05:00:00.000Z",
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("reuse seqs"), "{error}");
    let report = prepare(
        &state,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T05:00:00.000Z",
        true,
    )
    .await
    .unwrap();
    // No segment, only a snapshot of notices 1, 3 and 4 after seq 8.
    assert_eq!((report.seqs, report.snapshot), (None, true));
    finish(&state, &bucket, PREFIX, &signer, "2026-11-18T05:00:00.000Z")
        .await
        .unwrap();
    let manifest = manifest_of(&bucket, key);
    assert!(manifest.segments.is_empty());
    assert_eq!(manifest.snapshot.through_seq, 11);

    // A client from either side of the rollback, and a new one, each reach
    // the rolled-back state in one sync.
    for client in [&mut early, &mut late, &mut Client::default()] {
        client.sync(&bucket, key);
        assert!(client.from_snapshot);
        client.assert_mirrors(&state);
    }

    // The next run continues the seqs; the old files stay a day.
    add(&state, 5, "재시작 뒤 공지");
    let report = run(&state, &bucket, &signer, "2026-11-18T06:00:00.000Z").await;
    assert_eq!(report.seqs, Some((12, 12)));
    let old_keys = listed_keys(&old);
    let old_files: Vec<&String> = old_keys
        .iter()
        .filter(|k| !k.ends_with("manifest.json") && !k.ends_with(".sig"))
        .collect();
    for key in &old_files {
        assert!(bucket.keys().contains(key), "{key}");
    }
    late.sync(&bucket, key);
    assert!(!late.from_snapshot);
    assert_eq!(late.applied, 1);
    late.assert_mirrors(&state);
    let report = run(&state, &bucket, &signer, "2026-11-19T06:00:00.000Z").await;
    assert!(report.deleted >= old_files.len());
    for key in &old_files {
        assert!(!bucket.keys().contains(key), "{key}");
    }
    late.sync(&bucket, key);
    late.assert_mirrors(&state);
}

#[tokio::test]
async fn a_restart_needs_a_published_feed_and_a_notice() {
    let bucket = Bucket::default();
    let signer = signer();
    let state = State::open_in_memory().unwrap();
    add(&state, 1, "공지");
    let error = prepare(
        &state,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T03:00:00.000Z",
        true,
    )
    .await
    .unwrap_err();
    assert!(error.contains("needs a published feed"), "{error}");
    run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    // A snapshot of nothing would leave a client at the old cursor as it is.
    let empty = State::open_in_memory().unwrap();
    let error = prepare(
        &empty,
        &bucket,
        PREFIX,
        &signer,
        "2026-11-18T04:00:00.000Z",
        true,
    )
    .await
    .unwrap_err();
    assert!(error.contains("no notice"), "{error}");
}

#[test]
fn the_signing_key_is_read_from_openssl_pem() {
    // RFC 8032 test 1 as `openssl genpkey -algorithm ed25519` would print it.
    let pem = "-----BEGIN PRIVATE KEY-----\n\
               MC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n\
               -----END PRIVATE KEY-----\n";
    let signer = signer_from_pem(pem).unwrap();
    assert_eq!(
        crate::hex(&signer.public_key()),
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    );
    for bad in [
        "",
        "MC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g",
        "-----BEGIN PRIVATE KEY-----\nnot*base64\n-----END PRIVATE KEY-----",
        "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----",
    ] {
        assert!(signer_from_pem(bad).is_err(), "{bad}");
    }
    assert_eq!(base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
}

#[test]
fn timestamps_are_rfc3339_with_milliseconds() {
    let time = UNIX_EPOCH + Duration::from_millis(1_795_489_200_123);
    assert_eq!(timestamp(time), "2026-11-24T03:00:00.123Z");
}

fn calendar_event(uid: &str, title: &str, day: &str) -> rhythm_sources::calendar::Event {
    rhythm_sources::calendar::Event {
        uid: uid.into(),
        title: title.into(),
        start_day: day.into(),
        end_day: day.into(),
    }
}

#[tokio::test]
async fn calendar_events_reach_clients_through_segments_and_snapshots() {
    const CALENDAR: &str = rhythm_sources::calendar::ACADEMIC_CALENDAR_ID;
    let state = State::open_in_memory().unwrap();
    let bucket = Bucket::default();
    let signer = signer();
    let key = signer.public_key();
    add(&state, 1, "공지");
    let events = [
        calendar_event("a", "개강", "2026-09-01"),
        calendar_event("b", "중간시험", "2026-10-20"),
    ];
    state
        .calendar_apply(CALENDAR, &events, "2026-11-18T02:00:00.000Z")
        .unwrap();
    let report = run(&state, &bucket, &signer, "2026-11-18T03:00:00.000Z").await;
    assert_eq!(report.seqs, Some((1, 3)), "the notice and two events");
    let manifest = manifest_of(&bucket, key);
    assert_eq!(manifest.format, "1.2");
    assert_eq!(manifest.calendars.len(), 1);
    assert_eq!(manifest.calendars[0].calendar_id, CALENDAR);
    assert_eq!(
        manifest.calendars[0].last_success_at.as_deref(),
        Some("2026-11-18T02:00:00.000Z")
    );
    let mut first = Client::default();
    first.sync(&bucket, key);
    first.assert_mirrors(&state);
    assert_eq!(first.events.len(), 2);

    // A changed title republishes the event; an unchanged one does not.
    let changed = [
        calendar_event("a", "개강미사", "2026-09-01"),
        events[1].clone(),
    ];
    state
        .calendar_apply(CALENDAR, &changed, "2026-11-18T03:30:00.000Z")
        .unwrap();
    let report = run(&state, &bucket, &signer, "2026-11-18T04:00:00.000Z").await;
    assert_eq!(report.seqs, Some((4, 4)));
    first.sync(&bucket, key);
    first.assert_mirrors(&state);

    // Absent twice a day apart: one removal.
    let only_a = &changed[..1];
    state
        .calendar_apply(CALENDAR, only_a, "2026-11-18T04:30:00.000Z")
        .unwrap();
    let report = run(&state, &bucket, &signer, "2026-11-18T05:00:00.000Z").await;
    assert_eq!(report.seqs, None, "missing is not removed yet");
    state
        .calendar_apply(CALENDAR, only_a, "2026-11-19T05:00:00.000Z")
        .unwrap();
    let report = run(&state, &bucket, &signer, "2026-11-19T06:00:00.000Z").await;
    assert_eq!(report.seqs, Some((5, 5)));
    assert!(report.snapshot);
    first.sync(&bucket, key);
    first.assert_mirrors(&state);
    assert_eq!(first.events.len(), 1);

    // A client starting now gets the events from the snapshot, which holds
    // no removal, and one holding the removed event drops it there.
    let mut fresh = Client::default();
    fresh.sync(&bucket, key);
    assert!(fresh.from_snapshot);
    fresh.assert_mirrors(&state);
    let mut stale = Client {
        cursor: Some(u64::MAX - 1),
        ..Client::default()
    };
    stale.events.insert(
        (CALENDAR.into(), "b".into()),
        Event {
            calendar_id: CALENDAR.into(),
            uid: "b".into(),
            title: "중간시험".into(),
            start_day: "2026-10-20".into(),
            end_day: "2026-10-20".into(),
        },
    );
    stale.sync(&bucket, key);
    assert!(stale.from_snapshot);
    stale.assert_mirrors(&state);
}
