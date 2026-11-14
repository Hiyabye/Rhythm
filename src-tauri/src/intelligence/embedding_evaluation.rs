//! Opt-in real-weight evaluation for local embedding classification.
//!
//! This is not part of normal tests: it downloads nothing itself and returns
//! early unless `RHYTHM_EMBEDDING_EVAL_MODEL` names a completed GGUF file and
//! the pinned `llama-server` is staged. It uses the committed synthetic Korean
//! fixtures only, never the production profile.
use super::{
    classification::{CATEGORIES, CATEGORY_PROTOTYPES},
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
    let mut configured = Worker::configured(&llama, &runtime, "auto", &pooling);
    if std::env::var("RHYTHM_EMBEDDING_EVAL_FORCE_CPU").as_deref() == Ok("1") {
        configured.force_cpu();
    }
    let worker = Arc::new(AsyncMutex::new(configured));
    let load_started = Instant::now();
    {
        // The first request starts the server and loads the model.
        let mut worker = worker.lock().await;
        worker
            .embed(&model, &["load".into()])
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

/// Paraphrased Korean queries and the fixtures they should find. They avoid the
/// fixture titles' own words where Korean allows, so they measure what full-text
/// search misses. Authored for this evaluation, not taken from real searches.
const SEARCH_QUERIES: [(&str, &[&str]); 12] = [
    (
        "생활비 지원",
        &[
            "scholarship",
            "complex_eligibility",
            "incomplete_source",
            "eligibility_exclusions",
        ],
    ),
    (
        "등록금 부담 줄이는 방법",
        &[
            "scholarship",
            "complex_eligibility",
            "incomplete_source",
            "eligibility_exclusions",
        ],
    ),
    ("학교를 잠시 쉬고 싶어요", &["academic"]),
    ("방학 동안 회사 실무 경험", &["internship", "time_purposes"]),
    ("빅데이터 시합", &["competition", "formatted_table"]),
    (
        "교수님 연구실에서 일해 보기",
        &["research", "relative_date"],
    ),
    ("취업 진로 강연", &["multiple_dates"]),
    ("논문 자료 찾는 법", &["no_deadline"]),
    ("책 읽는 모임", &["yearless_deadline"]),
    ("개발자 발표회", &["event_only"]),
    ("코딩 일자리", &["internship", "time_purposes"]),
    ("아이디어 제안 공모", &["formatted_table", "competition"]),
];

/// Run like `actual_embedding_classification_evaluation`:
/// `RHYTHM_EMBEDDING_EVAL_MODEL=/tmp/...gguf RHYTHM_EMBEDDING_EVAL_REPORT=/tmp/search.json`
/// `cargo test --locked --release -p rhythm actual_semantic_search_evaluation -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires an explicit embedding GGUF and the staged llama-server"]
async fn actual_semantic_search_evaluation() {
    let Some(llama) = super::staged_llama() else {
        eprintln!("SKIPPED: run `node scripts/stage-llama.mjs` first");
        return;
    };
    let model = PathBuf::from(
        std::env::var_os("RHYTHM_EMBEDDING_EVAL_MODEL").expect("embedding model path"),
    );
    assert!(model.is_file(), "missing model: {}", model.display());
    let cases = cases();
    let runtime = std::env::temp_dir().join(format!("rhythm-search-eval-{}", std::process::id()));
    let worker = AsyncMutex::new(Worker::configured(&llama, &runtime, "auto", "last"));
    // Notices are embedded exactly as the production classifier stores them.
    let notices = cases
        .iter()
        .map(|case| super::classification::input(&case.title, &case.body()))
        .collect::<Vec<_>>();
    let notice_vectors = embed_batch(&worker, &model, &notices)
        .await
        .expect("notice embeddings");
    let instructions = [
        ("retrieval", "Instruct: Given a Korean search query, retrieve relevant Korean university notices\nQuery: "),
        // The shipped instruction; the others are the alternatives it was chosen over.
        ("production", super::classification::SEARCH_INSTRUCTION),
        ("classify", super::classification::INSTRUCTION),
        ("plain", ""),
    ];
    let mut variants = Vec::new();
    for (name, instruction) in instructions {
        let inputs = SEARCH_QUERIES
            .iter()
            .map(|(query, _)| format!("{instruction}{query}"))
            .collect::<Vec<_>>();
        let started = Instant::now();
        let vectors = embed_batch(&worker, &model, &inputs)
            .await
            .expect("query embeddings");
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut reciprocal = 0.0;
        let mut hits_at_3 = 0;
        let mut relevant_scores = Vec::new();
        let mut other_scores = Vec::new();
        let mut queries = Vec::new();
        for ((query, relevant), vector) in SEARCH_QUERIES.iter().zip(&vectors) {
            let mut ranked = cases
                .iter()
                .zip(&notice_vectors)
                .map(|(case, notice)| {
                    let score = notice.iter().zip(vector).map(|(a, b)| a * b).sum::<f32>();
                    (case.id.as_str(), score)
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            let first = ranked
                .iter()
                .position(|(id, _)| relevant.contains(id))
                .map(|rank| rank + 1)
                .unwrap_or(ranked.len() + 1);
            reciprocal += 1.0 / first as f64;
            if first <= 3 {
                hits_at_3 += 1;
            }
            for (id, score) in &ranked {
                if relevant.contains(id) {
                    relevant_scores.push(*score);
                } else {
                    other_scores.push(*score);
                }
            }
            queries.push(json!({
                "query": query,
                "first_relevant_rank": first,
                "ranked": ranked.iter().map(|(id, score)| json!({"id": id, "score": score, "relevant": relevant.contains(id)})).collect::<Vec<_>>(),
            }));
        }
        let stats = |scores: &mut Vec<f32>| {
            scores.sort_by(|a, b| a.total_cmp(b));
            json!({
                "min": scores.first(),
                "median": scores.get(scores.len() / 2),
                "p90": scores.get(scores.len() * 9 / 10),
                "max": scores.last(),
            })
        };
        variants.push(json!({
            "instruction": name,
            "mrr": reciprocal / SEARCH_QUERIES.len() as f64,
            "hits_at_3": hits_at_3,
            "queries_total": SEARCH_QUERIES.len(),
            "mean_ms_per_query": elapsed_ms as f64 / SEARCH_QUERIES.len() as f64,
            "relevant_scores": stats(&mut relevant_scores),
            "other_scores": stats(&mut other_scores),
            "queries": queries,
        }));
    }
    // The shipped path end to end: stored vectors, the production query
    // instruction and the store's fused ranking, against text-only ranking.
    let directory = tempfile::tempdir().unwrap();
    let store = crate::storage::Store::open(&directory.path().join("search.sqlite3")).unwrap();
    super::evaluation::seed(&store);
    let sha = "e".repeat(64);
    let mut ids = std::collections::HashMap::new();
    for (case, vector) in cases.iter().zip(&notice_vectors) {
        let (id, _) = store
            .upsert_article("ai-evaluation", &case.article(), false)
            .unwrap();
        let revision = store.document(id, None).unwrap().revision_id;
        store.ai_save_embedding(id, revision, &sha, vector).unwrap();
        ids.insert(id, case.id.clone());
    }
    let inputs = SEARCH_QUERIES
        .iter()
        .map(|(query, _)| format!("{}{query}", super::classification::SEARCH_INSTRUCTION))
        .collect::<Vec<_>>();
    let query_vectors = embed_batch(&worker, &model, &inputs)
        .await
        .expect("query embeddings");
    let mut fused_queries = Vec::new();
    let (mut fused_hits, mut text_hits) = (0, 0);
    for ((query, relevant), vector) in SEARCH_QUERIES.iter().zip(&query_vectors) {
        let request = crate::archive::model::ArchiveQuery {
            query: (*query).into(),
            limit: Some(5),
            ..Default::default()
        };
        let named = |page: &crate::archive::model::ArchivePage| {
            page.items
                .iter()
                .map(|item| ids[&item.id].clone())
                .collect::<Vec<_>>()
        };
        let text = store.archive_ranked_query(&request, None).unwrap();
        let fused = store
            .archive_ranked_query(&request, Some((&sha, vector)))
            .unwrap();
        let hit = |names: &[String]| names.iter().any(|name| relevant.contains(&name.as_str()));
        let (text_names, fused_names) = (named(&text), named(&fused));
        text_hits += usize::from(hit(&text_names));
        fused_hits += usize::from(hit(&fused_names));
        fused_queries.push(json!({
            "query": query,
            "text_total": text.total,
            "text_top5": text_names,
            "fused_total": fused.total,
            "fused_top5": fused_names,
            "meaning_only": fused.meaning_only.iter().map(|id| ids[id].clone()).collect::<Vec<_>>(),
        }));
    }
    worker.lock().await.shutdown().await;
    let _ = std::fs::remove_dir_all(&runtime);
    report(&json!({
        "model": model.to_string_lossy(),
        "variants": variants,
        "store": {
            "text_queries_with_relevant_in_top5": text_hits,
            "fused_queries_with_relevant_in_top5": fused_hits,
            "queries_total": SEARCH_QUERIES.len(),
            "queries": fused_queries,
        },
    }));
}
