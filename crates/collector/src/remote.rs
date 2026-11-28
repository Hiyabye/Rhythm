//! The authoritative state in private R2 (#166 D23). A run downloads the
//! newest version into a new local file, crawls into it and uploads it as a
//! new version; the newest `KEEP` versions stay for rollback. A run that fails
//! before its upload writes nothing to R2, so the previous version stays current.
use crate::{
    r2::{self, ObjectStore},
    sha256_hex,
    state::State,
};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    time::SystemTime,
};

pub const BUCKET: &str = "rhythm-state";
/// About a day of rollback at hourly runs, small enough to stay in R2's free
/// storage (user decision, 2026-11-28).
pub const KEEP: usize = 24;
const SUFFIX: &str = ".sqlite3.zst";
/// A mid level: level 19 (#172's feed figures) costs tens of seconds on a
/// state of several hundred MB every run, for little gain on a private copy.
const LEVEL: i32 = 9;
/// The largest decompressed state a run accepts, a bound against a damaged or
/// hostile object rather than a size target. The state outgrew 512 MiB on
/// 2026-11-27 with about 53,000 notices, their vectors and a run segment that
/// republished every notice; the compressed object must also stay under
/// `r2::MAX_OBJECT`.
const MAX_STATE: u64 = 4 * 1024 * 1024 * 1024;

/// A prefix names one state history, such as `state/`.
pub fn validate_prefix(prefix: &str) -> Result<(), String> {
    let plain = prefix
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'/'));
    if !plain || !prefix.ends_with('/') || prefix.starts_with('/') || prefix.contains("//") {
        return Err(format!(
            "invalid state prefix {prefix}: use a form like state/"
        ));
    }
    Ok(())
}

/// The state versions under the prefix, oldest first. Other keys are ignored.
fn versions(keys: Vec<String>, prefix: &str) -> Vec<String> {
    let mut versions: Vec<_> = keys
        .into_iter()
        .filter(|key| {
            key.strip_prefix(prefix)
                .and_then(|name| name.strip_suffix(SUFFIX))
                .is_some_and(is_stamp)
        })
        .collect();
    versions.sort();
    versions
}

/// `YYYYMMDDTHHMMSSZ`, so names sort in time order.
fn is_stamp(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 16
        && bytes[8] == b'T'
        && bytes[15] == b'Z'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 8 || i == 15 || b.is_ascii_digit())
}

/// Downloads the newest version into `path`, which must not exist yet, and
/// returns its key. With `init`, an empty prefix yields `None` (a new state)
/// and an existing version is refused; without it, an empty prefix is an
/// error, so a wrong prefix never silently starts a new history.
pub async fn download<S: ObjectStore>(
    store: &S,
    prefix: &str,
    path: &Path,
    init: bool,
) -> Result<Option<String>, String> {
    validate_prefix(prefix)?;
    // A stale local copy must never be crawled into and uploaded as the next version.
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let versions = versions(store.list(prefix).await?, prefix);
    let key = match (versions.last(), init) {
        (None, true) => return Ok(None),
        (None, false) => {
            return Err(format!(
                "no collector state under {prefix}; pass --init to start a new one"
            ))
        }
        (Some(key), true) => return Err(format!("--init refused: {key} already exists")),
        (Some(key), false) => key.clone(),
    };
    let object = store.get(&key).await?;
    if object.sha256.as_deref() != Some(sha256_hex(&object.bytes).as_str()) {
        return Err(format!("{key} does not match its recorded SHA-256"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let written = decompress_into(&object.bytes, &mut file, MAX_STATE)
        .map_err(|e| format!("{key}: {e}"))
        .and_then(|()| {
            file.sync_all()
                .map_err(|e| format!("{}: {e}", path.display()))
        });
    if let Err(error) = written {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(Some(key))
}

/// Uploads the state as a new version and returns its key. `base` is the
/// version the run started from; if another version appeared since, or the
/// clock is behind the newest version, nothing is uploaded. `snapshot` is a
/// scratch path for the compact copy, removed afterwards.
pub async fn upload<S: ObjectStore>(
    store: &S,
    prefix: &str,
    state: &State,
    base: Option<&str>,
    now: SystemTime,
    snapshot: &Path,
) -> Result<String, String> {
    validate_prefix(prefix)?;
    let versions = versions(store.list(prefix).await?, prefix);
    let newest = versions.last().map(String::as_str);
    if newest != base {
        return Err(format!(
            "the state changed in R2 during this run (newest {}, started from {}); not uploading",
            newest.unwrap_or("none"),
            base.unwrap_or("none")
        ));
    }
    let key = format!("{prefix}{}{SUFFIX}", r2::amz_date(now));
    if newest.is_some_and(|newest| key.as_str() <= newest) {
        return Err(format!(
            "{key} does not sort after {}; is the clock behind?",
            newest.unwrap_or_default()
        ));
    }
    let _ = fs::remove_file(snapshot);
    let bytes = state
        .vacuum_into(snapshot)
        .and_then(|()| fs::read(snapshot).map_err(|e| e.to_string()));
    let _ = fs::remove_file(snapshot);
    store.put(&key, compress(&bytes?, r2::MAX_OBJECT)?).await?;
    Ok(key)
}

/// Deletes all but the newest `KEEP` versions and returns how many it deleted.
pub async fn prune<S: ObjectStore>(store: &S, prefix: &str) -> Result<usize, String> {
    let versions = versions(store.list(prefix).await?, prefix);
    let old = versions.len().saturating_sub(KEEP);
    for key in &versions[..old] {
        store.delete(key).await?;
    }
    Ok(old)
}

/// Compresses the state and refuses a result over `limit` bytes: the next run
/// could not read such a version back, and every run after it would fail.
fn compress(bytes: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut encoder = zstd::Encoder::new(Vec::new(), LEVEL).map_err(|e| e.to_string())?;
    // The frame checksum catches a damaged object even before SQLite reads it.
    encoder.include_checksum(true).map_err(|e| e.to_string())?;
    encoder.write_all(bytes).map_err(|e| e.to_string())?;
    let compressed = encoder.finish().map_err(|e| e.to_string())?;
    if compressed.len() > limit {
        return Err(format!(
            "the compressed state is {} bytes, over the {limit} a run can read back; not uploading",
            compressed.len()
        ));
    }
    Ok(compressed)
}

/// Streams the decompressed state into `out`, so a large state never sits in
/// memory twice, and refuses one over `limit` bytes.
fn decompress_into(bytes: &[u8], out: &mut impl Write, limit: u64) -> Result<(), String> {
    let decoder = zstd::Decoder::new(bytes).map_err(|e| e.to_string())?;
    let written = io::copy(&mut decoder.take(limit + 1), out).map_err(|e| e.to_string())?;
    if written > limit {
        return Err("decompressed state is too large".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
