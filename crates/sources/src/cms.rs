//! Boards of the school's old CMS (#316, #80 step 4): SW중심대학사업단
//! (scc.sogang.ac.kr, `siteId=sweducenter`) and 융합교육원
//! (convedu.sogang.ac.kr). Both sites run one engine and skin; a board is
//! one row of `BOARDS`. Lists are `/front/cmsboardlist.do`, which the site
//! posts but also answers to GET, and articles `/front/cmsboardview.do`.
//!
//! The article page shows any post of the CMS whatever `bbsConfigFK` asks
//! for, so only the board's own list binds a post to the board.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// SW중심대학사업단 on the scc host.
    Sweducenter,
    /// 융합교육원.
    Convedu,
}

impl Site {
    pub fn host(self) -> &'static str {
        match self {
            Site::Sweducenter => "scc.sogang.ac.kr",
            Site::Convedu => "convedu.sogang.ac.kr",
        }
    }
    pub fn site_id(self) -> &'static str {
        match self {
            Site::Sweducenter => "sweducenter",
            Site::Convedu => "convedu",
        }
    }
    /// The site's name in every page title (`<name> - <board>`).
    fn title(self) -> &'static str {
        match self {
            Site::Sweducenter => "SW중심대학사업단",
            Site::Convedu => "융합교육원",
        }
    }
}

pub struct CmsBoard {
    pub source_id: &'static str,
    pub name: &'static str,
    pub site: Site,
    /// The board's `bbsConfigFK`.
    pub config: u32,
}

/// 소식 (sweducenter) and 센터소식 (convedu) rarely post; they are kept for
/// their history (user decision, 2026-11-25).
pub const BOARDS: [CmsBoard; 5] = [
    CmsBoard {
        source_id: "sweducenter-notice",
        name: "공지사항",
        site: Site::Sweducenter,
        config: 7250,
    },
    CmsBoard {
        source_id: "sweducenter-news",
        name: "소식",
        site: Site::Sweducenter,
        config: 7251,
    },
    CmsBoard {
        source_id: "sweducenter-press",
        name: "언론보도",
        site: Site::Sweducenter,
        config: 7252,
    },
    CmsBoard {
        source_id: "convedu-notice",
        name: "공지사항",
        site: Site::Convedu,
        config: 5551,
    },
    CmsBoard {
        source_id: "convedu-news",
        name: "센터소식",
        site: Site::Convedu,
        config: 5550,
    },
];
pub(super) const LIST_PATH: &str = "/front/cmsboardlist.do";
pub(super) const VIEW_PATH: &str = "/front/cmsboardview.do";
pub(super) const FILE_PATH: &str = "/Download";

pub(super) fn board(source: &SourceDefinition) -> Result<&'static CmsBoard, String> {
    BOARDS
        .iter()
        .find(|board| board.source_id == source.id)
        .ok_or_else(|| PARSER_ERROR.into())
}
/// The board's first list page as a person opens it.
pub fn board_url(board: &CmsBoard) -> String {
    format!(
        "https://{}{LIST_PATH}?siteId={}&bbsConfigFK={}",
        board.site.host(),
        board.site.site_id(),
        board.config
    )
}
pub(super) fn list_url(board: &CmsBoard, page: u32) -> Result<Url, String> {
    let mut url = Url::parse(&board_url(board)).map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("currentPage", &page.to_string());
    Ok(url)
}
/// The article route with only the keys that select the post; list links
/// add the list's paging and search, which the page does not need.
fn article_url(board: &CmsBoard, id: &str) -> Result<Url, String> {
    let mut url = Url::parse(&format!("https://{}{VIEW_PATH}", board.site.host()))
        .map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("bbsConfigFK", &board.config.to_string())
        .append_pair("siteId", board.site.site_id())
        .append_pair("pkid", id);
    Ok(url)
}
pub(super) fn detail_url(board: &CmsBoard, entry: &ListingEntry) -> Result<Url, String> {
    let url = article_url(board, &entry.external_id)?;
    if !positive(&entry.external_id) || entry.url != url.as_str() {
        return invalid();
    }
    Ok(url)
}

