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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceInfo {
    #[serde(flatten)]
    pub definition: SourceDefinition,
    pub subscribed: bool,
    pub last_attempt: Option<String>,
    pub last_success: Option<String>,
    pub last_error: Option<String>,
    pub next_page: u32,
    pub history_complete: bool,
    pub status: String,
    pub discovered: u64,
    pub archived: u64,
    pub failed: u64,
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
    pub source_id: Option<String>,
    pub mode: String,
    pub page: u32,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct ListingEntry {
    pub external_id: String,
    pub url: String,
    pub pinned: bool,
}
#[derive(Clone, Debug)]
pub struct ListingPage {
    pub entries: Vec<ListingEntry>,
    pub has_next: bool,
    pub total_pages: u32,
}
