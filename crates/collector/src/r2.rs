//! A minimal client for the R2 S3 API: GET, PUT, LIST and DELETE of objects in
//! one bucket, signed with AWS Signature Version 4 by hand (#174 decision: no
//! S3 crate). A single PUT covers the whole state (R2 allows 5 GB per object).
use crate::{hex, sha256_hex};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Method, StatusCode};
use sha2::Sha256;
use std::{
    fmt,
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

const REGION: &str = "auto";
const SERVICE: &str = "s3";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// The largest object the collector reads back, far above the ~100 MB state (#172).
pub const MAX_OBJECT: usize = 512 * 1024 * 1024;
/// Every request is idempotent (PUT only writes new keys), so transient
/// failures are retried.
const ATTEMPTS: u32 = 3;
/// User metadata holding the SHA-256 of the stored bytes, checked on every read.
const SHA256_HEADER: &str = "x-amz-meta-sha256";

pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials { .. }")
    }
}

/// An object read back with the SHA-256 recorded when it was written.
pub struct Object {
    pub bytes: Vec<u8>,
    pub sha256: Option<String>,
}

/// The object operations the state round-trip needs; tests use an in-memory store.
pub trait ObjectStore {
    /// Every key under the prefix.
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>, String>>;
    fn get(&self, key: &str) -> impl Future<Output = Result<Object, String>>;
    /// Stores the bytes with their SHA-256 as metadata.
    fn put(&self, key: &str, bytes: Vec<u8>) -> impl Future<Output = Result<(), String>>;
    fn delete(&self, key: &str) -> impl Future<Output = Result<(), String>>;
}

pub struct R2 {
    http: reqwest::Client,
    endpoint: Url,
    bucket: String,
    credentials: Credentials,
}

impl R2 {
    pub fn new(account_id: &str, bucket: &str, credentials: Credentials) -> Result<Self, String> {
        // The account ID becomes the host name, so only its exact form is accepted.
        if account_id.len() != 32 || !account_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("the Cloudflare account ID must be 32 hex digits".into());
        }
        if bucket.len() < 3
            || bucket.len() > 63
            || !bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("invalid R2 bucket name {bucket}"));
        }
        let endpoint = Url::parse(&format!(
            "https://{}.r2.cloudflarestorage.com/",
            account_id.to_ascii_lowercase()
        ))
        .map_err(|e| e.to_string())?;
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            endpoint,
            bucket: bucket.into(),
            credentials,
        })
    }

    /// Reads `CF_ACCOUNT_ID`, `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY`.
    pub fn from_env(bucket: &str) -> Result<Self, String> {
        let var = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
        Self::new(
            &var("CF_ACCOUNT_ID")?,
            bucket,
            Credentials {
                access_key_id: var("R2_ACCESS_KEY_ID")?,
                secret_access_key: var("R2_SECRET_ACCESS_KEY")?,
            },
        )
    }

    async fn send(
        &self,
        method: Method,
        key: Option<&str>,
        query: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<reqwest::Response, String> {
        let mut path = format!("/{}", self.bucket);
        if let Some(key) = key {
            validate_key(key)?;
            path.push('/');
            path.push_str(&encode_path(key));
        }
        let query = canonical_query(query);
        let mut url = self.endpoint.clone();
        url.set_path(&path);
        url.set_query((!query.is_empty()).then_some(query.as_str()));
        let host = self.endpoint.host_str().unwrap_or_default().to_owned();
        let payload = if body.is_empty() {
            EMPTY_SHA256.to_owned()
        } else {
            sha256_hex(&body)
        };
        let target = format!("{method} {}", key.unwrap_or(&self.bucket));
        let mut failure = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
            let mut headers = vec![
                ("host".to_owned(), host.clone()),
                ("x-amz-content-sha256".to_owned(), payload.clone()),
                ("x-amz-date".to_owned(), amz_date(SystemTime::now())),
            ];
            if method == Method::PUT {
                headers.push((SHA256_HEADER.to_owned(), payload.clone()));
            }
            let authorization = authorization(
                &Request {
                    method: method.as_str(),
                    path: &path,
                    query: &query,
                    headers: &headers,
                    payload_sha256: &payload,
                },
                &self.credentials,
                REGION,
                SERVICE,
            );
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header("authorization", authorization);
            // reqwest sends Host itself, from the same URL.
            for (name, value) in &headers[1..] {
                request = request.header(name, value);
            }
            if !body.is_empty() {
                request = request.body(body.clone());
            }
            match request.send().await {
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS =>
                {
                    failure = format!("HTTP {}", response.status());
                }
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    let code = between(&text, "<Code>", "</Code>").unwrap_or_default();
                    return Err(format!("R2 {target}: HTTP {status} {code}"));
                }
                Err(error) => failure = error.to_string(),
            }
        }
        Err(format!("R2 {target}: {failure}"))
    }
}

