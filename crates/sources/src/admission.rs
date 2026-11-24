//! 입학처 공지사항 (admission.sogang.ac.kr, #80 step 4): one board, the 전체
//! tab of 입학자료실 공지사항, which lists every admission category (수시, 정시,
//! 재외국민, 편입학, 고교대학연계, 기타) as one board (user decision). The ASP
//! site posts its forms, but answers the same pages to GET with the form's
//! fields as the query: `notice.asp?s_page=N` and `noticeView.asp?idx=N`.
use super::*;

pub(super) const HOST: &str = "admission.sogang.ac.kr";
pub(super) const LIST_PATH: &str = "/enter/html/counsel/notice.asp";
pub(super) const VIEW_PATH: &str = "/enter/html/counsel/noticeView.asp";
pub(super) const FILE_PATH: &str = "/common/download.File.asp";
/// The stored file folder every attachment of the board names.
pub(crate) const FILE_FOLDER: &str = "/upload/BBS0004/";
pub(crate) const SOURCE_ID: &str = "admission-notice";
pub(crate) const NAME: &str = "공지사항";
/// Slots per page. Pins take slots on page 1 and repeat above the ten
/// regular rows of every later page.
const ROWS: u32 = 10;
/// The site's whole answer for a post that does not exist.
const MISSING: &str =
    "<script type=\"text/javascript\">alert(\"정보가 존재하지 않습니다.\"); history.back();</script>";

