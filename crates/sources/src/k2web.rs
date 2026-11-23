//! Boards of Sogang's k2web CMS (new-cms), such as the dormitory (벨라르미노
//! 학사) site. Every site is reached through `new-cms.sogang.ac.kr` (user
//! decision, #80): lists are `/<site>/<menu>/subview.do`, articles and
//! attachments live under `/bbs/<site>/<bbs>/...`. Robots.txt allows these
//! public routes and forbids only `/*/*Mngr`, login and top-login paths.
//! A board is one row of `K2WEB_BOARDS`; stable catalog IDs are independent of
//! the menu numbers.
use super::*;

const HOST: &str = "new-cms.sogang.ac.kr";

/// How a board's list page renders its rows. Both share the search form,
/// declared total and pager of the same CMS (census of the 129 active k2web
/// boards, 2026-11-23: 124 tables, 5 galleries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skin {
    /// `table.board-table` (`horizon1`, or `horizon4` with other columns),
    /// pins as leading `tr.notice` rows.
    Table,
    /// `.albumWrap` or `.thumnailWrap` cards with a `No. N` ordinal.
    Gallery,
}

pub struct K2webBoard {
    pub source_id: &'static str,
    /// The name the app shows.
    pub name: &'static str,
    /// The board's page title, which the list page must show.
    pub title: &'static str,
    pub organization: &'static str,
    pub group: &'static str,
    pub site: &'static str,
    pub menu: u32,
    pub bbs: u32,
    /// Regular rows per page. The page states only its total and page count,
    /// so this is data; the pager check catches a wrong value.
    pub rows: u32,
    pub skin: Skin,
}

#[path = "k2web/boards.rs"]
mod boards;
pub use boards::K2WEB_BOARDS;

