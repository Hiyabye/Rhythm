use super::*;
use serde::Deserialize;
use std::collections::BTreeMap;

/// The synthetic AI evaluation notices (`intelligence/evaluation.json`), which
/// the generation models were scored on; published 2026-01-01 like that harness.
#[derive(Deserialize)]
struct AiCase {
    id: String,
    text: String,
    deadline: Option<String>,
    #[serde(default)]
    additional_deadlines: Vec<String>,
    #[serde(default)]
    repeat_text: String,
    #[serde(default)]
    repeat_count: usize,
    #[serde(default)]
    suffix: String,
}

/// Held-out notices in common Sogang formats, written before the rules.
#[derive(Deserialize)]
struct Case {
    id: String,
    published_at: String,
    text: String,
    deadlines: Vec<String>,
    others: Vec<String>,
    labels: BTreeMap<String, String>,
}

struct Outcome {
    id: String,
    missing: Vec<String>,
    extra: Vec<String>,
    other_missing: Vec<String>,
    label_wrong: Vec<String>,
}

impl Outcome {
    fn deadlines_exact(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}

fn days(found: &[Found], kind: Kind) -> Vec<String> {
    found
        .iter()
        .filter(|item| item.kind == kind)
        .map(|item| item.day.clone())
        .collect()
}

fn compare(id: &str, found: &[Found], deadlines: &[String]) -> Outcome {
    let got = days(found, Kind::Deadline);
    Outcome {
        id: id.into(),
        missing: deadlines
            .iter()
            .filter(|d| !got.contains(d))
            .cloned()
            .collect(),
        extra: got
            .iter()
            .filter(|d| !deadlines.contains(d))
            .cloned()
            .collect(),
        other_missing: vec![],
        label_wrong: vec![],
    }
}

fn report(name: &str, outcomes: &[Outcome]) {
    let exact = outcomes.iter().filter(|o| o.deadlines_exact()).count();
    println!("{name}: {exact}/{} exact deadline sets", outcomes.len());
    for outcome in outcomes {
        if !outcome.deadlines_exact()
            || !outcome.other_missing.is_empty()
            || !outcome.label_wrong.is_empty()
        {
            println!(
                "  {}: missing {:?} extra {:?} other dates missing {:?} labels {:?}",
                outcome.id,
                outcome.missing,
                outcome.extra,
                outcome.other_missing,
                outcome.label_wrong
            );
        }
    }
}

#[test]
fn synthetic_ai_evaluation_deadlines() {
    let cases: Vec<AiCase> =
        serde_json::from_str(include_str!("../../intelligence/evaluation.json")).unwrap();
    let started = std::time::Instant::now();
    let mut characters = 0;
    let outcomes: Vec<Outcome> = cases
        .iter()
        .map(|case| {
            let body = format!(
                "{}{}{}",
                case.text,
                case.repeat_text.repeat(case.repeat_count),
                case.suffix
            );
            characters += body.chars().count();
            let found = extract(&body, "2026-01-01");
            // The LLM fixture expects no date because the body states no year; the
            // extractor uses the notice's publication year, as the app has one.
            let expected: Vec<String> = if case.id == "yearless_deadline" {
                vec!["2026-04-30".into()]
            } else {
                case.deadline
                    .iter()
                    .chain(&case.additional_deadlines)
                    .cloned()
                    .collect()
            };
            compare(&case.id, &found, &expected)
        })
        .collect();
    println!(
        "{} notices, {characters} characters in {:?}",
        cases.len(),
        started.elapsed()
    );
    report("AI evaluation set", &outcomes);
    assert!(outcomes.iter().all(Outcome::deadlines_exact));
}

#[test]
fn held_out_sogang_formats() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../fixtures/dates/notices.json")).unwrap();
    let outcomes: Vec<Outcome> = cases
        .iter()
        .map(|case| {
            let found = extract(&case.text, &case.published_at);
            let mut outcome = compare(&case.id, &found, &case.deadlines);
            let dated: Vec<&String> = found.iter().map(|item| &item.day).collect();
            outcome.other_missing = case
                .others
                .iter()
                .filter(|day| !dated.contains(day))
                .cloned()
                .collect();
            outcome.label_wrong = case
                .labels
                .iter()
                .filter(|(day, term)| {
                    !found
                        .iter()
                        .any(|item| &item.day == *day && item.label.contains(term.as_str()))
                })
                .map(|(day, term)| format!("{day} lacks {term}"))
                .collect();
            outcome
        })
        .collect();
    report("Held-out set", &outcomes);
    assert!(outcomes
        .iter()
        .all(|o| o.deadlines_exact() && o.other_missing.is_empty() && o.label_wrong.is_empty()));
}

#[test]
fn times_and_weekdays() {
    let found = extract(
        "신청 마감은 2026년 4월 30일(목) 오후 5시 30분입니다.",
        "2026-01-01",
    );
    assert_eq!(
        found,
        vec![Found {
            day: "2026-04-30".into(),
            time: Some("17:30".into()),
            label: "신청 마감".into(),
            kind: Kind::Deadline,
        }]
    );
    assert_eq!(weekday_of((2026, 11, 13)), 5);
    assert!(extract("신청 마감은 2026년 4월 30일(금)입니다.", "2026-01-01").is_empty());
    // A yearless date without a publication date has no year to resolve.
    assert!(extract("신청 마감은 4월 30일입니다.", "").is_empty());
}
