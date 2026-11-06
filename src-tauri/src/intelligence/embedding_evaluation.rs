//! Opt-in real-weight evaluation for local embedding classification.
//!
//! This is not part of normal tests: it downloads nothing itself and returns
//! early unless `RHYTHM_EMBEDDING_EVAL_MODEL` names a completed GGUF file and
//! the pinned `llama-server` is staged. It uses the committed synthetic Korean
//! fixtures only, never the production profile.
use super::{
    analysis::CATEGORIES,
    classification::CATEGORY_PROTOTYPES,
    evaluation::{cases, Case},
    Worker,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

/// The production prototypes, so the evaluation measures what ships.
fn prototypes(category: &str) -> &'static [&'static str] {
    CATEGORY_PROTOTYPES
        .iter()
        .find(|(owner, _)| *owner == category)
        .map_or(&[], |(_, texts)| *texts)
}

fn instruction(mode: &str) -> &'static str {
    match mode {
        "qwen" => "Instruct: Classify the given Korean university notice into exactly one category: 학사, 장학, 수업·교육, 연구, 공모전·대회, 인턴·채용, 행사·활동, 기타.\nQuery: ",
        "e5" => "query: ",
        _ => "",
    }
}

fn prototype_prefix(mode: &str) -> &'static str {
    match mode {
        "e5" => "passage: ",
        _ => "",
    }
}

