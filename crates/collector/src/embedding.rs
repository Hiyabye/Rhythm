//! Notice embeddings for the feed (#166 D15, D25–D27; user decisions
//! 2026-11-17/18). After the crawl, each run:
//!
//! 1. checks the drift canary once per UTC day: the first check records the
//!    reference vectors; a minimum cosine of at least 0.9995 is normal, below
//!    0.9970 stops new vectors until `State::reset_canary`, and anything
//!    between is a warning;
//! 2. embeds new and changed notices;
//! 3. backfills the rest, newest first, at most `BACKFILL_PER_RUN` notices and
//!    only while the day's Neurons stay below `BACKFILL_NEURONS`, which leaves
//!    the rest of the free 10,000 for new notices and the canary;
//! 4. classifies every current vector of the scope against the category
//!    prototypes (#166 O9), which are embedded once per scope and cached.
//!
//! Any Workers AI error stops embedding for the run, but cached prototypes
//! still classify the vectors already stored; crawling and publishing never
//! wait for it.
use crate::{
    sha256_hex,
    state::{State, Unclassified, Unembedded},
    workers_ai::{Embedder, BATCH, DIMENSIONS, MODEL},
};

pub const BACKFILL_PER_RUN: usize = 1_000;
pub const BACKFILL_NEURONS: f64 = 9_000.0;
/// Vectors classified per state transaction.
const CLASSIFY_BATCH: usize = 1_000;
const NORMAL: f64 = 0.9995;
const STOP: f64 = 0.9970;

/// The exact inputs measured in #170, independent of the app's test fixtures.
const CANARY: &str = include_str!("canary.json");

/// The vector scope: the model and the input contract.
pub fn scope() -> String {
    format!("{MODEL}#{}", rhythm_embed::config_hash())
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub backfill_per_run: usize,
    pub backfill_neurons: f64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            backfill_per_run: BACKFILL_PER_RUN,
            backfill_neurons: BACKFILL_NEURONS,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// The day's canary state and minimum cosine, when this run checked it.
    pub canary: Option<(String, Option<f64>)>,
    pub fresh: usize,
    pub backfill: usize,
    pub classified: usize,
    pub neurons: f64,
    /// Why embedding stopped early: a Workers AI error or the drift stop.
    pub stopped: Option<String>,
}

/// Embeds within the day's budget. Only a state error is returned as `Err`.
pub async fn run<E: Embedder>(
    state: &State,
    ai: &E,
    day: &str,
    limits: Limits,
) -> Result<Report, String> {
    let mut report = Report::default();
    if state.canary_stopped()? {
        report.stopped = Some("the drift canary stopped new vectors".into());
        return Ok(report);
    }
    if state.canary_check(day)?.is_none() {
        match canary(state, ai, day).await? {
            Ok((check, neurons)) => {
                report.neurons += neurons;
                let stopped = check.0 == "stopped";
                report.canary = Some(check);
                if stopped {
                    report.stopped = Some("the drift canary stopped new vectors".into());
                    return Ok(report);
                }
            }
            Err(error) => return stop(state, day, report, error),
        }
    }
    let scope = scope();
    let fresh = state.unembedded(&scope, true, usize::MAX)?;
    let mut failure = embed(state, ai, day, &scope, &fresh, &mut report, None)
        .await?
        .err();
    if failure.is_none() && state.neurons(day)? < limits.backfill_neurons {
        let backfill = state.unembedded(&scope, false, limits.backfill_per_run)?;
        let budget = Some(limits.backfill_neurons);
        failure = embed(state, ai, day, &scope, &backfill, &mut report, budget)
            .await?
            .err();
    }
    // After an error, only prototypes cached by an earlier run classify.
    let may_embed = failure.is_none();
    let prototypes = match prototypes(state, ai, day, &scope, may_embed, &mut report).await? {
        Ok(prototypes) => prototypes,
        Err(error) => {
            failure = Some(error);
            None
        }
    };
    if let Some(prototypes) = prototypes {
        report.classified = classify(state, &scope, &prototypes)?;
    }
    match failure {
        Some(error) => stop(state, day, report, error),
        None => Ok(report),
    }
}

fn stop(state: &State, day: &str, mut report: Report, error: String) -> Result<Report, String> {
    state.record_ai_error(day, &error)?;
    report.stopped = Some(error);
    Ok(report)
}

/// The scope's prototypes with their categories: cached, or embedded now when
/// `may_embed`. `None` when they are neither.
async fn prototypes<E: Embedder>(
    state: &State,
    ai: &E,
    day: &str,
    scope: &str,
    may_embed: bool,
    report: &mut Report,
) -> Result<Result<Option<Vec<(&'static str, Vec<f32>)>>, String>, String> {
    let inputs = rhythm_embed::prototype_inputs();
    let mut vectors = state.prototypes(scope)?;
    if vectors.len() != inputs.len() {
        if !may_embed {
            return Ok(Ok(None));
        }
        vectors.clear();
        for batch in inputs.chunks(BATCH) {
            let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
            match ai.embed(&texts).await {
                Ok(embedded) => {
                    state.add_neurons(day, embedded.neurons)?;
                    report.neurons += embedded.neurons;
                    vectors.extend(embedded.vectors.iter().map(|vector| encode(vector)));
                }
                Err(error) => return Ok(Err(error)),
            }
        }
        state.store_prototypes(scope, &vectors)?;
    }
    Ok(Ok(Some(
        inputs
            .iter()
            .zip(&vectors)
            .map(|((category, _), vector)| (*category, decode(vector)))
            .collect(),
    )))
}