/// What a `/Download` link's `pathStr` names: the post, the file's position
/// and the board. The site encodes `pkid|<id>#!#index|<n>#!#filePath|<config>`
/// reversed, as `##`-joined decimal character codes, in Base64.
#[derive(Debug, PartialEq, Eq)]
pub struct DownloadKey {
    pub pkid: String,
    pub config: u32,
}
pub fn download_key(path_str: &str) -> Option<DownloadKey> {
    if path_str.is_empty() || path_str.len() > 1024 {
        return None;
    }
    // A `+` in the query decodes as a space.
    let codes = STANDARD.decode(path_str.replace(' ', "+")).ok()?;
    let codes = std::str::from_utf8(&codes).ok()?;
    let mut text = String::new();
    for code in codes.split("##") {
        if code.is_empty() || code.len() > 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let byte: u8 = code.parse().ok()?;
        if !byte.is_ascii_graphic() {
            return None;
        }
        text.push(byte as char);
    }
    let text: String = text.chars().rev().collect();
    let fields: Vec<_> = text.split("#!#").collect();
    let [pkid, index, path] = fields[..] else {
        return None;
    };
    let pkid = pkid.strip_prefix("pkid|").filter(|v| positive(v))?;
    index.strip_prefix("index|").filter(|v| positive(v))?;
    let config = path
        .strip_prefix("filePath|")
        .filter(|v| positive(v))?
        .parse()
        .ok()?;
    Some(DownloadKey {
        pkid: pkid.into(),
        config,
    })
}

