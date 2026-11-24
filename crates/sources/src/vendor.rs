//! Boards of the two continuing-education sites that share one outside
//! vendor's PHP board engine (#80 step 4): 미래교육원 (scec.sogang.ac.kr) and
//! 한국어교육원 (klec.sogang.ac.kr). The engines differ in markup and routes, so
//! each site has its own list and article parser; a board is one row of
//! `BOARDS`.
//!
//! scec's 공지사항 is read as the 전체 tab only: 학점은행제, 비학위과정 and
//! 교육원소식 are category filters of the same board (`bbsid=f_notice`), and the
//! article page names a notice's category (user decision, 2026-11-24). klec's
//! 학사공지 is left out because every post on it needs a login (user decision).
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// 미래교육원: `/fro_end/html/<page>?pageno=N`, articles at the same page
    /// with `smode=view`. The server binds an article to the board.
    Scec,
    /// 한국어교육원: lists through `/?url=<page>&pageno=N`, articles at
    /// `/index.php`, which serves any board's post whatever `bbsid` asks for.
    Klec,
}

pub struct VendorBoard {
    pub source_id: &'static str,
    pub name: &'static str,
    pub organization: &'static str,
    pub group: &'static str,
    pub site: Site,
    /// The board's page path (scec: the list and article route; klec: the
    /// `url` value of the list route).
    pub page: &'static str,
    pub bbsid: &'static str,
    /// klec's photo board renders cards instead of a numbered table.
    pub gallery: bool,
}

pub const BOARDS: [VendorBoard; 3] = [
    VendorBoard {
        source_id: "scec-notice",
        name: "공지사항",
        organization: "서강대학교 · 미래교육원",
        group: "sogang-scec",
        site: Site::Scec,
        page: "/fro_end/html/dep_05/5100.php",
        bbsid: "f_notice",
        gallery: false,
    },
    VendorBoard {
        source_id: "klec-notice",
        name: "일반공지",
        organization: "서강대학교 · 한국어교육원",
        group: "sogang-klec",
        site: Site::Klec,
        page: "/dep_04/4110.php",
        bbsid: "00010",
        gallery: false,
    },
    VendorBoard {
        source_id: "klec-news",
        name: "한국어교육원 소식",
        organization: "서강대학교 · 한국어교육원",
        group: "sogang-klec",
        site: Site::Klec,
        page: "/dep_04/4300.php",
        bbsid: "00011",
        gallery: true,
    },
];
pub(super) const SCEC_HOST: &str = "scec.sogang.ac.kr";
pub(super) const KLEC_HOST: &str = "klec.sogang.ac.kr";
/// Rows per page: scec's and klec's tables show ten regular rows, klec's
/// gallery twelve cards.
const ROWS: u32 = 10;
const GALLERY_ROWS: u32 = 12;

