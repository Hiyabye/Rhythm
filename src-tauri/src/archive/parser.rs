use super::model::{ArticleInput, ListingEntry, ListingPage, ResourceInput, SourceDefinition};
use scraper::{ElementRef, Html};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use url::Url;

#[path = "community.rs"]
mod community;
#[path = "dormitory.rs"]
mod dormitory;
#[path = "engineering.rs"]
mod engineering;
#[path = "mechanical.rs"]
mod mechanical;
#[path = "semiconductor.rs"]
mod semiconductor;

const PARSER_ERROR: &str = "학교 페이지 형식이 예상과 다릅니다. 기존 보관 자료는 유지됩니다.";
fn invalid<T>() -> Result<T, String> {
    Err(PARSER_ERROR.into())
}
use super::selector;
fn text(element: ElementRef<'_>) -> String {
    normalize(&element.text().collect::<Vec<_>>().join(" "))
}
fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn one<'a>(root: ElementRef<'a>, css: &str) -> Result<ElementRef<'a>, String> {
    let found: Vec<_> = root.select(selector(css)).collect();
    if found.len() == 1 {
        Ok(found[0])
    } else {
        invalid()
    }
}
fn board(source: &SourceDefinition) -> Result<&str, String> {
    match source.id.as_str() {
        "computing-academic" => Ok("academicNotice"),
        "computing-graduate" => Ok("graduateNotice"),
        "computing-external" => Ok("externalInfo"),
        "computing-updates" => Ok("news"),
        "computing-career" => Ok("career"),
        _ => Err("아직 원문 수집이 지원되지 않는 정보원입니다.".into()),
    }
}
fn positive(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|v| v > 0)
}
fn id(value: &Value) -> Result<String, String> {
    let out = value
        .as_u64()
        .map(|v| v.to_string())
        .or_else(|| value.as_str().map(str::to_owned))
        .ok_or(PARSER_ERROR)?;
    if positive(&out) {
        Ok(out)
    } else {
        invalid()
    }
}
fn date(value: &str) -> Result<String, String> {
    if !value.is_ascii() {
        return invalid();
    }
    let day = if value.len() >= 10 && value.as_bytes().get(4) == Some(&b'-') {
        value[..10].to_string()
    } else if value.len() >= 8 && value.is_ascii() && value[..8].bytes().all(|b| b.is_ascii_digit())
    {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..8])
    } else {
        return invalid();
    };
    if day.len() != 10
        || day.as_bytes()[7] != b'-'
        || day
            .bytes()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7) && !byte.is_ascii_digit())
    {
        return invalid();
    }
    let parts: Vec<_> = day.split('-').map(|v| v.parse::<u32>()).collect();
    if parts.len() != 3 || parts.iter().any(Result::is_err) {
        return invalid();
    }
    let (year, month, day_number) = (
        parts[0].as_ref().unwrap(),
        parts[1].as_ref().unwrap(),
        parts[2].as_ref().unwrap(),
    );
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    if !(1900..=2200).contains(year) || *day_number == 0 || *day_number > max {
        return invalid();
    }
    Ok(day)
}
fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| PARSER_ERROR.into())
}
pub(super) fn main_canonical(external: &str) -> Url {
    let mut url = Url::parse(&format!("https://www.sogang.ac.kr/ko/detail/{external}")).unwrap();
    url.query_pairs_mut().extend_pairs([
        ("bbsConfigFk", "2"),
        ("namepage", "AcademicNotice"),
        ("text", "학사 지원"),
        ("data", "%5B%5D"),
        ("title", "학부 학사공지"),
        ("redirect", "/ko/academic-support/notices"),
    ]);
    url
}
pub fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    if page == 0 || !source.supported {
        return invalid();
    }
    if source.family == "community" {
        return community::list_url(source, page);
    }
    if source.family == "engineering" {
        return engineering::list_url(source, page);
    }
    if source.family == "dormitory" {
        return dormitory::list_url(source, page);
    }
    if source.family == "mechanical" {
        return mechanical::list_url(source, page);
    }
    if source.family == "semiconductor" {
        return semiconductor::list_url(source, page);
    }
    if source.id == "sogang-academic" {
        let mut url =
            Url::parse("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData/boardList").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("pageNum", page.to_string().as_str()),
            ("pageSize", "50"),
            ("bbsConfigFk", "2"),
            ("category", ""),
            ("introPkId", ""),
            ("title", ""),
            ("content", ""),
            ("username", ""),
        ]);
        Ok(url)
    } else {
        let board = board(source)?;
        let mut url = Url::parse(&format!(
            "https://computing.sogang.ac.kr/ko/community/{board}/list"
        ))
        .unwrap();
        url.query_pairs_mut().append_pair("num", &page.to_string());
        Ok(url)
    }
}
pub fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    if !source.supported || !positive(&entry.external_id) {
        return invalid();
    }
    if source.family == "community" {
        return community::detail_url(source, entry);
    }
    if source.family == "engineering" {
        return engineering::detail_url(source, entry);
    }
    if source.family == "dormitory" {
        return dormitory::detail_url(source, entry);
    }
    if source.family == "mechanical" {
        return mechanical::detail_url(source, entry);
    }
    if source.family == "semiconductor" {
        return semiconductor::detail_url(source, entry);
    }
    if source.id == "sogang-academic" {
        let mut url = Url::parse("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData").unwrap();
        url.query_pairs_mut()
            .append_pair("pkId", &entry.external_id);
        if entry.url != main_canonical(&entry.external_id).as_str() {
            return invalid();
        }
        Ok(url)
    } else {
        let expected = format!(
            "https://computing.sogang.ac.kr/ko/community/{}/detail/{}",
            board(source)?,
            entry.external_id
        );
        if entry.url != expected {
            return invalid();
        }
        Url::parse(&expected).map_err(|_| PARSER_ERROR.into())
    }
}
pub fn listing(source: &SourceDefinition, page: u32, body: &str) -> Result<ListingPage, String> {
    if body.len() > 4 * 1024 * 1024 || page == 0 || !source.supported {
        return invalid();
    }
    if source.family == "community" {
        community::listing(source, page, body)
    } else if source.family == "engineering" {
        engineering::listing(source, page, body)
    } else if source.family == "dormitory" {
        dormitory::listing(source, page, body)
    } else if source.family == "mechanical" {
        mechanical::listing(source, page, body)
    } else if source.family == "semiconductor" {
        semiconductor::listing(source, page, body)
    } else if source.id == "sogang-academic" {
        main_listing(page, body)
    } else {
        computing_listing(source, page, body)
    }
}
fn main_listing(page: u32, body: &str) -> Result<ListingPage, String> {
    let value: Value = serde_json::from_str(body).map_err(|_| PARSER_ERROR)?;
    if value["statusCode"].as_u64() != Some(200) {
        return invalid();
    }
    let data = &value["data"];
    let pages = data["pages"]
        .as_u64()
        .filter(|v| *v <= u32::MAX as u64)
        .ok_or(PARSER_ERROR)? as u32;
    let total = data["total"].as_u64().ok_or(PARSER_ERROR)?;
    let rows = data["list"].as_array().ok_or(PARSER_ERROR)?;
    if data["pageNum"].as_u64() != Some(page as u64)
        || data["pageSize"].as_u64() != Some(50)
        || rows.len() > 100
        || (total > 0 && u64::from(pages) != total.div_ceil(50))
        || (total > 0 && (pages == 0 || page > pages || rows.is_empty()))
        || (total == 0 && (!rows.is_empty() || pages > 1))
    {
        return invalid();
    }
    let has_next = data["hasNextPage"].as_bool().ok_or(PARSER_ERROR)?;
    if has_next != (page < pages) {
        return invalid();
    }
    let mut seen = HashMap::new();
    let mut entries = Vec::new();
    for row in rows {
        if row["configId"].as_u64() != Some(2) {
            return invalid();
        }
        let external_id = id(&row["pkId"])?;
        let title = normalize(required_string(row, "title")?);
        let published = date(required_string(row, "regDate")?)?;
        let pinned = match row["isTop"].as_str() {
            Some("Y") => true,
            Some("N") => false,
            _ => return invalid(),
        };
        let identity = (title, published, pinned);
        if let Some(previous) = seen.insert(external_id.clone(), identity.clone()) {
            if !pinned || previous != identity {
                return invalid();
            }
        } else {
            entries.push(ListingEntry {
                url: main_canonical(&external_id).into(),
                external_id,
                pinned,
                modified_at: None,
            });
        }
    }
    // Some list renderers repeat identical pins. Count logical rows, but reject
    // truncated payloads and conflicting duplicates rather than completing history.
    if entries.len() > 50
        || (total > 0 && data["size"].as_u64() != Some(entries.len() as u64))
        || (total == 0
            && data
                .get("size")
                .is_some_and(|size| size.as_u64() != Some(0)))
    {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next,
        total_pages: pages.max(1),
    })
}
fn computing_listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    let board = board(source)?;
    let html = Html::parse_document(body);
    let root = html.root_element();
    let content = one(root, ".board-content")?;
    if text(one(content, ".board-content-header h2")?) != source.name {
        return invalid();
    }
    let selected = one(root, ".board-lnb a[aria-current=page]")?;
    if selected.value().attr("href") != Some(&format!("/ko/community/{board}/list")) {
        return invalid();
    }
    let keyword = one(content, "#board-search-keyword")?;
    if keyword.value().attr("value").is_some_and(|v| !v.is_empty()) {
        return invalid();
    }
    if let Some(category) = content
        .select(selector(".board-category button[aria-pressed=true]"))
        .next()
    {
        if text(category) != "전체" {
            return invalid();
        }
    }
    let declared: u32 = text(one(content, ".board-toolbar strong")?)
        .replace(',', "")
        .parse()
        .map_err(|_| PARSER_ERROR)?;
    let table = one(content, ".board-list table")?;
    let rows: Vec<_> = table.select(selector("tbody tr")).collect();
    let pins = rows
        .iter()
        .filter(|r| r.select(selector(".board-list-notice")).next().is_some())
        .count() as u32;
    let regular = declared.checked_sub(pins).ok_or(PARSER_ERROR)?;
    let total_pages = regular.div_ceil(10).max(1);
    if page > total_pages {
        return invalid();
    }
    if declared == 0 {
        if !text(table).contains("등록된 게시물이 없습니다")
            || rows
                .iter()
                .any(|r| r.select(selector(".board-list-title a")).next().is_some())
        {
            return invalid();
        }
        return Ok(ListingPage {
            entries: vec![],
            has_next: false,
            total_pages: 1,
        });
    }
    let current = one(content, ".board-pagination [aria-current=page]")?;
    if text(current).parse::<u32>().ok() != Some(page) {
        return invalid();
    }
    for (label, disabled) in [
        ("첫 페이지", page == 1),
        ("이전 페이지", page == 1),
        ("다음 페이지", page == total_pages),
        ("마지막 페이지", page == total_pages),
    ] {
        let button = one(
            content,
            &format!(".board-pagination button[aria-label='{label}']"),
        )?;
        if button.value().attr("disabled").is_some() != disabled {
            return invalid();
        }
    }
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut ordinals = Vec::new();
    for row in rows {
        let anchor = one(row, ".board-list-title a")?;
        if text(anchor).is_empty() {
            return invalid();
        }
        date(&text(one(row, ".board-list-date")?))?;
        let url = Url::parse("https://computing.sogang.ac.kr")
            .unwrap()
            .join(anchor.value().attr("href").ok_or(PARSER_ERROR)?)
            .map_err(|_| PARSER_ERROR)?;
        let prefix = format!("/ko/community/{board}/detail/");
        let external_id = url
            .path()
            .strip_prefix(&prefix)
            .filter(|v| positive(v))
            .ok_or(PARSER_ERROR)?
            .to_string();
        if url.host_str() != Some("computing.sogang.ac.kr")
            || url.scheme() != "https"
            || url.fragment().is_some()
        {
            return invalid();
        }
        let pinned = row.select(selector(".board-list-notice")).next().is_some();
        if !pinned {
            ordinals.push(
                text(one(row, ".board-list-number")?)
                    .parse::<u32>()
                    .map_err(|_| PARSER_ERROR)?,
            );
        }
        if !seen.insert(external_id.clone()) {
            return invalid();
        }
        entries.push(ListingEntry {
            external_id: external_id.clone(),
            url: format!("https://computing.sogang.ac.kr{prefix}{external_id}"),
            pinned,
            modified_at: None,
        });
    }
    let start = regular.saturating_sub((page - 1) * 10);
    let expected: Vec<_> = (start.saturating_sub(9).max(1)..=start).rev().collect();
    if ordinals != expected {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}
