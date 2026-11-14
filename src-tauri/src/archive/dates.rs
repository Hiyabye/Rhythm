//! Deterministic notice date extraction for the calendar (#226).
//!
//! Every date is a literal match in the notice text. A missing year comes from the
//! publication date; a written weekday only marks a bare `9.15`/`9/15` as a date.
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
/// A clause with one of these reports a replaced or cancelled date.
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
/// Periods and events: their `까지` ends an activity, not a submission.
const PERIOD: [&str; 15] = [
    "기간",
    "일시",
    "시간",
    "개최",
    "열리",
    "열립",
    "진행",
    "행사",
    "학회",
    "축제",
    "설명회",
    "면접",
    "발표",
    "교육",
    "운영",
];
/// Clause starts inside one-line bodies: bullets and markers notices use.
const BULLETS: [char; 15] = [
    '■', '□', '▶', '▷', '►', '○', '●', '◼', '◆', '◇', '※', '•', '📍', '🗓', '✅',
];
const RANGE: [&str; 6] = ["부터", "~", "∼", "〜", "–", "-"];
const WEEKDAYS: [char; 7] = ['일', '월', '화', '수', '목', '금', '토'];

pub fn extract(text: &str, published: &str) -> Vec<Found> {
    let published = published.get(..10).and_then(parse_iso);
    let mut found: Vec<Found> = Vec::new();
    for clause in clauses(text) {
        if SUPERSEDED.iter().any(|word| clause.contains(word)) {
            continue;
        }
        let first = found.len();
        let mut segment_start = 0;
        let mut index = 0;
        while index < clause.len() {
            let Some(expression) = expression_at(clause, index) else {
                index += clause[index..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            let before = &clause[segment_start..index];
            segment_start = expression.end;
            index = expression.end;
            if HEADER.iter().any(|word| before.contains(word)) {
                continue;
            }
            // Keywords count only between the previous date and a short window
            // after this one, so one item's 신청 does not reach the next item.
            let after: String = clause[expression.end..]
                .chars()
                .take(12)
                .take_while(|c| !matches!(c, '→' | '(' | ')' | ',' | '|') && !BULLETS.contains(c))
                .collect();
            let context = format!("{before}{after}");
            let action = ACTION.iter().any(|word| context.contains(word));
            let period = PERIOD.iter().any(|word| context.contains(word));
            let due_after = after.trim_start().starts_with("마감")
                || ACTION
                    .iter()
                    .any(|word| after.trim_start().starts_with(&format!("{word} 마감")));
            let due = due_after || DUE.iter().any(|word| before.contains(word));
            // `~9/29` is an open range ending on the date; a `~` right after a
            // number (`8.18~8.19` with an unrecognized start) continues a range.
            let open_end = before
                .trim_end()
                .strip_suffix(['~', '∼', '〜'])
                .is_some_and(|rest| {
                    !rest
                        .trim_end()
                        .ends_with(|c: char| c.is_ascii_digit() || c == ')')
                });
            let label = label(before);
            let Some(start) = resolve(&expression.start, None, published) else {
                continue;
            };
            // Nothing published after a date can still be due on it.
            let due_on = |date: (u32, u32, u32), deadline: bool| {
                if deadline && published.is_none_or(|p| date >= p) {
                    Kind::Deadline
                } else {
                    Kind::Date
                }
            };
            let mut push = |date: (u32, u32, u32), time: &Option<String>, kind: Kind| {
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
                    push(end_date, &end.time, due_on(end_date, deadline));
                }
                None => {
                    let until = expression.until || open_end;
                    let deadline = due || until && (action || !period);
                    push(start, &expression.start.time, due_on(start, deadline));
                }
            }
        }
        // `마감 11. 20. → 연장 11. 27.`: only the last date of an extension is due.
        if clause.contains("연장") && found.len() > first + 1 {
            let last = found.len() - 1;
            if found[last].kind == Kind::Deadline
                || found[first..last].iter().any(|f| f.kind == Kind::Deadline)
            {
                for item in &mut found[first..last] {
                    item.kind = Kind::Date;
                }
                found[last].kind = Kind::Deadline;
            }
        }
    }
    deduplicate(found)
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

/// Keyword scopes. Archived bodies are often one long line, so besides lines and
/// a sentence-final `다.`/`요.` a clause also ends before a bullet, a ` - ` item or
/// a numbered item (`2. 공모분야`, `3) 일정`). A date such as `11. 14.` never
/// ends in `다`/`요`, and its `11. ` is followed by a digit, so dates stay whole.
fn clauses(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    for line in text.lines() {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let next = |position: usize| chars.get(position).map(|&(_, c)| c);
        let mut cuts = vec![0];
        for (position, &(at, c)) in chars.iter().enumerate() {
            let after_space = position > 0 && chars[position - 1].1.is_whitespace();
            let cut = if BULLETS.contains(&c) {
                Some(at)
            } else if matches!(c, '다' | '요')
                && next(position + 1) == Some('.')
                && next(position + 2).is_some_and(char::is_whitespace)
            {
                Some(chars[position + 2].0)
            } else if c == '-'
                && after_space
                && next(position + 1).is_some_and(char::is_whitespace)
                && next(position + 2).is_some_and(|c| !c.is_ascii_digit())
            {
                Some(at)
            } else if c.is_ascii_digit() && after_space && !after_date_part(&chars, position) {
                let digits = chars[position..]
                    .iter()
                    .take_while(|(_, c)| c.is_ascii_digit())
                    .count();
                let end = position + digits;
                (digits <= 2
                    && matches!(next(end), Some('.' | ')'))
                    && next(end + 1).is_some_and(char::is_whitespace)
                    && next(end + 2).is_some_and(|c| !c.is_ascii_digit()))
                .then_some(at)
            } else {
                None
            };
            if let Some(cut) = cut.filter(|cut| cut > cuts.last().unwrap()) {
                cuts.push(cut);
            }
        }
        cuts.push(line.len());
        result.extend(cuts.windows(2).map(|pair| &line[pair[0]..pair[1]]));
    }
    result
}

/// `2026. 6. 16.`: a number right after `<digit>.` continues a date rather
/// than starting a numbered item.
fn after_date_part(chars: &[(usize, char)], position: usize) -> bool {
    let mut previous = chars[..position]
        .iter()
        .rev()
        .map(|&(_, c)| c)
        .skip_while(|c| c.is_whitespace());
    previous.next() == Some('.') && previous.next().is_some_and(|c| c.is_ascii_digit())
}

/// Text just before a date, trimmed to its nearest clause, without a trailing particle.
fn label(before: &str) -> String {
    let clause = before
        .rsplit(['|', ',', '，', '('])
        .next()
        .unwrap_or(before)
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(c, ':' | '：' | '-' | '·' | '~' | '∼' | '〜' | '→')
                || BULLETS.contains(&c)
        });
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
/// `M. D.` (spaced or with a weekday), `M/D(요일)` and, as a range end, `D일`
/// or `D.(요일)`.
fn date_at(text: &str, index: usize, range_end: bool) -> Option<(Written, usize)> {
    let (first, after_first) = number_at(text, index)?;
    let digits = after_first - index;
    let (mut written, mut cursor) = if digits == 4 {
        full_date(text, first, after_first)?
    } else if digits <= 2 {
        short_date(text, index, first, after_first, range_end)?
    } else {
        return None;
    };
    if let Some(after) = weekday_at(text, cursor) {
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
            time: None,
        },
        end,
    ))
}

