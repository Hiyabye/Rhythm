//! The 식단 page of the `meals` extension (#318).

use super::App;
use crate::{
    extensions::MEALS,
    storage::{self, with_store},
};

pub struct Meals<'a>(pub(super) &'a App);

impl Meals<'_> {
    /// The dormitory menu's current and next week. Menu weeks come only from
    /// the feed; this starts no network work.
    pub async fn weeks(&self) -> Result<storage::meals::Meals, String> {
        let _lease = self.0.operation_gate.shared().await?;
        // Menu weeks are Korean dates; the page opens on today there.
        let today = (chrono::Utc::now() + chrono::Duration::hours(9))
            .format("%Y-%m-%d")
            .to_string();
        with_store(&self.0.store, |s| {
            if !s.enabled(MEALS)? {
                return Err("식단 확장 기능이 꺼져 있습니다.".into());
            }
            s.meal_weeks(&today)
        })
    }
}