pub(super) fn list_url(page: u32) -> Result<Url, String> {
    let mut url = Url::parse(&format!("https://{HOST}{LIST_PATH}")).map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("s_page", &page.to_string());
    Ok(url)
}
fn article_url(id: &str) -> Result<Url, String> {
    let mut url = Url::parse(&format!("https://{HOST}{VIEW_PATH}")).map_err(|_| PARSER_ERROR)?;
    url.query_pairs_mut().append_pair("idx", id);
    Ok(url)
}
pub(super) fn detail_url(entry: &ListingEntry) -> Result<Url, String> {
    let url = article_url(&entry.external_id)?;
    if !positive(&entry.external_id) || entry.url != url.as_str() {
        return invalid();
    }
    Ok(url)
}
pub(super) fn missing(body: &str) -> bool {
    body.trim() == MISSING
}
/// The number inside `<function>('N'); return false;`, the only form the
/// site's links take.
fn call_target(onclick: &str, function: &str) -> Result<String, String> {
    let value = onclick
        .strip_prefix(function)
        .and_then(|rest| rest.strip_prefix("('"))
        .and_then(|rest| rest.strip_suffix("'); return false;"))
        .filter(|value| positive(value))
        .ok_or(PARSER_ERROR)?;
    Ok(value.to_string())
}
/// The page's own title check: the heading of 공지사항 with the 전체 tab
/// active, so a category filter is never read as the board.
fn check_page(root: ElementRef<'_>) -> Result<(), String> {
    if text(one(root, "p.pageTitle")?) != "공지사항"
        || text(one(root, "ul.tabWrap > li.on")?) != "전체"
    {
        return invalid();
    }
    Ok(())
}
/// The numbered pages; the site has shown only one block so far, so the
/// previous/next links must be inert. Returns the last page.
fn pager(root: ElementRef<'_>, page: u32) -> Result<u32, String> {
    let pager = one(root, "div.pageArea > div.inner")?;
    let mut numbered = Vec::new();
    for anchor in pager.children().filter_map(ElementRef::wrap) {
        let classes: Vec<&str> = anchor.value().classes().collect();
        let onclick = anchor.value().attr("onclick").ok_or(PARSER_ERROR)?;
        if classes.contains(&"page") {
            // More than one block would need these to page; reject rather
            // than guess how they would render.
            if onclick != "return false;" {
                return invalid();
            }
            continue;
        }
        let target = call_target(onclick, "listData")?;
        if !classes.contains(&"num") || text(anchor) != target {
            return invalid();
        }
        let number: u32 = target.parse().map_err(|_| PARSER_ERROR)?;
        if classes.contains(&"on") != (number == page) {
            return invalid();
        }
        numbered.push(number);
    }
    let last = numbered.len() as u32;
    // Past the last page the site still lists the pins under an unmarked
    // pager, which the active-page check above cannot pass.
    if last == 0 || numbered.iter().copied().ne(1..=last) || page > last {
        return invalid();
    }
    Ok(last)
}
/// A row's title: the paragraph's text without the `N` (new) badge. Regular
/// rows begin with their category in brackets (`[정시] …`).
fn row_title(paragraph: ElementRef<'_>) -> String {
    let parts: Vec<String> = paragraph
        .descendants()
        .filter_map(|node| {
            let text = node.value().as_text()?;
            let badge = node
                .ancestors()
                .filter_map(ElementRef::wrap)
                .take_while(|ancestor| ancestor.id() != paragraph.id())
                .any(|ancestor| ancestor.value().classes().any(|class| class == "new"));
            (!badge).then(|| text.to_string())
        })
        .collect();
    normalize(&parts.join(""))
}
pub(super) fn listing(page: u32, body: &str) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    if page == 0 {
        return invalid();
    }
    check_page(root)?;
    let total = text(one(root, "div.boardSearch span.num")?)
        .strip_suffix('건')
        .map(|value| value.replace(',', ""))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(PARSER_ERROR)?;
    let last = pager(root, page)?;
    let items: Vec<_> = one(root, "ul.boardList")?
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| child.value().name() == "li")
        .collect();
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut pins = 0;
    for item in items {
        let anchor = one(item, "a")?;
        let id = call_target(
            anchor.value().attr("onclick").ok_or(PARSER_ERROR)?,
            "viewData",
        )?;
        let subject = one(anchor, "div.subJect")?;
        let badges: Vec<_> = subject.select(selector("span.hot")).collect();
        let pinned = match badges[..] {
            [] => false,
            [badge] if text(badge) == "공지" => true,
            _ => return invalid(),
        };
        if row_title(one(subject, "p")?).is_empty() {
            return invalid();
        }
        let shown = text(one(anchor, "p.date")?);
        date(
            &shown
                .strip_prefix("작성일 : ")
                .ok_or(PARSER_ERROR)?
                .replace('.', "-"),
        )?;
        // Pins lead the list.
        if pinned && entries.iter().any(|entry: &ListingEntry| !entry.pinned) {
            return invalid();
        }
        if !seen.insert(id.clone()) {
            return invalid();
        }
        pins += u32::from(pinned);
        let url = article_url(&id)?;
        entries.push(ListingEntry {
            external_id: id,
            url: url.into(),
            pinned,
            modified_at: None,
            title: None,
        });
    }
    // The declared total counts pins once. Page 1 shares its ten slots with
    // the pins; every later page shows ten regular rows below them.
    let regular_total = total.checked_sub(pins).ok_or(PARSER_ERROR)?;
    let first = (ROWS.saturating_sub(pins)).min(regular_total);
    let expected = if page == 1 {
        first
    } else {
        (regular_total - first)
            .saturating_sub((page - 2) * ROWS)
            .min(ROWS)
    };
    let pages = 1 + (regular_total - first).div_ceil(ROWS);
    if entries.len() as u32 - pins != expected || pages != last {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < last,
        total_pages: last,
    })
}
/// One attachment: the site's download script with the stored file under the
/// board's folder and the name to save it as.
fn attachment(entry: &ListingEntry, link: ElementRef<'_>) -> Result<ResourceInput, String> {
    let target =
        safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?).ok_or(PARSER_ERROR)?;
    let mut stored = None;
    let mut name = None;
    for (key, value) in target.query_pairs() {
        let slot = match key.as_ref() {
            "strRealFileName" => &mut stored,
            "strFileName" => &mut name,
            _ => return invalid(),
        };
        if slot.replace(value.into_owned()).is_some() {
            return invalid();
        }
    }
    let stored = stored.ok_or(PARSER_ERROR)?;
    if target.scheme() != "https"
        || target.host_str() != Some(HOST)
        || target.path() != FILE_PATH
        || target.fragment().is_some()
        || !stored_file(&stored)
        || name.is_none_or(|name| name.is_empty())
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
/// `/upload/BBS0004/<letters and digits>.<extension>`: the script reads this
/// path on the server, so nothing else is ever sent.
pub(crate) fn stored_file(value: &str) -> bool {
    value
        .strip_prefix(FILE_FOLDER)
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(stem, extension)| {
            !stem.is_empty()
                && stem.len() <= 64
                && stem.bytes().all(|b| b.is_ascii_alphanumeric())
                && (1..=5).contains(&extension.len())
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}
pub(super) fn article(entry: &ListingEntry, body: &str) -> Result<ArticleInput, String> {
    detail_url(entry)?;
    if missing(body) {
        return Err("학교 사이트에서 찾을 수 없는 공지입니다.".into());
    }
    let html = Html::parse_document(body);
    let root = html.root_element();
    // The page names no post (its form's `idx` is empty); the request URL is
    // the identity, and the title and layout bind it to this board.
    if text(one(root, "title")?) != "서강대학교 입학처 | 입학자료실 | 공지사항" {
        return invalid();
    }
    let view = one(root, "div.boardView")?;
    let title = text(one(view, "div.subject > p")?);
    let mut published_at = None;
    let mut resources = Vec::new();
    for field in view.select(selector("div.attr > dl")) {
        let label = text(one(field, "dt")?);
        match label.as_str() {
            "작성일" => {
                let day = date(&text(one(field, "dd")?).replace('.', "-"))?;
                if published_at.replace(day).is_some() {
                    return invalid();
                }
            }
            "첨부파일" => {
                for link in field.select(selector("dd > a")) {
                    resources.push(attachment(entry, link)?);
                }
            }
            _ => return invalid(),
        }
    }
    let published_at = published_at.ok_or(PARSER_ERROR)?;
    let content = one(view, "div.borderCont")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    merge_inline_resources(&mut resources, inline);
    if title.is_empty() || resources.len() > 200 || (body_text.is_empty() && resources.is_empty()) {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title,
        author: None,
        category: Some(NAME.into()),
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
#[path = "fixtures/admission_tests.rs"]
mod tests;
