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
    // 2026-04-30 is a Thursday; the mistyped weekday does not drop the date.
    assert_eq!(
        days(
            &extract("신청 마감은 2026년 4월 30일(금)입니다.", "2026-01-01"),
            Kind::Deadline
        ),
        ["2026-04-30"]
    );
    // `16. (화)` continues the date; it is not a numbered item.
    assert_eq!(
        days(
            &extract("4. 지원 기간: 2026. 6. 16. (화) 14시까지", "2026-06-01"),
            Kind::Deadline
        ),
        ["2026-06-16"]
    );
    // A yearless date without a publication date has no year to resolve.
    assert!(extract("신청 마감은 4월 30일입니다.", "").is_empty());
}

/// Opt-in, read-only comparison against a copied archive. Point
/// `RHYTHM_DATES_EVAL_DB` at a copy (never the live profile) and
/// `RHYTHM_DATES_EVAL_REPORT` at the JSON output. AI dates are a reference, not
/// ground truth; disagreements need a human read.
#[test]
#[ignore]
fn archived_dates_evaluation() {
    use rusqlite::{Connection, OpenFlags};
    let path = std::env::var("RHYTHM_DATES_EVAL_DB").expect("RHYTHM_DATES_EVAL_DB");
    let report = std::env::var("RHYTHM_DATES_EVAL_REPORT").expect("RHYTHM_DATES_EVAL_REPORT");
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = db
        .prepare(
            "SELECT d.id,d.source_id,d.title,d.published_at,d.body_text,
               (SELECT a.result_json FROM ai_results a JOIN document_revisions r ON r.id=a.revision_id
                 WHERE a.document_id=d.id AND a.prompt_version=5 ORDER BY a.revision_id DESC,a.id DESC LIMIT 1)
             FROM documents d WHERE d.id NOT IN (SELECT document_id FROM trashed_documents)",
        )
        .unwrap();
    let rows: Vec<(i64, String, String, String, String, Option<String>)> = statement
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let started = std::time::Instant::now();
    let mut entries = Vec::new();
    let (mut with_deadline, mut with_any, mut compared, mut agree) = (0, 0, 0, 0);
    for (id, source, title, published, body, ai) in &rows {
        let found = extract(&format!("{title}\n{body}"), published);
        with_deadline += usize::from(found.iter().any(|f| f.kind == Kind::Deadline));
        with_any += usize::from(!found.is_empty());
        let ai_deadlines: Option<Vec<String>> = ai.as_ref().map(|json| {
            let value: serde_json::Value = serde_json::from_str(json).unwrap();
            let mut days: Vec<String> = value["facts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|fact| fact["kind"] == "deadline")
                .filter_map(|fact| {
                    fact["date"]
                        .as_str()
                        .and_then(|d| d.get(..10))
                        .map(str::to_owned)
                })
                .collect();
            days.sort();
            days.dedup();
            days
        });
        let mut ours = days(&found, Kind::Deadline);
        ours.sort();
        if let Some(theirs) = &ai_deadlines {
            compared += 1;
            agree += usize::from(&ours == theirs);
        }
        entries.push(serde_json::json!({
            "id": id, "source": source, "title": title, "published": published,
            "found": found.iter().map(|f| serde_json::json!({"day": f.day, "time": f.time, "label": f.label, "kind": format!("{:?}", f.kind)})).collect::<Vec<_>>(),
            "ai_deadlines": ai_deadlines,
        }));
    }
    let summary = serde_json::json!({
        "notices": rows.len(), "elapsed_ms": started.elapsed().as_millis(),
        "with_deadline": with_deadline, "with_any_date": with_any,
        "compared_with_ai": compared, "same_deadline_set_as_ai": agree,
    });
    println!("{summary}");
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&serde_json::json!({"summary": summary, "entries": entries}))
            .unwrap(),
    )
    .unwrap();
}
