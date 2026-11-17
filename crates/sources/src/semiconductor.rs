//! Current Sogang semiconductor engineering (반도체공학과) boards. The
//! department site (se.sogang.ac.kr) renders every board through the same
//! query-string PHP routes: `board_list.php?board_id=<board>&page=<n>` for
//! paging and `board_view.php?board_id=<board>&no=<id>` for articles. The site's
//! robots.txt allows every path, so the only same-host resources are the
//! `/board/filedown.php` attachments and `/smart_editor/upload/` editor images.
//! Article pages expose no canonical or self-identity link, so the parser binds
//! the exact requested URL, the board label, the back-to-list link and the page
//! structure. Images on other school boards (`/se2/`, `/data/`) are recorded but
//! stay blocked by those hosts' robots policy.
use super::*;

#[derive(Clone, Copy)]
struct Board {
    id: &'static str,
    title: &'static str,
}

const ORIGIN: &str = "https://se.sogang.ac.kr";

fn board(source: &SourceDefinition) -> Result<Board, String> {
    match source.id.as_str() {
        "se-notices" => Ok(Board {
            id: "notice",
            title: "학과 공지사항",
        }),
        "se-graduate" => Ok(Board {
            id: "notice_master",
            title: "대학원 공지사항",
        }),
        "se-news" => Ok(Board {
            id: "news",
            title: "학과소식",
        }),
        "se-careers" => Ok(Board {
            id: "recruit",
            title: "채용/홍보",
        }),
        "se-industry" => Ok(Board {
            id: "notice_industry",
            title: "산학연 공지사항",
        }),
        _ => invalid(),
    }
}

fn list_url_for(board: Board, page: u32) -> Url {
    let mut url = Url::parse(&format!("{ORIGIN}/board/board_list.php")).unwrap();
    url.query_pairs_mut()
        .append_pair("board_id", board.id)
        .append_pair("page", &page.to_string());
    url
}

pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    if page == 0 || !source.supported {
        return invalid();
    }
    Ok(list_url_for(board(source)?, page))
}

pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    let board = board(source)?;
    if !positive(&entry.external_id) {
        return invalid();
    }
    let mut url = Url::parse(&format!("{ORIGIN}/board/board_view.php")).unwrap();
    url.query_pairs_mut()
        .append_pair("board_id", board.id)
        .append_pair("no", &entry.external_id);
    if entry.url != url.as_str() {
        return invalid();
    }
    Ok(url)
}

pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    if body.len() > 4 * 1024 * 1024 || page == 0 {
        return invalid();
    }
    let board = board(source)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    if text(one(root, "#title")?) != board.title {
        return invalid();
    }
    let selected = one(root, "input#board_id")?;
    if selected.value().attr("value") != Some(board.id) {
        return invalid();
    }
    let total: u32 = text(one(root, ".list_count strong")?)
        .replace(',', "")
        .parse()
        .map_err(|_| PARSER_ERROR)?;
    let total_pages = total.div_ceil(10).max(1);
    if page > total_pages {
        return invalid();
    }
    let list = one(root, "ul.notice_list")?;
    let rows: Vec<_> = list
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|element| element.value().name() == "li")
        .collect();
    if rows.len() > 100 {
        return invalid();
    }
    if total == 0 {
        if !rows.is_empty() || list.select(selector("a")).next().is_some() {
            return invalid();
        }
        return Ok(ListingPage {
            entries: vec![],
            has_next: false,
            total_pages: 1,
        });
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut ordinals = Vec::new();
    for row in rows {
        let anchor = one(row, "a")?;
        let url = Url::parse(ORIGIN)
            .unwrap()
            .join(anchor.value().attr("href").ok_or(PARSER_ERROR)?)
            .map_err(|_| PARSER_ERROR)?;
        if url.scheme() != "https"
            || url.host_str() != Some("se.sogang.ac.kr")
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.path() != "/board/board_view.php"
        {
            return invalid();
        }
        let (mut linked_board, mut external_id, mut return_page) = (None, None, None);
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "board_id" if linked_board.is_none() => linked_board = Some(value.into_owned()),
                "no" if external_id.is_none() => external_id = Some(value.into_owned()),
                "page" if return_page.is_none() => return_page = Some(value.into_owned()),
                _ => return invalid(),
            }
        }
        if linked_board.as_deref() != Some(board.id) {
            return invalid();
        }
        let external_id = external_id.ok_or(PARSER_ERROR)?;
        if !positive(&external_id) || return_page.is_some_and(|value| !positive(&value)) {
            return invalid();
        }
        let number = one(anchor, "p.num")?;
        let pinned = match number.select(selector("span.notice")).next() {
            Some(label) => {
                if text(label) != "공지" || normalize(&text(number)) != "공지" {
                    return invalid();
                }
                true
            }
            None => {
                ordinals.push(
                    text(number)
                        .parse::<u32>()
                        .map_err(|_| PARSER_ERROR.to_string())?,
                );
                false
            }
        };
        let title = text(one(anchor, "div.info div.txt_t p.txt")?);
        if title.is_empty() {
            return invalid();
        }
        date(&text(one(anchor, "div.info p.date span")?))?;
        if !seen.insert(external_id.clone()) {
            return invalid();
        }
        entries.push(ListingEntry {
            url: format!(
                "{ORIGIN}/board/board_view.php?board_id={}&no={external_id}",
                board.id
            ),
            external_id,
            pinned,
            modified_at: None,
        });
    }
    let start = total.saturating_sub((page - 1) * 10);
    let expected: Vec<_> = (start.saturating_sub(9).max(1)..=start).rev().collect();
    if ordinals != expected {
        return invalid();
    }
    let paging = one(root, ".paging")?;
    let links: Vec<_> = paging.select(selector("a")).collect();
    if links.is_empty() || links.len() > 40 {
        return invalid();
    }
    let mut first = false;
    let mut last = false;
    let mut current = false;
    for link in links {
        let target = Url::parse(ORIGIN)
            .unwrap()
            .join(link.value().attr("href").ok_or(PARSER_ERROR)?)
            .map_err(|_| PARSER_ERROR)?;
        let mut linked_board = None;
        let mut linked_page = None;
        for (key, value) in target.query_pairs() {
            match key.as_ref() {
                "board_id" if linked_board.is_none() => linked_board = Some(value.into_owned()),
                "page" if linked_page.is_none() => linked_page = Some(value.into_owned()),
                _ => return invalid(),
            }
        }
        let linked_page: u32 = linked_page
            .ok_or(PARSER_ERROR)?
            .parse()
            .map_err(|_| PARSER_ERROR)?;
        if linked_board.as_deref() != Some(board.id)
            || target.path() != "/board/board_list.php"
            || target.scheme() != "https"
            || target.host_str() != Some("se.sogang.ac.kr")
            || target.fragment().is_some()
            || linked_page == 0
            || linked_page > total_pages
        {
            return invalid();
        }
        let classes: Vec<_> = link.value().classes().collect();
        if classes.contains(&"on") {
            if current || linked_page != page || text(link).parse::<u32>().ok() != Some(page) {
                return invalid();
            }
            current = true;
        } else if text(link).parse::<u32>().ok() == Some(linked_page) {
            // A numbered page link is the page it points at.
        } else if classes.contains(&"first") {
            if first || page == 1 || linked_page != 1 {
                return invalid();
            }
            first = true;
        } else if classes.contains(&"last") {
            if last || page == total_pages || linked_page != total_pages {
                return invalid();
            }
            last = true;
        } else {
            return invalid();
        }
    }
    if !current || first != (page > 1) || last != (page < total_pages) {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}