impl ObjectStore for R2 {
    async fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        let response = self
            .send(
                Method::GET,
                None,
                &[("list-type", "2"), ("prefix", prefix)],
                Vec::new(),
            )
            .await?;
        let text = response.text().await.map_err(|e| e.to_string())?;
        parse_list(&text)
    }

    async fn get(&self, key: &str) -> Result<Object, String> {
        let mut response = self.send(Method::GET, Some(key), &[], Vec::new()).await?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_OBJECT as u64)
        {
            return Err(format!("R2 object {key} is too large"));
        }
        let sha256 = response
            .headers()
            .get(SHA256_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len() + chunk.len() > MAX_OBJECT {
                return Err(format!("R2 object {key} is too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Object { bytes, sha256 })
    }

    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.send(Method::PUT, Some(key), &[], bytes).await?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        self.send(Method::DELETE, Some(key), &[], Vec::new())
            .await?;
        Ok(())
    }
}

/// The collector writes only plain keys, so nothing it sends needs escaping
/// beyond what SigV4 itself defines.
fn validate_key(key: &str) -> Result<(), String> {
    let plain = key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if key.is_empty() || !plain || key.starts_with('/') || key.contains("//") {
        return Err(format!("invalid R2 key {key}"));
    }
    Ok(())
}

/// The `ListObjectsV2` keys. A truncated listing is refused rather than read
/// as complete: the newest version could be on a later page.
fn parse_list(xml: &str) -> Result<Vec<String>, String> {
    if !xml.contains("<ListBucketResult") {
        return Err("R2 returned no object listing".into());
    }
    if between(xml, "<IsTruncated>", "</IsTruncated>") == Some("true") {
        return Err("R2 listing has more than one page".into());
    }
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        rest = &rest[start + 5..];
        let end = rest.find("</Key>").ok_or("R2 listing is malformed")?;
        keys.push(
            rest[..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        );
        rest = &rest[end..];
    }
    Ok(keys)
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)?;
    Some(&text[start..start + end])
}

/// A request as SigV4 sees it: an encoded path, a canonical query and the
/// headers to sign (`host` and `x-amz-date` among them).
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub headers: &'a [(String, String)],
    pub payload_sha256: &'a str,
}

/// SigV4 URI encoding: everything but the unreserved characters.
fn encode(value: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && byte == b'/')
        {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// An S3 object path, encoded once (S3 does not double-encode).
pub fn encode_path(path: &str) -> String {
    encode(path, true)
}

pub fn canonical_query(pairs: &[(&str, &str)]) -> String {
    let mut encoded: Vec<_> = pairs
        .iter()
        .map(|(name, value)| (encode(name, false), encode(value, false)))
        .collect();
    encoded.sort();
    encoded
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn canonical_headers(headers: &[(String, String)]) -> (String, String) {
    let mut headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.to_ascii_lowercase(),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    headers.sort();
    let canonical = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    (canonical, signed)
}

pub fn canonical_request(request: &Request) -> String {
    let (headers, signed) = canonical_headers(request.headers);
    format!(
        "{}\n{}\n{}\n{headers}\n{signed}\n{}",
        request.method, request.path, request.query, request.payload_sha256
    )
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// The `Authorization` header value. The request time comes from its
/// `x-amz-date` header.
pub fn authorization(
    request: &Request,
    credentials: &Credentials,
    region: &str,
    service: &str,
) -> String {
    let amz_date = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-amz-date"))
        .map(|(_, value)| value.as_str())
        .unwrap_or_default();
    let day = amz_date.get(..8).unwrap_or_default();
    let scope = format!("{day}/{region}/{service}/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request(request))
    );
    let mut key = hmac(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        day,
    );
    for part in [region, service, "aws4_request"] {
        key = hmac(&key, part);
    }
    let signature = hex(&hmac(&key, &to_sign));
    let (_, signed) = canonical_headers(request.headers);
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        credentials.access_key_id
    )
}

/// `YYYYMMDDTHHMMSSZ` in UTC, the SigV4 request time and the state version name.
pub fn amz_date(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Howard Hinnant's civil-from-days: the proleptic Gregorian date of a day count.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests;
