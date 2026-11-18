//! The feed publisher (docs/FEED.md, #175; user decisions 2026-11-18). A run
//! with `--publish` calls `prepare` before the state upload and `finish`
//! after it:
//!
//! 1. `prepare` reads the live manifest and refuses to publish when it lists
//!    a seq this state has not assigned (a rolled-back state would reuse it)
//!    or is not older than this run's clock. It deletes files retired at least
//!    a day ago and objects no state row names (left by a run whose state
//!    upload failed). With `restart` it first forgets what the state
//!    published, so the feed starts again at seq 1 (`--reset-feed`, for a
//!    rolled-back state); the old files stay while the live manifest lists
//!    them and the next run deletes them. On the first run of a UTC day it
//!    compacts earlier days' run segments into daily segments and writes a
//!    snapshot. The records
//!    that changed become one run segment. Files are uploaded before the
//!    state records them.
//! 2. `finish` writes and signs the manifest from the state, so clients only
//!    see seqs an uploaded state knows.
use crate::{
    embedding,
    r2::{self, ObjectStore},
    sha256_hex,
    state::{Commit, FileRow, Published, Scanned, State},
    workers_ai::MODEL,
};
use rhythm_feed::{
    manifest::{EmbeddingInfo, Segment, Snapshot},
    record::{self, Record, Removal},
    vector, FileRef, Manifest, Signer, DIMENSIONS, FORMAT, VECTOR_BYTES,
};
use std::{
    collections::{BTreeMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};

pub const BUCKET: &str = "rhythm-feed";
const MANIFEST: &str = "manifest.json";
const SIGNATURE: &str = "manifest.json.sig";
/// The secret holding the maintainer's PEM PKCS#8 signing key.
pub const KEY_VARIABLE: &str = "RHYTHM_FEED_SIGNING_KEY";

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// The seqs of this run's segment, when anything changed.
    pub seqs: Option<(u64, u64)>,
    pub compacted_days: usize,
    pub snapshot: bool,
    pub deleted: usize,
}

/// A record with its f16 vector as stored, before quantization.
type Pending<'a> = (u64, Record, Option<&'a [u8]>);
type Line = (u64, Record, Option<[u8; VECTOR_BYTES]>);
/// A file to upload: its path under the prefix and its bytes.
type Upload = (String, Vec<u8>);

