//! Current Sogang dormitory (벨라르미노 학사) CMS boards. The public menu pages render the
//! `JW_bbs_table` list shell; article and attachment routes live under
//! `/bbs/sgdormitory/<board>/...` on the same host. Robots.txt allows these
//! public routes and forbids only `/*/*Mngr`, login and top-login paths.
//! Stable catalog IDs are independent of the menu numbers.
use super::*;

#[derive(Clone, Copy)]
struct Board {
    menu: u32,
    bbs: u32,
}

/// Every current dormitory board renders seven regular rows per page plus its
/// repeated pins. Pins occupy the leading rows on every page, not only page one.
const PAGE_ROWS: u32 = 7;

fn board(source: &SourceDefinition) -> Result<Board, String> {
    match source.id.as_str() {
        "dorm-general" => Ok(Board {
            menu: 6975,
            bbs: 2231,
        }),
        "dorm-admission" => Ok(Board {
            menu: 3947,
            bbs: 1644,
        }),
        "dorm-menu" => Ok(Board {
            menu: 3951,
            bbs: 1648,
        }),
        _ => invalid(),
    }
}
fn base(board: Board) -> Result<Url, String> {
    Url::parse(&format!(
        "https://new-cms.sogang.ac.kr/sgdormitory/{}/subview.do",
        board.menu
    ))
    .map_err(|_| PARSER_ERROR.into())
}
fn article_base(board: Board) -> String {
    format!("/bbs/sgdormitory/{}/", board.bbs)
}
pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    let mut url = base(board(source)?)?;
    url.query_pairs_mut().append_pair("page", &page.to_string());
    Ok(url)
}
pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    let board = board(source)?;
    let expected = format!(
        "https://new-cms.sogang.ac.kr/bbs/sgdormitory/{}/{}/artclView.do",
        board.bbs, entry.external_id
    );
    if !positive(&entry.external_id) || entry.url != expected {
        return invalid();
    }
    Url::parse(&expected).map_err(|_| PARSER_ERROR.into())
}
/// The list page's search form binds the menu to its board id, and the hidden
/// `layout` value is the site@@menu@@fnct token. Both must agree with the catalog.
fn layout_token(menu: u32) -> String {
    let plain = format!("sgdormitory@@{menu}@@fnct1");
    let mut hex = String::with_capacity(plain.len() * 2);
    for byte in plain.bytes() {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hex
}
fn hidden(form: ElementRef<'_>, name: &str) -> Result<String, String> {
    Ok(one(form, &format!("input[name={name}]"))?
        .value()
        .attr("value")
        .unwrap_or("")
        .to_string())
}
fn page_target(href: &str) -> Result<u32, String> {
    let value = href
        .strip_prefix("javascript:page_link('")
        .and_then(|value| value.strip_suffix("')"))
        .filter(|value| positive(value))
        .ok_or(PARSER_ERROR)?;
    value.parse::<u32>().map_err(|_| PARSER_ERROR.into())
}
/// Validates the numbered pager block (ten pages), its active page and the
/// first/previous/next/last links. Beyond the last public page the site keeps
/// serving a pins-only page, so the declared total is authoritative.
fn pager(root: ElementRef<'_>, page: u32, total_pages: u32) -> Result<(), String> {
    let pager = one(root, "._paging")?;
    let current = text(one(pager, "._curPage")?)
        .parse::<u32>()
        .map_err(|_| PARSER_ERROR)?;
    let declared = text(one(pager, "._totPage")?)
        .parse::<u32>()
        .map_err(|_| PARSER_ERROR)?;
    if current != page || declared != total_pages || page > total_pages {
        return invalid();
    }
    let block_start = (page - 1) / 10 * 10 + 1;
    let block_end = (block_start + 9).min(total_pages);
    let mut special = HashMap::new();
    let mut numbered = Vec::new();
    let mut active = None;
    for anchor in pager.select(&selector("a, strong")) {
        let classes: Vec<&str> = anchor.value().classes().collect();
        // The site prefixes these classes with an underscore (`_first`).
        if let Some(kind) = ["first", "prev", "listPrev", "listNext", "next", "last"]
            .into_iter()
            .find(|kind| {
                classes
                    .iter()
                    .any(|class| class.trim_start_matches('_') == *kind)
            })
        {
            if special
                .insert(
                    kind,
                    page_target(anchor.value().attr("href").ok_or(PARSER_ERROR)?)?,
                )
                .is_some()
            {
                return invalid();
            }
            continue;
        }
        let target = text(anchor).parse::<u32>().map_err(|_| PARSER_ERROR)?;
        let href = anchor.value().attr("href");
        if href.is_none() {
            if active.replace(target).is_some() {
                return invalid();
            }
        } else if classes.contains(&"on") {
            return invalid();
        }
        numbered.push((target, href.map(page_target).transpose()?));
    }
    if active != Some(page)
        || special.get("first") != Some(&1)
        || numbered.len() != (block_end - block_start + 1) as usize
    {
        return invalid();
    }
    let expected: Vec<u32> = (block_start..=block_end).collect();
    if numbered
        .iter()
        .map(|(target, _)| *target)
        .ne(expected.iter().copied())
        || numbered.iter().any(|(target, href)| match href {
            Some(link) => *link != *target || *target == page,
            None => *target != page,
        })
    {
        return invalid();
    }
    let expect = |kind: &str| special.get(kind).copied();
    if expect("listPrev") != (page > 1).then_some(page - 1)
        || expect("listNext") != (page < total_pages).then_some(page + 1)
        || expect("prev") != (block_start > 1).then_some(block_start - 1)
        || expect("next") != (block_end < total_pages).then_some(block_end + 1)
        // The site keeps the last target on the last page, unlike `_listNext`.
        || expect("last") != Some(total_pages)
    {
        return invalid();
    }
    Ok(())
}
pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    let board = board(source)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    if text(one(root, "title")?) != source.name {
        return invalid();
    }
    let list = one(root, ".board-list")?;
    let search = one(list, "form[name=searchForm]")?;
    let action = format!("{}artclList.do", article_base(board));
    let layout = layout_token(board.menu);
    if search.value().attr("action") != Some(action.as_str())
        || hidden(search, "layout")? != layout
        || !hidden(search, "findWord")?.is_empty()
        || !hidden(search, "findOpnwrd")?.is_empty()
    {
        return invalid();
    }
    let declared = text(one(list, ".srch_counts strong")?).replace(',', "");
    let total = declared.parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let total_pages = total.div_ceil(PAGE_ROWS).max(1);
    if page == 0 || page > total_pages {
        return invalid();
    }
    if total == 0 {
        return Ok(ListingPage {
            entries: vec![],
            has_next: false,
            total_pages: 1,
        });
    }
    pager(root, page, total_pages)?;
    let table = one(list, "table.board-table")?;
    let rows: Vec<_> = table.select(&selector("tbody tr")).collect();
    let start = total
        .checked_sub((page - 1) * PAGE_ROWS)
        .ok_or(PARSER_ERROR)?;
    let expected_regular = start.min(PAGE_ROWS);
    let mut entries: Vec<ListingEntry> = Vec::new();
    let mut pinned_ids = HashSet::new();
    let mut regular = 0u32;
    let mut seen_regular = HashSet::new();
    if rows.len() > 100 {
        return invalid();
    }
    for row in rows {
        let number = one(row, "td.td-num")?;
        let date_cell = one(row, "td.td-date")?;
        let published = text(date_cell).replace('.', "-");
        date(&published)?;
        let anchor = one(row, "td.td-title a")?;
        if text(anchor).is_empty() {
            return invalid();
        }
        let href = anchor.value().attr("href").ok_or(PARSER_ERROR)?;
        let url = safe_url("https://new-cms.sogang.ac.kr", href).ok_or(PARSER_ERROR)?;
        let suffix = url
            .path()
            .strip_suffix("/artclView.do")
            .ok_or(PARSER_ERROR)?;
        let external = suffix
            .strip_prefix(&article_base(board))
            .filter(|value| positive(value))
            .ok_or(PARSER_ERROR)?;
        if url.scheme() != "https"
            || url.host_str() != Some("new-cms.sogang.ac.kr")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return invalid();
        }
        let pinned = row.value().classes().any(|class| class == "notice");
        if pinned {
            if !pinned_ids.insert(external.to_string())
                || text(one(number, "span.notice-title")?).is_empty()
            {
                return invalid();
            }
            if let Some(entry) = entries
                .iter_mut()
                .find(|entry| entry.external_id == external)
            {
                entry.pinned = true;
            } else {
                entries.push(ListingEntry {
                    external_id: external.into(),
                    url: url.into(),
                    pinned: true,
                    modified_at: None,
                });
            }
            continue;
        }
        let ordinal = text(number).parse::<u32>().map_err(|_| PARSER_ERROR)?;
        if regular >= expected_regular
            || ordinal != start - regular
            || !seen_regular.insert(external.to_string())
        {
            return invalid();
        }
        regular += 1;
        if entries
            .iter()
            .any(|entry| entry.external_id == external && !entry.pinned)
        {
            return invalid();
        }
        // A pin repeated in the regular rows keeps its pinned entry.
        if !entries.iter().any(|entry| entry.external_id == external) {
            entries.push(ListingEntry {
                external_id: external.into(),
                url: url.into(),
                pinned: false,
                modified_at: None,
            });
        }
    }
    if regular != expected_regular {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}