fn truncate_bytes(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn notice_text(case: &Case, body_bytes: usize) -> String {
    let body = case.body();
    // `body_bytes` bounds characters, then the whole classifier input is capped
    // by bytes because Korean characters are three bytes each.
    let body = body.chars().take(body_bytes).collect::<String>();
    truncate_bytes(format!("{}\n{}", case.title, body), 7 * 1024)
}

#[derive(Clone, Copy, Debug)]
enum Scoring {
    Max,
    TopTwoMean,
}

fn classify(vector: &[f32], prototypes: &[(&'static str, Vec<f32>)], scoring: Scoring) -> Value {
    let mut best: Vec<(&str, f32)> = Vec::new();
    for category in CATEGORIES {
        let mut scores = prototypes
            .iter()
            .filter(|(owner, _)| *owner == category)
            .map(|(_, prototype)| {
                prototype
                    .iter()
                    .zip(vector)
                    .map(|(a, b)| a * b)
                    .sum::<f32>()
            })
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| b.total_cmp(a));
        let score = match scoring {
            Scoring::Max => scores.first().copied().unwrap_or(-1.0),
            Scoring::TopTwoMean => {
                if scores.len() >= 2 {
                    (scores[0] + scores[1]) / 2.0
                } else {
                    scores.first().copied().unwrap_or(-1.0)
                }
            }
        };
        best.push((category, score));
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    json!({
        "category": best[0].0,
        "score": best[0].1,
        "margin": best[0].1 - best.get(1).map(|entry| entry.1).unwrap_or(-1.0),
        "scores": best.iter().map(|(category, score)| json!({"category": category, "score": score})).collect::<Vec<_>>(),
    })
}

async fn peak_rss(pid: Option<u32>, stop: Arc<AtomicBool>) -> Option<u64> {
    let pid = pid?;
    let mut peak = 0u64;
    while !stop.load(Ordering::Relaxed) {
        if let Some(current) = super::evaluation::worker_rss_kib(Some(pid)) {
            peak = peak.max(current);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    peak.into()
}

async fn embed_batch(
    worker: &AsyncMutex<Worker>,
    path: &Path,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let mut worker = worker.lock().await;
    let mut vectors = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(super::MAX_EMBED_BATCH) {
        vectors.extend(worker.embed(path, chunk).await?);
    }
    Ok(vectors)
}

fn report(value: &Value) {
    let text = serde_json::to_string_pretty(value).unwrap();
    eprintln!("{text}");
    if let Some(path) = std::env::var_os("RHYTHM_EMBEDDING_EVAL_REPORT") {
        let _ = std::fs::write(PathBuf::from(path), text);
    }
}

/// Run with:
/// `RHYTHM_EMBEDDING_EVAL_MODEL=/tmp/...gguf RHYTHM_EMBEDDING_EVAL_ID=qwen3`
/// `RHYTHM_EMBEDDING_EVAL_POOLING=last RHYTHM_EMBEDDING_EVAL_PREFIX=qwen`
/// `RHYTHM_EMBEDDING_EVAL_REPORT=/tmp/report.json`
/// `cargo test --locked --release -p rhythm actual_embedding_classification_evaluation -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires an explicit embedding GGUF and the staged llama-server"]
async fn actual_embedding_classification_evaluation() {
    let Some(llama) = super::staged_llama() else {
        eprintln!("SKIPPED: run `node scripts/stage-llama.mjs` first");
        return;
    };
    let model = PathBuf::from(
        std::env::var_os("RHYTHM_EMBEDDING_EVAL_MODEL").expect("embedding model path"),
    );
    assert!(model.is_file(), "missing model: {}", model.display());
    let id = std::env::var("RHYTHM_EMBEDDING_EVAL_ID").unwrap_or_else(|_| "embedding".into());
    let pooling = std::env::var("RHYTHM_EMBEDDING_EVAL_POOLING").unwrap_or_else(|_| "last".into());
    let prefix = std::env::var("RHYTHM_EMBEDDING_EVAL_PREFIX").unwrap_or_else(|_| "plain".into());
    let cases = cases();
    let runtime =
        std::env::temp_dir().join(format!("rhythm-embedding-eval-{}", std::process::id()));
    let mut configured = Worker::embedding_configured(&llama, &runtime, "auto", &pooling);
    if std::env::var("RHYTHM_EMBEDDING_EVAL_FORCE_CPU").as_deref() == Ok("1") {
        configured.force_cpu();
    }
    let worker = Arc::new(AsyncMutex::new(configured));
    let load_started = Instant::now();
    {
        let mut worker = worker.lock().await;
        worker
            .load(&model)
            .await
            .expect("embedding server must load the model");
    }
    let load_ms = load_started.elapsed().as_millis() as u64;
    let pid = { worker.lock().await.process_id() };
    let stop = Arc::new(AtomicBool::new(false));
    let monitor = tokio::spawn(peak_rss(pid, stop.clone()));

    let mut prototype_texts = Vec::new();
    let mut prototype_owners: Vec<&'static str> = Vec::new();
    for category in CATEGORIES {
        for text in prototypes(category) {
            prototype_texts.push(format!("{}{}", prototype_prefix(&prefix), text));
            prototype_owners.push(category);
        }
    }
    let started = Instant::now();
    let prototype_vectors = embed_batch(&worker, &model, &prototype_texts)
        .await
        .expect("prototype embeddings");
    let prototype_ms = started.elapsed().as_millis() as u64;
    let prototypes = prototype_owners
        .into_iter()
        .zip(prototype_vectors)
        .collect::<Vec<_>>();

    let mut variants = Vec::new();
    for body_bytes in [0usize, 300, 800, 1500] {
        let inputs = cases
            .iter()
            .map(|case| format!("{}{}", instruction(&prefix), notice_text(case, body_bytes)))
            .collect::<Vec<_>>();
        let started = Instant::now();
        let vectors = embed_batch(&worker, &model, &inputs)
            .await
            .expect("notice embeddings");
        let elapsed_ms = started.elapsed().as_millis() as u64;
        for scoring in [Scoring::Max, Scoring::TopTwoMean] {
            let predictions = cases
                .iter()
                .zip(&vectors)
                .map(|(case, vector)| {
                    let prediction = classify(vector, &prototypes, scoring);
                    json!({
                        "id": case.id,
                        "expected": case.category,
                        "predicted": prediction["category"],
                        "correct": prediction["category"] == case.category,
                        "score": prediction["score"],
                        "margin": prediction["margin"],
                    })
                })
                .collect::<Vec<_>>();
            let correct = predictions
                .iter()
                .filter(|prediction| prediction["correct"] == true)
                .count();
            let mean_seconds = elapsed_ms as f64 / 1000.0 / cases.len() as f64;
            variants.push(json!({
                "body_bytes": body_bytes,
                "scoring": match scoring { Scoring::Max => "max", Scoring::TopTwoMean => "top_two_mean" },
                "correct": correct,
                "total": cases.len(),
                "elapsed_ms": elapsed_ms,
                "mean_ms_per_notice": mean_seconds * 1000.0,
                "predictions": predictions,
            }));
        }
    }
    stop.store(true, Ordering::Relaxed);
    let peak_rss_kib = monitor.await.ok().flatten();
    {
        let mut worker = worker.lock().await;
        worker.shutdown().await;
    }
    let _ = std::fs::remove_dir_all(&runtime);
    report(&json!({
        "id": id,
        "model": model.to_string_lossy(),
        "pooling": pooling,
        "forced_cpu": std::env::var("RHYTHM_EMBEDDING_EVAL_FORCE_CPU").as_deref() == Ok("1"),
        "prefix": prefix,
        "dimension": prototypes.first().map(|(_, vector)| vector.len()),
        "prototype_count": prototypes.len(),
        "prototype_ms": prototype_ms,
        "load_ms": load_ms,
        "peak_rss_kib": peak_rss_kib,
        "variants": variants,
    }));
}
