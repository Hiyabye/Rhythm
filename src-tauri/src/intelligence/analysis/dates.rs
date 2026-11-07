use super::*;

pub(super) fn normalize_date(value: &str, evidence: &str) -> Result<Option<String>, String> {
    let mut dates = HashSet::new();
    for literal in literal_dates(value) {
        if !grounded_date(&literal.date, evidence) {
            return Err("추출 날짜가 원문과 일치하지 않습니다. value에 연도를 보충하지 말고 해당 text의 날짜 표기를 그대로 유지하세요.".into());
        }
        dates.insert(literal.date);
    }
    validate_times(value, evidence)?;
    // A period with an omitted end-year is not a single normalized deadline.
    // Do not confuse a clock-time range (17:00~18:00) with a calendar range.
    let abbreviated_period = ["~", "∼", "〜", "–", " - ", "부터"].iter().any(|marker| {
        value.match_indices(marker).any(|(start, _)| {
            let rest = value[start + marker.len()..].trim_start();
            number_at(rest, 0).is_some_and(|(_, end)| {
                rest[skip_space(rest, end)..]
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, '년' | '월' | '일' | '.' | '/' | '-'))
            })
        })
    });
    Ok(if dates.len() == 1 && !abbreviated_period {
        dates.into_iter().next()
    } else {
        None
    })
}
pub(super) struct LiteralDate {
    pub(super) date: String,
    pub(super) span: std::ops::Range<usize>,
}
pub(super) fn skip_space(text: &str, mut index: usize) -> usize {
    while let Some(c) = text[index..].chars().next().filter(|c| c.is_whitespace()) {
        index += c.len_utf8();
    }
    index
}
pub(super) fn number_at(text: &str, start: usize) -> Option<(u32, usize)> {
    let count = text.as_bytes()[start..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    (count > 0)
        .then(|| {
            text[start..start + count]
                .parse()
                .ok()
                .map(|n| (n, start + count))
        })
        .flatten()
}
pub(super) fn calendar_date(year: u32, month: u32, day: u32) -> bool {
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
pub(super) fn literal_dates(text: &str) -> Vec<LiteralDate> {
    let mut dates = Vec::new();
    for (start, c) in text.char_indices() {
        if !c.is_ascii_digit() || start > 0 && text.as_bytes()[start - 1].is_ascii_digit() {
            continue;
        }
        let Some((year, after_year)) = number_at(text, start) else {
            continue;
        };
        if after_year - start != 4 {
            continue;
        }
        let separator_at = skip_space(text, after_year);
        let Some(separator) = text[separator_at..]
            .chars()
            .next()
            .filter(|c| matches!(c, '년' | '.' | '/' | '-'))
        else {
            continue;
        };
        let month_start = skip_space(text, separator_at + separator.len_utf8());
        let Some((month, after_month)) = number_at(text, month_start) else {
            continue;
        };
        if after_month - month_start > 2 {
            continue;
        }
        let second_at = skip_space(text, after_month);
        let second = if separator == '년' { '월' } else { separator };
        if !text[second_at..].starts_with(second) {
            continue;
        }
        let day_start = skip_space(text, second_at + second.len_utf8());
        let Some((day, after_day)) = number_at(text, day_start) else {
            continue;
        };
        if after_day - day_start > 2 || !calendar_date(year, month, day) {
            continue;
        }
        let end = if separator == '년' {
            let at = skip_space(text, after_day);
            if !text[at..].starts_with('일') {
                continue;
            }
            at + '일'.len_utf8()
        } else {
            after_day
        };
        dates.push(LiteralDate {
            date: format!("{year:04}-{month:02}-{day:02}"),
            span: start..end,
        });
    }
    dates
}
pub(super) fn grounded_date(date: &str, evidence: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && literal_dates(evidence)
            .iter()
            .any(|literal| literal.date == date)
}

pub(super) struct LiteralTime {
    pub(super) minutes: u32,
    pub(super) ambiguous: bool,
    pub(super) valid: bool,
    pub(super) start: usize,
}
pub(super) fn english_meridiem(text: &str) -> Option<bool> {
    let marker = text.get(..2)?;
    if text
        .as_bytes()
        .get(2)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    {
        return None;
    }
    if marker.eq_ignore_ascii_case("am") {
        Some(false)
    } else if marker.eq_ignore_ascii_case("pm") {
        Some(true)
    } else {
        None
    }
}
pub(super) fn literal_times(text: &str) -> Vec<LiteralTime> {
    let mut times = Vec::new();
    let dates = literal_dates(text);
    let mut consumed_until = 0;
    for (start, c) in text.char_indices() {
        if start < consumed_until
            || !c.is_ascii_digit()
            || start > 0 && text.as_bytes()[start - 1].is_ascii_digit()
        {
            continue;
        }
        // In 2026-05-15 AM 9:15, the day is not an additional 15 AM clock.
        if dates.iter().any(|date| date.span.contains(&start)) {
            continue;
        }
        let Some((hour, after_hour)) = number_at(text, start) else {
            continue;
        };
        let at = skip_space(text, after_hour);
        // Durations such as 10시간 are not clock-time claims.
        if text[at..].starts_with("시간") {
            continue;
        }
        let prefix = text[..start].trim_end();
        // A word ending in AM (e.g. PROGRAM) is not a meridiem marker.
        let prefix_at = prefix.len().saturating_sub(2);
        let preceding = prefix.get(..prefix_at).unwrap_or("");
        let english_prefix = prefix
            .get(prefix_at..)
            .filter(|_| {
                !preceding
                    .as_bytes()
                    .last()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            })
            .and_then(english_meridiem);
        let korean = text[at..].starts_with('시');
        let (minute, after_time) = if text[at..].starts_with(':') {
            let minute_start = skip_space(text, at + 1);
            let Some((minute, after_minute)) = number_at(text, minute_start) else {
                continue;
            };
            if after_minute - minute_start != 2 {
                continue;
            }
            (minute, after_minute)
        } else if korean {
            let minute_start = skip_space(text, at + '시'.len_utf8());
            match number_at(text, minute_start) {
                Some((minute, end)) if text[skip_space(text, end)..].starts_with('분') => {
                    (minute, skip_space(text, end) + '분'.len_utf8())
                }
                _ if text[minute_start..].starts_with('반') => {
                    (30, minute_start + '반'.len_utf8())
                }
                _ => (0, minute_start),
            }
        } else if english_meridiem(&text[at..]).is_some() || english_prefix.is_some() {
            (0, at)
        } else {
            continue;
        };
        let korean_prefix = if english_prefix.is_some() {
            preceding.trim_end()
        } else {
            prefix
        };
        let suffix_at = skip_space(text, after_time);
        let english_suffix = english_meridiem(&text[suffix_at..]);
        let repeated_suffix =
            english_suffix.and_then(|_| english_meridiem(&text[skip_space(text, suffix_at + 2)..]));
        let am = korean_prefix.ends_with("오전")
            || [english_prefix, english_suffix, repeated_suffix].contains(&Some(false));
        let pm = korean_prefix.ends_with("오후")
            || [english_prefix, english_suffix, repeated_suffix].contains(&Some(true));
        let valid = !(am && pm)
            && minute < 60
            && if am || pm {
                (1..=12).contains(&hour)
            } else {
                hour <= 23 || hour == 24 && minute == 0
            };
        let hour = if am {
            hour % 12
        } else if pm {
            hour % 12 + 12
        } else {
            hour
        };
        // Do not reinterpret the minute component of 5:30 PM as a second hour.
        consumed_until = after_time;
        times.push(LiteralTime {
            minutes: hour.saturating_mul(60).saturating_add(minute),
            ambiguous: korean && !am && !pm && hour <= 12,
            valid,
            start,
        });
    }
    times
}
/// A claimed time is literally supported by a valid source time; an ambiguous
/// Korean claim (no meridiem) matches either half of the day.
pub(super) fn time_supports(claimed: &LiteralTime, source: &LiteralTime) -> bool {
    source.valid
        && if claimed.ambiguous {
            claimed.minutes % 720 == source.minutes % 720
        } else {
            !source.ambiguous && claimed.minutes == source.minutes
        }
}
pub(super) fn validate_times(value: &str, evidence: &str) -> Result<(), String> {
    let claimed_dates = literal_dates(value);
    let source_dates = literal_dates(evidence);
    for claimed in literal_times(value) {
        // Bind a claimed clock time to its date, not an unrelated time elsewhere
        // in a multi-date passage. This is literal support, not semantic entailment.
        let subject = claimed_dates
            .iter()
            .rev()
            .find(|date| date.span.start < claimed.start)
            .or_else(|| claimed_dates.first());
        let supported = source_dates
            .iter()
            .enumerate()
            .filter(|(_, date)| subject.is_none_or(|subject| subject.date == date.date))
            .any(|(index, date)| {
                let window = if source_dates.len() == 1 {
                    evidence
                } else {
                    &evidence[date.span.start
                        ..source_dates
                            .get(index + 1)
                            .map_or(evidence.len(), |next| next.span.start)]
                };
                literal_times(window)
                    .iter()
                    .any(|source| time_supports(&claimed, source))
            });
        let supported = supported
            || subject.is_none()
                && literal_times(evidence)
                    .iter()
                    .any(|source| time_supports(&claimed, source));
        if !claimed.valid || !supported {
            return Err("추출 시간이 원문의 날짜·시간과 일치하지 않습니다.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::analysis::tests::SOURCE;
    #[test]
    fn abbreviated_range_repair_identifies_the_unsupported_year_before_clock_binding() {
        let source = "모집기간 2026년 9월 1일(화) 9시 – 9월 14일(월) 17시까지";
        let error = normalize_date("2026년 9월 14일 17시", source).unwrap_err();
        assert!(error.contains("연도를 보충하지"));
        assert!(normalize_date("9월 14일(월) 17시", source)
            .unwrap()
            .is_none());
    }
    #[test]
    fn date_matching_preserves_digit_boundaries_and_supports_times() {
        for value in [
            "2026. 4. 30 17:00",
            "2026-04-30 오후 5시",
            "2026/4/30 오전 9시 30분",
        ] {
            assert_eq!(
                normalize_date(value, value).unwrap().as_deref(),
                Some("2026-04-30")
            );
        }
        assert!(normalize_date("2026.4.30", "2026.4.3 0명").is_err());
        assert!(!grounded_date("2026-04-30", "12026.4.30"));
        assert!(normalize_date("4월 30일", "2026년 4월 30일")
            .unwrap()
            .is_none());
        assert!(normalize_date("2026년 4월 30일 오후 3시", "2026년 4월 30일 오후 5시").is_err());
        assert_eq!(
            normalize_date("2026년 4월 30일 오후 5시", "2026년 4월 30일 17:00")
                .unwrap()
                .as_deref(),
            Some("2026-04-30")
        );
    }
    #[test]
    fn english_meridiems_preserve_clock_grounding() {
        assert!(normalize_date("2026-05-15 오전 5시", "2026-05-15 5:00 PM").is_err());
        for (claim, source) in [
            ("2026-05-15 오후 5시", "2026-05-15 5:00 PM"),
            ("2026-05-15 17:00", "2026-05-15 pm 5:00"),
            ("2026-05-15 17:00", "2026-05-15 PM 5"),
            ("2026-05-15 00:00", "2026-05-15 12 AM"),
            ("2026-05-15 12:00", "2026-05-15 12pm"),
            ("2026-05-15 5pM", "2026-05-15 오후 5시"),
            ("2026-05-15 5:30 PM", "2026-05-15 17:30"),
            ("2026-05-15 AM 9:15", "2026-05-15 오전 9시 15분"),
        ] {
            assert_eq!(
                normalize_date(claim, source)
                    .unwrap_or_else(|error| panic!("{claim:?} / {source:?}: {error}"))
                    .as_deref(),
                Some("2026-05-15")
            );
        }
        assert!(normalize_date("2026-05-15 5 AM", "2026-05-15 오후 5시").is_err());
        assert!(normalize_date("2026-05-15 5:00 AM", "2026-05-15 5:00 PM").is_err());
        assert!(normalize_date("2026-05-15 13 PM", "2026-05-15 13 PM").is_err());
        assert!(normalize_date("2026-05-15 5 AM PM", "2026-05-15 05:00").is_err());
        assert!(literal_times("PROGRAM 5:00, 10시간, 5AMP, 5PM2")
            .iter()
            .all(|time| time.minutes == 300));
        assert_eq!(literal_times("PROGRAM 5:00, 10시간, 5AMP, 5PM2").len(), 1);
    }
    #[test]
    fn literal_dates_and_times_remain_conservative_across_ambiguity() {
        assert!(normalize_date(
            "2026년 4월 30일 15:00",
            "2026년 4월 30일 17:00 마감. 결과는 2026년 5월 8일 15:00 발표."
        )
        .is_err());
        assert_eq!(
            normalize_date("2026년 4월 30일 오후 5시 30분", "2026-04-30 17:30")
                .unwrap()
                .as_deref(),
            Some("2026-04-30")
        );
        assert!(normalize_date("2026년 4월 30일 오후 5시", "2026년 4월 30일 5시").is_err());
        assert!(normalize_date("2026년 4월 30일 25:99", "2026년 4월 30일 25:99").is_err());
        assert!(normalize_date("2026년 4월 30일 4294967295시", SOURCE).is_err());
        for value in ["4월 30일", "다음 달 말", "내일 오후 3시"] {
            assert!(normalize_date(value, value).unwrap().is_none());
        }
        assert!(normalize_date(
            "2026년 4월 1일부터 2026년 4월 30일까지",
            "2026년 4월 1일부터 2026년 4월 30일까지"
        )
        .unwrap()
        .is_none());
        for period in [
            "2026년 4월 1일~4월 30일",
            "2026.4.1~30일",
            "2026.4.1부터 4.30까지",
        ] {
            assert!(normalize_date(period, period).unwrap().is_none());
        }
        assert_eq!(
            normalize_date("2026.4.30 17:00~18:00", "2026.4.30 17:00~18:00")
                .unwrap()
                .as_deref(),
            Some("2026-04-30")
        );
        assert!(!grounded_date("2026-04-30", "2026 4 30"));
        assert!(!grounded_date("2026-04-30", "2026.4.3 0명"));
        assert!(!grounded_date("2026-02-29", "2026.2.29"));
    }
}
