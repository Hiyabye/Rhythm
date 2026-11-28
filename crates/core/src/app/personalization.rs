//! Personalization: the 나를 위한 소식 list and the 개인화 settings pane, which
//! belong to the `recommendations` extension, and the reader's per-notice
//! feedback, which the core reader shows whatever is on. Everything stays in
//! SQLite; nothing here makes a network call.

use super::App;
use crate::{
    extensions::RECOMMENDATIONS,
    storage::{
        personalization::{
            PersonalInterestInput, PersonalizationProfile, PersonalizationRecommendations,
            PersonalizationSnapshot,
        },
        with_store, Store,
    },
};

const OFF: &str = "나를 위한 소식 확장 기능이 꺼져 있습니다.";

pub struct Personalization<'a>(pub(super) &'a App);

impl Personalization<'_> {
    pub async fn snapshot(&self) -> Result<PersonalizationSnapshot, String> {
        self.pane(|s| s.personalization_snapshot()).await
    }

    pub async fn save_profile(
        &self,
        profile: PersonalizationProfile,
    ) -> Result<PersonalizationSnapshot, String> {
        self.pane(move |s| s.personalization_save_profile(&profile))
            .await
    }

    pub async fn save_interest(
        &self,
        interest: PersonalInterestInput,
    ) -> Result<PersonalizationSnapshot, String> {
        self.pane(move |s| s.personalization_save_interest(&interest))
            .await
    }

    pub async fn remove_interest(&self, id: i64) -> Result<PersonalizationSnapshot, String> {
        self.pane(move |s| s.personalization_remove_interest(id))
            .await
    }

    pub async fn set_learning(&self, enabled: bool) -> Result<PersonalizationSnapshot, String> {
        self.pane(move |s| s.personalization_set_learning(enabled))
            .await
    }

    /// Removes personalization state; the archive, saved/read state and AI
    /// results stay.
    pub async fn reset(&self) -> Result<PersonalizationSnapshot, String> {
        self.pane(|s| s.personalization_reset()).await
    }

    /// At most `limit` ranked notices for 오늘.
    pub async fn recommendations(
        &self,
        limit: u32,
    ) -> Result<PersonalizationRecommendations, String> {
        self.pane(move |s| s.personalization_recommendations(limit))
            .await
    }

    /// The reader's 관심 있어요/관심 없어요 choice for one notice. The reader is
    /// core, so feedback works with every extension off; it is evidence only
    /// once recommendations or notifications read it.
    pub async fn feedback(&self, document_id: i64) -> Result<Option<String>, String> {
        self.call(move |s| s.personalization_feedback(document_id))
            .await
    }

    pub async fn set_feedback(
        &self,
        document_id: i64,
        feedback: Option<String>,
    ) -> Result<Option<String>, String> {
        self.call(move |s| s.personalization_set_feedback(document_id, feedback.as_deref()))
            .await
    }

    /// A call of the list or the settings pane, refused while
    /// `recommendations` is off.
    async fn pane<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Store) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        self.call(move |s| {
            if !s.enabled(RECOMMENDATIONS)? {
                return Err(OFF.into());
            }
            operation(s)
        })
        .await
    }

    /// Runs `operation` off the async runtime under a shared lease: ranking
    /// reads many rows.
    async fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Store) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let _lease = self.0.operation_gate.shared().await?;
        let store = self.0.store.clone();
        tokio::task::spawn_blocking(move || with_store(&store, operation))
            .await
            .map_err(|e| e.to_string())?
    }
}
