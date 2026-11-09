//! Pure parsing of the library's bounded legacy HTML. No HTML is executed or
//! returned to the WebView. Tests build synthetic pages, not school captures.
use super::{model::SeatData, rooms};
use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Utc};
use scraper::{ElementRef, Html, Selector};
use std::{collections::BTreeMap, sync::LazyLock};
use url::Url;

pub const MAX_PAGE_BYTES: usize = 1_000_000;
const INVALID_COUNTS: &str = "전체·사용·빈 좌석 수가 일치하지 않습니다.";

static ROW: LazyLock<Selector> = LazyLock::new(|| Selector::parse("tr").unwrap());
static LINK: LazyLock<Selector> = LazyLock::new(|| Selector::parse("a[href]").unwrap());
static LAYERS: LazyLock<Selector> = LazyLock::new(|| Selector::parse("div[id]").unwrap());
static CELLS: LazyLock<Selector> = LazyLock::new(|| Selector::parse("td").unwrap());

pub fn decode_page(raw: &[u8]) -> Result<String, String> {
    if raw.len() > MAX_PAGE_BYTES {
        return Err("예상보다 큰 페이지입니다.".into());
    }
    if let Ok(value) = std::str::from_utf8(raw) {
        return Ok(value.trim_start_matches('\u{feff}').into());
    }
    // WHATWG's Korean decoder covers the Windows CP949 extension. Use the
    // non-replacing API: corrupt bytes must never turn into plausible counts.
    encoding_rs::EUC_KR
        .decode_without_bom_handling_and_without_replacement(raw)
        .map(|value| value.into_owned())
        .ok_or_else(|| "페이지 문자 인코딩을 해석할 수 없습니다.".into())
}

fn normalized(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn cells(row: ElementRef<'_>) -> Vec<String> {
    row.children()
        .filter_map(ElementRef::wrap)
        .filter(|cell| matches!(cell.value().name(), "td" | "th"))
        .map(normalized)
        .collect()
}

fn number(value: &str) -> Result<u32, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("좌석 수가 숫자가 아닙니다.".into());
    }
    value
        .parse()
        .map_err(|_| "좌석 수가 올바르지 않습니다.".into())
}

fn total(value: &str) -> Result<u32, String> {
    if let Some((count, disabled)) = value.split_once('(') {
        let disabled = disabled
            .strip_suffix(')')
            .ok_or("전체 좌석 수 형식이 달라졌습니다.")?;
        number(disabled)?;
        number(count.trim_end())
    } else {
        number(value)
    }
}

fn validate_counts(total: u32, used: u32, remaining: u32, capacity: u32) -> Result<(), String> {
    if total != capacity || used.checked_add(remaining) != Some(total) {
        return Err(INVALID_COUNTS.into());
    }
    Ok(())
}

fn date_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

fn parse_time(text: &str, now: i64) -> Result<String, String> {
    let words: Vec<_> = text.split_whitespace().collect();
    for chunk in words.windows(4) {
        if !date_shape(chunk[0]) || !matches!(chunk[1], "오전" | "오후") || chunk[3] != "현재"
        {
            continue;
        }
        let parts: Vec<_> = chunk[2].split(':').collect();
        if parts.len() != 3
            || !(1..=2).contains(&parts[0].len())
            || parts[1].len() != 2
            || parts[2].len() != 2
        {
            return Err("페이지 시각이 잘못되었습니다.".into());
        }
        let hour = number(parts[0])?;
        let minute = number(parts[1])?;
        let second = number(parts[2])?;
        if !(1..=12).contains(&hour) {
            return Err("페이지 시각이 잘못되었습니다.".into());
        }
        let hour = hour % 12 + if chunk[1] == "오후" { 12 } else { 0 };
        let date: Vec<_> = chunk[0].split('-').collect();
        let year: i32 = date[0]
            .parse()
            .map_err(|_| "페이지 시각이 잘못되었습니다.")?;
        // Python's datetime (the reference parser) has no astronomical year 0.
        if year < 1 {
            return Err("페이지 시각이 잘못되었습니다.".into());
        }
        let month = number(date[1])?;
        let day = number(date[2])?;
        let time = FixedOffset::east_opt(9 * 3600)
            .unwrap()
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .ok_or("페이지 시각이 잘못되었습니다.")?;
        let age = now.saturating_sub(time.timestamp());
        if !(-60..=300).contains(&age) {
            return Err("페이지 시각이 오래되었거나 PC 시계와 맞지 않습니다.".into());
        }
        return Ok(time
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Secs, true));
    }
    Err("페이지 기준 시각을 찾지 못했습니다.".into())
}

pub fn timestamp_seconds(value: &str) -> Result<i64, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp())
        .map_err(|_| "페이지 기준 시각을 해석할 수 없습니다.".into())
}

pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn document(html: &str) -> Result<Html, String> {
    if html.len() > MAX_PAGE_BYTES * 3 || html.contains('\0') {
        return Err("페이지 형식을 확인할 수 없습니다.".into());
    }
    Ok(Html::parse_document(html))
}

