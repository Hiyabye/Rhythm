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

#[cfg(test)]
mod tests;