pub(super) fn board(source: &SourceDefinition) -> Result<&'static K2webBoard, String> {
    K2WEB_BOARDS
        .iter()
        .find(|board| board.source_id == source.id)
        .ok_or_else(|| PARSER_ERROR.into())
}
/// `/bbs/<site>/<bbs>/`, the prefix of the board's article, list-form and
/// attachment routes.
fn article_base(board: &K2webBoard) -> String {
    format!("/bbs/{}/{}/", board.site, board.bbs)
}
pub(super) fn list_url(board: &K2webBoard, page: u32) -> Result<Url, String> {
    let mut url = Url::parse(&format!(
        "https://{HOST}/{}/{}/subview.do",
        board.site, board.menu
    ))
    .map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut().append_pair("page", &page.to_string());
    Ok(url)
}
pub(super) fn detail_url(board: &K2webBoard, entry: &ListingEntry) -> Result<Url, String> {
    let expected = format!(
        "https://{HOST}{}{}/artclView.do",
        article_base(board),
        entry.external_id
    );
    if !positive(&entry.external_id) || entry.url != expected {
        return invalid();
    }
    Url::parse(&expected).map_err(|_| PARSER_ERROR.into())
}
/// The CMS answers with a message page instead of an article: a lone form
/// whose text is the notice. Returns that text.
fn message(html: &Html) -> Option<String> {
    html.select(selector("#errorMessege p.txt strong"))
        .next()
        .map(|strong| {
            normalize(
                &strong
                    .children()
                    .filter_map(|child| child.value().as_text())
                    .map(|text| text.to_string())
                    .collect::<String>(),
            )
        })
}
pub(super) fn missing(body: &str) -> bool {
    message(&Html::parse_document(body)).as_deref() == Some("게시물이(가) 없습니다.")
}
/// The list page's search form binds the menu to its board id, and the hidden
/// `layout` value is the hex of `site@@menu@@fnct<n>`, where `n` is the board
/// widget's slot on the page (1 on 128 of 129 census boards, 2 on one). Site
/// and menu must agree with the board.
fn layout_matches(board: &K2webBoard, hex: &str) -> bool {
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect();
    let Some(plain) = bytes.and_then(|bytes| String::from_utf8(bytes).ok()) else {
        return false;
    };
    plain
        .strip_prefix(&format!("{}@@{}@@fnct", board.site, board.menu))
        .is_some_and(|slot| (1..=2).contains(&slot.len()) && positive(slot))
        // Lower-case hex only, as the site writes it.
        && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
    for anchor in pager.select(selector("a, strong")) {
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
/// A link post: a row that shows another board's article through the CMS's
/// encoded menu link, `https://<host>.sogang.ac.kr/<site>/<menu>/subview.do?enc=…`.
/// It keeps its ordinal but is not this board's notice.
fn link_post(href: &str) -> bool {
    let Some(url) = safe_url(&format!("https://{HOST}"), href) else {
        return false;
    };
    let parts: Vec<_> = url.path().trim_start_matches('/').split('/').collect();
    let pairs: Vec<_> = url.query_pairs().collect();
    url.scheme() == "https"
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".sogang.ac.kr"))
        && url.fragment().is_none()
        && parts.len() == 3
        && !parts[0].is_empty()
        && positive(parts[1])
        && parts[2] == "subview.do"
        && pairs.len() == 1
        && pairs[0].0 == "enc"
        && !pairs[0].1.is_empty()
}
/// An article link of this board: `/bbs/<site>/<bbs>/<id>/artclView.do` on the
/// CMS host, without query or fragment.
fn article_link(board: &K2webBoard, href: &str) -> Result<(String, Url), String> {
    let url = safe_url(&format!("https://{HOST}"), href).ok_or(PARSER_ERROR)?;
    let external = url
        .path()
        .strip_suffix("/artclView.do")
        .and_then(|path| path.strip_prefix(&article_base(board)))
        .filter(|value| positive(value))
        .ok_or(PARSER_ERROR)?
        .to_string();
    if url.scheme() != "https"
        || url.host_str() != Some(HOST)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return invalid();
    }
    Ok((external, url))
}
/// A row's title text without the `새글` (new post) badge, which galleries
/// print inside the title element.
fn listed_title(element: ElementRef<'_>) -> String {
    let parts: Vec<String> = element
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let badge = node
                .ancestors()
                .take_while(|ancestor| ancestor.id() != element.id())
                .filter_map(ElementRef::wrap)
                .any(|ancestor| ancestor.value().classes().any(|class| class == "new"));
            (!badge).then(|| text.to_string())
        })
        .collect();
    normalize(&parts.join(" "))
}
/// Text directly inside an element, without its labels' text.
fn own_text(element: ElementRef<'_>) -> String {
    normalize(
        &element
            .children()
            .filter_map(|child| child.value().as_text())
            .map(|text| text.to_string())
            .collect::<Vec<_>>()
            .join(" "),
    )
}
/// Collects one page's entries. Regular rows must carry the descending
/// ordinals `start, start - 1, ...`; a pin also listed as a regular row stays
/// one pinned entry.
struct Rows<'a> {
    start: u32,
    expected: u32,
    regular: u32,
    entries: Vec<ListingEntry>,
    pinned: HashSet<String>,
    seen: HashSet<String>,
    board: &'a K2webBoard,
}
impl Rows<'_> {
    fn pin(&mut self, external: String, url: Url, title: String) -> Result<(), String> {
        if !self.pinned.insert(external.clone()) {
            return invalid();
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.external_id == external)
        {
            entry.pinned = true;
        } else {
            self.entries.push(ListingEntry {
                external_id: external,
                url: url.into(),
                pinned: true,
                modified_at: None,
                title: Some(title),
            });
        }
        Ok(())
    }
    /// Checks and counts the next regular row's ordinal.
    fn count(&mut self, ordinal: u32) -> Result<(), String> {
        if self.regular >= self.expected || ordinal != self.start - self.regular {
            return invalid();
        }
        self.regular += 1;
        Ok(())
    }
    fn regular(
        &mut self,
        ordinal: u32,
        external: String,
        url: Url,
        title: String,
    ) -> Result<(), String> {
        if !self.seen.insert(external.clone()) {
            return invalid();
        }
        self.count(ordinal)?;
        // A pin repeated in the regular rows keeps its pinned entry.
        if !self.pinned.contains(&external) {
            self.entries.push(ListingEntry {
                external_id: external,
                url: url.into(),
                pinned: false,
                modified_at: None,
                title: Some(title),
            });
        }
        Ok(())
    }
}
const PRIVATE_POST: &str = "비공개된 게시물입니다.";
fn table_rows(list: ElementRef<'_>, rows: &mut Rows<'_>) -> Result<(), String> {
    let table = one(list, "table.board-table")?;
    let cells: Vec<_> = table.select(selector("tbody tr")).collect();
    if cells.len() > 100 {
        return invalid();
    }
    for row in cells {
        let number = one(row, "td.td-num")?;
        date(&text(one(row, "td.td-date")?).replace('.', "-"))?;
        // A private post keeps its ordinal but shows no link, only this text
        // and a `clsdr` marker (gradsch 1244), so there is nothing to archive.
        let cell = one(row, "td.td-title")?;
        if cell.select(selector("a")).next().is_none()
            && !row.value().classes().any(|class| class == "notice")
        {
            if text(one(cell, "span.clsdr")?) != "clsdr" || own_text(cell) != PRIVATE_POST {
                return invalid();
            }
            rows.count(text(number).parse::<u32>().map_err(|_| PARSER_ERROR)?)?;
            continue;
        }
        let anchor = one(cell, "a")?;
        // A board with categories prints `[category]` beside the `strong`
        // title, which is the title the article page confirms.
        let title = match anchor.select(selector("strong")).count() {
            0 => listed_title(anchor),
            _ => listed_title(one(anchor, "strong")?),
        };
        if title.is_empty() {
            return invalid();
        }
        let href = anchor.value().attr("href").ok_or(PARSER_ERROR)?;
        let pinned = row.value().classes().any(|class| class == "notice");
        if !pinned && link_post(href) {
            rows.count(text(number).parse::<u32>().map_err(|_| PARSER_ERROR)?)?;
            continue;
        }
        let (external, url) = article_link(rows.board, href)?;
        if pinned {
            if text(one(number, "span.notice-title")?).is_empty() {
                return invalid();
            }
            rows.pin(external, url, title)?;
        } else {
            let ordinal = text(number).parse::<u32>().map_err(|_| PARSER_ERROR)?;
            rows.regular(ordinal, external, url, title)?;
        }
    }
    Ok(())
}
fn gallery_rows(list: ElementRef<'_>, rows: &mut Rows<'_>) -> Result<(), String> {
    let wrap = one(list, ".albumWrap, .thumnailWrap")?;
    let mut lists = wrap
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| child.value().name() == "ul");
    let list = lists.next().ok_or(PARSER_ERROR)?;
    if lists.next().is_some() {
        return invalid();
    }
    let items: Vec<_> = list
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| child.value().name() == "li")
        .collect();
    if items.len() > 100 {
        return invalid();
    }
    for item in items {
        // No gallery with pins has been observed; reject rather than guess how
        // one would render.
        if item.value().classes().any(|class| class == "notice") {
            return invalid();
        }
        let anchor = one(item, "a")?;
        let href = anchor.value().attr("href").ok_or(PARSER_ERROR)?;
        let title = listed_title(one(anchor, "strong.title")?);
        if title.is_empty() {
            return invalid();
        }
        let mut published = None;
        for field in anchor.select(selector("ul.detail li")) {
            if text(one(field, "strong")?) == "등록일"
                && published.replace(own_text(field)).is_some()
            {
                return invalid();
            }
        }
        date(&published.ok_or(PARSER_ERROR)?.replace('.', "-"))?;
        // `No. 287` or `NO. 5`.
        let number = text(one(anchor, "span.num")?);
        let ordinal = number
            .get(..3)
            .filter(|prefix| prefix.eq_ignore_ascii_case("no."))
            .and_then(|_| number[3..].trim().parse::<u32>().ok())
            .ok_or(PARSER_ERROR)?;
        if link_post(href) {
            rows.count(ordinal)?;
        } else {
            let (external, url) = article_link(rows.board, href)?;
            rows.regular(ordinal, external, url, title)?;
        }
    }
    Ok(())
}
pub(super) fn listing(board: &K2webBoard, page: u32, body: &str) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    if text(one(root, "title")?) != board.title {
        return invalid();
    }
    // The board's widget is the search form's parent (`.board-list` or
    // `.album-list`); the count, rows and pager all live inside it.
    let search = one(root, "form[name=searchForm]")?;
    let list = search
        .parent()
        .and_then(ElementRef::wrap)
        .ok_or(PARSER_ERROR)?;
    let action = format!("{}artclList.do", article_base(board));
    if search.value().attr("action") != Some(action.as_str())
        || !layout_matches(board, &hidden(search, "layout")?)
        || !hidden(search, "findWord")?.is_empty()
        || !hidden(search, "findOpnwrd")?.is_empty()
    {
        return invalid();
    }
    let declared = text(one(list, ".srch_counts strong")?).replace(',', "");
    let total = declared.parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let total_pages = total.div_ceil(board.rows).max(1);
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
    pager(list, page, total_pages)?;
    let start = total
        .checked_sub((page - 1) * board.rows)
        .ok_or(PARSER_ERROR)?;
    let mut rows = Rows {
        start,
        expected: start.min(board.rows),
        regular: 0,
        entries: Vec::new(),
        pinned: HashSet::new(),
        seen: HashSet::new(),
        board,
    };
    match board.skin {
        Skin::Table => table_rows(list, &mut rows)?,
        Skin::Gallery => gallery_rows(list, &mut rows)?,
    }
    if rows.regular != rows.expected {
        return invalid();
    }
    Ok(ListingPage {
        entries: rows.entries,
        has_next: page < total_pages,
        total_pages,
    })
}
/// The shown title without the board's category prefix, checked against the
/// hidden `artclViewTitle`. Boards with categories may show `[category] title`
/// (with or without a `분류` item, which the prefix must then match), and a
/// title may itself start with brackets, so the hidden title decides. Some
/// sites escape the hidden value twice (`&amp;quot;`, even `&amp;#39S`), so it
/// also counts after one more decoding as page text.
fn shown_title(shown: &str, hidden: &str, class: Option<&str>) -> Result<String, String> {
    let hidden = normalize(hidden);
    let decoded = text(Html::parse_fragment(&hidden.replace('<', "&lt;")).root_element());
    let prefixed = shown
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .filter(|(prefix, _)| !prefix.is_empty() && !prefix.contains(['[', ']']))
        .filter(|(prefix, _)| class.is_none_or(|class| class == *prefix))
        .map(|(_, title)| title);
    [Some(shown), prefixed]
        .into_iter()
        .flatten()
        .find(|title| !title.is_empty() && (*title == hidden || *title == decoded))
        .map(str::to_owned)
        .ok_or_else(|| PARSER_ERROR.into())
}
fn once<T>(slot: &mut Option<T>, value: T) -> Result<(), String> {
    if slot.replace(value).is_some() {
        invalid()
    } else {
        Ok(())
    }
}
pub(super) fn article(
    board: &K2webBoard,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let url = detail_url(board, entry)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    // A post only its members may read answers with this message page (user
    // decision, #80): a permanent failure the collector records as skipped.
    if message(&html).as_deref() == Some("접근 권한이 없습니다.") {
        return Err("학교 사이트에 로그인해야 볼 수 있는 공지라 보관하지 않았습니다.".into());
    }
    // Some sites print `?` here; the list form and hidden title still bind
    // the page to the board and the notice.
    let canonical = one(root, "meta[property='og:url']")?
        .value()
        .attr("content")
        .ok_or(PARSER_ERROR)?;
    if canonical != "?" && canonical != url.as_str() {
        return invalid();
    }
    let view = one(root, "div.board-view div.view.viewCont")?;
    let list_action = format!("{}artclList.do", article_base(board));
    let form = one(root, "form[name=listForm]")?;
    if form.value().attr("action") != Some(list_action.as_str()) {
        return invalid();
    }
    let mut published_at = None;
    let mut author = None;
    let mut class = None;
    let items: Vec<_> = view.select(selector(".title ul.detail li")).collect();
    if items.is_empty() || items.len() > 10 {
        return invalid();
    }
    for item in items {
        let label = text(one(item, "span")?);
        let value = own_text(item);
        if value.is_empty() {
            continue;
        }
        match label.as_str() {
            "작성일" => once(&mut published_at, date(&value.replace('.', "-"))?)?,
            "작성자" => once(&mut author, value)?,
            "분류" => once(&mut class, value)?,
            _ => {}
        }
    }
    let published_at = published_at.ok_or(PARSER_ERROR)?;
    let hidden_title = one(root, "input#artclViewTitle")?
        .value()
        .attr("value")
        .ok_or(PARSER_ERROR)?;
    let title = shown_title(
        &text(one(view, ".title > strong")?),
        hidden_title,
        class.as_deref(),
    )?;
    let content = one(view, ".txt")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    let mut resources = Vec::new();
    // Boards without files render no attachment block at all.
    let blocks: Vec<_> = view.select(selector("div.attachment")).collect();
    if blocks.len() > 1 {
        return invalid();
    }
    if let Some(attachment) = blocks.first() {
        let no_file = attachment.select(selector(".no-file")).next().is_some();
        let links: Vec<_> = attachment.select(selector("ul li a")).collect();
        if no_file == !links.is_empty() {
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
                || target.host_str() != Some(HOST)
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
        category: Some(board.name.into()),
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
#[path = "fixtures/k2web_tests.rs"]
mod tests;