pub fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    detail_url(source, entry)?;
    if body.len() > 4 * 1024 * 1024 {
        return invalid();
    }
    if source.family == "community" {
        return community::article(source, entry, body);
    }
    if source.family == "engineering" {
        return engineering::article(source, entry, body);
    }
    if source.family == "dormitory" {
        return dormitory::article(source, entry, body);
    }
    if source.family == "mechanical" {
        return mechanical::article(source, entry, body);
    }
    if source.family == "semiconductor" {
        return semiconductor::article(source, entry, body);
    }
    let (title, author, category, published_at, modified_at, content, mut resources) =
        if source.id == "sogang-academic" {
            let value: Value = serde_json::from_str(body).map_err(|_| PARSER_ERROR)?;
            let data = &value["data"];
            // The public API returns secret="Y" even for publicly linked 551637.
            // That field is not an authorization result. No authenticated endpoint
            // is used; denied or missing content fails without any workaround.
            if value["statusCode"].as_u64() != Some(200)
                || id(&data["pkId"])? != entry.external_id
                || data["configPkId"].as_u64() != Some(2)
                || data["del"].as_str() == Some("Y")
            {
                return invalid();
            }
            let mut resources = Vec::new();
            for n in 1..=5 {
                if let Some(value) = data[format!("fileValue{n}")]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                {
                    if let Some(url) = safe_url(&entry.url, value) {
                        resources.push(ResourceInput {
                            filename: filename(&url),
                            url: url.into(),
                            kind: "attachment".into(),
                        });
                    } else {
                        return invalid();
                    }
                }
            }
            let category = data["category"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    data["introTitleList"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .filter(|v| !v.is_empty())
                });
            (
                normalize(required_string(data, "title")?),
                data["displayName"]
                    .as_str()
                    .or_else(|| data["userName"].as_str())
                    .filter(|v| !v.trim().is_empty())
                    .map(normalize),
                category,
                date(required_string(data, "regDate")?)?,
                data["modDate"].as_str().map(date).transpose()?,
                required_string(data, "content")?.to_owned(),
                resources,
            )
        } else {
            let html = Html::parse_document(body);
            let root = html.root_element();
            let canonical = one(root, "link[rel=canonical]")?
                .value()
                .attr("href")
                .ok_or(PARSER_ERROR)?;
            let mut canonical = Url::parse(canonical).map_err(|_| PARSER_ERROR)?;
            canonical.set_query(None);
            if canonical.as_str() != entry.url {
                return invalid();
            }
            let article = one(root, "article.board-detail")?;
            let title = text(one(article, ".board-detail-header h3")?);
            if title.is_empty() {
                return invalid();
            }
            let mut metadata = HashMap::new();
            for div in article.select(selector(".board-detail-meta > div")) {
                let label = text(one(div, "dt")?);
                let value = text(one(div, "dd")?);
                metadata.insert(label, value);
            }
            let author = metadata.get("작성자").filter(|v| !v.is_empty()).cloned();
            let published = date(metadata.get("작성일").ok_or(PARSER_ERROR)?)?;
            let content = one(
                article,
                ".board-detail-reading-area .sg-content.tiptap-view",
            )?
            .inner_html();
            let resources = computing_files(root, entry, body)?;
            (
                title,
                author,
                metadata.get("종류").filter(|v| !v.is_empty()).cloned(),
                published,
                None,
                content,
                resources,
            )
        };
    let (mut body_html, mut body_text, inline) = sanitize(&content, &entry.url)?;
    merge_inline_resources(&mut resources, inline);
    let unsupported_iframe_body = source.id == "sogang-academic"
        && body_text.is_empty()
        && !resources.iter().any(|r| r.kind == "image")
        && iframe_only(&content);
    if unsupported_iframe_body {
        body_html.clear();
        body_text.clear();
        // A school PDF-viewer iframe shows a PDF as the whole body. The PDF is
        // kept as an attachment; once local, its text becomes the body
        // (`with_pdf_text`). Other iframes are never fetched.
        for pdf in viewer_pdfs(&content) {
            if !resources.iter().any(|r| r.url == pdf.url) {
                resources.push(pdf);
            }
        }
    }
    if resources.len() > 200
        || (body_text.is_empty()
            && !resources.iter().any(|r| r.kind == "image")
            && !unsupported_iframe_body)
    {
        return invalid();
    }
    Ok(ArticleInput {
        external_id: entry.external_id.clone(),
        canonical_url: entry.url.clone(),
        title,
        author,
        category,
        published_at,
        modified_at,
        original: body.into(),
        unsupported_body: unsupported_iframe_body,
        body_html,
        body_text,
        resources,
        etag: None,
        last_modified: None,
    })
}
// One URL needs only one download. When also referenced by <img>, preserve its
// image role so the reader hydrates it rather than treating it as attachment-only.
fn merge_inline_resources(resources: &mut Vec<ResourceInput>, inline: Vec<ResourceInput>) {
    for image in inline {
        if let Some(existing) = resources.iter_mut().find(|r| r.url == image.url) {
            existing.kind = "image".into();
        } else {
            resources.push(image);
        }
    }
    let mut seen = HashSet::new();
    resources.retain(|r| seen.insert(r.url.clone()));
}
fn filename(url: &Url) -> String {
    url.query_pairs()
        .find(|(key, _)| key == "sg")
        .map(|(_, v)| v.into_owned())
        .or_else(|| {
            url.path_segments()
                .and_then(Iterator::last)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "첨부 파일".into())
}
/// Document and raster extensions the archive downloads as attachments. The
/// download transport (`network.rs`) enforces the same list.
fn attachment_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".pdf", ".hwp", ".hwpx", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip", ".txt",
        ".png", ".jpg", ".jpeg", ".gif", ".webp",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}
