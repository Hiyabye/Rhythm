//! Deterministic notice date extraction (#226 prototype; not wired into the app).
//!
//! Every date is a literal match in the notice text. A missing year comes from the
//! publication date, and a written weekday must agree with the resolved date.
//! Classification is lexical: it decides deadline vs. other date from nearby
//! Korean keywords, not from meaning.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A due date: an application/submission end or an explicit 마감/기한.
    Deadline,
    /// The first day of a range whose end is the meaningful date.
    Start,
    /// Any other dated item (event, announcement, activity period).
    Date,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub day: String,
    pub time: Option<String>,
    pub label: String,
    pub kind: Kind,
}

/// A date with nothing before it except one of these is page metadata.
const HEADER: [&str; 4] = ["작성일", "게시일", "등록일", "수정일"];
/// A sentence with one of these reports a replaced or cancelled date.
const SUPERSEDED: [&str; 6] = [
    "취소합니다",
    "취소되었",
    "취소됩니다",
    "이전 안내",
    "변경 전",
    "정정 전",
];
const DUE: [&str; 2] = ["마감", "기한"];
const ACTION: [&str; 9] = [
    "신청", "접수", "모집", "제출", "지원", "등록", "납부", "응시", "취소",
];
const PERIOD: [&str; 3] = ["기간", "일시", "시간"];
const RANGE: [&str; 6] = ["부터", "~", "∼", "〜", "–", "-"];
const WEEKDAYS: [char; 7] = ['일', '월', '화', '수', '목', '금', '토'];

pub fn extract(text: &str, published: &str) -> Vec<Found> {
    let published = published.get(..10).and_then(parse_iso);
    let mut found: Vec<Found> = Vec::new();
    for sentence in sentences(text) {
        if SUPERSEDED.iter().any(|word| sentence.contains(word)) {
            continue;
        }
        let action = ACTION.iter().any(|word| sentence.contains(word));
        let period = PERIOD.iter().any(|word| sentence.contains(word));
        let mut segment_start = 0;
        let mut index = 0;
        while index < sentence.len() {
            let Some(expression) = expression_at(sentence, index) else {
                index += sentence[index..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            let before = &sentence[segment_start..index];
            segment_start = expression.end;
            index = expression.end;
            if HEADER.iter().any(|word| before.contains(word)) {
                continue;
            }
            let due = DUE.iter().any(|word| before.contains(word));
            let label = label(before);
            let Some(start) = resolve(&expression.start, None, published) else {
                continue;
            };
            let mut push = |date: (u32, u32, u32), time: &Option<String>, kind| {
                found.push(Found {
                    day: format!("{:04}-{:02}-{:02}", date.0, date.1, date.2),
                    time: time.clone(),
                    label: label.clone(),
                    kind,
                })
            };
            match &expression.end_date {
                Some(end) => {
                    push(start, &expression.start.time, Kind::Start);
                    let Some(end_date) = resolve(end, Some(start), published) else {
                        continue;
                    };
                    let deadline = due || action || expression.until && !period;
                    push(end_date, &end.time, kind(deadline));
                }
                None => {
                    let deadline = due || expression.until && (action || !period);
                    push(start, &expression.start.time, kind(deadline));
                }
            }
        }
    }
    deduplicate(found)
}

fn kind(deadline: bool) -> Kind {
    if deadline {
        Kind::Deadline
    } else {
        Kind::Date
    }
}

/// One entry per (day, kind); a day that is a deadline anywhere is only a deadline.
fn deduplicate(found: Vec<Found>) -> Vec<Found> {
    let deadlines: Vec<String> = found
        .iter()
        .filter(|item| item.kind == Kind::Deadline)
        .map(|item| item.day.clone())
        .collect();
    let mut unique: Vec<Found> = Vec::new();
    for item in found {
        if item.kind != Kind::Deadline && deadlines.contains(&item.day) {
            continue;
        }
        if !unique
            .iter()
            .any(|seen| seen.day == item.day && seen.kind == item.kind)
        {
            unique.push(item);
        }
    }
    unique
}

/// Lines, further split after a sentence-final `다.`/`요.`. Dates such as
/// `11. 14.` never end in those syllables, so they are not split.
fn sentences(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    for line in text.lines() {
        let mut start = 0;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for window in chars.windows(3) {
            let [(_, syllable), (dot, '.'), (_, space)] = window else {
                continue;
            };
            if matches!(syllable, '다' | '요') && space.is_whitespace() {
                result.push(&line[start..dot + 1]);
                start = dot + 1;
            }
        }
        result.push(&line[start..]);
    }
    result
}

/// Text just before a date, trimmed to its nearest clause, without a trailing particle.
fn label(before: &str) -> String {
    let clause = before
        .rsplit(['|', ',', '，'])
        .next()
        .unwrap_or(before)
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '：' | '-' | '·'));
    let clause = ["에는", "은", "는", "이", "가", "을", "를", "에"]
        .iter()
        .find_map(|particle| clause.strip_suffix(particle))
        .unwrap_or(clause)
        .trim_end();
    let skip = clause.chars().count().saturating_sub(40);
    clause.chars().skip(skip).collect()
}

