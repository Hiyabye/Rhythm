//! The Workers AI embedding client (#166 D25/D26): the app's exact prefixed
//! input strings through the `text` field of `@cf/qwen/qwen3-embedding-0.6b`.
//! Every vector is checked and normalized before it reaches the state.
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::{future::Future, time::Duration};

pub const MODEL: &str = "@cf/qwen/qwen3-embedding-0.6b";
pub const DIMENSIONS: usize = 1024;
/// Texts per request, as measured in #170.
pub const BATCH: usize = 10;
/// Workers AI bills this model at 1,075 Neurons per million input tokens,
/// used when a response reports tokens but no Neurons.
const NEURONS_PER_TOKEN: f64 = 1_075.0 / 1_000_000.0;
const ATTEMPTS: u32 = 3;

/// Unit vectors in input order and the Neurons the request used.
pub struct Embedded {
    pub vectors: Vec<Vec<f32>>,
    pub neurons: f64,
}

/// Any error stops embedding for the run (quota, 429, authentication or an
/// unusable response); crawling and publishing go on without new vectors.
pub trait Embedder {
    fn embed(&self, texts: &[String]) -> impl Future<Output = Result<Embedded, String>>;
}

pub struct WorkersAi {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl WorkersAi {
    pub fn new(account_id: &str, token: String) -> Result<Self, String> {
        // The account ID becomes part of the URL path.
        if account_id.len() != 32 || !account_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("the Cloudflare account ID must be 32 hex digits".into());
        }
        if token.is_empty() || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("the Workers AI token is not a plain token".into());
        }
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            url: format!(
                "https://api.cloudflare.com/client/v4/accounts/{}/ai/run/{MODEL}",
                account_id.to_ascii_lowercase()
            ),
            token,
        })
    }

    /// Reads `CF_ACCOUNT_ID` and `CF_AI_TOKEN`.
    pub fn from_env() -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
        Self::new(&var("CF_ACCOUNT_ID")?, var("CF_AI_TOKEN")?)
    }
}

impl Embedder for WorkersAi {
    async fn embed(&self, texts: &[String]) -> Result<Embedded, String> {
        split(self, texts).await
    }
}

/// Why one request failed: only an input too big for the model is retried,
/// in smaller requests.
enum Failure {
    TooBig(String),
    Other(String),
}

/// One request for some texts, as `split` sends it.
trait Request {
    fn request(&self, texts: &[String]) -> impl Future<Output = Result<Embedded, Failure>>;
}

impl Request for WorkersAi {
    async fn request(&self, texts: &[String]) -> Result<Embedded, Failure> {
        let mut failure = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2 << attempt)).await;
            }
            let response = self
                .http
                .post(&self.url)
                .bearer_auth(&self.token)
                .header("content-type", "application/json")
                .body(json!({ "text": texts }).to_string())
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_server_error() => {
                    failure = format!("HTTP {}", response.status());
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response
                        .text()
                        .await
                        .map_err(|e| Failure::Other(e.to_string()))?;
                    if status != StatusCode::OK {
                        let error = format!("Workers AI HTTP {status}{}", errors(&body));
                        return Err(if status == StatusCode::BAD_REQUEST && too_big(&body) {
                            Failure::TooBig(error)
                        } else {
                            Failure::Other(error)
                        });
                    }
                    return parse(&body, texts.len()).map_err(Failure::Other);
                }
                Err(error) => failure = error.without_url().to_string(),
            }
        }
        Err(Failure::Other(format!("Workers AI: {failure}")))
    }
}

/// Sends the texts in one request, or in halves while Workers AI answers that
/// the input is too big. The limit is on the tokens of the whole request,
/// summed over its texts: measured 2026-11-18 after the first production run
/// hit it, 27,565 tokens passed and about 33,000 failed (`3030 input too
/// big`). A single input is at most 7 KiB (`rhythm_embed::input`), at most
/// about 5,500 tokens in the same measurement, so the halving ends.
async fn split<R: Request>(requests: &R, texts: &[String]) -> Result<Embedded, String> {
    let mut embedded = Embedded {
        vectors: Vec::with_capacity(texts.len()),
        neurons: 0.0,
    };
    // A stack of the parts still to send, the next one on top.
    let mut parts = vec![texts];
    while let Some(part) = parts.pop() {
        match requests.request(part).await {
            Ok(done) => {
                embedded.vectors.extend(done.vectors);
                embedded.neurons += done.neurons;
            }
            Err(Failure::TooBig(_)) if part.len() > 1 => {
                let (first, second) = part.split_at(part.len() / 2);
                parts.extend([second, first]);
            }
            Err(Failure::TooBig(error) | Failure::Other(error)) => return Err(error),
        }
    }
    Ok(embedded)
}

