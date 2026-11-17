//! Rhythm's v2 server collector (#166, #174): the authoritative crawl state and
//! the crawl of the public Sogang boards. One invocation is one bounded run;
//! the R2 state round-trip, embeddings and publishing are added on top.
//!
//! The crawl rules are ported from the desktop app's `archive/sync` (user
//! decision, #174): the app keeps its own copy until its board crawler is
//! removed (#178), so a crawl-rule fix in the meantime belongs in both.
pub mod crawl;
pub mod state;