/// A date as written: the year is optional; a day-only range end takes the
/// start's year and month.
#[derive(Debug, Clone)]
struct Written {
    year: Option<u32>,
    month: Option<u32>,
    day: u32,
    weekday: Option<u32>,
    time: Option<String>,
}

struct Expression {
    start: Written,
    end_date: Option<Written>,
    /// `까지` follows the last date (not a clock-time range end).
    until: bool,
    end: usize,
}

fn expression_at(text: &str, index: usize) -> Option<Expression> {
    if index > 0 && text.as_bytes()[index - 1].is_ascii_digit() {
        return None;
    }
    let (start, mut cursor) = date_at(text, index, false)?;
    let mut end_date = None;
    let mut time_range = false;
    // Without a following date or time (`9월 21일부터 발급`) the date is an open
    // start and the cursor stays at the date.
    if let Some(after_marker) = range_marker(text, cursor) {
        let next = skip_space(text, after_marker);
        if let Some((mut end, after)) = date_at(text, next, true) {
            end.month = end.month.or(start.month);
            end_date = Some(end);
            cursor = after;
        } else if let Some((_, after)) = time_at(text, next).filter(|_| start.time.is_some()) {
            time_range = true;
            cursor = after;
        }
    }
    let until = !time_range && text[skip_space(text, cursor)..].starts_with("까지");
    Some(Expression {
        start,
        end_date,
        until,
        end: cursor,
    })
}

fn range_marker(text: &str, cursor: usize) -> Option<usize> {
    let at = skip_space(text, cursor);
    RANGE
        .iter()
        .find(|marker| text[at..].starts_with(*marker))
        .map(|marker| at + marker.len())
}

/// `YYYY년 M월 D일`, `YYYY. M. D.`, `YYYY-MM-DD`, `YYYY/MM/DD`, `M월 D일`,
/// `M. D.` (spaced or with a weekday), `M/D(요일)` and, as a range end, `D일`.
fn date_at(text: &str, index: usize, range_end: bool) -> Option<(Written, usize)> {
    let (first, after_first) = number_at(text, index)?;
    let digits = after_first - index;
    let (mut written, mut cursor) = if digits == 4 {
        full_date(text, first, after_first)?
    } else if digits <= 2 {
        short_date(text, first, after_first, range_end)?
    } else {
        return None;
    };
    if let Some((weekday, after)) = weekday_at(text, cursor) {
        written.weekday = Some(weekday);
        cursor = after;
    }
    if let Some((time, after)) = time_at(text, skip_space(text, cursor)) {
        written.time = Some(time);
        cursor = after;
    }
    Some((written, cursor))
}

fn full_date(text: &str, year: u32, after_year: usize) -> Option<(Written, usize)> {
    let (month, day, end) = if let Some(after_word) = expect(text, after_year, "년") {
        let (month, after_month) = number_at(text, skip_space(text, after_word))?;
        let after_month = expect(text, after_month, "월")?;
        let (day, after_day) = number_at(text, skip_space(text, after_month))?;
        (month, day, expect(text, after_day, "일")?)
    } else {
        let separator = text[after_year..]
            .chars()
            .next()
            .filter(|c| matches!(c, '.' | '-' | '/'))?;
        let (month, after_month) = number_at(text, skip_space(text, after_year + 1))?;
        let after_month = expect(text, after_month, &separator.to_string())?;
        let (day, after_day) = number_at(text, skip_space(text, after_month))?;
        let end = if separator == '.' {
            expect(text, after_day, ".").unwrap_or(after_day)
        } else {
            after_day
        };
        (month, day, end)
    };
    Some((
        Written {
            year: Some(year),
            month: Some(month),
            day,
            weekday: None,
            time: None,
        },
        end,
    ))
}

fn short_date(
    text: &str,
    first: u32,
    after_first: usize,
    range_end: bool,
) -> Option<(Written, usize)> {
    let month_day = |month, day, end| {
        Some((
            Written {
                year: None,
                month: Some(month),
                day,
                weekday: None,
                time: None,
            },
            end,
        ))
    };
    if let Some(after_month) = expect(text, after_first, "월") {
        let (day, after_day) = number_at(text, skip_space(text, after_month))?;
        return month_day(first, day, expect(text, after_day, "일")?);
    }
    if let Some(after_dot) = expect(text, after_first, ".") {
        let spaced = text[after_dot..].starts_with(' ');
        let (day, after_day) = number_at(text, skip_space(text, after_dot))?;
        let end = expect(text, after_day, ".").unwrap_or(after_day);
        // `11. 14.` needs the space and closing dot, `11.14.(금)` a weekday;
        // `4.5.` is more likely a number than a date.
        if !(spaced && end > after_day) && weekday_at(text, end).is_none() {
            return None;
        }
        return month_day(first, day, end);
    }
    if let Some(after_slash) = expect(text, after_first, "/") {
        let (day, after_day) = number_at(text, after_slash)?;
        weekday_at(text, after_day)?;
        return month_day(first, day, after_day);
    }
    if range_end {
        let end = expect(text, after_first, "일")?;
        return Some((
            Written {
                year: None,
                month: None,
                day: first,
                weekday: None,
                time: None,
            },
            end,
        ));
    }
    None
}

