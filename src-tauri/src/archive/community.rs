//! Current Electronic and System Semiconductor department boards. Both sites run
//! the same aseoul board engine (the `?m=v&idx=` detail contract, `pNo` paging and
//! pins-first ordering) with different skins. Their robots.txt disallows `/se2/`
//! and `/data/`, and sse serves its attachments from the equally disallowed
//! `/bbs/`. The network policy therefore approves only routes the sites permit;
//! blocked resources stay visible as unsupported instead of being fetched.
use super::*;

#[derive(Clone, Copy)]
enum Layout {
    Table,
    Gallery(u32),
}

#[derive(Clone, Copy)]
struct Board {
    host: &'static str,
    path: &'static str,
    code: Option<&'static str>,
    layout: Layout,
}

fn board(source: &SourceDefinition) -> Result<Board, String> {
    let board = match source.id.as_str() {
        "ee-news" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/community/notice01.php",
            code: Some("news"),
            layout: Layout::Gallery(12),
        },
        "ee-general" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/community/notice02.php",
            code: Some("general"),
            layout: Layout::Table,
        },
        "ee-academic" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/community/notice03.php",
            code: Some("academic"),
            layout: Layout::Table,
        },
        "ee-seminars" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/community/seminar.php",
            code: Some("seminar"),
            layout: Layout::Table,
        },
        "ee-employment" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/community/employment.php",
            code: Some("jobs"),
            layout: Layout::Table,
        },
        "ee-recruit" => Board {
            host: "ee.sogang.ac.kr",
            path: "/kor/recruit/recruit.php",
            code: Some("recruit"),
            layout: Layout::Table,
        },
        "sse-notices" => Board {
            host: "sse.sogang.ac.kr",
            path: "/kor/community/notice.php",
            code: None,
            layout: Layout::Table,
        },
        "sse-news" => Board {
            host: "sse.sogang.ac.kr",
            path: "/kor/community/news.php",
            code: None,
            layout: Layout::Gallery(9),
        },
        "sse-seminars" => Board {
            host: "sse.sogang.ac.kr",
            path: "/kor/community/seminar.php",
            code: None,
            layout: Layout::Table,
        },
        _ => return invalid(),
    };
    Ok(board)
}
fn ee(board: &Board) -> bool {
    board.host == "ee.sogang.ac.kr"
}
fn page_size(board: &Board) -> u32 {
    match board.layout {
        Layout::Table => 10,
        Layout::Gallery(size) => size,
    }
}
fn base(board: &Board) -> Result<Url, String> {
    Url::parse(&format!("https://{}{}", board.host, board.path)).map_err(|_| PARSER_ERROR.into())
}
fn url_with_pno(board: &Board, page: u32) -> Result<Url, String> {
    let mut url = base(board)?;
    url.query_pairs_mut().append_pair("pNo", &page.to_string());
    if let Some(code) = board.code {
        url.query_pairs_mut().append_pair("code", code);
    }
    Ok(url)
}
pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    url_with_pno(&board(source)?, page)
}
fn same_route(board: &Board, url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(board.host)
        && url.path() == board.path
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}
fn query_map(url: &Url) -> Result<HashMap<String, String>, String> {
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let mut map = HashMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        if map.insert(key, value).is_some() {
            return invalid();
        }
    }
    Ok(map)
}
/// Returns the public notice identity and the listing page carried by the URL.
fn article_parts(board: &Board, url: &Url) -> Result<(String, u32), String> {
    if !same_route(board, url) {
        return invalid();
    }
    let query = query_map(url)?;
    let expected = 3 + usize::from(board.code.is_some());
    if query.len() != expected
        || query.get("m").map(String::as_str) != Some("v")
        || board
            .code
            .is_some_and(|code| query.get("code").map(String::as_str) != Some(code))
    {
        return invalid();
    }
    let external_id = query
        .get("idx")
        .filter(|value| positive(value))
        .ok_or(PARSER_ERROR)?
        .clone();
    let page = query
        .get("pNo")
        .filter(|value| positive(value))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or(PARSER_ERROR)?;
    Ok((external_id, page))
}
pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    let board = board(source)?;
    let url = Url::parse(&entry.url).map_err(|_| PARSER_ERROR)?;
    if article_parts(&board, &url)?.0 != entry.external_id {
        return invalid();
    }
    Ok(url)
}
fn declared_total(header: ElementRef<'_>) -> Result<u32, String> {
    let value = text(header);
    let before = value
        .find("건의 게시물")
        .map(|index| &value[..index])
        .ok_or(PARSER_ERROR)?;
    let digits: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return invalid();
    }
    digits
        .replace(',', "")
        .parse::<u32>()
        .map_err(|_| PARSER_ERROR.into())
}
fn link_target(board: &Board, href: &str) -> Result<u32, String> {
    if !href.starts_with('/') {
        return invalid();
    }
    let url = Url::parse(&format!("https://{}{}", board.host, href)).map_err(|_| PARSER_ERROR)?;
    if !same_route(board, &url) {
        return invalid();
    }
    let query = query_map(&url)?;
    if query.len() != 1 + usize::from(board.code.is_some())
        || board
            .code
            .is_some_and(|code| query.get("code").map(String::as_str) != Some(code))
    {
        return invalid();
    }
    query
        .get("pNo")
        .filter(|value| positive(value))
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| PARSER_ERROR.into())
}
/// Validates the pager block, the active page and the declared last page.
fn pager(board: &Board, root: ElementRef<'_>, page: u32, total: u32) -> Result<u32, String> {
    let pager = one(root, if ee(board) { ".b_page_no" } else { ".page_bx" })?;
    let anchors: Vec<_> = pager.select(&selector("a")).collect();
    let mut numbered = Vec::new();
    let mut active = None;
    let mut special = HashMap::new();
    for anchor in anchors {
        let classes: Vec<&str> = anchor.value().classes().collect();
        let href = anchor.value().attr("href");
        if let Some(kind) = ["first", "prev", "next", "last"]
            .into_iter()
            .find(|kind| classes.iter().any(|class| class.ends_with(kind)))
        {
            special.insert(kind, href.map(|href| link_target(board, href)).transpose()?);
            continue;
        }
        let target = text(anchor).parse::<u32>().map_err(|_| PARSER_ERROR)?;
        if href.is_none() {
            if classes.contains(&"on") && active.replace(target).is_some() {
                return invalid();
            }
        } else if classes.contains(&"on") {
            return invalid();
        }
        numbered.push((
            target,
            href.map(|href| link_target(board, href)).transpose()?,
        ));
    }
    // A single-block pager renders no target on LAST, so the block end is the
    // declared last page in that case.
    let total_pages = special
        .get("last")
        .copied()
        .flatten()
        .or_else(|| numbered.iter().map(|(target, _)| *target).max())
        .unwrap_or(1);
    if total_pages == 0
        || total_pages != total.div_ceil(page_size(board)).max(1)
        || page > total_pages
    {
        return invalid();
    }
    if active != Some(page) || special.len() != 4 {
        return invalid();
    }
    let block_start = (page - 1) / 5 * 5 + 1;
    let block_end = (block_start + 4).min(total_pages);
    let expected: Vec<u32> = (block_start..=block_end).collect();
    if numbered.len() != expected.len()
        || numbered
            .iter()
            .map(|(target, _)| *target)
            .ne(expected.iter().copied())
        || numbered.iter().any(|(target, href)| match href {
            Some(target_link) => *target == page || *target_link != *target,
            None => *target != page,
        })
    {
        return invalid();
    }
    let expect = |kind: &str| special.get(kind).copied().flatten();
    if expect("first") != (block_start > 1).then_some(1)
        || expect("prev") != (block_start > 1).then_some(block_start - 1)
        || expect("next") != (block_end < total_pages).then_some(block_end + 1)
        || expect("last") != (block_end < total_pages).then_some(total_pages)
    {
        return invalid();
    }
    Ok(total_pages)
}
fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}
fn listing_entry(
    board: &Board,
    page: u32,
    href: &str,
    pinned: bool,
    seen: &mut HashSet<String>,
) -> Result<ListingEntry, String> {
    if !href.starts_with('/') {
        return invalid();
    }
    let url = Url::parse(&format!("https://{}{}", board.host, href)).map_err(|_| PARSER_ERROR)?;
    let (external_id, link_page) = article_parts(board, &url)?;
    if link_page != page || !seen.insert(external_id.clone()) {
        return invalid();
    }
    Ok(ListingEntry {
        external_id,
        url: url.into(),
        pinned,
        modified_at: None,
    })
}
fn row_entries(
    board: &Board,
    root: ElementRef<'_>,
    page: u32,
    total: u32,
    expected_rows: u32,
) -> Result<Vec<ListingEntry>, String> {
    let table = one(
        root,
        if ee(board) {
            "table.basic_list"
        } else {
            ".list_st02 table"
        },
    )?;
    let rows: Vec<_> = table.select(&selector("tbody tr")).collect();
    if rows.len() != expected_rows as usize {
        return invalid();
    }
    let mut entries = Vec::new();
    let mut ordinals = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let cells: Vec<_> = row.select(&selector("td")).collect();
        if cells.len() != 4 {
            return invalid();
        }
        let number = text(cells[0]);
        let pinned = if ee(board) {
            number == "공지"
        } else {
            row.value().classes().any(|class| class == "on")
        };
        if pinned {
            if number != "공지" {
                return invalid();
            }
        } else {
            let ordinal = number
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0 && *value <= total)
                .ok_or_else(|| PARSER_ERROR.to_string())?;
            ordinals.push(ordinal);
        }
        let anchor = one(cells[1], "a")?;
        let title = text(anchor);
        if title.is_empty() {
            return invalid();
        }
        date(&text(cells[2]).replace('.', "-"))?;
        if !digits(&text(cells[3]).replace(',', "")) {
            return invalid();
        }
        entries.push(listing_entry(
            board,
            page,
            anchor.value().attr("href").ok_or(PARSER_ERROR)?,
            pinned,
            &mut seen,
        )?);
    }
    if ordinals.windows(2).any(|pair| pair[0] != pair[1] + 1) {
        return invalid();
    }
    Ok(entries)
}
fn gallery_entries(
    board: &Board,
    root: ElementRef<'_>,
    page: u32,
    expected_rows: u32,
) -> Result<Vec<ListingEntry>, String> {
    let (container, item, title) = if ee(board) {
        ("div.gallery_list", "a[href]", ".tit")
    } else {
        (".list_st01 ul", "li > a[href]", ".subject")
    };
    let list = one(root, container)?;
    let items: Vec<_> = list.select(&selector(item)).collect();
    if items.len() != expected_rows as usize {
        return invalid();
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        if text(one(item, title)?).is_empty() {
            return invalid();
        }
        date(&text(one(item, ".date")?).replace('.', "-"))?;
        entries.push(listing_entry(
            board,
            page,
            item.value().attr("href").ok_or(PARSER_ERROR)?,
            false,
            &mut seen,
        )?);
    }
    Ok(entries)
}
pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    let board = board(source)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let name = one(
        root,
        if ee(&board) {
            "#sub_tit"
        } else {
            ".ttl01 strong"
        },
    )?;
    if text(name) != source.name {
        return invalid();
    }
    let total = declared_total(one(root, ".list_top")?)?;
    let total_pages = pager(&board, root, page, total)?;
    if total == 0 {
        if page != 1 || total_pages != 1 {
            return invalid();
        }
        return Ok(ListingPage {
            entries: vec![],
            has_next: false,
            total_pages: 1,
        });
    }
    // Pins occupy the leading slots of the combined pins-first ordering, so a
    // later page can still begin with pins. The count is the page's row budget.
    let expected_rows = (total - (page - 1) * page_size(&board)).min(page_size(&board));
    let entries = match board.layout {
        Layout::Table => row_entries(&board, root, page, total, expected_rows)?,
        Layout::Gallery(_) => gallery_entries(&board, root, page, expected_rows)?,
    };
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}
/// Drops an editor badge rendered as a direct child span, but keeps a title that
/// legitimately begins with "[공지]".
fn page_title(strong: ElementRef<'_>) -> String {
    let mut parts = Vec::new();
    let mut skipped = false;
    for child in strong.children() {
        if let Some(value) = child.value().as_text() {
            parts.push(value.to_string());
        } else if let Some(element) = ElementRef::wrap(child) {
            let value = text(element);
            if !skipped
                && element.value().name() == "span"
                && matches!(value.as_str(), "[공지]" | "공지")
            {
                skipped = true;
                continue;
            }
            parts.push(value);
        }
    }
    normalize(&parts.join(" "))
}
fn file_target(board: &Board, url: &Url, external_id: &str, path: &str) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some(board.host)
        || url.path() != path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return invalid();
    }
    let query = query_map(url)?;
    if query.len() != 2
        || !query.get("d").is_some_and(|value| positive(value))
        || query.get("idx").map(String::as_str) != Some(external_id)
    {
        return invalid();
    }
    Ok(())
}
pub(super) fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let board = board(source)?;
    let url = detail_url(source, entry)?;
    let (external_id, page) = article_parts(&board, &url)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let name = one(
        root,
        if ee(&board) {
            "#sub_tit"
        } else {
            ".ttl01 strong"
        },
    )?;
    if text(name) != source.name {
        return invalid();
    }
    let (container, title, meta, content) = if ee(&board) {
        (
            "div.board_view",
            ".v_title strong",
            ".v_title ul",
            ".v_cont",
        )
    } else {
        ("div.view_bx", ".subject strong", ".subject ul", ".txt")
    };
    let view = one(root, container)?;
    let title = page_title(one(view, title)?);
    let meta: Vec<_> = one(view, meta)?.select(&selector("li")).collect();
    if title.is_empty() || meta.len() != 2 {
        return invalid();
    }
    let published_at = date(&text(meta[0]).replace('.', "-"))?;
    if !digits(&text(meta[1]).replace(',', "")) {
        return invalid();
    }
    // The sse skin puts the back-to-list button outside `.view_bx`.
    let list = one(root, ".btm_btn a")?;
    let list = Url::parse(&format!(
        "https://{}{}",
        board.host,
        list.value().attr("href").ok_or(PARSER_ERROR)?
    ))
    .map_err(|_| PARSER_ERROR)?;
    if !same_route(&board, &list) {
        return invalid();
    }
    let query = query_map(&list)?;
    if query.get("pNo").and_then(|value| value.parse::<u32>().ok()) != Some(page)
        || board
            .code
            .is_some_and(|code| query.get("code").map(String::as_str) != Some(code))
    {
        return invalid();
    }
    let (body_html, body_text, inline) = sanitize(&one(view, content)?.inner_html(), &entry.url)?;
    let mut resources = Vec::new();
    if ee(&board) {
        let files: Vec<_> = view.select(&selector("dl.v_file")).collect();
        if files.len() > 1 {
            return invalid();
        }
        if let Some(files) = files.first() {
            if text(one(*files, "dt")?) != "첨부파일" {
                return invalid();
            }
            for link in files.select(&selector("dd a")) {
                let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
                    .ok_or(PARSER_ERROR)?;
                file_target(&board, &target, &external_id, "/aseoul_bbs/down.php")?;
                let filename = text(one(link, "span")?);
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
    } else {
        let files = one(view, "div.file")?;
        if text(one(files, "span")?) != "첨부파일" {
            return invalid();
        }
        for link in files.select(&selector("a")) {
            let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
                .ok_or(PARSER_ERROR)?;
            file_target(&board, &target, &external_id, "/bbs/down.php")?;
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
#[path = "fixtures/community_tests.rs"]
mod tests;