/// Everything before the state upload. Files uploaded before a later failure
/// are named by no state row, so the next run deletes them.
pub async fn prepare<S: ObjectStore>(
    state: &State,
    store: &S,
    prefix: &str,
    signer: &Signer,
    now: &str,
    restart: bool,
) -> Result<Report, String> {
    let keys = store.list(prefix).await?;
    let live = live_manifest(store, prefix, signer, &keys).await?;
    if restart {
        state.feed_reset()?;
    }
    let next_seq = state.feed_next_seq()?;
    if let Some(live) = &live {
        let last = live
            .segments
            .last()
            .map_or(0, |s| s.to_seq)
            .max(live.snapshot.through_seq);
        if last >= next_seq && !restart {
            return Err(format!(
                "the published feed reaches seq {last} but this state only {}: refusing to reuse seqs",
                next_seq - 1
            ));
        }
        if live.generated_at.as_str() >= now {
            return Err("the published manifest is not older than this run's clock".into());
        }
    }
    let mut report = Report {
        deleted: clean(state, store, prefix, live.as_ref(), &keys, now).await?,
        ..Report::default()
    };

    let day = &now[..10];
    let scan = state.feed_scan()?;
    let mut commit = Commit::default();
    let (changes, current) = plan(&scan, next_seq, &mut commit)?;
    let mut uploads = Vec::new();
    let listed = state.feed_files(true)?;

    // Earlier days' run segments become one daily segment per day.
    let mut days: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for file in listed
        .iter()
        .filter(|f| f.role == "run" && f.day.as_str() < day)
    {
        let range = days
            .entry(&file.day)
            .or_insert((file.from_seq, file.to_seq));
        *range = (range.0.min(file.from_seq), range.1.max(file.to_seq));
        commit.retire.push(file.records_path.clone());
    }
    for (file_day, (from, to)) in &days {
        let lines = state
            .feed_records(*from, *to)?
            .into_iter()
            .map(|(seq, json, vector)| stored_line(seq, &json, vector))
            .collect::<Result<Vec<_>, _>>()?;
        let (row, files) = build("daily", file_day, *from, *to, &lines)?;
        commit.files.push(row);
        uploads.extend(files);
        commit.compacted_through = Some(commit.compacted_through.unwrap_or(0).max(*to));
    }
    report.compacted_days = days.len();

    if let (Some(first), Some(last)) = (changes.first(), changes.last()) {
        let lines = quantized(&changes)?;
        let (row, files) = build("run", day, first.0, last.0, &lines)?;
        commit.files.push(row);
        uploads.extend(files);
        commit.records = lines
            .iter()
            .map(|(seq, record, vector)| {
                serde_json::to_string(record)
                    .map(|json| (*seq, json, vector.map(|v| v.to_vec())))
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?;
        report.seqs = Some((first.0, last.0));
    }

    let snapshot = listed.iter().find(|f| f.role == "snapshot");
    if snapshot.is_none_or(|s| s.day.as_str() < day) {
        let through = commit.next_seq - 1;
        let (row, files) = build("snapshot", day, through, through, &quantized(&current)?)?;
        match snapshot {
            // Nothing changed since: the same file serves the new day.
            Some(old) if old.records_path == row.records_path => {
                commit.redate = Some((old.records_path.clone(), day.to_owned()));
            }
            old => {
                commit.retire.extend(old.map(|s| s.records_path.clone()));
                commit.files.push(row);
                uploads.extend(files);
            }
        }
        report.snapshot = true;
    }

    for (path, bytes) in uploads {
        store.put(&format!("{prefix}{path}"), bytes).await?;
    }
    state.feed_commit(&commit, now)?;
    Ok(report)
}

/// The manifest, after the state that lists its files was uploaded.
pub async fn finish<S: ObjectStore>(
    state: &State,
    store: &S,
    prefix: &str,
    signer: &Signer,
    now: &str,
) -> Result<(), String> {
    let manifest = manifest(state, now)?;
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    // Checked as a client will check it, before anyone can download it.
    Manifest::parse(&bytes).map_err(|e| format!("the new manifest is invalid: {e}"))?;
    let signature = signer.sign(&bytes);
    store.put(&format!("{prefix}{MANIFEST}"), bytes).await?;
    store.put(&format!("{prefix}{SIGNATURE}"), signature).await
}

fn manifest(state: &State, now: &str) -> Result<Manifest, String> {
    let files = state.feed_files(true)?;
    let reference = |path: &str, sha256: &str, size: u64| FileRef {
        path: path.to_owned(),
        sha256: sha256.to_owned(),
        size,
    };
    let refs = |file: &FileRow| {
        (
            reference(&file.records_path, &file.records_sha256, file.records_size),
            file.vectors
                .as_ref()
                .map(|(path, sha256, size)| reference(path, sha256, *size)),
        )
    };
    let snapshot = files
        .iter()
        .find(|f| f.role == "snapshot")
        .ok_or("the state has no published snapshot")?;
    let (records, vectors) = refs(snapshot);
    Ok(Manifest {
        format: FORMAT.into(),
        generated_at: now.into(),
        moved_to: None,
        embedding: EmbeddingInfo {
            model: MODEL.into(),
            scope: embedding::scope(),
            dimensions: DIMENSIONS as u32,
            canary: state.canary_latest()?,
        },
        boards: state.feed_boards()?,
        snapshot: Snapshot {
            through_seq: snapshot.to_seq,
            records,
            vectors,
        },
        segments: files
            .iter()
            .filter(|f| f.role != "snapshot")
            .map(|file| {
                let (records, vectors) = refs(file);
                Segment {
                    from_seq: file.from_seq,
                    to_seq: file.to_seq,
                    records,
                    vectors,
                }
            })
            .collect(),
    })
}

async fn live_manifest<S: ObjectStore>(
    store: &S,
    prefix: &str,
    signer: &Signer,
    keys: &[String],
) -> Result<Option<Manifest>, String> {
    let key = format!("{prefix}{MANIFEST}");
    if !keys.contains(&key) {
        return Ok(None);
    }
    let manifest = store.get(&key).await?.bytes;
    let signature = store.get(&format!("{prefix}{SIGNATURE}")).await?.bytes;
    rhythm_feed::verify(&manifest, &signature, &[signer.public_key()])
        .map(Some)
        .map_err(|e| format!("the published manifest does not verify with this key: {e}"))
}

/// Deletes files retired a day ago that no live manifest lists, and objects
/// no state row names. Returns how many objects went.
async fn clean<S: ObjectStore>(
    state: &State,
    store: &S,
    prefix: &str,
    live: Option<&Manifest>,
    keys: &[String],
    now: &str,
) -> Result<usize, String> {
    let mut listed = HashSet::new();
    if let Some(live) = live {
        let pairs = std::iter::once((&live.snapshot.records, &live.snapshot.vectors))
            .chain(live.segments.iter().map(|s| (&s.records, &s.vectors)));
        for (records, vectors) in pairs {
            listed.insert(records.path.clone());
            listed.extend(vectors.iter().map(|v| v.path.clone()));
        }
    }
    let paths = |file: &FileRow| {
        std::iter::once(file.records_path.clone())
            .chain(file.vectors.iter().map(|(path, _, _)| path.clone()))
            .collect::<Vec<_>>()
    };
    let mut deleted = HashSet::new();
    for file in state.feed_expired(now)? {
        // A manifest that failed to replace the live one may still list it.
        if listed.contains(&file.records_path) {
            continue;
        }
        for path in paths(&file) {
            let key = format!("{prefix}{path}");
            if keys.contains(&key) {
                store.delete(&key).await?;
                deleted.insert(key);
            }
        }
        state.feed_forget(&file.records_path)?;
    }
    let known: HashSet<String> = state.feed_files(false)?.iter().flat_map(paths).collect();
    for key in keys {
        let path = &key[prefix.len()..];
        let orphan = path != MANIFEST
            && path != SIGNATURE
            && !known.contains(path)
            && !listed.contains(path)
            && !deleted.contains(key);
        if orphan {
            store.delete(key).await?;
            deleted.insert(key.clone());
        }
    }
    Ok(deleted.len())
}

/// Assigns seqs from `next_seq` to every record that changed since its last
/// publish and fills `commit` with the new published state. Returns the
/// changes and every current upsert (for a snapshot), both in seq order.
fn plan<'a>(
    scan: &'a [Scanned],
    next_seq: u64,
    commit: &mut Commit,
) -> Result<(Vec<Pending<'a>>, Vec<Pending<'a>>), String> {
    let mut seq = next_seq;
    let mut changes = Vec::new();
    let mut current = Vec::new();
    for notice in scan {
        let mut published = notice.published.clone();
        let mut emit = |record: Record, vector: Option<&'a [u8]>| {
            changes.push((seq, record, vector));
            seq += 1;
            seq - 1
        };
        if let Some(removed_at) = &notice.removed_at {
            // Only a notice clients hold is removed; the cleared digests
            // publish it again if it is ever restored.
            if published.notice_seq.is_some() && published.removed_seq.is_none() {
                let removal = Record::Remove(Removal {
                    source_id: notice.notice.source_id.clone(),
                    external_id: notice.notice.external_id.clone(),
                    removed_at: removed_at.clone(),
                });
                published = Published {
                    removed_seq: Some(emit(removal, None)),
                    ..Published::default()
                };
            }
        } else {
            let digest = sha256_hex(serde_json::to_vec(&notice.notice).map_err(|e| e.to_string())?);
            if published.notice_digest.as_deref() != Some(digest.as_str()) {
                published.notice_seq =
                    Some(emit(Record::Notice(Box::new(notice.notice.clone())), None));
                published.notice_digest = Some(digest);
                published.removed_seq = None;
            }
            current.push((
                published.notice_seq.expect("published above"),
                Record::Notice(Box::new(notice.notice.clone())),
                None,
            ));
            if let Some((embedding, f16)) = &notice.embedding {
                let digest = format!("{}\n{}", embedding.revision_hash, embedding.scope);
                let record = Record::Embedding(embedding.clone());
                if published.embedding_digest.as_deref() != Some(digest.as_str()) {
                    published.embedding_seq = Some(emit(record.clone(), Some(f16.as_slice())));
                    published.embedding_digest = Some(digest);
                }
                current.push((
                    published.embedding_seq.expect("published above"),
                    record,
                    Some(f16.as_slice()),
                ));
            }
            if let Some(classification) = &notice.classification {
                let digest =
                    sha256_hex(serde_json::to_vec(classification).map_err(|e| e.to_string())?);
                let record = Record::Classification(classification.clone());
                if published.classification_digest.as_deref() != Some(digest.as_str()) {
                    published.classification_seq = Some(emit(record.clone(), None));
                    published.classification_digest = Some(digest);
                }
                current.push((
                    published.classification_seq.expect("published above"),
                    record,
                    None,
                ));
            }
        }
        if published != notice.published {
            commit.published.push((notice.notice_id, published));
        }
    }
    commit.next_seq = seq;
    current.sort_by_key(|(seq, _, _)| *seq);
    Ok((changes, current))
}