/// Classifies every unclassified current vector of the scope.
fn classify(
    state: &State,
    scope: &str,
    prototypes: &[(&'static str, Vec<f32>)],
) -> Result<usize, String> {
    let mut count = 0;
    loop {
        let notices: Vec<Unclassified> = state.unclassified(scope, CLASSIFY_BATCH)?;
        let rows: Vec<_> = notices
            .iter()
            .map(|notice| {
                let mut classified =
                    rhythm_embed::classify_with(&decode(&notice.vector), prototypes);
                // f16 rounding can put near-identical vectors just above 1.
                classified.score = classified.score.clamp(-1.0, 1.0);
                (notice, classified)
            })
            .collect();
        state.store_classifications(scope, &rows)?;
        count += rows.len();
        if notices.len() < CLASSIFY_BATCH {
            return Ok(count);
        }
    }
}

/// Embeds the notices in batches, storing each batch as it arrives. A
/// backfill passes its daily Neuron budget and stops once the day reaches it.
/// The outer `Result` is the state; the inner one is Workers AI.
async fn embed<E: Embedder>(
    state: &State,
    ai: &E,
    day: &str,
    scope: &str,
    notices: &[Unembedded],
    report: &mut Report,
    backfill_budget: Option<f64>,
) -> Result<Result<(), String>, String> {
    for batch in notices.chunks(BATCH) {
        if let Some(budget) = backfill_budget {
            if state.neurons(day)? >= budget {
                break;
            }
        }
        let texts: Vec<String> = batch
            .iter()
            .map(|notice| rhythm_embed::input(&notice.title, &notice.body_text))
            .collect();
        let embedded = match ai.embed(&texts).await {
            Ok(embedded) => embedded,
            Err(error) => return Ok(Err(error)),
        };
        let vectors: Vec<_> = batch
            .iter()
            .zip(&embedded.vectors)
            .map(|(notice, vector)| (notice, encode(vector)))
            .collect();
        state.store_embeddings(scope, day, &vectors, embedded.neurons)?;
        report.neurons += embedded.neurons;
        if backfill_budget.is_some() {
            report.backfill += batch.len();
        } else {
            report.fresh += batch.len();
        }
    }
    Ok(Ok(()))
}

/// The canary texts with their SHA-256.
fn canary_texts() -> Vec<(String, String)> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(CANARY).expect("committed canary texts");
    entries
        .iter()
        .map(|entry| {
            let text = entry["text"].as_str().expect("canary text").to_owned();
            (sha256_hex(&text), text)
        })
        .collect()
}

/// Embeds the canary and records the day's check. Returns its state, minimum
/// cosine and Neurons, or the Workers AI error.
async fn canary<E: Embedder>(
    state: &State,
    ai: &E,
    day: &str,
) -> Result<Result<((String, Option<f64>), f64), String>, String> {
    let texts = canary_texts();
    let mut vectors = Vec::new();
    let mut neurons = 0.0;
    for batch in texts.chunks(BATCH) {
        let batch: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();
        match ai.embed(&batch).await {
            Ok(embedded) => {
                state.add_neurons(day, embedded.neurons)?;
                neurons += embedded.neurons;
                vectors.extend(embedded.vectors);
            }
            Err(error) => return Ok(Err(error)),
        }
    }
    let reference = state.canary_reference()?;
    let same_texts = reference.len() == texts.len()
        && reference
            .iter()
            .zip(&texts)
            .all(|((stored, _), (current, _))| stored == current);
    if !same_texts {
        // A first check, or a deliberately changed canary set.
        let encoded: Vec<_> = texts
            .iter()
            .zip(&vectors)
            .map(|((sha256, _), vector)| (sha256.clone(), encode(vector)))
            .collect();
        state.record_canary(day, "recorded", None, Some(&encoded))?;
        return Ok(Ok((("recorded".into(), None), neurons)));
    }
    let min = reference
        .iter()
        .zip(&vectors)
        .map(|((_, stored), vector)| cosine(&decode(stored), vector))
        .fold(f64::INFINITY, f64::min);
    let check = if min >= NORMAL {
        "normal"
    } else if min < STOP {
        "stopped"
    } else {
        "warning"
    };
    state.record_canary(day, check, Some(min), None)?;
    Ok(Ok(((check.into(), Some(min)), neurons)))
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    dot / (norm(a) * norm(b))
}

/// A vector as little-endian f16 values (O19b).
pub fn encode(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|x| f16_bits(*x).to_le_bytes())
        .collect()
}
pub fn decode(bytes: &[u8]) -> Vec<f32> {
    debug_assert_eq!(bytes.len(), DIMENSIONS * 2);
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f16_value(u16::from_le_bytes(*pair)))
        .collect()
}

/// IEEE 754 binary16, rounding to nearest even.
fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    // The significand with its implicit bit, and how far it is shifted out.
    let (significand, shift, base) = if half_exponent > 0 {
        (mantissa, 13, (half_exponent as u32) << 10)
    } else if half_exponent >= -10 {
        (mantissa | 0x80_0000, (14 - half_exponent) as u32, 0)
    } else {
        return sign;
    };
    let kept = significand >> shift;
    let rest = significand & ((1 << shift) - 1);
    let halfway = 1 << (shift - 1);
    let round = rest > halfway || (rest == halfway && kept & 1 == 1);
    // A carry out of the mantissa moves into the exponent, which is correct.
    sign | (base + kept + u32::from(round)) as u16
}

fn f16_value(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        0x1f if mantissa == 0.0 => f32::INFINITY,
        0x1f => f32::NAN,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

#[cfg(test)]
mod tests;
