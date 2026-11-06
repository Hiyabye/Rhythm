//! Parsers for the eClass (ilos LMS) pages the connector reads. The markup was
//! observed on 2026-11-06 with a Korean `Accept-Language`; the request flow
//! follows rycont/eclass-cli (MIT, permission from its author).
use crate::archive::model::ResourceInput;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;

pub const ORIGIN: &str = "https://eclass.sogang.ac.kr";
const PAGE_ERROR: &str = "사이버캠퍼스 페이지 형식이 예상과 다릅니다. 기존 보관 자료는 유지됩니다.";

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static CSS selector")
}
fn text(element: ElementRef<'_>) -> String {
    normalize(&element.text().collect::<Vec<_>>().join(" "))
}
fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn first_text(root: ElementRef<'_>, css: &str) -> Option<String> {
    root.select(&selector(css))
        .next()
        .map(text)
        .filter(|value| !value.is_empty())
}
/// eClass identifiers are short ASCII keys; anything else is not used in a
/// request or a document identity.
fn key(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then(|| value.to_owned())
}
/// The quoted arguments of a JavaScript call such as `goLecture('a','b','c')`.
fn call_arguments<'a>(value: &'a str, function: &str) -> Option<Vec<&'a str>> {
    let start = value.find(&format!("{function}("))? + function.len() + 1;
    let end = start + value[start..].find(')')?;
    Some(
        value[start..end]
            .split(',')
            .map(|argument| argument.trim().trim_matches(|c| c == '\'' || c == '"'))
            .collect(),
    )
}
/// A literal `name : "value"` field in an inline script object.
fn script_field<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = block;
    while let Some(index) = rest.find(name) {
        let after = rest[index + name.len()..].trim_start();
        if let Some(value) = after.strip_prefix(':') {
            let value = value.trim_start().strip_prefix('"')?;
            return value.find('"').map(|end| &value[..end]);
        }
        rest = &rest[index + name.len()..];
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub year: String,
    pub term: String,
}
/// `YearInfo[n] = "2026^3";` entries, newest first as the site lists them.
pub fn terms(body: &str) -> Vec<Term> {
    let mut result = Vec::new();
    for part in body.split("YearInfo[").skip(1) {
        let Some(value) = part
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(value, _)| value)
        else {
            continue;
        };
        if let Some((year, term)) = value.split_once('^') {
            if year.len() == 4
                && year.bytes().all(|b| b.is_ascii_digit())
                && !term.is_empty()
                && term.len() <= 2
                && term.bytes().all(|b| b.is_ascii_digit())
            {
                result.push(Term {
                    year: year.into(),
                    term: term.into(),
                });
            }
        }
    }
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Course {
    pub key: String,
    pub name: String,
}
pub fn courses(body: &str) -> Vec<Course> {
    let document = Html::parse_document(body);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for anchor in document.select(&selector("a[onclick*=\"eclassRoom(\"]")) {
        let onclick = anchor.value().attr("onclick").unwrap_or("");
        let Some(course_key) = call_arguments(onclick, "eclassRoom")
            .and_then(|arguments| arguments.first().and_then(|value| key(value)))
        else {
            continue;
        };
        let name = anchor
            .value()
            .attr("title")
            .map(|title| normalize(title.trim_end_matches("강의실 들어가기")))
            .filter(|name| !name.is_empty());
        if let Some(name) = name {
            if seen.insert(course_key.clone()) {
                result.push(Course {
                    key: course_key,
                    name,
                });
            }
        }
    }
    result
}