/// Whether Cloudflare's errors say the input is too big for the model.
fn too_big(body: &str) -> bool {
    serde_json::from_str::<Value>(body).is_ok_and(|value| {
        value["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|error| {
                error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input too big"))
            })
    })
}

/// Cloudflare's error codes and messages, for the run log.
fn errors(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    let list: Vec<String> = value["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|error| {
            let message: String = error["message"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            format!("{} {message}", error["code"])
        })
        .collect();
    if list.is_empty() {
        String::new()
    } else {
        format!(": {}", list.join("; "))
    }
}

fn parse(body: &str, count: usize) -> Result<Embedded, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|_| "Workers AI returned no JSON".to_string())?;
    if value["success"] != Value::Bool(true) {
        return Err(format!("Workers AI reported a failure{}", errors(body)));
    }
    let result = &value["result"];
    let rows = result["data"]
        .as_array()
        .filter(|rows| rows.len() == count)
        .ok_or("Workers AI returned a different number of vectors")?;
    let mut vectors = Vec::with_capacity(count);
    for row in rows {
        let vector: Vec<f32> = row
            .as_array()
            .filter(|row| row.len() == DIMENSIONS)
            .ok_or("Workers AI returned a vector of another size")?
            .iter()
            .map(|x| x.as_f64().map(|x| x as f32).filter(|x| x.is_finite()))
            .collect::<Option<_>>()
            .ok_or("Workers AI returned a non-numeric vector")?;
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if !norm.is_normal() {
            return Err("Workers AI returned a zero vector".into());
        }
        vectors.push(vector.iter().map(|x| x / norm).collect());
    }
    let usage = &result["usage"];
    let neurons = usage["neurons"]
        .as_f64()
        .unwrap_or_else(|| usage["prompt_tokens"].as_f64().unwrap_or_default() * NEURONS_PER_TOKEN);
    Ok(Embedded { vectors, neurons })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(rows: usize, dims: usize, usage: Value) -> String {
        let row: Vec<f32> = (0..dims).map(|i| if i == 0 { 3.0 } else { 0.0 }).collect();
        json!({
            "success": true,
            "errors": [],
            "result": { "shape": [rows, dims], "data": vec![row; rows], "usage": usage }
        })
        .to_string()
    }

    #[test]
    fn vectors_are_checked_and_normalized() {
        let parsed = parse(
            &response(2, DIMENSIONS, json!({"prompt_tokens": 40, "neurons": 0.05})),
            2,
        )
        .unwrap();
        assert_eq!(parsed.vectors.len(), 2);
        assert_eq!(parsed.vectors[0][0], 1.0);
        assert_eq!(parsed.neurons, 0.05);
        // Without a Neuron count the token price is used.
        let tokens = parse(&response(1, DIMENSIONS, json!({"prompt_tokens": 1000})), 1).unwrap();
        assert!((tokens.neurons - 1.075).abs() < 1e-9);
    }

    #[test]
    fn unusable_responses_are_errors() {
        assert!(parse(&response(1, DIMENSIONS, json!({})), 2).is_err());
        assert!(parse(&response(2, 512, json!({})), 2).is_err());
        assert!(parse(
            &response(1, DIMENSIONS, json!({})).replace("3.0", "null"),
            1
        )
        .is_err());
        assert!(parse(&response(1, DIMENSIONS, json!({})).replace("3.0", "0.0"), 1).is_err());
        assert!(parse("<html>", 1).is_err());
        let failed = json!({"success": false, "errors": [{"code": 4006, "message": "daily free allocation used"}]});
        let error = parse(&failed.to_string(), 1).err().unwrap();
        assert!(error.contains("4006"), "{error}");
    }

    /// Answers like Workers AI with a context of `limit` bytes per request:
    /// each vector points along the axis its text names.
    struct Sized {
        limit: usize,
        calls: std::cell::RefCell<Vec<usize>>,
    }
    impl Request for Sized {
        async fn request(&self, texts: &[String]) -> Result<Embedded, Failure> {
            self.calls.borrow_mut().push(texts.len());
            if texts.iter().map(String::len).sum::<usize>() > self.limit {
                return Err(Failure::TooBig("input too big".into()));
            }
            let axis = |text: &String| {
                let mut vector = vec![0.0; DIMENSIONS];
                vector[text.trim_start_matches('x').parse::<usize>().unwrap()] = 1.0;
                vector
            };
            Ok(Embedded {
                vectors: texts.iter().map(axis).collect(),
                neurons: texts.len() as f64,
            })
        }
    }

    fn texts(lengths: &[usize]) -> Vec<String> {
        lengths
            .iter()
            .enumerate()
            .map(|(i, length)| format!("{}{i}", "x".repeat(*length)))
            .collect()
    }

    fn axes(embedded: &Embedded) -> Vec<usize> {
        let axis = |v: &Vec<f32>| v.iter().position(|x| *x == 1.0).unwrap();
        embedded.vectors.iter().map(axis).collect()
    }

    #[tokio::test]
    async fn a_batch_too_big_is_sent_in_halves_in_order() {
        // Each text is its length plus its index digit: 10 × 101 bytes.
        let fake = Sized {
            limit: 1_010,
            calls: Default::default(),
        };
        let all = split(&fake, &texts(&[100; 10])).await.unwrap();
        assert_eq!(*fake.calls.borrow(), [10], "one request when it fits");
        assert_eq!(axes(&all), (0..10).collect::<Vec<_>>());

        let fake = Sized {
            limit: 1_000,
            calls: Default::default(),
        };
        let long = texts(&[300, 300, 300, 300, 300, 10, 10, 10, 10, 900]);
        let embedded = split(&fake, &long).await.unwrap();
        assert_eq!(axes(&embedded), (0..10).collect::<Vec<_>>());
        assert_eq!(embedded.neurons, 10.0, "only answered requests count");
        // 2,450 bytes; then 1,505 (texts 0-4) splits into 602 and 903, and
        // 955 (texts 5-9) fits.
        assert_eq!(*fake.calls.borrow(), [10, 5, 2, 3, 5]);
    }

    #[tokio::test]
    async fn only_a_single_input_too_big_or_another_error_fails() {
        let fake = Sized {
            limit: 100,
            calls: Default::default(),
        };
        let error = split(&fake, &texts(&[10, 500])).await.err().unwrap();
        assert!(error.contains("too big"), "{error}");
        assert_eq!(*fake.calls.borrow(), [2, 1, 1]);

        struct Quota;
        impl Request for Quota {
            async fn request(&self, _: &[String]) -> Result<Embedded, Failure> {
                Err(Failure::Other("4006 daily free allocation used".into()))
            }
        }
        let error = split(&Quota, &texts(&[10, 10])).await.err().unwrap();
        assert!(error.contains("4006"), "{error}");
    }

    #[test]
    fn the_too_big_answer_is_recognized() {
        // The body of the first production run's failure.
        let body = json!({"success": false, "errors": [{"code": 3030, "message": "AiError: AiError: input too big (cb3bfee4-866d-4d29-b0a0-80308a7bef10)"}]});
        assert!(too_big(&body.to_string()));
        let quota = json!({"success": false, "errors": [{"code": 4006, "message": "daily free allocation used"}]});
        assert!(!too_big(&quota.to_string()));
        assert!(!too_big("<html>"));
    }

    #[test]
    fn the_account_id_and_token_are_checked() {
        let account = "0123456789abcdef0123456789abcdef";
        assert!(WorkersAi::new(account, "token".into()).is_ok());
        assert!(WorkersAi::new("../evil", "token".into()).is_err());
        assert!(WorkersAi::new(account, String::new()).is_err());
        assert!(WorkersAi::new(account, "a b".into()).is_err());
    }
}
