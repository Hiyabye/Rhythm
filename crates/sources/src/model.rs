use serde::{Deserialize, Serialize};

/// The closed set of source families. `SourceDefinition::family` stays the
/// stored string (catalog, SQLite, feed); this enum is parsed from it at each
/// use so a new family is a compile error wherever it must be handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Main,
    Computing,
    Community,
    Engineering,
    K2web,
    Mechanical,
    Semiconductor,
    Vendor,
    Wordpress,
    Admission,
    /// The eClass connector: not a public board, so no parser handles it.
    Cybercampus,
}

impl Family {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "main" => Self::Main,
            "computing" => Self::Computing,
            "community" => Self::Community,
            "engineering" => Self::Engineering,
            "k2web" => Self::K2web,
            "mechanical" => Self::Mechanical,
            "semiconductor" => Self::Semiconductor,
            "vendor" => Self::Vendor,
            "wordpress" => Self::Wordpress,
            "admission" => Self::Admission,
            "cybercampus" => Self::Cybercampus,
            _ => return None,
        })
    }

    /// Whether listings and notices are JSON documents (the main site's
    /// BbsData API and the WordPress REST API) rather than HTML pages.
    pub fn json_documents(self) -> bool {
        matches!(self, Self::Main | Self::Wordpress)
    }

    /// Whether the listing is a sitemap of every current notice, so the
    /// collector audits deletions by set difference and cannot window it.
    pub fn sitemap_listing(self) -> bool {
        self == Self::Mechanical
    }

    /// Whether a response content type is what a listing (`listing`) or a
    /// notice of this family should have. A different type is more likely a
    /// block or error page. The mechanical sitemap is XML; its notices must
    /// still be HTML.
    pub fn accepts_mime(self, mime: Option<&str>, listing: bool) -> bool {
        if self.json_documents() {
            mime == Some("application/json")
        } else if self.sitemap_listing() && listing {
            matches!(
                mime,
                Some("text/html" | "application/xhtml+xml" | "application/xml" | "text/xml")
            )
        } else {
            matches!(mime, Some("text/html" | "application/xhtml+xml"))
        }
    }
}

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
impl SourceDefinition {
    /// The parsed family; an unknown stored string is an error, never a
    /// fallback to another family's parser.
    pub fn family(&self) -> Result<Family, String> {
        Family::parse(&self.family)
            .ok_or_else(|| format!("알 수 없는 정보원 종류입니다: {}", self.family))
    }
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
    /// The row's title as the list shows it, from a family whose articles
    /// carry no change marker (k2web). The collector's edit window re-fetches
    /// a known notice only while this differs from the archived title.
    pub title: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ListingPage {
    pub entries: Vec<ListingEntry>,
    pub has_next: bool,
    pub total_pages: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_source_has_a_family() {
        for source in crate::catalog::definitions() {
            assert!(source.family().is_ok(), "{}: {}", source.id, source.family);
        }
    }

    /// `Family::Main` replaced `parser::main_board(id).is_some()` in the JSON
    /// and size rules, so the two must name the same boards.
    #[test]
    fn main_family_is_exactly_the_main_boards() {
        for source in crate::catalog::definitions() {
            assert_eq!(
                source.family() == Ok(Family::Main),
                crate::parser::main_board(&source.id).is_some(),
                "{}",
                source.id
            );
        }
    }

    #[test]
    fn unknown_family_is_rejected() {
        assert_eq!(Family::parse("computin"), None);
        assert_eq!(Family::parse(""), None);
        let mut source = crate::catalog::definitions().remove(0);
        source.family = "mistyped".into();
        assert!(source.family().is_err());
        assert!(crate::parser::list_url(&source, 1).is_err());
        assert!(crate::parser::check_document_mime(&source, Some("text/html"), false).is_err());
    }

    #[test]
    fn mechanical_sitemap_alone_accepts_xml_listings() {
        let xml = Some("application/xml");
        assert!(Family::Mechanical.accepts_mime(xml, true));
        assert!(!Family::Mechanical.accepts_mime(xml, false));
        assert!(!Family::Computing.accepts_mime(xml, true));
        assert!(Family::Wordpress.accepts_mime(Some("application/json"), true));
        assert!(!Family::Main.accepts_mime(Some("text/html"), false));
    }
}