fn once<T>(slot: &mut Option<T>, value: T) -> Result<(), String> {
    if slot.replace(value).is_some() {
        invalid()
    } else {
        Ok(())
    }
}
fn meta_value(element: ElementRef<'_>) -> String {
    normalize(
        &element
            .children()
            .filter_map(|child| child.value().as_text())
            .map(|text| text.to_string())
            .collect::<Vec<_>>()
            .join(" "),
    )
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
    let canonical = one(root, "meta[property='og:url']")?
        .value()
        .attr("content")
        .ok_or(PARSER_ERROR)?;
    if canonical != url.as_str() {
        return invalid();
    }
    let view = one(root, "div.board-view div.view.viewCont")?;
    let title = text(one(view, ".title > strong")?);
    let hidden_title = one(root, "input#artclViewTitle")?
        .value()
        .attr("value")
        .ok_or(PARSER_ERROR)?;
    if title.is_empty() || normalize(hidden_title) != title {
        return invalid();
    }
    let list_action = format!("{}artclList.do", article_base(board));
    let form = one(root, "form[name=listForm]")?;
    if form.value().attr("action") != Some(list_action.as_str()) {
        return invalid();
    }
    let mut published_at = None;
    let mut author = None;
    let items: Vec<_> = view.select(&selector(".title ul.detail li")).collect();
    if items.is_empty() || items.len() > 10 {
        return invalid();
    }
    for item in items {
        let label = text(one(item, "span")?);
        let value = meta_value(item);
        if value.is_empty() {
            continue;
        }
        match label.as_str() {
            "작성일" => once(&mut published_at, date(&value.replace('.', "-"))?)?,
            "작성자" => once(&mut author, value)?,
            _ => {}
        }
    }
    let published_at = published_at.ok_or(PARSER_ERROR)?;
    let content = one(view, ".txt")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    let mut resources = Vec::new();
    let attachment = one(view, "div.attachment")?;
    let no_file = attachment.select(&selector(".no-file")).next().is_some();
    let links: Vec<_> = attachment.select(&selector("ul li a")).collect();
    if no_file && !links.is_empty() {
        return invalid();
    }
    if !no_file {
        if links.is_empty() {
            return invalid();
        }
        for link in links {
            let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
                .ok_or(PARSER_ERROR)?;
            let suffix = target
                .path()
                .strip_suffix("/download.do")
                .ok_or(PARSER_ERROR)?;
            if target.scheme() != "https"
                || target.host_str() != Some("new-cms.sogang.ac.kr")
                || target.query().is_some()
                || target.fragment().is_some()
                || suffix
                    .strip_prefix(&article_base(board))
                    .filter(|value| positive(value))
                    .is_none()
            {
                return invalid();
            }
            let filename = text(link);
            if filename.is_empty() {
                return invalid();
            }
            resources.push(ResourceInput {
                url: target.into(),
                filename,
                kind: "attachment".into(),
            });
        }
    }
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
#[path = "fixtures/dormitory_tests.rs"]
mod tests;
