//! Current Engineering SSR pages. Stable catalog IDs are independent of routes.
//! This parser uses public HTML, not the retired CMS interface or executable data.
use super::*;

fn board(source: &SourceDefinition) -> Result<&'static str, String> {
    match source.id.as_str() {
        "eng-general" => Ok("notice"),
        "eng-academic" => Ok("academic"),
        "eng-research" => Ok("research"),
        "eng-careers" => Ok("recruit"),
        _ => invalid(),
    }
}
fn base(source: &SourceDefinition) -> Result<Url, String> {
    Url::parse(&format!(
        "https://eng.sogang.ac.kr/board/{}",
        board(source)?
    ))
    .map_err(|_| PARSER_ERROR.into())
}
pub(super) fn list_url(source: &SourceDefinition, page: u32) -> Result<Url, String> {
    let mut url = base(source)?;
    url.query_pairs_mut().append_pair("page", &page.to_string());
    Ok(url)
}
pub(super) fn detail_url(source: &SourceDefinition, entry: &ListingEntry) -> Result<Url, String> {
    let expected = format!("{}/{}", base(source)?, entry.external_id);
    if entry.url != expected {
        return invalid();
    }
    Url::parse(&expected).map_err(|_| PARSER_ERROR.into())
}
fn canonical(root: ElementRef<'_>) -> Result<Url, String> {
    Url::parse(
        one(root, "link[rel=canonical]")?
            .value()
            .attr("href")
            .ok_or(PARSER_ERROR)?,
    )
    .map_err(|_| PARSER_ERROR.into())
}
pub(super) fn listing(
    source: &SourceDefinition,
    page: u32,
    body: &str,
) -> Result<ListingPage, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    let canonical = canonical(root)?;
    if canonical != list_url(source, page)? && !(page == 1 && canonical == base(source)?) {
        return invalid();
    }
    let list = one(root, ".board--list")?;
    let form = one(list, ".board-search")?;
    let expected_path = format!("/board/{}", board(source)?);
    if form.value().attr("action") != Some(expected_path.as_str())
        || one(form, "input[name=q]")?
            .value()
            .attr("value")
            .is_some_and(|v| !v.is_empty())
    {
        return invalid();
    }
    let total: u32 = text(one(list, ".board-total b")?)
        .replace(',', "")
        .parse()
        .map_err(|_| PARSER_ERROR)?;
    let rows: Vec<_> = list
        .select(&selector(".blist > .brow:not(.brow-head)"))
        .collect();
    let pins = rows
        .iter()
        .filter(|r| r.value().classes().any(|c| c == "is-notice"))
        .count() as u32;
    // The total includes pins, but this SSR family emits them only on page one.
    // Subsequent pages retain regular-row ordinals, which prove their offset.
    let regular = if page == 1 {
        total.checked_sub(pins).ok_or(PARSER_ERROR)?
    } else {
        if pins != 0 {
            return invalid();
        }
        let first = rows.first().ok_or(PARSER_ERROR)?;
        let ordinal = text(one(*first, ".c-no")?)
            .replace(',', "")
            .parse::<u32>()
            .map_err(|_| PARSER_ERROR)?;
        let regular = ordinal
            .checked_add((page - 1).checked_mul(10).ok_or(PARSER_ERROR)?)
            .ok_or(PARSER_ERROR)?;
        if regular > total || total - regular > 100 {
            return invalid();
        }
        regular
    };
    let total_pages = regular.div_ceil(10).max(1);
    if page > total_pages || rows.len() > 100 || (total > 0 && rows.is_empty()) {
        return invalid();
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut ordinals = Vec::new();
    let prefix = format!("{expected_path}/");
    for row in rows {
        let link = one(row, "a.c-title")?;
        let href = link.value().attr("href").ok_or(PARSER_ERROR)?;
        let external = href
            .strip_prefix(&prefix)
            .filter(|v| positive(v))
            .ok_or(PARSER_ERROR)?;
        if !seen.insert(external.to_string()) || text(one(link, "span")?).is_empty() {
            return invalid();
        }
        date(&text(one(row, ".c-date")?).replace('.', "-"))?;
        let pinned = row.value().classes().any(|c| c == "is-notice");
        if pinned {
            if text(one(row, ".c-no")?) != "공지" {
                return invalid();
            }
        } else {
            ordinals.push(
                text(one(row, ".c-no")?)
                    .replace(',', "")
                    .parse::<u32>()
                    .map_err(|_| PARSER_ERROR)?,
            );
        }
        entries.push(ListingEntry {
            external_id: external.into(),
            url: format!("https://eng.sogang.ac.kr{href}"),
            pinned,
        });
    }
    let start = regular.saturating_sub((page - 1) * 10);
    let expected: Vec<_> = (start.saturating_sub(9).max(1)..=start).rev().collect();
    if ordinals != expected {
        return invalid();
    }
    let pagers: Vec<_> = list.select(&selector(".pager")).collect();
    if total_pages > 1 {
        if pagers.len() != 1 || text(one(pagers[0], "[aria-current=page]")?) != page.to_string() {
            return invalid();
        }
        let mut links = HashSet::new();
        for link in pagers[0].select(&selector("a[href]")) {
            let url = safe_url(
                base(source)?.as_str(),
                link.value().attr("href").ok_or(PARSER_ERROR)?,
            )
            .ok_or(PARSER_ERROR)?;
            if url.origin() != base(source)?.origin() || url.path() != expected_path {
                return invalid();
            }
            let query: Vec<_> = url.query_pairs().collect();
            if query.len() != 1 || query[0].0 != "page" {
                return invalid();
            }
            let target = query[0].1.parse::<u32>().map_err(|_| PARSER_ERROR)?;
            if target == 0 || target > total_pages {
                return invalid();
            }
            links.insert(target);
        }
        if let Some(last) = pagers[0].select(&selector("a[aria-label=last]")).next() {
            let url = safe_url(
                base(source)?.as_str(),
                last.value().attr("href").ok_or(PARSER_ERROR)?,
            )
            .ok_or(PARSER_ERROR)?;
            if url
                .query_pairs()
                .find(|(k, _)| k == "page")
                .map(|(_, v)| v.into_owned())
                != Some(total_pages.to_string())
            {
                return invalid();
            }
        }
        if page < total_pages && !links.contains(&(page + 1)) {
            return invalid();
        }
    } else if !pagers.is_empty() {
        return invalid();
    }
    if total == 0 && !text(list).contains("등록된 게시물이 없습니다") {
        return invalid();
    }
    Ok(ListingPage {
        entries,
        has_next: page < total_pages,
        total_pages,
    })
}

