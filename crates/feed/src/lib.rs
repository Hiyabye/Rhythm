//! The signed notice feed (`docs/FEED.md`): the records the collector
//! publishes and the app applies, the int8 vectors beside them, and the
//! manifest that lists both with an Ed25519 signature. No I/O: callers fetch
//! and store the bytes.
use std::fmt;

pub mod manifest;
pub mod record;
pub mod vector;

pub use manifest::{verify, FileRef, Manifest, Signer};
pub use record::{Entry, Record};

pub const FORMAT_MAJOR: u32 = 1;
pub const FORMAT_MINOR: u32 = 0;
/// `FORMAT_MAJOR.FORMAT_MINOR`, as the manifest writes it.
pub const FORMAT: &str = "1.0";
/// Body characters a notice record carries instead of its body (#166 O2).
pub const EXCERPT_CHARS: usize = 320;
pub const DIMENSIONS: usize = 1024;
pub const VECTOR_BYTES: usize = 4 + DIMENSIONS;
/// A records file may not decompress beyond this, which bounds what a
/// malicious or broken file can make a client allocate.
pub const MAX_DECODED_BYTES: u64 = 256 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A major version this build cannot read: the app must be updated.
    UnsupportedFormat(String),
    /// No trusted key signed the manifest, or the signature is malformed.
    BadSignature,
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => write!(f, "unsupported feed format {format}"),
            Self::BadSignature => f.write_str("the feed manifest has no valid trusted signature"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

/// A notice body's excerpt: its first `EXCERPT_CHARS` characters.
pub fn excerpt(body_text: &str) -> String {
    body_text.chars().take(EXCERPT_CHARS).collect()
}

/// The parts of a parsed notice that `revision_hash` covers. Markup is left
/// out, so a markup-only change is not a revision.
pub struct Revision<'a> {
    pub canonical_url: &'a str,
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub category: Option<&'a str>,
    pub published_at: &'a str,
    pub body_text: &'a str,
    /// `(url, filename, kind)` in the order the page lists them.
    pub resources: &'a [(&'a str, &'a str, &'a str)],
}

/// A notice's `revision_hash`: the SHA-256 of the JSON array of `revision`'s
/// fields in declaration order. The collector publishes it and the app
/// recomputes it over a body it fetched, to tell whether that body is the
/// revision the feed describes.
pub fn revision_hash(revision: &Revision) -> String {
    let encoded = serde_json::to_vec(&(
        revision.canonical_url,
        revision.title,
        revision.author,
        revision.category,
        revision.published_at,
        revision.body_text,
        revision.resources,
    ))
    .expect("strings serialize");
    manifest::sha256_hex(&encoded)
}

#[cfg(test)]
mod tests;
