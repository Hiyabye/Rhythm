//! Current Sogang mechanical engineering (기계공학과) boards. The department site
//! renders board paging only as `?page=N`, and its robots.txt forbids every
//! query string for crawlers (`Disallow: /*?`), while the legacy `/bbs/`
//! gnuboard routes answer a Vercel challenge. Discovery therefore uses the
//! public `/sitemap.xml` that robots allows: it lists every board article under
//! `/ko/board/<board>/<id>` with the site's own `lastmod`. The sync skips an
//! unchanged article while that timestamp still matches the archived value.
//! Attachments live on the department's public Supabase storage and stay
//! recorded-but-unsupported references; article images embedded in a notice are
//! fetched under the embedded-image rule.
use super::*;

#[derive(Clone, Copy)]
struct Board {
    slug: &'static str,
    title: &'static str,
}

const SITEMAP: &str = "https://me.sogang.ac.kr/sitemap.xml";

fn board(source: &SourceDefinition) -> Result<Board, String> {
    match source.id.as_str() {
        "me-general" => Ok(Board {
            slug: "notice",
            title: "일반공지",
        }),
        "me-academic" => Ok(Board {
            slug: "academic",
            title: "학사공지",
        }),
        "me-research" => Ok(Board {
            slug: "research",
            title: "연구성과",
        }),
        "me-awards" => Ok(Board {
            slug: "award",
            title: "수상",
        }),
        "me-careers" => Ok(Board {
            slug: "scholarship",
            title: "장학·취업정보",
        }),
        "me-events" => Ok(Board {
            slug: "events",
            title: "외부 행사",
        }),
        "me-alumni" => Ok(Board {
            slug: "alumni_news",
            title: "동문·구성원 소식",
        }),
        _ => invalid(),
    }
}
fn article_path(board: Board) -> String {
    format!("/ko/board/{}/", board.slug)
}
pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    // One sitemap document lists every article; there is no second page.
    if page != 1 || !source.supported {
        return invalid();
    }
    Url::parse(SITEMAP).map_err(|_| PARSER_ERROR.into())
}
pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    let board = board(source)?;
    let expected = format!(
        "https://me.sogang.ac.kr{}{}",
        article_path(board),
        entry.external_id
    );
    if !positive(&entry.external_id) || entry.url != expected {
        return invalid();
    }
    Url::parse(&expected).map_err(|_| PARSER_ERROR.into())
}
/// The sitemap `lastmod` is compared with the archived value verbatim, so only
/// the exact UTC form the site emits is accepted. Any other value stays `None`
/// and the article is fetched normally.
fn timestamp(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];
    if bytes.len() != 24
        || separators
            .iter()
            .any(|(index, byte)| bytes[*index] != *byte)
        || bytes.iter().enumerate().any(|(index, byte)| {
            !separators.iter().any(|(i, _)| *i == index) && !byte.is_ascii_digit()
        })
    {
        return None;
    }
    date(&value[..10]).ok()?;
    let hour = value[11..13].parse::<u32>().ok()?;
    let minute = value[14..16].parse::<u32>().ok()?;
    let second = value[17..19].parse::<u32>().ok()?;
    (hour < 24 && minute < 60 && second < 60).then(|| value.to_string())
}
pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    if body.len() > 4 * 1024 * 1024 || page != 1 {
        return invalid();
    }
    let board = board(source)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let urlset = one(root, "urlset")?;
    let prefix = article_path(board);
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut count = 0usize;
    for node in urlset.select(&selector("url")) {
        count += 1;
        if count > 100_000 {
            return invalid();
        }
        let loc = one(node, "loc")?;
        let value = text(loc);
        if value.len() > 2048 {
            return invalid();
        }
        let url = Url::parse(&value).map_err(|_| PARSER_ERROR.to_string())?;
        if url.scheme() != "https"
            || url.host_str() != Some("me.sogang.ac.kr")
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return invalid();
        }
        let lastmods: Vec<_> = node.select(&selector("lastmod")).collect();
        if lastmods.len() > 1 {
            return invalid();
        }
        let modified_at = lastmods
            .first()
            .map(|value| text(*value))
            .and_then(|value| timestamp(&value));
        let Some(rest) = url.path().strip_prefix(&prefix) else {
            continue;
        };
        if rest.contains('/') || !positive(rest) || !seen.insert(rest.to_string()) {
            return invalid();
        }
        entries.push(ListingEntry {
            external_id: rest.into(),
            url: url.into(),
            pinned: false,
            modified_at,
        });
    }
    if entries.is_empty() {
        return invalid();
    }
    // Newest recorded change first; numeric identity breaks ties deterministically.
    entries.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| right.external_id.len().cmp(&left.external_id.len()))
            .then_with(|| right.external_id.cmp(&left.external_id))
    });
    Ok(ListingPage {
        entries,
        has_next: false,
        total_pages: 1,
    })
}
/// The attachment section is the only place the site exposes files. They live on
/// public file hosts (the department's Supabase project or a legacy R2 bucket),
/// which the transport records but never downloads as attachments.
fn attachment_filename(url: &Url, fallback: &str) -> String {
    url.query_pairs()
        .find(|(key, _)| key == "download")
        .map(|(_, value)| value.into_owned())
        .or_else(|| {
            url.path_segments()
                .and_then(Iterator::last)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| fallback.to_owned())
}
pub(super) fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let board = board(source)?;
    let url = detail_url(source, entry)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let canonical = one(root, "link[rel=canonical]")?
        .value()
        .attr("href")
        .ok_or(PARSER_ERROR)?;
    if canonical != url.as_str() {
        return invalid();
    }
    let view = one(root, "article.container-site")?;
    let header = one(view, "header")?;
    if text(one(header, "span.eyebrow")?) != board.title {
        return invalid();
    }
    let title = text(one(header, "h1")?);
    if title.is_empty() {
        return invalid();
    }
    let mut published_at = None;
    let mut author = None;
    let spans: Vec<_> = header.select(&selector("div > span")).collect();
    if spans.is_empty() || spans.len() > 10 {
        return invalid();
    }
    for span in spans {
        let Some((label, value)) = text(span)
            .split_once('·')
            .map(|(label, value)| (label.trim().to_owned(), normalize(value)))
        else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        match label.as_str() {
            "작성자" => {
                if author.replace(value).is_some() {
                    return invalid();
                }
            }
            "날짜" => {
                if published_at
                    .replace(date(&value.replace('.', "-"))?)
                    .is_some()
                {
                    return invalid();
                }
            }
            "조회" => {}
            _ => {}
        }
    }
    let published_at = published_at.ok_or(PARSER_ERROR)?;
    // Direct block children hold the optional hero image or video block and the
    // body; the header, attachments, neighbouring-post navigation and list link
    // are siblings and stay out of the archived body.
    one(view, "div.prose-sg")?;
    let mut content = String::new();
    for child in view.children() {
        if let Some(element) =
            ElementRef::wrap(child).filter(|element| element.value().name() == "div")
        {
            content.push_str(&element.inner_html());
        }
    }
    let mut resources = Vec::new();
    for section in view.select(&selector("section")) {
        let is_files = section
            .select(&selector("p.eyebrow"))
            .next()
            .is_some_and(|label| text(label) == "첨부파일");
        if !is_files {
            continue;
        }
        let links: Vec<_> = section.select(&selector("ul li a")).collect();
        if links.is_empty() || links.len() > 100 {
            return invalid();
        }
        for link in links {
            let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
                .ok_or(PARSER_ERROR)?;
            let filename = attachment_filename(&target, "");
            // An HTTP reference is recorded, never downloaded; the transport
            // rejects it and the reader shows its unsupported state.
            if target.fragment().is_some()
                || target.host_str().is_none()
                || !attachment_extension(&filename)
            {
                return invalid();
            }
            resources.push(ResourceInput {
                filename,
                url: target.into(),
                kind: "attachment".into(),
            });
        }
        break;
    }
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    merge_inline_resources(&mut resources, inline);
    if resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title,
        author,
        category: Some(source.name.clone()),
        published_at,
        // The listing's own change timestamp; the sync compares it with the
        // archived value and skips an unchanged notice without a request.
        modified_at: entry.modified_at.clone(),
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
#[path = "fixtures/mechanical_tests.rs"]
mod tests;
