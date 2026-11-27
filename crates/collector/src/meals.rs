//! The Bellarmine dormitory menu (#318, user decisions 2026-11-25). The menu
//! board posts each week as one image, which the collector workflow reads in
//! three steps (crates/collector/AGENTS.md "Dormitory menu"):
//!
//! 1. `offer` (in the crawl run): for each current week, the image of the
//!    newest post is fetched under the crawl rules and written with a job
//!    file into a folder, and recorded as `pending` in the state.
//! 2. `meal-ocr/meal_ocr.py` reads the folder without secrets and writes one
//!    result per image.
//! 3. `apply` (in a `--meal-results` run): every pending image's result is
//!    validated here again and recorded as an `ok` or `unreadable` week.
//!
//! The folder is shared with third-party OCR code, so nothing in it but the
//! results is read back: what an image is (post, week, hash) comes from the
//! state.
use crate::{crawl, sha256_hex, state::Offered, state::State};
use rhythm_feed::record::{Meal, MealCell, MealDay, MealStatus, MealWeek};
use rhythm_sources::{
    robots::Robots,
    transport::{self as network, Access, Fetch, Transport},
};
use serde::Deserialize;
use std::{path::Path, sync::atomic::AtomicBool};
use url::Url;

/// The menu board (`주간 메뉴` of 벨라르미노 학사).
pub const SOURCE_ID: &str = "dorm-menu";
/// Must equal `VERSION` in `meal-ocr/meal_ocr.py`. A result of another version
/// is never read; an image read by an older version is offered again.
pub const OCR_VERSION: &str = "meal-ocr-1";
/// The OCR step's own limits on an image.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 256 * 1024;
/// Weeks are offered from the one that holds today to this many days ahead.
const AHEAD_DAYS: i64 = 28;
/// A title's week lies this close to the post's publication.
const NEAR_PUBLICATION_DAYS: i64 = 45;
/// Narrower menu images (the board's inline PNGs, about 1,088 px) are read
/// with many more wrong letters than its 3,508 px JPEGs (#318, 2026-11-25).
const LOW_RESOLUTION_WIDTH: u32 = 2000;
const MAX_PIXELS: u64 = 20_000_000;
const MAX_ITEMS: usize = 20;
const MAX_ITEM_CHARS: usize = 150;
const WEEKDAYS: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];

/// What `offer` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Offers {
    /// Images written for the OCR step.
    pub offered: usize,
    /// Current weeks whose newest image already has a result.
    pub read: usize,
    /// Current posts without a usable week title or with other than one image.
    pub skipped: usize,
    pub failed: Vec<String>,
}

/// What `apply` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub ok: usize,
    pub unreadable: usize,
    /// Results that could not be read at all: the image stays pending.
    pub invalid: Vec<String>,
    /// Pending images without a result (the OCR step failed on them).
    pub missing: usize,
}

/// Step 1: writes the image of the newest post of each current week that has
/// no result yet into `folder`, with its job, and records it as pending.
/// `today` is the date in Korea.
pub async fn offer<T: Transport>(
    state: &State,
    transport: &T,
    folder: &Path,
    today: &str,
    now: &str,
    cancel: &AtomicBool,
) -> Result<Offers, String> {
    let today = parse_day(today).ok_or("invalid date")?;
    let mut offers = Offers::default();
    let mut weeks = Vec::new();
    let mut written = Vec::new();
    // Newest first, so the first post of a week is the one that counts.
    for notice in state.meal_notices(SOURCE_ID)? {
        let Some(week) = parse_day(&notice.published_at).and_then(|p| title_week(&notice.title, p))
        else {
            continue;
        };
        if week + 6 < today || week > today + AHEAD_DAYS || weeks.contains(&week) {
            continue;
        }
        weeks.push(week);
        let [image_url] = notice.images.as_slice() else {
            offers.skipped += 1;
            continue;
        };
        if state.meal_image_read(image_url, &format_day(week), OCR_VERSION)? {
            offers.read += 1;
            continue;
        }
        let offered = Offered {
            image_url: image_url.clone(),
            source_id: SOURCE_ID.into(),
            external_id: notice.external_id.clone(),
            week: format_day(week),
            published_at: notice.published_at.clone(),
            image_sha256: String::new(),
        };
        match fetch_image(transport, image_url, cancel).await {
            // The same image posted for two weeks can be only one of them;
            // its result is read against the first.
            Ok((bytes, _)) if written.contains(&sha256_hex(&bytes)) => offers.skipped += 1,
            Ok((bytes, extension)) => {
                let offered = Offered {
                    image_sha256: sha256_hex(&bytes),
                    ..offered
                };
                written.push(offered.image_sha256.clone());
                write_job(folder, &offered, &bytes, extension)?;
                state.meal_offer(&offered, OCR_VERSION, now)?;
                offers.offered += 1;
            }
            Err(error) => offers
                .failed
                .push(format!("{}: {error}", notice.external_id)),
        }
    }
    Ok(offers)
}

