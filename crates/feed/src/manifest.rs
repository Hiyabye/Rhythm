//! The signed manifest: `manifest.json` lists the snapshot and segments with
//! their SHA-256, and `manifest.json.sig` carries an Ed25519 signature over its
//! exact bytes by a key the client trusts (#166 D10).
use crate::{Error, FORMAT_MAJOR};
use ring::{
    digest::{digest, SHA256},
    signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// `major.minor`; a client accepts any minor of its own major.
    pub format: String,
    pub generated_at: String,
    /// Where the feed lives now, when it has moved; signed like the rest.
    pub moved_to: Option<String>,
    pub embedding: EmbeddingInfo,
    pub boards: Vec<BoardHealth>,
    /// Format 1.2: per calendar; empty from an older publisher.
    #[serde(default)]
    pub calendars: Vec<CalendarHealth>,
    /// Format 1.3: per menu board (#318); empty from an older publisher.
    #[serde(default)]
    pub meals: Vec<MealHealth>,
    pub snapshot: Snapshot,
    /// Contiguous and in seq order.
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingInfo {
    pub model: String,
    /// The scope new vectors are embedded in; records name their own.
    pub scope: String,
    pub dimensions: u32,
    /// The latest drift check (#166 D27): `recorded`, `normal`, `warning` or
    /// `stopped`; absent before the first one.
    pub canary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardHealth {
    pub source_id: String,
    pub last_success_at: Option<String>,
    pub consecutive_failures: u32,
    pub history_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarHealth {
    pub calendar_id: String,
    pub last_success_at: Option<String>,
    pub consecutive_failures: u32,
}

/// The menu OCR's health: the last menu image read and how many images
/// since were unreadable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MealHealth {
    pub source_id: String,
    pub last_success_at: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub through_seq: u64,
    pub records: FileRef,
    pub vectors: Option<FileRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub from_seq: u64,
    pub to_seq: u64,
    pub records: FileRef,
    pub vectors: Option<FileRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    /// Relative to the manifest's location.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

impl FileRef {
    pub fn new(path: String, bytes: &[u8]) -> Self {
        Self {
            path,
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
        }
    }

    /// Checks downloaded bytes before anything decodes them.
    pub fn check(&self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() as u64 != self.size || sha256_hex(bytes) != self.sha256 {
            return Err(Error::Invalid(format!(
                "{} does not match the manifest",
                self.path
            )));
        }
        Ok(())
    }
}

impl Manifest {
    /// Parses and checks a manifest whose signature was already verified.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        // The major version is read first, so a newer format with another
        // shape still reports itself as one.
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        let format = value["format"].as_str().unwrap_or_default();
        let major = format
            .split_once('.')
            .filter(|(major, minor)| digits(major) && digits(minor))
            .and_then(|(major, _)| major.parse::<u32>().ok())
            .ok_or_else(|| Error::Invalid(format!("format {format:?} is not major.minor")))?;
        if major != FORMAT_MAJOR {
            return Err(Error::UnsupportedFormat(format.to_owned()));
        }
        let manifest: Self =
            serde_json::from_value(value).map_err(|e| Error::Invalid(e.to_string()))?;
        manifest.check()?;
        Ok(manifest)
    }

    fn check(&self) -> Result<(), Error> {
        let invalid = |message: &str| Err(Error::Invalid(message.into()));
        if self
            .moved_to
            .as_ref()
            .is_some_and(|url| !url.starts_with("https://"))
        {
            return invalid("moved_to is not an HTTPS URL");
        }
        let files = std::iter::once((&self.snapshot.records, &self.snapshot.vectors))
            .chain(self.segments.iter().map(|s| (&s.records, &s.vectors)))
            .flat_map(|(records, vectors)| std::iter::once(records).chain(vectors));
        for file in files {
            if !safe_path(&file.path) || !is_sha256(&file.sha256) {
                return invalid("a file has an unsafe path or a malformed SHA-256");
            }
        }
        if self.segments.iter().any(|s| s.from_seq > s.to_seq)
            || self
                .segments
                .windows(2)
                .any(|pair| pair[0].to_seq.checked_add(1) != Some(pair[1].from_seq))
        {
            return invalid("segments are not contiguous");
        }
        // A client starting from the snapshot continues with the segment
        // after its seq, so the snapshot must fall inside the listed range.
        if let (Some(first), Some(last)) = (self.segments.first(), self.segments.last()) {
            let through = self.snapshot.through_seq;
            if through
                .checked_add(1)
                .is_none_or(|next| next < first.from_seq)
                || through > last.to_seq
            {
                return invalid("the snapshot is outside the listed segments");
            }
        }
        Ok(())
    }
}

/// The contents of `manifest.json.sig`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub key_id: String,
    /// Hex Ed25519 signature over the exact manifest bytes.
    pub signature: String,
}

/// The first 8 bytes of the public key's SHA-256, in hex.
pub fn key_id(public_key: &[u8; 32]) -> String {
    sha256_hex(public_key)[..16].to_owned()
}

/// The publisher's key, from the PKCS#8 document the maintainer generated.
pub struct Signer(Ed25519KeyPair);

impl Signer {
    pub fn from_pkcs8(document: &[u8]) -> Result<Self, Error> {
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(document)
            .map(Self)
            .map_err(|_| Error::Invalid("not an Ed25519 PKCS#8 key".into()))
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.0
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public keys are 32 bytes")
    }

    /// The `manifest.json.sig` bytes for `manifest`.
    pub fn sign(&self, manifest: &[u8]) -> Vec<u8> {
        let signature = Signature {
            key_id: key_id(&self.public_key()),
            signature: hex(self.0.sign(manifest).as_ref()),
        };
        serde_json::to_vec(&signature).expect("a signature serializes")
    }
}

/// Verifies the signature with one of `trusted`, then parses the manifest.
pub fn verify(manifest: &[u8], signature: &[u8], trusted: &[[u8; 32]]) -> Result<Manifest, Error> {
    let bad = || Error::BadSignature;
    let signature: Signature = serde_json::from_slice(signature).map_err(|_| bad())?;
    let key = trusted
        .iter()
        .find(|key| key_id(key) == signature.key_id)
        .ok_or_else(bad)?;
    let bytes = unhex(&signature.signature).ok_or_else(bad)?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(manifest, &bytes)
        .map_err(|_| bad())?;
    Manifest::parse(manifest)
}

/// Relative, `/`-separated, no empty, `.` or `..` component, and only ASCII
/// letters, digits, `.`, `_` and `-`.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(digest(&SHA256, bytes).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}
