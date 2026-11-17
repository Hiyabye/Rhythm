use super::*;
use crate::workers_ai::Embedded;
use rhythm_sources::model::ArticleInput;
use std::sync::Mutex;

const BOARD: &str = "sogang-academic";

/// Deterministic unit vectors per text. `cosine` below 1 bends every vector
/// by exactly that cosine from its usual direction (model drift).
struct Fake {
    calls: Mutex<Vec<Vec<String>>>,
    cosine: Mutex<f64>,
    neurons_per_text: Mutex<f64>,
    /// The call (counted from 0) that fails, if any.
    fail_at: Mutex<Option<usize>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            calls: Mutex::default(),
            cosine: Mutex::new(1.0),
            neurons_per_text: Mutex::new(0.5),
            fail_at: Mutex::default(),
        }
    }
    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn texts(&self) -> Vec<String> {
        self.calls.lock().unwrap().concat()
    }
}
fn direction(seed: &str) -> Vec<f64> {
    let mut values = Vec::with_capacity(DIMENSIONS);
    let mut block = 0;
    while values.len() < DIMENSIONS {
        let hash = sha256_hex(format!("{seed}/{block}"));
        for pair in hash.as_bytes().chunks(2) {
            let byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
            values.push(f64::from(byte) - 127.5);
        }
        block += 1;
    }
    values.truncate(DIMENSIONS);
    let norm = values.iter().map(|x| x * x).sum::<f64>().sqrt();
    values.iter().map(|x| x / norm).collect()
}
impl Embedder for Fake {
    async fn embed(&self, texts: &[String]) -> Result<Embedded, String> {
        let call = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(texts.to_vec());
            calls.len() - 1
        };
        if *self.fail_at.lock().unwrap() == Some(call) {
            return Err("Workers AI HTTP 429 Too Many Requests".into());
        }
        let cosine = *self.cosine.lock().unwrap();
        let vectors = texts
            .iter()
            .map(|text| {
                let v = direction(text);
                let e = direction(&format!("other:{text}"));
                let along: f64 = v.iter().zip(&e).map(|(a, b)| a * b).sum();
                let w: Vec<f64> = e.iter().zip(&v).map(|(e, v)| e - along * v).collect();
                let norm = w.iter().map(|x| x * x).sum::<f64>().sqrt();
                let sine = (1.0 - cosine * cosine).sqrt();
                v.iter()
                    .zip(&w)
                    .map(|(v, w)| (cosine * v + sine * w / norm) as f32)
                    .collect()
            })
            .collect();
        Ok(Embedded {
            vectors,
            neurons: *self.neurons_per_text.lock().unwrap() * texts.len() as f64,
        })
    }
}

fn add_notice(state: &State, id: u32, historical: bool, published: &str) {
    state.board(BOARD).unwrap();
    let article = ArticleInput {
        external_id: id.to_string(),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        title: format!("공지 {id}"),
        author: None,
        category: None,
        published_at: published.into(),
        modified_at: None,
        original: String::new(),
        unsupported_body: false,
        body_html: format!("<p>본문 {id}</p>"),
        body_text: format!("본문 {id}"),
        resources: Vec::new(),
        etag: None,
        last_modified: None,
    };
    state.upsert(BOARD, &article, historical).unwrap();
}
fn input(id: u32) -> String {
    rhythm_embed::input(&format!("공지 {id}"), &format!("본문 {id}"))
}
fn embedded(state: &State) -> Vec<String> {
    state
        .with(|c| {
            c.prepare(
                "SELECT n.external_id FROM embeddings e JOIN notices n ON n.id=e.notice_id
                 ORDER BY n.external_id+0",
            )?
            .query_map([], |row| row.get(0))?
            .collect()
        })
        .unwrap()
}
/// One run on `day` with every vector bent by `cosine`; returns the canary check.
async fn band(state: &State, fake: &Fake, day: &str, cosine: f64) -> (String, Option<f64>) {
    *fake.cosine.lock().unwrap() = cosine;
    run_day(state, fake, day, Limits::default())
        .await
        .canary
        .unwrap()
}
async fn run_day(state: &State, fake: &Fake, day: &str, limits: Limits) -> Report {
    run(state, fake, day, limits).await.unwrap()
}