pub(super) fn board(source: &SourceDefinition) -> Result<&'static VendorBoard, String> {
    BOARDS
        .iter()
        .find(|board| board.source_id == source.id)
        .ok_or_else(|| PARSER_ERROR.into())
}
fn host(board: &VendorBoard) -> &'static str {
    match board.site {
        Site::Scec => SCEC_HOST,
        Site::Klec => KLEC_HOST,
    }
}
/// The page a person opens for the board.
pub fn board_url(board: &VendorBoard) -> String {
    match board.site {
        Site::Scec => format!("https://{SCEC_HOST}{}", board.page),
        Site::Klec => format!("https://{KLEC_HOST}/?url={}", board.page),
    }
}
pub(super) fn list_url(board: &VendorBoard, page: u32) -> Result<Url, String> {
    let mut url = match board.site {
        Site::Scec => Url::parse(&format!("https://{SCEC_HOST}{}", board.page)),
        Site::Klec => Url::parse(&format!("https://{KLEC_HOST}/")),
    }
    .map_err(|_| PARSER_ERROR)?;
    {
        let mut query = url.query_pairs_mut();
        if board.site == Site::Klec {
            query.append_pair("url", board.page);
        }
        query.append_pair("pageno", &page.to_string());
    }
    Ok(url)
}
/// The article route with only the keys that select the post; list links add
/// paging, search and an opaque return token the server does not need.
fn article_url(board: &VendorBoard, id: &str) -> Result<Url, String> {
    let mut url = match board.site {
        Site::Scec => Url::parse(&format!("https://{SCEC_HOST}{}", board.page)),
        Site::Klec => Url::parse(&format!("https://{KLEC_HOST}/index.php")),
    }
    .map_err(|_| PARSER_ERROR)?;
    {
        let mut query = url.query_pairs_mut();
        match board.site {
            Site::Scec => {
                query.append_pair("smode", "view");
                query.append_pair("bbs_idx", id);
                query.append_pair("bbsid", board.bbsid);
            }
            Site::Klec => {
                query.append_pair("bbs_idx", id);
                query.append_pair("pagekind", "c");
                query.append_pair("bbsid", board.bbsid);
            }
        }
    }
    Ok(url)
}
pub(super) fn detail_url(board: &VendorBoard, entry: &ListingEntry) -> Result<Url, String> {
    let url = article_url(board, &entry.external_id)?;
    if !positive(&entry.external_id) || entry.url != url.as_str() {
        return invalid();
    }
    Ok(url)
}
/// The one value of a query key; a key repeated with different values (klec
/// links repeat some keys) or a missing key is an error.
fn query_value(url: &Url, key: &str) -> Result<String, String> {
    let mut values = url
        .query_pairs()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned());
    let first = values.next().ok_or(PARSER_ERROR)?;
    if values.any(|value| value != first) {
        return invalid();
    }
    Ok(first)
}
/// The post a list link opens, after checking it is this board's article
/// route on the site's host.
fn article_link(board: &VendorBoard, href: &str) -> Result<(String, Url), String> {
    let base = format!("https://{}{}", host(board), board.page);
    let link = safe_url(&base, href).ok_or(PARSER_ERROR)?;
    let route = match board.site {
        Site::Scec => link.path() == board.page && query_value(&link, "smode")? == "view",
        Site::Klec => link.path() == "/index.php" && query_value(&link, "pagekind")? == "c",
    };
    let id = query_value(&link, "bbs_idx")?;
    if link.scheme() != "https"
        || link.host_str() != Some(host(board))
        || link.port().is_some()
        || link.fragment().is_some()
        || !route
        || !positive(&id)
        || query_value(&link, "bbsid")? != board.bbsid
    {
        return invalid();
    }
    let url = article_url(board, &id)?;
    Ok((id, url))
}
/// A pager link's target page, on this board's list.
fn page_target(board: &VendorBoard, href: &str) -> Result<u32, String> {
    let base = format!("https://{}{}", host(board), board.page);
    let link = safe_url(&base, href).ok_or(PARSER_ERROR)?;
    let value = query_value(&link, "pageno")?;
    if link.host_str() != Some(host(board))
        || query_value(&link, "bbsid")? != board.bbsid
        || !positive(&value)
    {
        return invalid();
    }
    value.parse().map_err(|_| PARSER_ERROR.into())
}
/// `#` (no target) or a page number.
fn optional_target(board: &VendorBoard, href: &str) -> Result<Option<u32>, String> {
    if href == "#" || href == "#none" {
        Ok(None)
    } else {
        page_target(board, href).map(Some)
    }
}
fn entry(id: String, url: Url, pinned: bool) -> ListingEntry {
    ListingEntry {
        external_id: id,
        url: url.into(),
        pinned,
        modified_at: None,
        title: None,
    }
}

