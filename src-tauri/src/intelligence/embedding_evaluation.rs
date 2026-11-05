//! Opt-in real-weight evaluation for local embedding classification.
//!
//! This is not part of normal tests: it downloads nothing itself and returns
//! early unless `RHYTHM_EMBEDDING_EVAL_MODEL` names a completed GGUF file and
//! the pinned `llama-server` is staged. It uses the committed synthetic Korean
//! fixtures only, never the production profile.
use super::{analysis::CATEGORIES, Worker};
use serde::Deserialize;
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

#[derive(Deserialize)]
struct Case {
    id: String,
    title: String,
    text: String,
    category: String,
    #[serde(default)]
    repeat_text: String,
    #[serde(default)]
    repeat_count: usize,
    #[serde(default)]
    suffix: String,
}
impl Case {
    fn body(&self) -> String {
        format!(
            "{}{}{}",
            self.text,
            self.repeat_text.repeat(self.repeat_count),
            self.suffix
        )
    }
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("evaluation.json")).expect("valid evaluation fixtures")
}

/// Authored Korean prototypes, not the evaluation cases. They encode the same
/// public category definitions the generation prompt uses.
fn prototypes(category: &str) -> &'static [&'static str] {
    match category {
        "학사" => &[
            "학사 운영 안내: 휴학, 복학, 졸업, 학적 변경, 성적 정정, 수강신청, 등록금 납부, 학위 수여",
            "2026학년도 학사일정과 수강신청 기간을 안내합니다.",
            "졸업 요건 확인 및 학위 논문 제출 안내",
            "전과, 다전공, 부전공 신청 안내",
            "학생证 재발급과 재학증명서 발급 안내",
        ],
        "장학" => &[
            "장학금 신청 안내: 성적우수 장학금, 국가장학금, 교내외 장학금, 등록금과 생활비 지원",
            "희망장학금 신청 자격과 제출 서류를 안내합니다.",
            "복수 조건 장학금 지원 자격 안내",
            "중복 지원 제한이 있는 장학금 안내",
            "생활비 지원 장학생 선발 공고",
        ],
        "수업·교육" => &[
            "교육 과정과 강좌 수강생 모집 안내: 교과목, 비교과 프로그램, 학습 지원",
            "도서관 이용 교육 및 학술정보 검색 방법 안내",
            "교육 프로그램 신청 일정 정정 안내",
            "영어 강좌, 소프트웨어 교육, 자격증 과정 수강 안내",
            "교수학습센터 학습법 특강과 튜터링 프로그램 안내",
        ],
        "연구" => &[
            "학부생 연구 참여자 모집: 연구실 인턴, 학부 연구 프로그램, 공동 연구 과제",
            "하계 연구 체험 프로그램 사전 안내",
            "연구비 신청과 논문 작성 지원 안내",
            "학술 연구 과제 공모와 연구실 소개",
            "연구 참여 학생 모집 및 연구 주제 안내",
        ],
        "공모전·대회" => &[
            "공모전과 경진대회 참가자 모집 안내: 아이디어 공모, 데이터 분석 대회, 해커톤",
            "대학생 연구 아이디어 공모 일정표",
            "창업 경진대회와 캡스톤 디자인 대회 안내",
            "프로그래밍 대회와 알고리즘 경진대회 참가 신청",
            "작품 공모전 수상작 발표",
        ],
        "인턴·채용" => &[
            "인턴 모집과 채용 공고 안내: 하계 인턴, 현장실습, 신입 채용, 커리어 프로그램",
            "하계 인턴 접수 마감 및 면접 시간 안내",
            "기업 채용 설명회와 직무 체험 안내",
            "현장실습생 모집과 채용 연계 프로그램",
            "취업 준비 특강과 채용 정보 안내",
        ],
        "행사·활동" => &[
            "행사와 활동 안내: 특강, 세미나, 축제, 동아리, 진로 프로그램, 독서 모임",
            "오픈소스 기술 세미나 개최 안내",
            "진로 특강 사전 신청 및 행사 일정",
            "독서 모임 참가 신청 안내",
            "교내 축제와 학생 활동 프로그램 안내",
        ],
        _ => &[],
    }
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

fn rss_kib(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

async fn peak_rss(pid: Option<u32>, stop: Arc<AtomicBool>) -> Option<u64> {
    let pid = pid?;
    let mut peak = 0u64;
    while !stop.load(Ordering::Relaxed) {
        if let Some(current) = rss_kib(pid) {
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

fn report(name: &str, value: &Value) {
    let text = serde_json::to_string_pretty(value).unwrap();
    eprintln!("{text}");
    if let Some(path) = std::env::var_os("RHYTHM_EMBEDDING_EVAL_REPORT") {
        let _ = std::fs::write(PathBuf::from(path), text);
    }
    let _ = name;
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
    report(
        &id,
        &json!({
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
        }),
    );
}