/// A course-entry response is JSON with `isError`; the server keeps the
/// entered course in the session for the following board requests.
pub fn entered(body: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|_| PAGE_ERROR.to_string())?;
    if value["isError"].as_bool() == Some(false) {
        Ok(())
    } else {
        Err(value["message"]
            .as_str()
            .map(|message| format!("강의실에 들어가지 못했습니다: {}", normalize(message)))
            .unwrap_or_else(|| "강의실에 들어가지 못했습니다.".into()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeRow {
    pub seq: String,
    pub title: String,
    /// The list's body preview. It changes when the notice is edited, so the
    /// connector refetches a notice only when the title or preview differs.
    pub preview: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeList {
    pub rows: Vec<NoticeRow>,
    pub total: u32,
}
pub fn notice_list(body: &str) -> Result<NoticeList, String> {
    let total = body
        .split_once(".total_num\").text('")
        .and_then(|(_, rest)| rest.split_once('\''))
        .and_then(|(value, _)| value.parse::<u32>().ok())
        .ok_or(PAGE_ERROR)?;
    let document = Html::parse_document(body);
    let mut rows = Vec::new();
    for button in document.select(&selector("button.board_list_wrap")) {
        let onclick = button.value().attr("onclick").unwrap_or("");
        let Some(seq) = call_arguments(onclick, "noticeViewPop")
            .and_then(|arguments| arguments.first().and_then(|value| key(value)))
        else {
            continue;
        };
        let Some(title) = first_text(button, ".font_subtitle1") else {
            continue;
        };
        rows.push(NoticeRow {
            seq,
            title,
            preview: first_text(button, ".board_text").unwrap_or_default(),
        });
    }
    if rows.is_empty() && total > 0 {
        return Err(PAGE_ERROR.into());
    }
    Ok(NoticeList { rows, total })
}

/// The values the page's own script posts to `efile_list.acl`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRequest {
    pub path: String,
    pub fields: Vec<(&'static str, String)>,
}
fn file_request(body: &str) -> Option<FileRequest> {
    let start = body.find("efile_list.acl\"")?;
    let path_start = body[..start].rfind('"')? + 1;
    let path = format!("{}efile_list.acl", &body[path_start..start]);
    let block_end = start + body[start..].find('}')?;
    let block = &body[start..block_end];
    let mut fields = Vec::new();
    for name in ["ud", "ky", "pf_st_flag", "CONTENT_SEQ"] {
        let value = script_field(block, name)?;
        if value.len() > 64 || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        fields.push((name, value.to_owned()));
    }
    ["/ilos/cls/st/co/efile_list.acl", "/ilos/co/efile_list.acl"]
        .contains(&path.as_str())
        .then_some(FileRequest { path, fields })
}

#[derive(Clone, Debug)]
pub struct NoticeView {
    pub title: String,
    pub author: Option<String>,
    pub posted: String,
    pub body_html: String,
    pub files: Option<FileRequest>,
}
fn table_value(root: ElementRef<'_>, label: &str) -> Option<String> {
    for row in root.select(&selector("tr")) {
        let Some(heading) = row.select(&selector("th")).next() else {
            continue;
        };
        if text(heading).contains(label) {
            return row.select(&selector("td")).next().map(text);
        }
    }
    None
}
pub fn notice_view(body: &str) -> Result<NoticeView, String> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let title = first_text(root, ".view_title .font_headline2").ok_or(PAGE_ERROR)?;
    let posted = table_value(root, "게시일")
        .filter(|value| !value.is_empty())
        .ok_or(PAGE_ERROR)?;
    let content = root
        .select(&selector("div.editor_content"))
        .next()
        .ok_or(PAGE_ERROR)?;
    Ok(NoticeView {
        title,
        author: table_value(root, "작성자").filter(|value| !value.is_empty()),
        posted,
        body_html: content.inner_html(),
        files: file_request(body),
    })
}

/// `efile_list.acl` returns an HTML fragment of download links.
pub fn attachments(body: &str) -> Vec<ResourceInput> {
    let fragment = Html::parse_fragment(body);
    let mut result: Vec<ResourceInput> = Vec::new();
    for link in fragment.select(&selector("a.file_down[href]")) {
        let Some(url) = download_url(link.value().attr("href").unwrap_or("")) else {
            continue;
        };
        let name = text(link);
        if name.is_empty() || result.iter().any(|resource| resource.url == url) {
            continue;
        }
        result.push(ResourceInput {
            url,
            filename: name,
            kind: "attachment".into(),
        });
    }
    result
}
/// Only eClass's own file-download route on the exact origin.
fn download_url(href: &str) -> Option<String> {
    let url = url::Url::parse(ORIGIN).ok()?.join(href).ok()?;
    (url.origin().ascii_serialization() == ORIGIN
        && url.path() == "/ilos/co/efile_download.acl"
        && url.query_pairs().any(|(name, _)| name == "FILE_SEQ"))
    .then(|| url.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentRow {
    pub seq: String,
    pub title: String,
    pub week: Option<u32>,
    /// `status_2` rows are open for submission; closed rows show `만료됨`.
    pub open: bool,
}
pub fn assignments(body: &str) -> Vec<AssignmentRow> {
    let document = Html::parse_document(body);
    let mut week = None;
    let mut result = Vec::new();
    // A group selector yields matches in document order, so each assignment
    // takes the week heading that precedes it.
    for element in document.select(&selector(
        "div.activity_week, a[id^=\"class_menu_report_\"]",
    )) {
        if element.value().name() == "div" {
            week = element
                .value()
                .attr("data-week")
                .and_then(|value| value.parse().ok());
            continue;
        }
        let Some(seq) = element
            .value()
            .attr("id")
            .and_then(|id| id.strip_prefix("class_menu_report_"))
            .and_then(key)
        else {
            continue;
        };
        let Some(title) = first_text(element, ".activity_title") else {
            continue;
        };
        result.push(AssignmentRow {
            seq,
            title,
            week,
            open: element
                .value()
                .has_class("status_2", scraper::CaseSensitivity::CaseSensitive),
        });
    }
    result
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssignmentDetail {
    pub title: String,
    pub opens_at: Option<String>,
    pub due_at: Option<String>,
    pub late_due_at: Option<String>,
    /// `정상제출`, `지각제출` or `미제출` as the page states it.
    pub status: Option<String>,
    pub submitted_at: Option<String>,
}
pub fn assignment(body: &str) -> Result<AssignmentDetail, String> {
    let document = Html::parse_document(body);
    let root = document.root_element();
    let title = first_text(root, ".font_headline2").ok_or(PAGE_ERROR)?;
    let due = table_value(root, "마감일").ok_or(PAGE_ERROR)?;
    let late = table_value(root, "지각제출").and_then(|value| {
        value
            .split_once("마감일")
            .and_then(|(_, rest)| full_datetime(rest.trim_start_matches([' ', ':'])))
    });
    let (status, submitted_at) = match root.select(&selector(".submit_info_box")).next() {
        Some(submitted) => (
            first_text(submitted, ".txt"),
            first_text(submitted, ".date").and_then(|value| full_datetime(&value)),
        ),
        None => (first_text(root, ".no_submit"), None),
    };
    Ok(AssignmentDetail {
        title,
        opens_at: table_value(root, "공개일").and_then(|value| full_datetime(&value)),
        due_at: full_datetime(&due),
        late_due_at: late,
        status,
        submitted_at,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoItem {
    pub course_key: String,
    pub category: String,
    pub item_id: String,
    /// The page's own label, such as `과제` or `퀴즈`.
    pub label: String,
    pub title: String,
    /// The deadline as shown, without a year (`11월 9일 (월) 23:59`).
    pub due: String,
}
pub fn todo(body: &str) -> Vec<TodoItem> {
    let fragment = Html::parse_fragment(body);
    let mut result = Vec::new();
    for item in fragment.select(&selector("a.todo_wrap.on[href]")) {
        let href = item.value().attr("href").unwrap_or("");
        let Some(arguments) = call_arguments(href, "goLecture") else {
            continue;
        };
        let (Some(course_key), Some(category), Some(item_id)) = (
            arguments.first().and_then(|v| key(v)),
            arguments.get(1).and_then(|v| key(v)),
            arguments.get(2).and_then(|v| key(v)),
        ) else {
            continue;
        };
        let Some(heading) = item.select(&selector(".todo_title")).next() else {
            continue;
        };
        let label = first_text(heading, "span")
            .map(|value| value.trim_matches(|c| c == '[' || c == ']').to_owned())
            .unwrap_or_default();
        let full = text(heading);
        let title = normalize(full.strip_prefix(&format!("[{label}]")).unwrap_or(&full));
        if title.is_empty() {
            continue;
        }
        result.push(TodoItem {
            course_key,
            category,
            item_id,
            label,
            title,
            due: first_text(item, "span.todo_date").unwrap_or_default(),
        });
    }
    result
}

/// `2026.10.25 (일) 23:59` as `2026-10-25T23:59` (Korean local time).
pub fn full_datetime(value: &str) -> Option<String> {
    let value = value.trim();
    let date = value.get(..10)?;
    let parts: Vec<&str> = date.split('.').collect();
    if parts.len() != 3 || parts[0].len() != 4 {
        return None;
    }
    let year = parts[0].parse::<i32>().ok()?;
    let month = parts[1].parse::<u32>().ok()?;
    let day = parts[2].parse::<u32>().ok()?;
    valid_date(year, month, day)?;
    let time = clock(value);
    Some(stamp(year, month, day, time))
}
/// A month/day value without a year, such as `10월 23일 (금) 23:42`. The year
/// is the one, of the course year and the next, whose weekday matches the
/// shown weekday; a spring-term notice can never fall in the next year, but a
/// fall-term deadline in January can.
pub fn month_day(value: &str, course_year: i32) -> Option<String> {
    if let Some(full) = full_datetime(value) {
        return Some(full);
    }
    let (month, rest) = value.trim().split_once('월')?;
    let (day, rest) = rest.trim().split_once('일')?;
    let month = month.trim().parse::<u32>().ok()?;
    let day = day.trim().parse::<u32>().ok()?;
    let weekday = rest
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.chars().next())
        .and_then(|day| "일월화수목금토".chars().position(|c| c == day));
    let year = [course_year, course_year + 1]
        .into_iter()
        .find(|year| {
            valid_date(*year, month, day).is_some()
                && weekday.is_some_and(|weekday| weekday_of(*year, month, day) == weekday)
        })
        .unwrap_or(course_year);
    valid_date(year, month, day)?;
    Some(stamp(year, month, day, clock(rest)))
}
fn clock(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.rsplit_once(':')?;
    let hour = hour
        .get(hour.len().saturating_sub(2)..)?
        .trim()
        .parse::<u32>()
        .ok()?;
    let minute = minute.get(..2)?.parse::<u32>().ok()?;
    (hour < 24 && minute < 60).then_some((hour, minute))
}
fn stamp(year: i32, month: u32, day: u32, time: Option<(u32, u32)>) -> String {
    match time {
        Some((hour, minute)) => format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}"),
        None => format!("{year:04}-{month:02}-{day:02}"),
    }
}
fn valid_date(year: i32, month: u32, day: u32) -> Option<()> {
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    ((2000..=2200).contains(&year) && day >= 1 && day <= max).then_some(())
}
/// 0 = Sunday (Sakamoto's method).
fn weekday_of(year: i32, month: u32, day: u32) -> usize {
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let year = if month < 3 { year - 1 } else { year };
    ((year + year / 4 - year / 100 + year / 400 + OFFSETS[month as usize - 1] + day as i32) % 7)
        as usize
}
