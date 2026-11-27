//! WordPress sites, read through the site's public REST API (`/wp-json/wp/v2`,
//! #80 step 4). The one site so far is 아트&테크놀로지학과 (creative.sogang.ac.kr),
//! whose only public board is the News category. The API is used instead of the
//! theme's HTML or the RSS feed because it gives every post one stable document
//! (`posts/<id>`), which the app also fetches when a notice is opened, and each
//! post's own change time, so the collector skips unchanged posts like the
//! mechanical sitemap. `_envelope` puts the page count, which WordPress sends
//! only as a header, into the list body. The site's other posts (faculty,
//! curriculum, publications) live in other categories and are never listed.
use super::*;

pub(super) const HOST: &str = "creative.sogang.ac.kr";
const POSTS: &str = "https://creative.sogang.ac.kr/wp-json/wp/v2/posts";
pub(crate) const LIST_FIELDS: &str = "id,modified_gmt";
pub(crate) const POST_FIELDS: &str = "id,date,modified_gmt,status,type,title,content,categories";
pub(crate) const PER_PAGE: u32 = 50;

/// Each board and the WordPress category it lists.
pub(crate) const BOARDS: [(&str, u64); 1] = [("creative-news", 192)];
fn board(source: &SourceDefinition) -> Result<u64, String> {
    BOARDS
        .iter()
        .find(|(id, _)| *id == source.id)
        .map(|(_, category)| *category)
        .ok_or_else(|| PARSER_ERROR.into())
}
/// `?p=<id>` is the site's own short link: it redirects to the post's current
/// permalink, which changes when an editor renames the post.
fn canonical(id: &str) -> String {
    format!("https://{HOST}/?p={id}")
}
pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    let category = board(source)?.to_string();
    let mut url = Url::parse(POSTS).map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut().extend_pairs([
        ("categories", category.as_str()),
        ("orderby", "date"),
        ("order", "desc"),
        ("per_page", PER_PAGE.to_string().as_str()),
        ("page", page.to_string().as_str()),
        ("_fields", LIST_FIELDS),
        ("_envelope", ""),
    ]);
    Ok(url)
}
pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    board(source)?;
    if entry.url != canonical(&entry.external_id) {
        return invalid();
    }
    let mut url =
        Url::parse(&format!("{POSTS}/{}", entry.external_id)).map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut().append_pair("_fields", POST_FIELDS);
    Ok(url)
}
/// WordPress writes UTC times without a zone; the feed carries RFC 3339 UTC.
fn utc(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if bytes.len() != 19
        || separators
            .iter()
            .any(|(index, byte)| bytes[*index] != *byte)
        || bytes.iter().enumerate().any(|(index, byte)| {
            !separators.iter().any(|(i, _)| *i == index) && !byte.is_ascii_digit()
        })
    {
        return invalid();
    }
    date(&value[..10])?;
    let hour = value[11..13].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let minute = value[14..16].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let second = value[17..19].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    if hour > 23 || minute > 59 || second > 59 {
        return invalid();
    }
    Ok(format!("{value}Z"))
}
/// `title.rendered` is HTML text (entities such as `&amp;` and `&#8216;`).
fn rendered_title(value: &Value) -> Result<String, String> {
    let html = Html::parse_fragment(required_string(&value["title"], "rendered")?);
    let title = text(html.root_element());
    if title.is_empty() || title.len() > 1024 {
        return invalid();
    }
    Ok(title)
}
pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    board(source)?;
    let value: Value = serde_json::from_str(body).map_err(|_| PARSER_ERROR)?;
    let headers = &value["headers"];
    match value["status"].as_u64() {
        Some(200) => {}
        // Asked beyond the last page: posts were deleted since the history
        // checkpoint was recorded. The listing simply ended there.
        Some(400) if value["body"]["code"] == "rest_post_invalid_page_number" && page > 1 => {
            return Ok(ListingPage {
                entries: Vec::new(),
                has_next: false,
                total_pages: page - 1,
            });
        }
        _ => return invalid(),
    }
    let total = headers["X-WP-Total"].as_u64().ok_or(PARSER_ERROR)?;
    let pages = headers["X-WP-TotalPages"]
        .as_u64()
        .filter(|pages| *pages <= u64::from(u32::MAX))
        .ok_or(PARSER_ERROR)? as u32;
    let rows = value["body"].as_array().ok_or(PARSER_ERROR)?;
    // The envelope must agree with itself before any arithmetic on it.
    if u64::from(pages) != total.div_ceil(u64::from(PER_PAGE)) || (page > pages && page != 1) {
        return invalid();
    }
    let expected = if page < pages {
        u64::from(PER_PAGE)
    } else if page == pages {
        total - u64::from(PER_PAGE) * u64::from(pages - 1)
    } else {
        // Only an empty first page lies beyond the page count (no posts yet).
        0
    };
    if rows.len() as u64 != expected {
        return invalid();
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for row in rows {
        let external_id = id(&row["id"])?;
        if !seen.insert(external_id.clone()) {
            return invalid();
        }
        entries.push(ListingEntry {
            url: canonical(&external_id),
            // The change time covers title and body edits alike; a listed
            // title would make the collector skip a body-only edit.
            modified_at: Some(utc(required_string(row, "modified_gmt")?)?),
            title: None,
            external_id,
            pinned: false,
        });
    }
    Ok(ListingPage {
        entries,
        has_next: page < pages,
        total_pages: pages.max(1),
    })
}
pub(super) fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let category = board(source)?;
    let value: Value = serde_json::from_str(body).map_err(|_| PARSER_ERROR)?;
    // A post moved out of the board's category, a draft or a page is not a
    // notice of this board.
    if id(&value["id"])? != entry.external_id
        || value["status"] != "publish"
        || value["type"] != "post"
        || !value["categories"]
            .as_array()
            .is_some_and(|categories| categories.iter().any(|c| c.as_u64() == Some(category)))
        || value["content"]["protected"] != false
    {
        return invalid();
    }
    let content = value["content"]["rendered"].as_str().ok_or(PARSER_ERROR)?;
    let (body_html, body_text, resources) = sanitize(content, &entry.url)?;
    if resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title: rendered_title(&value)?,
        author: None,
        category: Some(source.name.clone()),
        // The site's local (Korea) date, as on its own pages.
        published_at: date(required_string(&value, "date")?)?,
        modified_at: Some(utc(required_string(&value, "modified_gmt")?)?),
        original: body.into(),
        unsupported_body: false,
        body_html,
        body_text,
        resources,
        etag: None,
        last_modified: None,
    })
}

#[cfg(test)]
#[path = "fixtures/wordpress_tests.rs"]
mod tests;