pub fn parse_status(
    html: &str,
    now: i64,
) -> Result<BTreeMap<String, Result<SeatData, String>>, String> {
    let page = document(html)?;
    let rows: Vec<_> = page.select(&ROW).map(|row| (row, cells(row))).collect();
    if !rows.iter().any(|(_, cells)| {
        cells.len() >= 5 && cells[2..5] == ["전체 좌석수", "사용 좌석수", "잔여 좌석수"]
    }) {
        return Err("현황 표의 열 구성이 달라졌습니다.".into());
    }
    let source_time = parse_time(
        &page.root_element().text().collect::<Vec<_>>().join(" "),
        now,
    )?;
    let base = Url::parse(rooms::STATUS_URL).expect("fixed library URL");
    let mut results = BTreeMap::new();
    for room in rooms::all() {
        let matches: Vec<_> = rows
            .iter()
            .filter(|(row, _)| {
                row.select(&LINK).any(|link| {
                    base.join(link.value().attr("href").unwrap_or_default())
                        .ok()
                        .is_some_and(|url| {
                            let ids: Vec<_> = url
                                .query_pairs()
                                .filter(|(key, _)| key == "room_no")
                                .map(|(_, value)| value)
                                .collect();
                            url.path().ends_with("roomview5.asp")
                                && ids.len() == 1
                                && ids[0] == room.id
                        })
                })
            })
            .collect();
        let result = (|| {
            if matches.len() != 1 {
                return Err(format!("{} 행을 유일하게 식별하지 못했습니다.", room.label));
            }
            let cells = &matches[0].1;
            if cells.len() < 5 {
                return Err(format!("{} 행이 불완전합니다.", room.label));
            }
            let total = total(&cells[2])?;
            let used = number(&cells[3])?;
            let remaining = number(&cells[4])?;
            validate_counts(total, used, remaining, room.capacity)?;
            if cells[1] != room.name {
                return Err("열람실 이름이 일치하지 않습니다.".into());
            }
            Ok(SeatData {
                total,
                used,
                remaining,
                source_time: source_time.clone(),
                seat_numbers: None,
                source_url: rooms::STATUS_URL.into(),
            })
        })();
        results.insert(room.id.into(), result);
    }
    Ok(results)
}

pub fn parse_seats(html: &str, room_id: &str, now: i64) -> Result<SeatData, String> {
    let room = rooms::get(room_id)?;
    let page = document(html)?;
    let summaries: Vec<_> = page
        .select(&ROW)
        .map(cells)
        .filter(|cells| cells.first().is_some_and(|name| name == room.name))
        .collect();
    if summaries.len() != 1 || summaries[0].len() != 5 {
        return Err(format!(
            "{} 좌석 배치 요약을 확인할 수 없습니다.",
            room.label
        ));
    }
    let summary = &summaries[0];
    let mut counts = Vec::with_capacity(3);
    for (cell, label) in summary[1..4]
        .iter()
        .zip(["총 좌석수", "사용 좌석수", "잔여 좌석수"])
    {
        let (actual_label, value) = cell
            .split_once(':')
            .ok_or("좌석 배치 요약 형식이 달라졌습니다.")?;
        if actual_label.trim() != label {
            return Err("좌석 배치 요약 형식이 달라졌습니다.".into());
        }
        counts.push(number(value.trim())?);
    }
    let (total, used, remaining) = (counts[0], counts[1], counts[2]);
    validate_counts(total, used, remaining, room.capacity)?;
    let source_time = parse_time(&summary[4], now)?;
    let seat_elements: Vec<_> = page
        .select(&LAYERS)
        .filter(|element| {
            element
                .value()
                .attr("id")
                .unwrap_or_default()
                .starts_with("Layer")
        })
        .collect();
    let mut seats = BTreeMap::new();
    for element in seat_elements {
        let identity = element
            .value()
            .attr("id")
            .unwrap()
            .strip_prefix("Layer")
            .unwrap();
        let id = number(identity).map_err(|_| "좌석 번호 요소가 올바르지 않습니다.")?;
        if id == 0 || id > total || seats.contains_key(&id) {
            return Err("좌석 번호가 누락되었거나 중복되었습니다.".into());
        }
        let colors: Vec<_> = element
            .select(&CELLS)
            .map(|cell| {
                cell.value()
                    .attr("bgcolor")
                    .unwrap_or_default()
                    .to_ascii_lowercase()
            })
            .collect();
        if normalized(element) != id.to_string()
            || colors.len() != 1
            || !matches!(colors[0].as_str(), "red" | "gray")
        {
            return Err("좌석 번호 또는 상태 색상을 확인할 수 없습니다.".into());
        }
        seats.insert(id, colors[0] == "gray");
    }
    if seats.len() != total as usize {
        return Err("좌석 번호가 누락되었거나 중복되었습니다.".into());
    }
    let legend: Vec<_> = page
        .select(&CELLS)
        .filter(|cell| {
            !cell
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(|ancestor| {
                    ancestor
                        .value()
                        .attr("id")
                        .is_some_and(|id| id.starts_with("Layer"))
                })
        })
        .map(|cell| {
            (
                cell.value()
                    .attr("bgcolor")
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                normalized(cell),
            )
        })
        .collect();
    if !legend
        .iter()
        .any(|(color, label)| color == "red" && label == "사용중")
        || !legend
            .iter()
            .any(|(color, label)| color == "gray" && label == "미사용")
    {
        return Err("좌석 상태 범례가 달라졌습니다.".into());
    }
    let available: Vec<_> = seats
        .into_iter()
        .filter_map(|(id, available)| available.then_some(id))
        .collect();
    if available.len() != remaining as usize {
        return Err("빈 좌석 번호와 빈 좌석 수가 일치하지 않습니다.".into());
    }
    Ok(SeatData {
        total,
        used,
        remaining,
        source_time,
        seat_numbers: Some(available),
        source_url: room.url(),
    })
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
