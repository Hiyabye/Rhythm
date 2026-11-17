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