/// Step 3: records the result of every pending image found in `folder`.
pub fn apply(state: &State, folder: &Path, now: &str) -> Result<Applied, String> {
    let mut applied = Applied::default();
    for image in state.meal_offered(OCR_VERSION)? {
        let path = folder.join(format!("{}.result.json", image.image_sha256));
        let bytes = match std::fs::metadata(&path) {
            Err(_) => {
                applied.missing += 1;
                continue;
            }
            Ok(meta) if meta.len() > MAX_RESULT_BYTES => {
                applied
                    .invalid
                    .push(format!("{}: result too large", image.external_id));
                continue;
            }
            Ok(_) => std::fs::read(&path).map_err(|e| e.to_string())?,
        };
        let outcome = match read_result(&bytes, &image) {
            Ok(outcome) => outcome,
            Err(error) => {
                applied
                    .invalid
                    .push(format!("{}: {error}", image.external_id));
                continue;
            }
        };
        let (week, reason) = match outcome {
            Ok(week) => {
                applied.ok += 1;
                (week, None)
            }
            Err(reason) => {
                applied.unreadable += 1;
                (unreadable(&image), Some(reason))
            }
        };
        state.meal_record(&image, &week, reason.as_deref(), now)?;
    }
    Ok(applied)
}

pub fn offers_summary(result: &Result<Offers, String>) -> String {
    match result {
        Ok(offers) => {
            let mut line = format!(
                "meal images: {} offered, {} already read, {} skipped, {} failed",
                offers.offered,
                offers.read,
                offers.skipped,
                offers.failed.len()
            );
            for failure in &offers.failed {
                line.push_str(&format!("\n  {failure}"));
            }
            line
        }
        Err(error) => format!("meal images: failed: {error}"),
    }
}

pub fn applied_summary(applied: &Applied) -> String {
    let mut line = format!(
        "meal results: {} ok, {} unreadable, {} invalid, {} without a result",
        applied.ok,
        applied.unreadable,
        applied.invalid.len(),
        applied.missing
    );
    for invalid in &applied.invalid {
        line.push_str(&format!("\n  {invalid}"));
    }
    line
}

/// The week a title like `11월 23일 ~ 11월 29일 식단` or `3월 12일~18일 식단`
/// names: its Monday, in the year that puts it nearest to the post's
/// publication day `published`, when the named end is that Monday's Sunday.
fn title_week(title: &str, published: i64) -> Option<i64> {
    let compact: String = title.split_whitespace().collect();
    let (left, right) = compact.split_once('~')?;
    let (_, after_month) = left.rsplit_once('월')?;
    let month = trailing_number(&left[..left.len() - after_month.len() - '월'.len_utf8()])?;
    let day = leading_number(after_month.strip_suffix('일')?)?;
    let (first, rest) = leading_number_split(right)?;
    let (end_month, end_day, rest) = match rest.strip_prefix('월') {
        Some(rest) => {
            let (end_day, rest) = leading_number_split(rest)?;
            (Some(first), end_day, rest)
        }
        None => (None, first, rest),
    };
    if !rest.starts_with('일') {
        return None;
    }
    let (year, _, _) = civil(published);
    let start = [year - 1, year, year + 1]
        .into_iter()
        .filter_map(|year| days(year, month, day))
        .min_by_key(|candidate| (candidate - published).abs())?;
    let (_, sunday_month, sunday_day) = civil(start + 6);
    let named_end = end_day == sunday_day && end_month.is_none_or(|month| month == sunday_month);
    (weekday(start) == 0 && named_end && (start - published).abs() <= NEAR_PUBLICATION_DAYS)
        .then_some(start)
}

