use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceDefinition {
    pub id: String,
    pub name: String,
    pub organization: String,
    pub description: String,
    pub url: String,
    pub family: String,
    pub supported: bool,
    /// The innermost settings group (`SourceGroup`) the board is listed under.
    #[serde(default)]
    pub group: Option<String>,
}
/// A nested settings group of boards. Turning a group off stops every board
/// under it without changing each board's own pick.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceGroup {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub enabled: bool,
}
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
pub struct ResourceInput {
    pub url: String,
    pub filename: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticleInput {
    pub external_id: String,
    pub canonical_url: String,
    pub title: String,
    pub author: Option<String>,
    pub category: Option<String>,
    pub published_at: String,
    pub modified_at: Option<String>,
    pub original: String,
    pub unsupported_body: bool,
    pub body_html: String,
    pub body_text: String,
    pub resources: Vec<ResourceInput>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
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
    pub revision_id: i64,
    pub revisions: Vec<RevisionInfo>,
    pub resources: Vec<ResourceRecord>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArchiveQuery {
    #[serde(default)]
    pub query: String,
    pub source_id: Option<String>,
    pub category: Option<String>,
    #[serde(default)]
    pub saved_only: bool,
    #[serde(default)]
    pub unread_only: bool,
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveSummary {
    pub documents: u64,
    pub unread: u64,
    pub saved: u64,
    pub new_since_sync: u64,
    pub resource_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatus {
    pub running: bool,
    pub mode: String,
    pub message: String,
    /// Boards collecting now; boards on different hosts run at once.
    pub active: Vec<ActiveSource>,
}
/// `joined` is whether a running refresh of every board collects the board.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscribeResult {
    pub sources: Vec<SourceInfo>,
    pub groups: Vec<SourceGroup>,
    pub joined: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveSource {
    pub source_id: String,
    pub mode: String,
    pub page: u32,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct ListingEntry {
    pub external_id: String,
    pub url: String,
    pub pinned: bool,
    /// The site's own modification timestamp from a listing document whose
    /// entries carry one (the mechanical sitemap). The sync skips an unchanged
    /// entry without a request while the archived value matches.
    pub modified_at: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ListingPage {
    pub entries: Vec<ListingEntry>,
    pub has_next: bool,
    pub total_pages: u32,
}