/// scec's pager: blocks of five numbered pages between `이전`/`다음` arrows,
/// which point at the neighbouring blocks or at `#`.
fn scec_pager(
    board: &VendorBoard,
    root: ElementRef<'_>,
    page: u32,
    total_pages: u32,
) -> Result<(), String> {
    let pager = one(root, "div.bbs-paging > ul")?;
    let block_start = (page - 1) / 5 * 5 + 1;
    let block_end = (block_start + 4).min(total_pages);
    let mut numbered = Vec::new();
    let mut arrows = HashMap::new();
    for item in pager.children().filter_map(ElementRef::wrap) {
        let classes: Vec<&str> = item.value().classes().collect();
        let href = one(item, "a")?.value().attr("href").ok_or(PARSER_ERROR)?;
        if classes.contains(&"arrow") {
            let kind = ["prev", "next"]
                .into_iter()
                .find(|kind| classes.contains(kind))
                .ok_or(PARSER_ERROR)?;
            if arrows.insert(kind, optional_target(board, href)?).is_some() {
                return invalid();
            }
            continue;
        }
        let number = text(item).parse::<u32>().map_err(|_| PARSER_ERROR)?;
        let active = classes.contains(&"on");
        let target = optional_target(board, href)?;
        if active != (number == page) || (active != target.is_none()) {
            return invalid();
        }
        if target.is_some_and(|target| target != number) {
            return invalid();
        }
        numbered.push(number);
    }
    if numbered.iter().copied().ne(block_start..=block_end)
        || arrows.get("prev") != Some(&(block_start > 1).then(|| block_start - 1))
        || arrows.get("next") != Some(&(block_end < total_pages).then(|| block_end + 1))
    {
        return invalid();
    }
    Ok(())
}
fn scec_listing(board: &VendorBoard, page: u32, body: &str) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    // The board's heading and the 전체 tab: a category tab would list only
    // part of the board.
    if text(one(root, "div.title > h3")?) != board.name
        || text(one(root, ".sub-tabmenu li.active a")?) != "전체"
    {
        return invalid();
    }
    let list = one(root, "section.list-wrap")?;
    let total = text(one(list, "p.total span.total-numb")?)
        .replace(',', "")
        .parse::<u32>()
        .map_err(|_| PARSER_ERROR)?;
    let total_pages = total.div_ceil(ROWS).max(1);
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
    scec_pager(board, list, page, total_pages)?;
    // Pins take the leading slots of the pins-first ordering, so the first
    // pages can begin with pins and every page holds its share of the total.
    let rows: Vec<_> = one(list, "table.bbs-table")?
        .select(selector("tbody tr"))
        .collect();
    if rows.len() as u32 != (total - (page - 1) * ROWS).min(ROWS) {
        return invalid();
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut ordinals = HashSet::new();
    for row in rows {
        let number = text(one(row, "td.numb")?);
        let pinned = number == "공지";
        if pinned != row.value().classes().any(|class| class == "bbs-bbs-noti") {
            return invalid();
        }
        if pinned {
            // A pin after a regular row would break the leading-slot layout.
            if entries.iter().any(|entry: &ListingEntry| !entry.pinned) {
                return invalid();
            }
        } else {
            // The site orders by date, so a re-dated post can sit out of
            // ordinal order (page 5 began with 336 before 343).
            let ordinal = number
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0 && *value <= total)
                .ok_or(PARSER_ERROR)?;
            if !ordinals.insert(ordinal) {
                return invalid();
            }
        }
        let anchor = one(row, "td.tit a")?;
        if text(anchor).is_empty() {
            return invalid();
        }
        date(&text(one(row, "td.date")?).replace('.', "-"))?;
        let (id, url) = article_link(board, anchor.value().attr("href").ok_or(PARSER_ERROR)?)?;
        if !seen.insert(id.clone()) {
            return invalid();
        }
        entries.push(entry(id, url, pinned));
    }
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}