fn leading_number_split(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    Some((text[..end].parse().ok()?, &text[end..]))
}

fn leading_number(text: &str) -> Option<u32> {
    match leading_number_split(text)? {
        (number, "") => Some(number),
        _ => None,
    }
}

fn trailing_number(text: &str) -> Option<u32> {
    let start = text.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |at| {
        at + text[at..].chars().next().map_or(1, char::len_utf8)
    });
    text[start..].parse().ok()
}

async fn fetch_image<T: Transport>(
    transport: &T,
    image_url: &str,
    cancel: &AtomicBool,
) -> Result<(Vec<u8>, &'static str), String> {
    let url = Url::parse(image_url).map_err(|e| e.to_string())?;
    network::validate(&url, Access::Source)?;
    let robots = Robots::load(transport, &url, cancel, Access::Source).await?;
    let Fetch::Body { bytes, mime, .. } =
        crawl::fetch(transport, &url, None, MAX_IMAGE_BYTES, cancel, &robots).await?
    else {
        return Err("unexpected response".into());
    };
    let extension = match (mime.as_deref(), bytes.as_slice()) {
        (Some("image/jpeg"), [0xff, 0xd8, 0xff, ..]) => "jpg",
        (Some("image/png"), [0x89, b'P', b'N', b'G', ..]) => "png",
        _ => return Err("not a JPEG or PNG image".into()),
    };
    Ok((bytes, extension))
}