#[tokio::test]
async fn the_canary_is_recorded_then_checked_once_a_day_in_its_bands() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    let report = run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    assert_eq!(report.canary, Some(("recorded".into(), None)));
    assert_eq!(fake.calls(), 3, "28 texts in batches of 10");
    assert_eq!(fake.texts().len(), 28);
    run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    assert_eq!(fake.calls(), 3, "once a day");

    let (normal, min) = band(&state, &fake, "2026-11-19", 1.0).await;
    assert_eq!(normal, "normal");
    assert!(min.unwrap() > 0.99999, "{min:?}");
    // A warning still embeds new notices.
    add_notice(&state, 1, false, "2026-11-20");
    let (warning, min) = band(&state, &fake, "2026-11-20", 0.999).await;
    assert_eq!(warning, "warning");
    assert!((min.unwrap() - 0.999).abs() < 1e-4, "{min:?}");
    assert_eq!(embedded(&state), ["1"]);

    add_notice(&state, 2, false, "2026-11-21");
    let (stopped, _) = band(&state, &fake, "2026-11-21", 0.99).await;
    assert_eq!(stopped, "stopped");
    assert_eq!(embedded(&state), ["1"], "no new vectors after a drift stop");
    // The stop holds on later days without any request, until reset.
    let calls = fake.calls();
    *fake.cosine.lock().unwrap() = 1.0;
    let later = run_day(&state, &fake, "2026-11-22", Limits::default()).await;
    assert!(later.stopped.unwrap().contains("canary"));
    assert_eq!(fake.calls(), calls);
    state.reset_canary().unwrap();
    let reset = run_day(&state, &fake, "2026-11-22", Limits::default()).await;
    assert_eq!(reset.canary, Some(("recorded".into(), None)));
    assert_eq!(embedded(&state), ["1", "2"]);
}

#[tokio::test]
async fn new_notices_are_embedded_every_run_and_history_backfills_newest_first() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    for id in 1..=25 {
        add_notice(&state, id, true, &format!("2026-10-{id:02}"));
    }
    add_notice(&state, 100, false, "2026-11-01");
    let limits = Limits {
        backfill_per_run: 10,
        backfill_neurons: BACKFILL_NEURONS,
    };
    let report = run_day(&state, &fake, "2026-11-18", limits).await;
    assert_eq!((report.fresh, report.backfill), (1, 10));
    let mut first: Vec<String> = (16..=25).map(|id| id.to_string()).collect();
    first.push("100".into());
    assert_eq!(embedded(&state), first, "the newest ten historical notices");
    // The notice texts are the app's exact input.
    assert!(fake.texts().contains(&input(100)));

    // A revision is embedded again in the next run, before any backfill.
    add_notice(&state, 101, false, "2026-11-02");
    state
        .with(|c| c.execute_batch("UPDATE notices SET body_text='고친 본문', revision_hash='new' WHERE external_id='100'"))
        .unwrap();
    let report = run_day(&state, &fake, "2026-11-18", limits).await;
    assert_eq!((report.fresh, report.backfill), (2, 10));
    // 101 is new; 100 already had a vector of its old revision.
    assert_eq!(embedded(&state).len(), 22);

    // Vectors of another scope rejoin the backfill after never-embedded
    // notices; removed notices are never embedded.
    state
        .with(|c| {
            c.execute_batch(
                "UPDATE embeddings SET scope='old' WHERE notice_id IN
                   (SELECT id FROM notices WHERE external_id IN ('25','100'));
                 UPDATE notices SET removed_at='2026-11-18T00:00:00.000Z' WHERE external_id='1';",
            )
        })
        .unwrap();
    let before = fake.texts().len();
    let report = run_day(&state, &fake, "2026-11-18", limits).await;
    assert_eq!((report.fresh, report.backfill), (0, 6));
    let sent = &fake.texts()[before..];
    assert_eq!(
        sent,
        [
            input(5),
            input(4),
            input(3),
            input(2),
            rhythm_embed::input("공지 100", "고친 본문"),
            input(25)
        ]
    );
    assert!(!embedded(&state).contains(&"1".to_string()));
}

