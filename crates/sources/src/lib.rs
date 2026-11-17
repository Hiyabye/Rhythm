//! Sogang notice sources shared by the desktop app and the collector: the board
//! catalog, page parsers, the URL policy for automatic fetches, PDF text and
//! notice date extraction. Nothing here performs I/O or touches storage.
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

    #[test]
    fn a_selector_is_parsed_once_and_shared() {
        let first = selector(".board-list-title a");
        assert!(std::ptr::eq(first, selector(".board-list-title a")));
        assert!(!std::ptr::eq(first, selector(".board-list-date")));
        let built = format!("input[name={}]", "idx");
        assert!(std::ptr::eq(selector(&built), selector("input[name=idx]")));
    }
}