fn short_date(
    text: &str,
    index: usize,
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
                time: None,
            },
            end,
        ))
    };
    if let Some(after_month) = expect(text, after_first, "월") {
        let (day, after_day) = number_at(text, skip_space(text, after_month))?;
        return month_day(first, day, expect(text, after_day, "일")?);
    }
    // `9.15`/`9/15` alone may be a number (`4.5`, `1/2`); a weekday, `~` or
    // `마감`/`기한` before, or `까지`/`마감` after marks it as a date.
    let marked = |end: usize| {
        let rest = &text[skip_space(text, end)..];
        let before = text[..index].trim_end();
        weekday_at(text, end).is_some()
            || before.ends_with(['~', '∼', '〜'])
            || DUE.iter().any(|word| {
                before
                    .trim_end_matches([':', '：'])
                    .trim_end()
                    .ends_with(word)
            })
            || rest.starts_with("까지")
            || rest.starts_with("마감")
    };
    if let Some(after_dot) = expect(text, after_first, ".") {
        // A range end `~22.(일)` is a day of the start's month.
        if range_end && weekday_at(text, after_dot).is_some() {
            return Some((
                Written {
                    year: None,
                    month: None,
                    day: first,
                    time: None,
                },
                after_dot,
            ));
        }
        let spaced = text[after_dot..].starts_with(char::is_whitespace);
        let (day, after_day) = number_at(text, skip_space(text, after_dot))?;
        let end = expect(text, after_day, ".").unwrap_or(after_day);
        // `11. 14.` with its space and closing dot is a date by itself.
        if !(spaced && end > after_day) && !marked(end) {
            return None;
        }
        return month_day(first, day, end);
    }
    if let Some(after_slash) = expect(text, after_first, "/") {
        let (day, after_day) = number_at(text, after_slash)?;
        marked(after_day).then_some(())?;
        return month_day(first, day, after_day);
    }
    if range_end {
        // `~ 7일` or `~ 19(금)`; date_at reads the weekday itself.
        let end = expect(text, after_first, "일")
            .or_else(|| weekday_at(text, after_first).map(|_| after_first))?;
        return Some((
            Written {
                year: None,
                month: None,
                day: first,
                time: None,
            },
            end,
        ));
    }
    None
}

/// `(목)` or `(목요일)`, optionally after one space; returns the end.
fn weekday_at(text: &str, index: usize) -> Option<usize> {
    let index = expect(text, index, " ").unwrap_or(index);
    let rest = text[index..].strip_prefix('(')?;
    let name = rest.chars().next()?;
    WEEKDAYS.contains(&name).then_some(())?;
    let mut cursor = index + 1 + name.len_utf8();
    cursor = expect(text, cursor, "요일").unwrap_or(cursor);
    expect(text, cursor, ")")
}

/// `17:00`, `18시`, `오후 5시 30분`, `밤 12시`, normalized to 24-hour `HH:MM`.
fn time_at(text: &str, index: usize) -> Option<(String, usize)> {
    let night = expect(text, index, "밤");
    let (afternoon, cursor) = if let Some(after) = expect(text, index, "오후").or(night) {
        (true, skip_space(text, after))
    } else if let Some(after) = expect(text, index, "오전") {
        (false, skip_space(text, after))
    } else {
        (false, index)
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
    // `밤 12시` is midnight at the end of that day, written 24:00.
    if afternoon && hour < 12 || night.is_some() && hour == 12 {
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
    // A written weekday that disagrees is a typo in the notice; the date number
    // is what readers act on, so it is kept.
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
    while let Some(c) = text[index..].chars().next().filter(|c| c.is_whitespace()) {
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