/// The attachment name is the query value the download route uses; the visible
/// link must agree with it. An unlisted file type is still recorded; the
/// download transport refuses it, so it stays visible as unsupported.
fn attachment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

pub(super) fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let board = board(source)?;
    detail_url(source, entry)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    if text(one(root, "#title")?) != board.title {
        return invalid();
    }
    let view = one(root, ".view_tit")?;
    let title = text(one(view, "h3.v_tit")?);
    if title.is_empty() {
        return invalid();
    }
    let published_at = date(&text(one(view, "p.v_list span")?))?;
    let back = one(root, "p.btn_list a")?;
    let expected_list = list_url_for(board, 1);
    let back_target = Url::parse(ORIGIN)
        .unwrap()
        .join(back.value().attr("href").ok_or(PARSER_ERROR)?)
        .map_err(|_| PARSER_ERROR)?;
    if back_target.as_str() != expected_list.as_str() {
        return invalid();
    }
    one(root, "div.view_file")?;
    let mut resources = Vec::new();
    for link in root.select(selector(".view_file dl.attachments dd ul li a")) {
        if resources.len() > 100 {
            return invalid();
        }
        let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
            .ok_or(PARSER_ERROR)?;
        if target.scheme() != "https"
            || target.host_str() != Some("se.sogang.ac.kr")
            || target.port_or_known_default() != Some(443)
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
            || target.path() != "/board/filedown.php"
        {
            return invalid();
        }
        let (mut filepath, mut filename) = (None, None);
        for (key, value) in target.query_pairs() {
            match key.as_ref() {
                "filepath" if filepath.is_none() => filepath = Some(value.into_owned()),
                "filename" if filename.is_none() => filename = Some(value.into_owned()),
                _ => return invalid(),
            }
        }
        if !filepath.is_some_and(|value| !value.is_empty() && value.len() <= 2048) {
            return invalid();
        }
        let filename = filename.ok_or(PARSER_ERROR)?;
        if !attachment_name(&filename) || normalize(&text(link)) != normalize(&filename) {
            return invalid();
        }
        resources.push(ResourceInput {
            filename,
            url: target.into(),
            kind: "attachment".into(),
        });
    }
    if resources.len() > 100 {
        return invalid();
    }
    let content = one(root, "div.view_con")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    merge_inline_resources(&mut resources, inline);
    if resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title,
        author: None,
        category: Some(source.name.clone()),
        published_at,
        modified_at: None,
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
#[path = "fixtures/semiconductor_tests.rs"]
mod tests;
