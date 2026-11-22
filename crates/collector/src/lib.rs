//! Rhythm's v2 server collector (#166, #174): the authoritative crawl state and
//! the crawl of the public Sogang boards. One invocation is one bounded run;
//! the R2 state round-trip, embeddings and publishing are added on top.
//!
//! The crawl rules are ported from the desktop app's `archive/sync` (user
//! decision, #174): the app keeps its own copy until its board crawler is
//! removed (#178), so a crawl-rule fix in the meantime belongs in both.
pub mod crawl;
pub mod embedding;
pub mod measured;
pub mod publish;
pub mod r2;
pub mod remote;
pub mod state;
pub mod workers_ai;

use sha2::{Digest, Sha256};

/// The collector's version; skipped notices get fresh attempts when it changes.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Lowercase hex SHA-256, the form every stored and signed hash takes.
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
