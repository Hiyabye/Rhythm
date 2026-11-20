use serde::{Deserialize, Serialize};

pub use rhythm_sources::model::{
    ArticleInput, ListingEntry, ResourceInput, SourceDefinition, SourceGroup,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    #[serde(flatten)]
    pub definition: SourceDefinition,
    /// Effective subscription: the board's pick with every enclosing group on.
    /// Collection, scheduling, ranking and notifications read only this.
    pub subscribed: bool,
    /// The user's own pick, kept while an enclosing group is off.
    #[serde(default)]
    pub selected: bool,
    pub last_attempt: Option<String>,
    pub last_success: Option<String>,
    pub last_error: Option<String>,
    pub next_page: u32,
    pub history_complete: bool,
    pub status: String,
    pub discovered: u64,
    pub archived: u64,
    pub failed: u64,
    /// All skipped notices; `skipped` lists only the most recent of them.
    #[serde(default)]
    pub skipped_count: u64,
    #[serde(default)]
    pub skipped: Vec<SkippedArticle>,
    /// The extension that owns this source; `None` for a source no extension maps.
    #[serde(default)]
    pub extension: Option<String>,
    /// Archived notices of this source not yet read.
    #[serde(default)]
    pub unread: u64,
    /// The collector's health for this board from the last accepted feed
    /// manifest; `None` for Cybercampus and before the first sync.
    #[serde(default)]
    pub feed: Option<FeedBoardHealth>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedBoardHealth {
    pub last_success_at: Option<String>,
    pub consecutive_failures: u32,
    /// Listed as failing in the feed pane (`storage::feed::state::FAILING_AFTER`).
    pub failing: bool,
    pub history_complete: bool,
}
/// A notice whose fetched response could not be archived; see `skipped_articles`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkippedArticle {
    pub external_id: String,
    pub url: String,
    pub reason: String,
    pub skipped_at: String,
    /// An earlier version is archived and stays readable.
    pub archived: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentItem {
    pub id: i64,
    pub source_id: String,
    pub source_name: String,
    pub title: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub published_at: String,
    pub collected_at: String,
    pub is_read: bool,
    pub saved: bool,
    pub historical: bool,
    pub snippet: String,
    pub content_status: String,
    /// In 휴지통: hidden from every list except the trash itself.
    #[serde(default)]
    pub trashed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceRecord {
    pub id: i64,
    pub url: String,
    pub filename: String,
    pub kind: String,
    pub status: String,
    pub mime: Option<String>,
    pub bytes: Option<u64>,
    pub local_path: Option<String>,
    pub sha256: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionInfo {
    pub id: i64,
    pub collected_at: String,
    pub content_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentDetail {
    pub item: DocumentItem,
    pub canonical_url: String,
    pub body_html: String,
    pub body_text: String,
    /// This revision holds only the feed's excerpt; its body has not been
    /// fetched from the school.
    pub summary_only: bool,
    /// The school deleted the notice; it stays because it was saved.
    pub removed: bool,
    pub revision_id: i64,
    pub revisions: Vec<RevisionInfo>,
    pub resources: Vec<ResourceRecord>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArchiveQuery {
    #[serde(default)]
    pub query: String,
    pub source_id: Option<String>,
    /// The boards placed directly in this settings group, not its nested groups.
    /// Exclusive with `source_id`.
    #[serde(default)]
    pub group_id: Option<String>,
    pub category: Option<String>,
    /// A local AI category. Only notices whose latest revision was classified
    /// under the installed embedding model and the current config match.
    #[serde(default)]
    pub ai_category: Option<String>,
    #[serde(default)]
    pub saved_only: bool,
    #[serde(default)]
    pub unread_only: bool,
    /// Lists 휴지통 instead of the notices outside it.
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub ascending: bool,
    #[serde(default)]
    pub offset: u32,
    pub limit: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivePage {
    pub items: Vec<DocumentItem>,
    pub total: u64,
    pub categories: Vec<String>,
    /// Relevance search only: notices found by meaning that match no search term.
    #[serde(default)]
    pub meaning_only: Vec<i64>,
    /// Relevance search only: whether a query vector took part in the ranking.
    #[serde(default)]
    pub semantic: bool,
    /// AI category counts under every other filter, while the AI extension is
    /// on and an embedding model is installed.
    #[serde(default)]
    pub ai_categories: Option<AiCategoryCounts>,
}
/// `counts` follows the classifier's category order and includes zeros.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiCategoryCounts {
    pub counts: Vec<AiCategoryCount>,
    /// Notices whose latest revision has no classification in this scope.
    pub unclassified: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiCategoryCount {
    pub category: String,
    pub count: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveSummary {
    pub documents: u64,
    pub unread: u64,
    pub saved: u64,
    pub new_since_sync: u64,
    pub resource_bytes: u64,
    pub trashed: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatus {
    pub running: bool,
    pub mode: String,
    pub message: String,
    /// The source collecting now, if any.
    pub active: Vec<ActiveSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscribeResult {
    pub sources: Vec<SourceInfo>,
    pub groups: Vec<SourceGroup>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveSource {
    pub source_id: String,
    pub mode: String,
    pub page: u32,
    pub message: String,
}