fn quantized(pending: &[Pending]) -> Result<Vec<Line>, String> {
    pending
        .iter()
        .map(|(seq, record, f16)| {
            let vector = f16
                .map(|f16| vector::quantize(&embedding::decode(f16)))
                .transpose()
                .map_err(|e| e.to_string())?;
            Ok((*seq, record.clone(), vector))
        })
        .collect()
}

fn stored_line(seq: u64, json: &str, vector: Option<Vec<u8>>) -> Result<Line, String> {
    let record =
        serde_json::from_str(json).map_err(|e| format!("stored feed record {seq}: {e}"))?;
    let vector = vector
        .map(|v| <[u8; VECTOR_BYTES]>::try_from(v.as_slice()))
        .transpose()
        .map_err(|_| format!("stored feed vector {seq} has the wrong size"))?;
    Ok((seq, record, vector))
}

/// One records file and its vectors file, named after their seqs and
/// content. Embedding records get their index into the vectors file.
fn build(
    role: &str,
    day: &str,
    from: u64,
    to: u64,
    lines: &[Line],
) -> Result<(FileRow, Vec<Upload>), String> {
    let mut vectors = Vec::new();
    let entries: Vec<(u64, Record)> = lines
        .iter()
        .map(|(seq, record, vector)| {
            let mut record = record.clone();
            if let (Record::Embedding(embedding), Some(vector)) = (&mut record, vector) {
                embedding.vector = vectors.len() as u32;
                vectors.push(*vector);
            }
            (*seq, record)
        })
        .collect();
    let records = record::encode(&entries).map_err(|e| e.to_string())?;
    let stem = match role {
        "snapshot" => format!("snapshots/{to}"),
        "daily" => format!("segments/day-{from}-{to}"),
        _ => format!("segments/run-{from}-{to}"),
    };
    let records_path = format!("{stem}-{}.jsonl.zst", &sha256_hex(&records)[..16]);
    let mut files = Vec::new();
    let vectors = if vectors.is_empty() {
        None
    } else {
        let bytes = vector::encode(&vectors);
        let path = format!("{stem}-{}.vec", &sha256_hex(&bytes)[..16]);
        let reference = (path.clone(), sha256_hex(&bytes), bytes.len() as u64);
        files.push((path, bytes));
        Some(reference)
    };
    let row = FileRow {
        records_path: records_path.clone(),
        role: role.into(),
        day: day.into(),
        from_seq: from,
        to_seq: to,
        records_sha256: sha256_hex(&records),
        records_size: records.len() as u64,
        vectors,
    };
    files.push((records_path, records));
    Ok((row, files))
}