/// `(목)` or `(목요일)`, returned as 0 = Sunday.
fn weekday_at(text: &str, index: usize) -> Option<(u32, usize)> {
    let rest = text[index..].strip_prefix('(')?;
    let name = rest.chars().next()?;
    let weekday = WEEKDAYS.iter().position(|day| *day == name)? as u32;
    let mut cursor = index + 1 + name.len_utf8();
    cursor = expect(text, cursor, "요일").unwrap_or(cursor);
    Some((weekday, expect(text, cursor, ")")?))
}

/// `17:00`, `18시`, `오후 5시 30분`, normalized to 24-hour `HH:MM`.
fn time_at(text: &str, index: usize) -> Option<(String, usize)> {
    let (afternoon, cursor) = if let Some(after) = expect(text, index, "오후") {
        (Some(true), skip_space(text, after))
    } else if let Some(after) = expect(text, index, "오전") {
        (Some(false), skip_space(text, after))
    } else {
        (None, index)
    };
    let (hour, after_hour) = number_at(text, cursor)?;
    let (mut hour, minute, end) = if let Some(after_colon) = expect(text, after_hour, ":") {
        let (minute, after_minute) = number_at(text, after_colon)?;
        (after_minute - after_colon == 2).then_some(())?;
        (hour, minute, after_minute)
    } else {
        let after_hour = expect(text, after_hour, "시")?;
        let minutes = number_at(text, skip_space(text, after_hour))
            .and_then(|(minute, after)| expect(text, after, "분").map(|end| (minute, end)));
        match minutes {
            Some((minute, end)) => (hour, minute, end),
            None => (hour, 0, after_hour),
        }
    };
    if afternoon == Some(true) && hour < 12 {
        hour += 12;
    }
    (hour <= 24 && minute < 60).then(|| (format!("{hour:02}:{minute:02}"), end))
}

/// The written date as a calendar day. A missing year is the one that puts the
/// date between 90 days before and 275 days after publication; a range end
/// without a year follows its start (rolling into the next year if earlier).
fn resolve(
    written: &Written,
    start: Option<(u32, u32, u32)>,
    published: Option<(u32, u32, u32)>,
) -> Option<(u32, u32, u32)> {
    let month = written.month?;
    let date = match (written.year, start) {
        (Some(year), _) => (year, month, written.day),
        (None, Some(start)) => {
            let same = (start.0, month, written.day);
            if (same.1, same.2) < (start.1, start.2) {
                (start.0 + 1, month, written.day)
            } else {
                same
            }
        }
        (None, None) => {
            let published = published?;
            let anchor = day_number(published);
            [published.0 - 1, published.0, published.0 + 1]
                .into_iter()
                .map(|year| (year, month, written.day))
                .find(|date| {
                    valid(*date) && (anchor - 90..anchor + 275).contains(&day_number(*date))
                })?
        }
    };
    if !valid(date) {
        return None;
    }
    if written
        .weekday
        .is_some_and(|weekday| weekday != weekday_of(date))
    {
        return None;
    }
    Some(date)
}

fn valid((year, month, day): (u32, u32, u32)) -> bool {
    let days = match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1900..=2200).contains(&year) && (1..=12).contains(&month) && (1..=days).contains(&day)
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn day_number((year, month, day): (u32, u32, u32)) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 0 = Sunday.
fn weekday_of(date: (u32, u32, u32)) -> u32 {
    // 1970-01-01 was a Thursday.
    (day_number(date) + 4).rem_euclid(7) as u32
}

fn parse_iso(value: &str) -> Option<(u32, u32, u32)> {
    let date = (
        value.get(0..4)?.parse().ok()?,
        value.get(5..7)?.parse().ok()?,
        value.get(8..10)?.parse().ok()?,
    );
    valid(date).then_some(date)
}

fn expect(text: &str, index: usize, literal: &str) -> Option<usize> {
    text[index..]
        .starts_with(literal)
        .then(|| index + literal.len())
}

fn skip_space(text: &str, mut index: usize) -> usize {
    while let Some(c) = text[index..]
        .chars()
        .next()
        .filter(|c| *c == ' ' || *c == '\t')
    {
        index += c.len_utf8();
    }
    index
}

fn number_at(text: &str, start: usize) -> Option<(u32, usize)> {
    let count = text.as_bytes()[start..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if count == 0 || count > 4 {
        return None;
    }
    Some((text[start..start + count].parse().ok()?, start + count))
}

#[cfg(test)]
mod tests;