/// klec's pager: blocks of ten numbered pages with first/previous/next/last
/// links. The list states no total, so `last` gives the page count; on the
/// last page it and `next` are `#`.
fn klec_pager(board: &VendorBoard, root: ElementRef<'_>, page: u32) -> Result<u32, String> {
    let pager = one(root, "div.pagination > div.page-num")?;
    let mut numbered = Vec::new();
    let mut special = HashMap::new();
    for anchor in pager.children().filter_map(ElementRef::wrap) {
        if anchor.value().name() != "a" {
            return invalid();
        }
        let classes: Vec<&str> = anchor.value().classes().collect();
        let target = optional_target(board, anchor.value().attr("href").ok_or(PARSER_ERROR)?)?;
        if classes.contains(&"page-btn") {
            let kind = ["first", "prev", "next", "last"]
                .into_iter()
                .find(|kind| classes.contains(kind))
                .ok_or(PARSER_ERROR)?;
            if special.insert(kind, target).is_some() {
                return invalid();
            }
            continue;
        }
        let number = text(anchor).parse::<u32>().map_err(|_| PARSER_ERROR)?;
        let active = classes.contains(&"on");
        if active != (number == page)
            || active != target.is_none()
            || target.is_some_and(|target| target != number)
        {
            return invalid();
        }
        numbered.push(number);
    }
    let last = special.get("last").copied().ok_or(PARSER_ERROR)?;
    let total_pages = last.unwrap_or(page);
    // A page past the end still renders the last block, without an active
    // page, which the numbered check below rejects.
    if total_pages < page || (last.is_some() && total_pages == page) {
        return invalid();
    }
    let block_start = (page - 1) / 10 * 10 + 1;
    let block_end = (block_start + 9).min(total_pages);
    let first = special.get("first").copied().ok_or(PARSER_ERROR)?;
    if numbered.iter().copied().ne(block_start..=block_end)
        || !matches!(first, None | Some(1))
        || special.get("prev") != Some(&(block_start > 1).then(|| block_start - 1))
        || special.get("next") != Some(&(block_end < total_pages).then(|| block_end + 1))
    {
        return invalid();
    }
    Ok(total_pages)
}
/// A post only members may read: the list shows its title with an alert
/// instead of a link.
fn klec_locked(href: &str) -> bool {
    href.starts_with("javascript:alert(") && href.contains("권한이 없습니다")
}
fn klec_table(
    board: &VendorBoard,
    list: ElementRef<'_>,
    last_page: bool,
) -> Result<Vec<ListingEntry>, String> {
    let rows: Vec<_> = one(list, "table.table01-sub")?
        .select(selector("tbody tr"))
        .collect();
    if rows.len() > 100 {
        return invalid();
    }
    let mut entries: Vec<ListingEntry> = Vec::new();
    let mut ordinals = Vec::new();
    let mut pinned = HashSet::new();
    let mut seen = HashSet::new();
    for row in rows {
        let number = one(row, "td[data-label='번호']")?;
        let anchor = one(row, "td[data-label='제목'] a.bbs-tit")?;
        if text(anchor).is_empty() {
            return invalid();
        }
        date(&text(one(row, "td[data-label='작성일']")?))?;
        let href = anchor.value().attr("href").ok_or(PARSER_ERROR)?;
        let pin = number.select(selector("span.notice")).next().is_some();
        if pin {
            if text(number) != "공지" {
                return invalid();
            }
            let (id, url) = article_link(board, href)?;
            if !pinned.insert(id.clone()) {
                return invalid();
            }
            entries.push(entry(id, url, true));
            continue;
        }
        ordinals.push(text(number).parse::<u32>().map_err(|_| PARSER_ERROR)?);
        if klec_locked(href) {
            continue;
        }
        let (id, url) = article_link(board, href)?;
        if !seen.insert(id.clone()) {
            return invalid();
        }
        // A pin repeated among the regular rows keeps its pinned entry.
        if !pinned.contains(&id) {
            entries.push(entry(id, url, false));
        }
    }
    // Regular rows count down by one; a full page holds ten and the last page
    // ends at 1.
    if ordinals.is_empty()
        || ordinals.len() as u32 > ROWS
        || ordinals.windows(2).any(|pair| pair[0] != pair[1] + 1)
        || (!last_page && ordinals.len() as u32 != ROWS)
        || (last_page && ordinals.last() != Some(&1))
    {
        return invalid();
    }
    Ok(entries)
}
fn klec_gallery(
    board: &VendorBoard,
    list: ElementRef<'_>,
    last_page: bool,
) -> Result<Vec<ListingEntry>, String> {
    let items: Vec<_> = one(list, "ul.photo_wrap")?
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| child.value().name() == "li")
        .collect();
    let count = items.len() as u32;
    if count == 0 || count > GALLERY_ROWS || (!last_page && count != GALLERY_ROWS) {
        return invalid();
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let anchor = one(item, "a")?;
        if text(one(anchor, "div.tit")?).is_empty() {
            return invalid();
        }
        date(&text(one(anchor, "div.date")?))?;
        let (id, url) = article_link(board, anchor.value().attr("href").ok_or(PARSER_ERROR)?)?;
        if !seen.insert(id.clone()) {
            return invalid();
        }
        entries.push(entry(id, url, false));
    }
    Ok(entries)
}
fn klec_listing(board: &VendorBoard, page: u32, body: &str) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    if page == 0 {
        return invalid();
    }
    // The list form names the board and page it shows, and so does the pager.
    let form = one(root, "form[name=SearchForm]")?;
    if hidden(form, "input#bbsid")? != board.bbsid
        || hidden(form, "input#pageno")? != page.to_string()
    {
        return invalid();
    }
    let total_pages = klec_pager(board, form, page)?;
    let last_page = page == total_pages;
    let entries = if board.gallery {
        klec_gallery(board, form, last_page)?
    } else {
        klec_table(board, form, last_page)?
    };
    Ok(ListingPage {
        entries,
        has_next: !last_page,
        total_pages,
    })
}
pub(super) fn listing(board: &VendorBoard, page: u32, body: &str) -> Result<ListingPage, String> {
    match board.site {
        Site::Scec => scec_listing(board, page, body),
        Site::Klec => klec_listing(board, page, body),
    }
}