#[tokio::test]
async fn the_backfill_stops_at_the_daily_budget_but_new_notices_do_not() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    *fake.neurons_per_text.lock().unwrap() = 0.0;
    run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    for id in 1..=30 {
        add_notice(&state, id, true, &format!("2026-10-{id:02}"));
    }
    add_notice(&state, 100, false, "2026-11-01");
    *fake.neurons_per_text.lock().unwrap() = 100.0;
    let limits = Limits {
        backfill_per_run: 1_000,
        backfill_neurons: 1_500.0,
    };
    // 100 Neurons for the new notice, then batches until the day passes 1,500.
    let report = run_day(&state, &fake, "2026-11-18", limits).await;
    assert_eq!((report.fresh, report.backfill), (1, 20));
    assert_eq!(state.neurons("2026-11-18").unwrap(), 2_100.0);
    add_notice(&state, 101, false, "2026-11-02");
    let report = run_day(&state, &fake, "2026-11-18", limits).await;
    assert_eq!((report.fresh, report.backfill), (1, 0));
    // A new UTC day has a new budget (after its canary, 2,800 here).
    *fake.neurons_per_text.lock().unwrap() = 10.0;
    let report = run_day(&state, &fake, "2026-11-19", limits).await;
    assert_eq!(report.backfill, 10);
}

#[tokio::test]
async fn a_workers_ai_error_stops_embedding_for_the_run_and_keeps_stored_batches() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    for id in 1..=25 {
        add_notice(&state, id, false, "2026-11-01");
    }
    // The canary used calls 0..3; the notices' second batch fails.
    *fake.fail_at.lock().unwrap() = Some(4);
    let report = run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    assert_eq!(report.fresh, 10);
    assert!(report.stopped.unwrap().contains("429"));
    assert_eq!(fake.calls(), 5, "no request after the error");
    let error: Option<String> = state
        .with(|c| {
            c.query_row(
                "SELECT last_error FROM ai_usage WHERE day='2026-11-18'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert!(error.unwrap().contains("429"));
    // The next run continues with the rest.
    *fake.fail_at.lock().unwrap() = None;
    let report = run_day(&state, &fake, "2026-11-18", Limits::default()).await;
    assert_eq!(report.fresh, 15);
    assert_eq!(embedded(&state).len(), 25);
}

#[test]
fn a_canary_failure_records_no_check() {
    // Checked through the state: a failed canary leaves the day unchecked.
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    *fake.fail_at.lock().unwrap() = Some(1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let report = runtime
        .block_on(run(&state, &fake, "2026-11-18", Limits::default()))
        .unwrap();
    assert!(report.stopped.is_some());
    assert_eq!(state.canary_check("2026-11-18").unwrap(), None);
    assert!(state.canary_reference().unwrap().is_empty());
}

#[test]
fn f16_matches_ieee_binary16() {
    // Expected bits from Python's `struct` "e" format.
    for (value, bits) in [
        (0.0f32, 0x0000u16),
        (-0.0, 0x8000),
        (1.0, 0x3c00),
        (-2.0, 0xc000),
        (0.5, 0x3800),
        (0.1, 0x2e66),
        (-0.333333, 0xb555),
        (65504.0, 0x7bff),
        (1e-8, 0x0000),
        (6.1e-5, 0x03ff),
        (5.96e-8, 0x0001),
        (3e-8, 0x0001),
        (1.0 + 2f32.powi(-10), 0x3c01),
        // Ties round to even.
        (1.0 + 2f32.powi(-11), 0x3c00),
        (1.0 + 3.0 * 2f32.powi(-11), 0x3c02),
        (0.031249, 0x2800),
    ] {
        assert_eq!(f16_bits(value), bits, "{value}");
    }
    assert_eq!(f16_bits(65520.0), 0x7c00);
    assert_eq!(f16_bits(f32::INFINITY), 0x7c00);
    assert!(f16_value(f16_bits(f32::NAN)).is_nan());
    assert_eq!(f16_value(0x2e66), 1638.0 / 16384.0);
    assert_eq!(f16_value(0x0001), 2f32.powi(-24));
    assert_eq!(f16_value(0xc000), -2.0);
    // A unit vector survives the round trip.
    let vector: Vec<f32> = direction("round trip").iter().map(|x| *x as f32).collect();
    let decoded = decode(&encode(&vector));
    assert!(cosine(&vector, &decoded) > 0.99999);
}

#[test]
fn the_committed_canary_has_the_measured_28_texts() {
    let texts = canary_texts();
    assert_eq!(texts.len(), 28);
    assert!(texts[..18]
        .iter()
        .all(|(_, text)| text.starts_with(rhythm_embed::INSTRUCTION)));
    assert!(texts[18..]
        .iter()
        .all(|(_, text)| text.starts_with(rhythm_embed::SEARCH_INSTRUCTION)));
}
