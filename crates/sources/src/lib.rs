//! Sogang notice sources shared by the desktop app and the collector: the board
//! catalog, page parsers, the URL policy for automatic fetches, the constrained
//! HTTPS transport with its TLS chain completion and robots.txt policy, PDF text
//! and notice date extraction. Nothing here touches storage or the filesystem.
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

pub mod catalog;
pub mod dates;
pub mod model;
pub mod parser;
pub mod pdf;
pub mod policy;
pub mod robots;
#[cfg(feature = "studium")]
pub mod studium;
pub mod tls;
pub mod transport;

/// Parses each distinct CSS selector once. Callers pass literals or values built
/// from a small fixed set, so the cache stays small and its entries live for the
/// whole process.
pub fn selector(value: &str) -> &'static scraper::Selector {
    static CACHE: LazyLock<Mutex<HashMap<String, &'static scraper::Selector>>> =
        LazyLock::new(Default::default);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = cache.get(value) {
        return found;
    }
    let parsed: &'static scraper::Selector = Box::leak(Box::new(
        scraper::Selector::parse(value).expect("static CSS selector"),
    ));
    cache.insert(value.to_owned(), parsed);
    parsed
}

#[cfg(test)]
mod tests {
    use super::selector;
    use crate::model::{ListingEntry, ListingPage};

    /// The listed entry with this external ID, shared by the parser fixture tests.
    pub(crate) fn entry_of(page: &ListingPage, external_id: &str) -> ListingEntry {
        page.entries
            .iter()
            .find(|entry| entry.external_id == external_id)
            .unwrap()
            .clone()
    }

    #[test]
    fn a_selector_is_parsed_once_and_shared() {
        let first = selector(".board-list-title a");
        assert!(std::ptr::eq(first, selector(".board-list-title a")));
        assert!(!std::ptr::eq(first, selector(".board-list-date")));
        let built = format!("input[name={}]", "idx");
        assert!(std::ptr::eq(selector(&built), selector("input[name=idx]")));
    }
}
