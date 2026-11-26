use crate::storage::Store;
pub use rhythm_core::extensions::*;

pub struct Registry {
    jobs: std::sync::Arc<crate::archive::sync::Controller>,
    studium: std::sync::Arc<crate::studium::controller::Controller>,
    feed: std::sync::Arc<crate::feed::Controller>,
}
impl Registry {
    pub fn new(
        jobs: std::sync::Arc<crate::archive::sync::Controller>,
        studium: std::sync::Arc<crate::studium::controller::Controller>,
        feed: std::sync::Arc<crate::feed::Controller>,
    ) -> Self {
        Self {
            jobs,
            studium,
            feed,
        }
    }
    pub fn list(&self, store: &Store) -> Result<Vec<ExtensionInfo>, String> {
        list(store)
    }
    /// Records the choice. Disabling a source extension signals archive
    /// cancellation under the caller's store lock, the same lock job admission
    /// takes, so no job of its sources can start or continue afterwards. Work
    /// that needs to await (AI, lifecycle) is stopped by the calling command.
    pub fn set(&self, store: &Store, id: &str, enabled: bool) -> Result<(), String> {
        if !is_built_in(id) {
            return Err("등록되지 않은 확장 기능입니다.".into());
        }
        // The caller holds Store's lock, which also protects admission. Commit
        // the preference before irreversible cancellation/clearing so a failed
        // write leaves the currently enabled extension and its work intact.
        store.set_enabled(id, enabled)?;
        if !enabled && matches!(id, NOTICES | CYBERCAMPUS) {
            self.jobs.cancel();
        }
        if !enabled && id == NOTICES {
            self.feed.cancel();
            self.jobs.bodies.cancel();
        }
        if !enabled && id == STUDIUM {
            self.studium.cancel();
            self.studium.clear();
        }
        Ok(())
    }
    pub fn stop_all(&self) {
        self.jobs.cancel();
        self.studium.cancel();
        self.feed.cancel();
        self.jobs.bodies.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_extension_preserves_preferences_without_automatic_networking() {
        let (_directory, store) = crate::storage::temp_store();
        let registry = Registry::new(
            crate::archive::sync::Controller::new(),
            crate::studium::controller::Controller::new(),
            Default::default(),
        );
        let listed = registry.list(&store).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.id).collect::<Vec<_>>(),
            [
                NOTICES,
                CYBERCAMPUS,
                SEARCH,
                SAVED,
                RECOMMENDATIONS,
                WEEK,
                CALENDAR,
                AI,
                PROACTIVE,
                STUDIUM,
                MEALS
            ]
        );
        // A new profile starts with every extension off.
        assert!(listed.iter().all(|e| !e.enabled && e.available));
        registry.set(&store, NOTICES, true).unwrap();
        registry.set(&store, AI, true).unwrap();
        let enabled = registry.list(&store).unwrap();
        let on = |listing: &[ExtensionInfo], id: &str| {
            listing.iter().find(|e| e.id == id).unwrap().enabled
        };
        assert!(on(&enabled, NOTICES) && on(&enabled, AI));
        assert!(!on(&enabled, CYBERCAMPUS));
        registry.set(&store, NOTICES, false).unwrap();
        assert!(!on(&registry.list(&store).unwrap(), NOTICES));
        assert_eq!(store.settings().unwrap().theme, "system");
        assert!(registry.set(&store, "unknown", true).is_err());
    }

    #[test]
    fn studium_can_toggle_without_changing_targets_or_starting_work() {
        let (_directory, store) = crate::storage::temp_store();
        let registry = Registry::new(
            crate::archive::sync::Controller::new(),
            crate::studium::controller::Controller::new(),
            Default::default(),
        );
        assert!(!store.enabled(STUDIUM).unwrap());
        registry.set(&store, STUDIUM, true).unwrap();
        store
            .studium_save_preferences(&["5".into(), "6".into()], "6")
            .unwrap();
        registry.set(&store, STUDIUM, false).unwrap();
        assert!(!store.enabled(STUDIUM).unwrap());
        assert_eq!(store.studium_preferences().unwrap().view_room, "6");
        assert_eq!(store.studium_preferences().unwrap().room_ids, ["5", "6"]);
    }
}