/// scec answers a post that is gone (or belongs to another board) with the
/// article frame left empty: no title and no date.
fn scec_empty(viewer: ElementRef<'_>) -> Result<bool, String> {
    let title = text(one(viewer, "h5.view-tit")?);
    Ok(title.is_empty()
        && scec_info(viewer)?
            .get("등록일")
            .is_none_or(String::is_empty))
}
fn scec_info(viewer: ElementRef<'_>) -> Result<HashMap<String, String>, String> {
    let mut info = HashMap::new();
    for item in viewer.select(selector("ul.view-info > li")) {
        let label = text(one(item, "p.view-info-tit")?);
        if info
            .insert(label, text(one(item, "p.view-info-txt")?))
            .is_some()
        {
            return invalid();
        }
    }
    Ok(info)
}
pub(super) fn missing(board: &VendorBoard, body: &str) -> bool {
    // klec answers a missing post with the same page as a members-only one,
    // so only scec can report one.
    board.site == Site::Scec
        && one(Html::parse_document(body).root_element(), "div.viewer")
            .and_then(scec_empty)
            .unwrap_or(false)
}
/// The hidden form fields that name the post the page shows.
fn hidden(root: ElementRef<'_>, css: &str) -> Result<String, String> {
    Ok(one(root, css)?
        .value()
        .attr("value")
        .ok_or(PARSER_ERROR)?
        .to_string())
}
/// Attachment links: `<route>?downloadname=<name>&filename=<stored>&bbs_idx=<id>`
/// on the site's own host.
fn attachments(
    board: &VendorBoard,
    entry: &ListingEntry,
    links: Vec<ElementRef<'_>>,
    route: &str,
) -> Result<Vec<ResourceInput>, String> {
    let mut resources = Vec::new();
    for link in links {
        let target = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
            .ok_or(PARSER_ERROR)?;
        if target.scheme() != "https"
            || target.host_str() != Some(host(board))
            || target.path() != route
            || target.fragment().is_some()
            || query_value(&target, "bbs_idx")? != entry.external_id
            || query_value(&target, "filename")?.is_empty()
            || query_value(&target, "downloadname")?.is_empty()
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
    Ok(resources)
}
struct Parts {
    title: String,
    author: Option<String>,
    category: Option<String>,
    published_at: String,
    content: String,
    resources: Vec<ResourceInput>,
}
fn scec_article(
    board: &VendorBoard,
    entry: &ListingEntry,
    root: ElementRef<'_>,
) -> Result<Parts, String> {
    let viewer = one(root, "div.viewer")?;
    if scec_empty(viewer)? {
        return Err("학교 사이트에서 찾을 수 없는 공지입니다.".into());
    }
    let form = one(root, "form#deleteForm")?;
    if hidden(form, "input[name=bbs_idx]")? != entry.external_id
        || hidden(form, "input[name=bbsid]")? != board.bbsid
    {
        return invalid();
    }
    let title = text(one(viewer, "h5.view-tit")?);
    let info = scec_info(viewer)?;
    let published_at = date(info.get("등록일").ok_or(PARSER_ERROR)?)?;
    let author = info
        .get("작성자")
        .filter(|value| !value.is_empty())
        .cloned();
    // `전체` marks a post without a category (user decision: the category is
    // the notice's own, the board name otherwise).
    let tags: Vec<_> = viewer.select(selector("p.view-tag")).collect();
    let category = match tags[..] {
        [] => None,
        [tag] => Some(text(tag)).filter(|tag| !tag.is_empty() && tag != "전체"),
        _ => return invalid(),
    };
    let content = one(viewer, "div.view-con > div.inner")?.inner_html();
    let blocks: Vec<_> = viewer.select(selector("div.view-attach")).collect();
    let links = match blocks[..] {
        [] => vec![],
        [block] => block.select(selector("ul.attach-list > li > a")).collect(),
        _ => return invalid(),
    };
    let resources = attachments(board, entry, links, "/nGsmart/modules/dn.php")?;
    Ok(Parts {
        title,
        author,
        category,
        published_at,
        content,
        resources,
    })
}
fn klec_article(
    board: &VendorBoard,
    entry: &ListingEntry,
    root: ElementRef<'_>,
) -> Result<Parts, String> {
    let views: Vec<_> = root.select(selector("div.bbs-view")).collect();
    // A members-only post, and a deleted one, render only this sentence where
    // the article would be.
    if views.is_empty() && text(root).contains("글보기 권한이 없습니다.") {
        return Err(
            "학교 사이트에 로그인해야 볼 수 있거나 삭제된 공지라 보관하지 않았습니다.".into(),
        );
    }
    let [view] = views[..] else {
        return invalid();
    };
    // The site serves any board's post whatever `bbsid` the request names;
    // the comment form's hidden fields carry the post's own board.
    if hidden(root, "input#bbs_idx")? != entry.external_id
        || hidden(root, "input#bbsid")? != board.bbsid
    {
        return invalid();
    }
    let title = text(one(view, "div.bbs-tit-wrap > div.bbs-tit")?);
    let published_at = date(&text(one(view, "div.bbs-date")?))?;
    let mut content = one(view, "div.bbs-con")?.inner_html();
    // Up to two blocks below the body: `첨부파일` (files) and `링크`, related
    // links the editor attached, which join the body so they are kept.
    let mut files = None;
    let mut related = None;
    for block in view.select(selector("div.bbs-attachments")) {
        let slot = match text(one(block, "div.attach-tit")?).as_str() {
            "첨부파일" => &mut files,
            "링크" => &mut related,
            _ => return invalid(),
        };
        if slot.replace(block).is_some() {
            return invalid();
        }
    }
    if let Some(block) = related {
        for link in one(block, "ul.file-wrap")?.select(selector("a")) {
            content.push_str(&format!("<p>{}</p>", link.html()));
        }
    }
    let links = files.map_or_else(Vec::new, |block| {
        block.select(selector("ul.file-wrap > li > a")).collect()
    });
    let resources = attachments(board, entry, links, "/univ/lms_bbs/dn.php")?;
    Ok(Parts {
        title,
        author: None,
        category: None,
        published_at,
        content,
        resources,
    })
}
pub(super) fn article(
    board: &VendorBoard,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    detail_url(board, entry)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let parts = match board.site {
        Site::Scec => scec_article(board, entry, root)?,
        Site::Klec => klec_article(board, entry, root)?,
    };
    if parts.title.is_empty() {
        return invalid();
    }
    let (body_html, body_text, inline) = sanitize(&parts.content, &entry.url)?;
    let mut resources = parts.resources;
    merge_inline_resources(&mut resources, inline);
    if resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title: parts.title,
        author: parts.author,
        category: Some(parts.category.unwrap_or_else(|| board.name.into())),
        published_at: parts.published_at,
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
#[path = "fixtures/vendor_tests.rs"]
mod tests;