fn safe_url(base: &str, value: &str) -> Option<Url> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(base).ok()?.join(value).ok()?;
    if !["https", "http"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    Some(url)
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
const PDF_VIEWER: &str = "https://scc.sogang.ac.kr/pdfviewer/web/viewer.html";
/// The PDFs that `scc.sogang.ac.kr/pdfviewer/web/viewer.html?file=<URL>` iframes
/// display, limited to file URLs the network policy approves.
fn viewer_pdfs(content: &str) -> Vec<ResourceInput> {
    let fragment = Html::parse_fragment(content);
    fragment
        .select(selector("iframe[src]"))
        .filter_map(|iframe| {
            let viewer = Url::parse(iframe.value().attr("src")?).ok()?;
            let mut page = viewer.clone();
            page.set_query(None);
            if page.as_str() != PDF_VIEWER || viewer.fragment().is_some() {
                return None;
            }
            let mut files = viewer.query_pairs().filter(|(key, _)| key == "file");
            let file = Url::parse(&files.next()?.1).ok()?;
            if files.next().is_some() || super::network::validate_url(&file).is_err() {
                return None;
            }
            let filename = file
                .query_pairs()
                .find(|(key, _)| key == "fileName")?
                .1
                .into_owned();
            Some(ResourceInput {
                url: file.into(),
                filename,
                kind: "attachment".into(),
            })
        })
        .collect()
}
/// The attachments of a metadata-only main-board notice that its PDF viewer
/// shows as the body.
pub fn viewer_pdf_resources(article: &ArticleInput) -> Vec<&ResourceInput> {
    if !article.unsupported_body {
        return Vec::new();
    }
    article
        .resources
        .iter()
        .filter(|r| {
            r.kind == "attachment"
                && Url::parse(&r.url).is_ok_and(|url| {
                    url.host_str() == Some("scc.sogang.ac.kr") && url.path() == "/Download3"
                })
        })
        .collect()
}
/// Makes the text of the viewer PDFs (filename, text) the body of a
/// metadata-only notice. Without any text it stays metadata-only.
pub fn with_pdf_text(article: &mut ArticleInput, pdfs: &[(String, String)]) {
    if !article.unsupported_body || pdfs.is_empty() {
        return;
    }
    let mut html =
        "<p><em>PDF 원문에서 추출한 텍스트입니다. 표와 서식은 첨부된 PDF에서 확인해 주세요.</em></p>"
            .to_string();
    let mut plain = Vec::new();
    for (filename, text) in pdfs {
        if pdfs.len() > 1 {
            html.push_str(&format!("<h3>{}</h3>", escape(filename)));
        }
        for line in text.lines() {
            html.push_str(&format!("<p>{}</p>", escape(line)));
        }
        plain.push(text.as_str());
    }
    article.body_html = html;
    article.body_text = normalize(&plain.join(" "));
    article.unsupported_body = false;
}
/// True when the body holds one or more iframes and nothing else: only blank
/// text and textless layout wrappers (as in `<div class="iframe-wrapper">`).
fn iframe_only(content: &str) -> bool {
    fn only_iframes(element: ElementRef<'_>, depth: usize, found: &mut bool) -> bool {
        depth <= MAX_DEPTH
            && element.children().all(|child| {
                if let Some(text) = child.value().as_text() {
                    text.trim().is_empty()
                } else if let Some(element) = ElementRef::wrap(child) {
                    match element.value().name() {
                        "iframe" => {
                            *found = true;
                            true
                        }
                        "p" | "div" | "span" | "br" => only_iframes(element, depth + 1, found),
                        _ => false,
                    }
                } else {
                    true
                }
            })
    }
    let fragment = Html::parse_fragment(content);
    let mut found = false;
    only_iframes(fragment.root_element(), 0, &mut found) && found
}

pub(super) fn sanitize(
    content: &str,
    base: &str,
) -> Result<(String, String, Vec<ResourceInput>), String> {
    let fragment = Html::parse_fragment(content);
    let mut html = String::new();
    let mut plain = String::new();
    let mut resources = Vec::new();
    let mut nodes = 0;
    render(
        fragment.root_element(),
        &mut html,
        &mut plain,
        &mut resources,
        base,
        0,
        &mut nodes,
    )?;
    Ok((html, normalize(&plain), resources))
}
const MAX_NODES: usize = 50000;
/// Legacy editors nest tables far deeper than a reader needs (130 levels in one
/// computing-career notice). Below this depth a subtree keeps its text and
/// images but not its structure, so recursion stays bounded for any markup.
const MAX_DEPTH: usize = 64;
const BLOCKED: [&str; 17] = [
    "script", "style", "iframe", "object", "embed", "svg", "math", "form", "input", "button",
    "textarea", "select", "nav", "footer", "header", "template", "noscript",
];
const INLINE: [&str; 11] = [
    "span", "strong", "b", "em", "i", "u", "s", "sub", "sup", "code", "a",
];
fn image(
    element: ElementRef<'_>,
    html: &mut String,
    plain: &mut String,
    resources: &mut Vec<ResourceInput>,
    base: &str,
) {
    if let Some(url) = element.value().attr("src").and_then(|v| safe_url(base, v)) {
        let alt = element.value().attr("alt").unwrap_or("");
        html.push_str(&format!(
            "<img data-resource-url=\"{}\" alt=\"{}\">",
            escape(url.as_str()),
            escape(alt)
        ));
        plain.push(' ');
        plain.push_str(alt);
        plain.push(' ');
        resources.push(ResourceInput {
            filename: filename(&url),
            url: url.into(),
            kind: "image".into(),
        });
    }
}
/// Renders a too-deep subtree iteratively as one block of text and images.
fn flatten(
    element: ElementRef<'_>,
    html: &mut String,
    plain: &mut String,
    resources: &mut Vec<ResourceInput>,
    base: &str,
    nodes: &mut usize,
) -> Result<(), String> {
    enum Item<'a> {
        Element(ElementRef<'a>),
        Text(&'a str),
        Break,
    }
    html.push_str("<div>");
    let mut stack = vec![Item::Element(element)];
    while let Some(item) = stack.pop() {
        match item {
            Item::Text(value) => {
                html.push_str(&escape(value));
                plain.push_str(value);
            }
            // Keep words from adjacent cells and paragraphs apart.
            Item::Break => {
                html.push(' ');
                plain.push(' ');
            }
            Item::Element(current) => {
                *nodes += 1;
                if *nodes > MAX_NODES {
                    return invalid();
                }
                let tag = current.value().name();
                if BLOCKED.contains(&tag) {
                    continue;
                }
                if tag == "img" {
                    image(current, html, plain, resources, base);
                    continue;
                }
                if !INLINE.contains(&tag) {
                    stack.push(Item::Break);
                }
                // Reverse so children are rendered in document order.
                let children = current.children().collect::<Vec<_>>();
                for child in children.into_iter().rev() {
                    if let Some(value) = child.value().as_text() {
                        stack.push(Item::Text(value));
                    } else if let Some(child) = ElementRef::wrap(child) {
                        stack.push(Item::Element(child));
                    }
                }
            }
        }
    }
    html.push_str("</div>");
    plain.push(' ');
    Ok(())
}
fn render(
    element: ElementRef<'_>,
    html: &mut String,
    plain: &mut String,
    resources: &mut Vec<ResourceInput>,
    base: &str,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return invalid();
    }
    if depth > MAX_DEPTH {
        return flatten(element, html, plain, resources, base, nodes);
    }
    let tag = element.value().name();
    if BLOCKED.contains(&tag) {
        return Ok(());
    }
    if tag == "img" {
        image(element, html, plain, resources, base);
        return Ok(());
    }
    let allowed = [
        "p",
        "div",
        "span",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "ul",
        "ol",
        "li",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "caption",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "s",
        "blockquote",
        "pre",
        "code",
        "br",
        "hr",
        "sub",
        "sup",
        "dl",
        "dt",
        "dd",
    ]
    .contains(&tag);
    let link = if tag == "a" {
        element.value().attr("href").and_then(|v| safe_url(base, v))
    } else {
        None
    };
    if let Some(url) = &link {
        html.push_str(&format!(
            "<a href=\"{}\" data-external=\"true\" rel=\"noreferrer noopener\">",
            escape(url.as_str())
        ));
    } else if allowed {
        html.push('<');
        html.push_str(tag);
        if ["td", "th"].contains(&tag) {
            for attr in ["rowspan", "colspan"] {
                if let Some(value) = element
                    .value()
                    .attr(attr)
                    .and_then(|v| v.parse::<u16>().ok())
                    .filter(|v| *v > 0 && *v <= 100)
                {
                    html.push_str(&format!(" {attr}=\"{value}\""));
                }
            }
        }
        html.push('>');
    }
    for child in element.children() {
        if let Some(value) = child.value().as_text() {
            html.push_str(&escape(value));
            plain.push_str(value);
        } else if let Some(child) = ElementRef::wrap(child) {
            render(child, html, plain, resources, base, depth + 1, nodes)?;
        }
    }
    if link.is_some() {
        html.push_str("</a>");
    } else if allowed && !["br", "hr"].contains(&tag) {
        html.push_str("</");
        html.push_str(tag);
        html.push('>');
    }
    if !INLINE.contains(&tag) {
        plain.push(' ');
    }
    Ok(())
}

/// Read only the serialized data subset used by Nuxt's devalue output. This is not a
/// JavaScript evaluator: scalar strings/numbers and identifier aliases are accepted,
/// expressions, calls, getters, and executable values are never evaluated.
fn computing_files(
    root: ElementRef<'_>,
    entry: &ListingEntry,
    body: &str,
) -> Result<Vec<ResourceInput>, String> {
    let names: Vec<_> = root
        .select(selector(".board-detail-files .board-detail-file-download"))
        .map(|a| {
            a.value()
                .attr("title")
                .map(str::to_owned)
                .unwrap_or_else(|| text(a))
        })
        .collect();
    if names.is_empty() {
        return Ok(vec![]);
    }
    let (data, aliases) = nuxt_data(body)?;
    let start = data.find("FILE_LIST:").ok_or(PARSER_ERROR)? + "FILE_LIST:".len();
    let array = data[start..].trim_start();
    if !array.starts_with('[') {
        return invalid();
    }
    let end = closing(array, 0, b'[', b']')?;
    // News uses a public route ID distinct from DETAIL.SN for some articles.
    // Keep the public identity, but bind file parents to the verified internal ID.
    let internal_id = if let Some(detail) = data.find("DETAIL:") {
        let object = data[detail + "DETAIL:".len()..].trim_start();
        if !object.starts_with('{') {
            return invalid();
        }
        let end = closing(object, 0, b'{', b'}')?;
        let fields = object_fields(&object[1..end])?;
        let internal = scalar(fields.get("SN").ok_or(PARSER_ERROR)?, &aliases)?;
        if !positive(&internal) {
            return invalid();
        }
        if internal != entry.external_id {
            let route = scalar_field(data, "routePath", &aliases)?;
            let url = Url::parse(&entry.url).map_err(|_| PARSER_ERROR)?;
            if !url.path().starts_with("/ko/community/news/detail/") || route != url.path() {
                return invalid();
            }
        }
        internal
    } else {
        entry.external_id.clone()
    };
    let mut resources = Vec::new();
    for object in split(&array[1..end], b',')? {
        let object = object.trim();
        if object.is_empty() {
            continue;
        }
        if !object.starts_with('{') || !object.ends_with('}') {
            return invalid();
        }
        let fields = object_fields(&object[1..object.len() - 1])?;
        let get = |key: &str| -> Result<String, String> {
            scalar(fields.get(key).ok_or(PARSER_ERROR)?, &aliases)
        };
        let path = get("FILE_PATH")?;
        let saved = get("SAVE_FILE_NAME")?;
        let name = get("FILE_NAME")?;
        let parent = get("PARENT_SEQ")?;
        if parent != internal_id
            || !names.contains(&name)
            || !path.starts_with("community/")
            || !path.ends_with(&format!("/{}/", entry.external_id))
            || path.split('/').any(|v| v == ".." || v == ".")
            || path.contains(['\\', ':', '?', '#'])
            || saved.contains(['/', '\\', ':', '?', '#'])
            || saved.is_empty()
        {
            return invalid();
        }
        let expected_board = Url::parse(&entry.url)
            .map_err(|_| PARSER_ERROR)?
            .path_segments()
            .and_then(|mut p| p.nth(2))
            .ok_or(PARSER_ERROR)?
            .to_string();
        if path != format!("community/{expected_board}/{}/", entry.external_id) {
            return invalid();
        }
        let url = Url::parse("https://computing.sogang.ac.kr/web/file/")
            .unwrap()
            .join(&format!("{path}{saved}"))
            .map_err(|_| PARSER_ERROR)?;
        resources.push(ResourceInput {
            url: url.into(),
            filename: name,
            kind: "attachment".into(),
        });
    }
    if resources.len() != names.len() {
        return invalid();
    }
    Ok(resources)
}
fn object_fields(value: &str) -> Result<HashMap<&str, &str>, String> {
    let mut fields = HashMap::new();
    for field in split(value, b',')? {
        let (key, value) = field.split_once(':').ok_or(PARSER_ERROR)?;
        if fields.insert(key.trim(), value.trim()).is_some() {
            return invalid();
        }
    }
    Ok(fields)
}
fn scalar_field(
    data: &str,
    key: &str,
    aliases: &HashMap<String, String>,
) -> Result<String, String> {
    let marker = format!("{key}:");
    let start = data.find(&marker).ok_or(PARSER_ERROR)? + marker.len();
    let rest = data[start..].trim_start();
    let end = if rest.starts_with('"') {
        let mut escaped = false;
        let mut end = None;
        for (index, byte) in rest.bytes().enumerate().skip(1) {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                end = Some(index + 1);
                break;
            }
        }
        end.ok_or(PARSER_ERROR)?
    } else {
        rest.bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
            .count()
    };
    scalar(&rest[..end], aliases)
}
fn nuxt_data(body: &str) -> Result<(&str, HashMap<String, String>), String> {
    let marker = "window.__NUXT__=(function(";
    let start = body.find(marker).ok_or(PARSER_ERROR)? + marker.len();
    let rest = &body[start..];
    let params_end = rest.find(')').ok_or(PARSER_ERROR)?;
    let params = split(&rest[..params_end], b',')?;
    let body_start = rest[params_end + 1..].find('{').ok_or(PARSER_ERROR)? + params_end + 1;
    let body_end = closing(rest, body_start, b'{', b'}')?;
    let invocation = rest[body_end + 1..].trim_start();
    if !invocation.starts_with('(') {
        return invalid();
    }
    let args_end = closing(invocation, 0, b'(', b')')?;
    let args = split(&invocation[1..args_end], b',')?;
    if args.len() != params.len() || args.len() > 2000 {
        return invalid();
    }
    let aliases = params
        .into_iter()
        .zip(args)
        .map(|(key, value)| (key.trim().into(), value.trim().into()))
        .collect();
    Ok((&rest[body_start + 1..body_end], aliases))
}
fn scalar(value: &str, aliases: &HashMap<String, String>) -> Result<String, String> {
    let value = aliases.get(value).map(String::as_str).unwrap_or(value);
    if value.starts_with('"') {
        serde_json::from_str::<String>(value).map_err(|_| PARSER_ERROR.into())
    } else if value.bytes().all(|b| b.is_ascii_digit()) && !value.is_empty() {
        Ok(value.into())
    } else {
        invalid()
    }
}
fn closing(value: &str, start: usize, open: u8, close: u8) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, &byte) in value.as_bytes().iter().enumerate().skip(start) {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        if byte == b'"' {
            quoted = true;
        } else if byte == open {
            depth += 1;
        } else if byte == close {
            depth = depth.checked_sub(1).ok_or(PARSER_ERROR)?;
            if depth == 0 {
                return Ok(index);
            }
        }
    }
    invalid()
}
fn split(value: &str, delimiter: u8) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut stack = Vec::new();
    for (index, &byte) in value.as_bytes().iter().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'[' | b'{' | b'(' => stack.push(byte),
            b']' | b'}' | b')' => {
                let expected = match byte {
                    b']' => b'[',
                    b'}' => b'{',
                    _ => b'(',
                };
                if stack.pop() != Some(expected) {
                    return invalid();
                }
            }
            _ if byte == delimiter && stack.is_empty() => {
                out.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quoted || !stack.is_empty() {
        return invalid();
    }
    out.push(&value[start..]);
    Ok(out)
}

#[cfg(test)]
#[path = "fixtures/parser_tests.rs"]
mod tests;