/// Cross-check machine UTC against displayed Korea time, including midnight and
/// year boundaries. Store the Korean calendar date, as for the existing boards.
fn published(time: ElementRef<'_>) -> Result<String, String> {
    let utc = time.value().attr("datetime").ok_or(PARSER_ERROR)?;
    if !utc.is_ascii()
        || utc.len() != 24
        || &utc[10..11] != "T"
        || &utc[13..14] != ":"
        || &utc[16..17] != ":"
        || &utc[19..20] != "."
        || &utc[23..] != "Z"
        || [11..13, 14..16, 17..19, 20..23]
            .iter()
            .any(|range| !utc[range.clone()].bytes().all(|b| b.is_ascii_digit()))
    {
        return invalid();
    }
    let mut day = date(utc)?;
    let hour = utc[11..13].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let minute = utc[14..16].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    let second = utc[17..19].parse::<u32>().map_err(|_| PARSER_ERROR)?;
    if hour > 23 || minute > 59 || second > 59 {
        return invalid();
    }
    if hour + 9 >= 24 {
        let mut year = day[..4].parse::<u32>().map_err(|_| PARSER_ERROR)?;
        let mut month = day[5..7].parse::<u32>().map_err(|_| PARSER_ERROR)?;
        let next = day[8..].parse::<u32>().map_err(|_| PARSER_ERROR)? + 1;
        day = format!("{year:04}-{month:02}-{next:02}");
        if date(&day).is_err() {
            month += 1;
            if month == 13 {
                year += 1;
                month = 1;
            }
            day = date(&format!("{year:04}-{month:02}-01"))?;
        }
    }
    let visible = format!(
        "{} {:02}:{minute:02}",
        day.replace('-', "."),
        (hour + 9) % 24
    );
    if text(time) != visible {
        return invalid();
    }
    Ok(day)
}
pub(super) fn article(
    source: &SourceDefinition,
    entry: &ListingEntry,
    body: &str,
) -> Result<ArticleInput, String> {
    let html = Html::parse_document(body);
    let root = html.root_element();
    if canonical(root)? != detail_url(source, entry)? {
        return invalid();
    }
    let post = one(root, "article.post")?;
    let title = text(one(post, ".post-title")?);
    let published_at = published(one(post, ".post-meta time")?)?;
    let mut author = None;
    for row in post.select(&selector(".post-meta li")) {
        let label = one(row, ".sr-only")?;
        if text(label) == "작성자" {
            let value = normalize(
                &row.children()
                    .filter_map(|n| n.value().as_text().map(|v| v.to_string()))
                    .collect::<String>(),
            );
            if value.is_empty() || author.replace(value).is_some() {
                return invalid();
            }
        }
    }
    if title.is_empty() || author.is_none() {
        return invalid();
    }
    let content = one(post, ".post-body")?.inner_html();
    let (body_html, body_text, inline) = sanitize(&content, &entry.url)?;
    let mut resources = Vec::new();
    let files: Vec<_> = post.select(&selector(".post-files")).collect();
    if !files.is_empty() {
        if files.len() != 1 {
            return invalid();
        }
        let count = text(one(files[0], ".post-files-t b")?)
            .parse::<usize>()
            .map_err(|_| PARSER_ERROR)?;
        for link in files[0].select(&selector("a[download]")) {
            let url = safe_url(&entry.url, link.value().attr("href").ok_or(PARSER_ERROR)?)
                .ok_or(PARSER_ERROR)?;
            let external = url
                .path()
                .strip_prefix("/download/")
                .filter(|v| positive(v))
                .ok_or(PARSER_ERROR)?;
            if url.as_str() != format!("https://eng.sogang.ac.kr/download/{external}") {
                return invalid();
            }
            let name = text(one(link, "span")?);
            if name.is_empty() {
                return invalid();
            }
            resources.push(ResourceInput {
                url: url.into(),
                filename: name,
                kind: "attachment".into(),
            });
        }
        if resources.len() != count {
            return invalid();
        }
    }
    merge_inline_resources(&mut resources, inline);
    if resources.len() > 200
        || (body_text.is_empty() && !resources.iter().any(|r| r.kind == "image"))
    {
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
#[path = "fixtures/engineering_tests.rs"]
mod tests;