fn write_job(folder: &Path, image: &Offered, bytes: &[u8], extension: &str) -> Result<(), String> {
    let name = &image.image_sha256;
    std::fs::write(folder.join(format!("{name}.{extension}")), bytes).map_err(|e| e.to_string())?;
    let job = serde_json::json!({
        "version": OCR_VERSION,
        "image": format!("{name}.{extension}"),
        "image_sha256": name,
        "week": image.week,
        "published": image.published_at,
    });
    std::fs::write(folder.join(format!("{name}.json")), job.to_string()).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcrResult {
    version: String,
    image_sha256: String,
    status: String,
    reason: Option<String>,
    image_size: Option<[u32; 2]>,
    #[serde(default)]
    banner: Vec<String>,
    #[serde(default)]
    headers: Vec<Vec<String>>,
    #[serde(default)]
    days: Vec<OcrDay>,
    breakfast_common: Option<Meal>,
    drinks: Option<Meal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OcrDay {
    day: String,
    breakfast_korean: Meal,
    breakfast_simple: Meal,
    cup_rice: Meal,
    dinner: Meal,
}

/// `Err` for a result that is not one of this image at all (the image stays
/// pending); otherwise the week, or the reason it is unreadable. A result
/// that names the image but fails a check here is unreadable too.
fn read_result(bytes: &[u8], image: &Offered) -> Result<Result<MealWeek, String>, String> {
    let result: OcrResult = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if result.version != OCR_VERSION {
        return Err(format!("OCR version {}", result.version));
    }
    if result.image_sha256 != image.image_sha256 {
        return Err("result of another image".into());
    }
    Ok(match result.status.as_str() {
        "rejected" => Err(match result.reason {
            Some(reason)
                if (1..=40).contains(&reason.len())
                    && reason.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') =>
            {
                reason
            }
            _ => "invalid_result".into(),
        }),
        "ok" => validate(result, image).ok_or_else(|| "invalid_result".to_string()),
        _ => Err("invalid_result".into()),
    })
}

/// The week of an accepted result, checked independently of the OCR step:
/// the banner and every header name the week's own dates, the days run from
/// its Monday, and every cell is well formed.
fn validate(result: OcrResult, image: &Offered) -> Option<MealWeek> {
    let start = parse_day(&image.week)?;
    let (year, month, day) = civil(start);
    let (_, end_month, end_day) = civil(start + 6);
    let banner: String = result.banner.concat().split_whitespace().collect();
    let banner = banner.replace([',', '/', '-'], ".");
    let start_named = [
        format!("{year}.{month:02}.{day:02}"),
        format!("{year}.{month}.{day}"),
    ]
    .iter()
    .any(|text| contains_number_text(&banner, text));
    let end_named = [
        format!("{end_month:02}.{end_day:02}"),
        format!("{end_month}.{end_day}"),
    ]
    .iter()
    .any(|text| contains_number_text(&banner, text));
    let [width, height] = result.image_size?;
    let pixels = u64::from(width) * u64::from(height);
    if !start_named
        || !end_named
        || result.headers.len() != 7
        || result.days.len() != 7
        || !(1..=MAX_PIXELS).contains(&pixels)
    {
        return None;
    }
    let mut days = Vec::new();
    for (index, (header, cells)) in result.headers.iter().zip(result.days).enumerate() {
        let date = start + index as i64;
        let (_, month, day) = civil(date);
        let header: String = header.concat().split_whitespace().collect();
        let named = number_before(&header, month, "월")
            && number_before(&header, day, "일")
            && header.contains(&format!("{}요일", WEEKDAYS[index]));
        if !named || cells.day != format_day(date) {
            return None;
        }
        let day = MealDay {
            day: cells.day,
            breakfast_korean: cell(cells.breakfast_korean)?,
            breakfast_simple: cell(cells.breakfast_simple)?,
            cup_rice: cell(cells.cup_rice)?,
            dinner: cell(cells.dinner)?,
        };
        days.push(day);
    }
    Some(MealWeek {
        week: image.week.clone(),
        source_id: image.source_id.clone(),
        external_id: image.external_id.clone(),
        image_url: image.image_url.clone(),
        status: MealStatus::Ok,
        low_resolution: width < LOW_RESOLUTION_WIDTH,
        days,
        breakfast_common: Some(cell(result.breakfast_common?)?),
        drinks: Some(cell(result.drinks?)?),
    })
}

/// The published form of a week whose image could not be read: its link only.
fn unreadable(image: &Offered) -> MealWeek {
    MealWeek {
        week: image.week.clone(),
        source_id: image.source_id.clone(),
        external_id: image.external_id.clone(),
        image_url: image.image_url.clone(),
        status: MealStatus::Unreadable,
        low_resolution: false,
        days: Vec::new(),
        breakfast_common: None,
        drinks: None,
    }
}

fn cell(meal: Meal) -> Option<Meal> {
    let valid = match meal.status {
        MealCell::Available => {
            (1..=MAX_ITEMS).contains(&meal.items.len())
                && meal.items.iter().all(|item| {
                    !item.trim().is_empty()
                        && item.chars().count() <= MAX_ITEM_CHARS
                        && !item.chars().any(char::is_control)
                })
                && meal.time.as_deref().is_none_or(clock)
        }
        MealCell::Closed | MealCell::Unknown => meal.items.is_empty() && meal.time.is_none(),
    };
    valid.then_some(meal)
}

/// `HH:MM`, 00:00–23:59.
fn clock(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && text[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && text[3..].parse::<u8>().is_ok_and(|m| m < 60)
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
}

/// Whether `text` occurs in `haystack` without digits right around it.
fn contains_number_text(haystack: &str, text: &str) -> bool {
    haystack.match_indices(text).any(|(at, _)| {
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + text.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// Whether `value` is written (optionally zero-padded) right before `unit`.
fn number_before(text: &str, value: u32, unit: &str) -> bool {
    text.match_indices(unit).any(|(at, _)| {
        let digits: String = text[..at]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        digits.parse() == Ok(value)
    })
}

/// Today's date in Korea (UTC+9) from a stored UTC time (`YYYY-MM-DDTHH:…`).
pub fn korean_day(now: &str) -> Option<String> {
    let day = parse_day(now.get(..10)?)?;
    let hour: i64 = now.get(11..13)?.parse().ok()?;
    Some(format_day(day + i64::from(hour >= 15)))
}

// Proleptic Gregorian days since 1970-01-01 (Howard Hinnant's algorithms).
fn days(year: i64, month: u32, day: u32) -> Option<i64> {
    let month = i64::from(month);
    let day = i64::from(day);
    let length = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if !(1..=length).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    Some(era * 146_097 + of_era_days - 719_468)
}

fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let of_era = z - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * of_year + 2) / 153;
    let day = (of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// 0 for Monday.
fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

fn parse_day(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    days(
        text[..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..].parse().ok()?,
    )
}

fn format_day(days: i64) -> String {
    let (year, month, day) = civil(days);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests;