/// The page's own title and its paging form bind the response to the board.
fn check_page(board: &CmsBoard, root: ElementRef<'_>) -> Result<(), String> {
    if text(one(root, "title")?) != format!("{} - {}", board.site.title(), board.name) {
        return invalid();
    }
    Ok(())
}
fn check_form(board: &CmsBoard, root: ElementRef<'_>) -> Result<(), String> {
    let form = one(root, "form[name=pageForm]")?;
    let config = board.config.to_string();
    for (name, value) in [
        ("bbsConfigFK", config.as_str()),
        ("siteId", board.site.site_id()),
        ("searchValue", ""),
    ] {
        let input = one(form, &format!("input[name={name}]"))?;
        if input.value().attr("value") != Some(value) {
            return invalid();
        }
    }
    Ok(())
}
/// The page number inside `sendPage('pageForm','N');return false`.
fn page_target(onclick: &str) -> Result<u32, String> {
    onclick
        .strip_prefix("sendPage('pageForm','")
        .and_then(|rest| rest.strip_suffix("');return false"))
        .filter(|value| positive(value))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| PARSER_ERROR.into())
}
/// The numbered pages around the current one. Returns the last page, which
/// `div.total_cnt` states (`/ N`).
fn pager(root: ElementRef<'_>, page: u32) -> Result<u32, String> {
    let paging = one(root, "div.board_paging")?;
    let last: u32 = text(one(paging, "div.total_cnt")?)
        .strip_prefix("/ ")
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or(PARSER_ERROR)?;
    let mut numbered = Vec::new();
    for item in one(paging, "ul.num_btns")?
        .children()
        .filter_map(ElementRef::wrap)
    {
        let anchor = one(item, "a")?;
        let number: u32 = text(anchor).parse().map_err(|_| PARSER_ERROR)?;
        let current = item.value().classes().any(|class| class == "on");
        match anchor.value().attr("onclick") {
            None if current && number == page => {}
            Some(onclick) if !current && page_target(onclick)? == number => {}
            _ => return invalid(),
        }
        numbered.push(number);
    }
    // Past the last page the site marks the last page as current, which the
    // check above refuses.
    let first = *numbered.first().ok_or(PARSER_ERROR)?;
    if numbered
        .iter()
        .copied()
        .ne(first..first + numbered.len() as u32)
        || !numbered.contains(&page)
        || numbered.last().is_some_and(|number| *number > last)
    {
        return invalid();
    }
    Ok(last)
}
/// An article link of the board's list page: the article route with the
/// list's paging and search, bound to this board and page.
fn row_link(board: &CmsBoard, page: u32, href: &str) -> Result<String, String> {
    let url = Url::parse(&format!("https://{}{LIST_PATH}", board.site.host()))
        .and_then(|base| base.join(href))
        .map_err(|_| PARSER_ERROR)?;
    let config = board.config.to_string();
    let current = page.to_string();
    let mut id = None;
    let mut keys = HashSet::new();
    for (key, value) in url.query_pairs() {
        if !keys.insert(key.to_string()) {
            return invalid();
        }
        let expected = match key.as_ref() {
            "currentPage" => current.as_str(),
            "searchField" | "searchLowItem" => "ALL",
            "searchValue" => "",
            "bbsConfigFK" => config.as_str(),
            "siteId" => board.site.site_id(),
            "pkid" => {
                id = Some(value.to_string());
                continue;
            }
            _ => return invalid(),
        };
        if value != expected {
            return invalid();
        }
    }
    let id = id.filter(|id| positive(id)).ok_or(PARSER_ERROR)?;
    if url.host_str() != Some(board.site.host()) || url.path() != VIEW_PATH || keys.len() != 7 {
        return invalid();
    }
    Ok(id)
}
pub(super) fn listing(board: &CmsBoard, page: u32, body: &str) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    check_page(board, root)?;
    check_form(board, root)?;
    let last = pager(root, page)?;
    let items: Vec<_> = one(root, "div.board_list_wrap div.list_box > ul")?
        .children()
        .filter_map(ElementRef::wrap)
        .collect();
    let mut entries: Vec<ListingEntry> = Vec::new();
    let mut seen = HashSet::new();
    let mut numbers = Vec::new();
    for item in items {
        if item.value().name() != "li" {
            return invalid();
        }
        let anchor = one(item, "a.title")?;
        let id = row_link(
            board,
            page,
            anchor.value().attr("href").ok_or(PARSER_ERROR)?,
        )?;
        // A pinned row's link starts with `<strong>[공지] </strong>`; a
        // regular row has its running number beside the link instead.
        let badge: Vec<_> = anchor.select(selector("strong")).collect();
        let row = item
            .children()
            .filter_map(ElementRef::wrap)
            .next()
            .filter(|row| row.value().name() == "div")
            .ok_or(PARSER_ERROR)?;
        let number = row
            .children()
            .filter_map(ElementRef::wrap)
            .next()
            .filter(|cell| cell.value().name() == "div" && cell.value().attr("class").is_none())
            .map(text);
        let pinned = match (&badge[..], number) {
            ([strong], None) if text(*strong) == "[공지]" => true,
            ([], Some(number)) => {
                numbers.push(number.parse::<u32>().map_err(|_| PARSER_ERROR)?);
                false
            }
            _ => return invalid(),
        };
        if text(anchor).is_empty() {
            return invalid();
        }
        let info: Vec<_> = one(item, "div.info")?
            .children()
            .filter_map(ElementRef::wrap)
            .collect();
        let [_, day, _] = info[..] else {
            return invalid();
        };
        date(&text(day).replace('.', "-"))?;
        // Pins lead the list.
        if pinned && entries.iter().any(|entry| !entry.pinned) {
            return invalid();
        }
        if !seen.insert(id.clone()) {
            return invalid();
        }
        let url = article_url(board, &id)?;
        entries.push(ListingEntry {
            external_id: id,
            url: url.into(),
            pinned,
            modified_at: None,
            title: None,
        });
    }
    // Regular rows carry the board's running numbers, newest first, ending at
    // 1 on the last page; a gap or an early end means rows went missing.
    let (Some(&top), Some(&bottom)) = (numbers.first(), numbers.last()) else {
        return invalid();
    };
    if numbers.iter().copied().ne((bottom..=top).rev()) || (bottom == 1) != (page == last) {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < last,
        total_pages: last,
    })
}
/// One attachment: the `/Download` route whose key names this post and
/// board, saved under the link text.
fn attachment(
    board: &CmsBoard,
    entry: &ListingEntry,
    link: ElementRef<'_>,
) -> Result<ResourceInput, String> {
    let target =
        safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?).ok_or(PARSER_ERROR)?;
    let mut key = None;
    let mut keys = HashSet::new();
    for (name, value) in target.query_pairs() {
        if !keys.insert(name.to_string()) {
            return invalid();
        }
        match name.as_ref() {
            "pathStr" => key = download_key(&value),
            "fileName" if !value.is_empty() => {}
            "gubun" if value == "board" => {}
            _ => return invalid(),
        }
    }
    let key = key.ok_or(PARSER_ERROR)?;
    if target.scheme() != "https"
        || target.host_str() != Some(board.site.host())
        || target.path() != FILE_PATH
        || target.fragment().is_some()
        || keys.len() != 3
        || key.pkid != entry.external_id
        || key.config != board.config
    {
        return invalid();
    }
    let filename = text(link);
    if filename.is_empty() {
        return invalid();
    }
    Ok(ResourceInput {
        url: target.into(),
        filename,
        kind: "attachment".into(),
    })
}
pub(super) fn article(
    board: &CmsBoard,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    detail_url(board, entry)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    check_page(board, root)?;
    let view = one(root, "div.board_read_wrap")?;
    let title = text(one(view, "div.post_info > div.title")?);
    let info: Vec<_> = one(view, "div.post_info > div.info")?
        .children()
        .filter_map(ElementRef::wrap)
        .collect();
    let [author, posted] = info[..] else {
        return invalid();
    };
    let author = text(author);
    // `YYYY.MM.DD HH:MM:SS`
    let posted = text(posted);
    let published_at = date(&posted.replace('.', "-"))?;
    if posted.len() != 19 {
        return invalid();
    }
    let mut resources = Vec::new();
    for link in view.select(selector("div.post_files div.files > div > a")) {
        resources.push(attachment(board, entry, link)?);
    }
    let content = one(view, "div.post_cont")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    merge_inline_resources(&mut resources, inline);
    if title.is_empty() || resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title,
        author: (!author.is_empty()).then_some(author),
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
#[path = "fixtures/cms_tests.rs"]
mod tests;