/// RFC 3339 UTC with milliseconds, the form every stored time takes.
pub fn timestamp(time: SystemTime) -> String {
    let stamp = r2::amz_date(time);
    let millis = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_millis())
        .unwrap_or_default();
    format!(
        "{}-{}-{}T{}:{}:{}.{millis:03}Z",
        &stamp[..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15]
    )
}

/// The signing key from `KEY_VARIABLE`.
pub fn signer_from_env() -> Result<Signer, String> {
    let pem = std::env::var(KEY_VARIABLE).map_err(|_| format!("{KEY_VARIABLE} is not set"))?;
    signer_from_pem(&pem)
}

/// A PEM `PRIVATE KEY` block, as `openssl genpkey -algorithm ed25519` prints it.
pub fn signer_from_pem(pem: &str) -> Result<Signer, String> {
    let invalid = || "the feed signing key is not a PEM Ed25519 private key".to_string();
    let body = pem
        .split_once("-----BEGIN PRIVATE KEY-----")
        .and_then(|(_, rest)| rest.split_once("-----END PRIVATE KEY-----"))
        .map(|(body, _)| body)
        .ok_or_else(invalid)?;
    let der = base64(body).ok_or_else(invalid)?;
    Signer::from_pkcs8(&der).map_err(|_| invalid())
}

/// Standard base64, ignoring whitespace and padding.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for byte in text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
    {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6 | u32::from(value)) & 0xffff;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

/// The run log line.
pub fn summary(report: &Report) -> String {
    let seqs = match report.seqs {
        Some((from, to)) => format!("seq {from} to {to}"),
        None => "no changes".into(),
    };
    format!(
        "published {seqs}; {} days compacted; snapshot {}; {} old objects deleted",
        report.compacted_days,
        if report.snapshot { "written" } else { "kept" },
        report.deleted
    )
}

#[cfg(test)]
mod tests;
