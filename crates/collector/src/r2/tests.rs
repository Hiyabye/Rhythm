//! SigV4 against published vectors: the generic cases of AWS's signing test
//! suite (awslabs/aws-c-auth `tests/aws-signing-test-suite/v4`, Apache-2.0) and
//! the S3 examples of AWS's "Signature Calculations for the Authorization
//! Header: Transferring Payload in a Single Chunk" (GET Object, PUT Object,
//! List Objects).
use super::*;

const SUITE_DATE: &str = "20150830T123600Z";

fn suite_credentials() -> Credentials {
    Credentials {
        access_key_id: "AKIDEXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
    }
}
fn s3_credentials() -> Credentials {
    Credentials {
        access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
        secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
    }
}
fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}
fn signature(authorization: &str) -> &str {
    authorization.rsplit("Signature=").next().unwrap()
}

/// One suite case: the request parts, its canonical request and signature.
fn suite(
    method: &str,
    path: &str,
    query: &[(&str, &str)],
    extra: &[(&str, &str)],
    payload: &str,
    canonical: &str,
    expected: &str,
) {
    let mut headers = headers(extra);
    headers.push(("Host".into(), "example.amazonaws.com".into()));
    headers.push(("X-Amz-Date".into(), SUITE_DATE.into()));
    let query = canonical_query(query);
    let request = Request {
        method,
        path: &encode_path(path),
        query: &query,
        headers: &headers,
        payload_sha256: &sha256_hex(payload),
    };
    assert_eq!(canonical_request(&request), canonical);
    let authorization = authorization(&request, &suite_credentials(), "us-east-1", "service");
    assert!(authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, "
    ));
    assert_eq!(signature(&authorization), expected);
}

#[test]
fn suite_get_vanilla() {
    suite(
        "GET",
        "/",
        &[],
        &[],
        "",
        "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
    );
}

#[test]
fn suite_get_vanilla_query_order_key_case() {
    suite(
        "GET",
        "/",
        &[("Param2", "value2"), ("Param1", "value1")],
        &[],
        "",
        "GET\n/\nParam1=value1&Param2=value2\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500",
    );
}

#[test]
fn suite_get_vanilla_query_unreserved() {
    let unreserved = "-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    suite(
        "GET",
        "/",
        &[(unreserved, unreserved)],
        &[],
        "",
        &format!("GET\n/\n{unreserved}={unreserved}\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        "9c3e54bfcdf0b19771a7f523ee5669cdf59bc7cc0884027167c21bb143a40197",
    );
}

#[test]
fn suite_get_unreserved() {
    suite(
        "GET",
        "/-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
        &[],
        &[],
        "",
        "GET\n/-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "07ef7494c76fa4850883e2b006601f940f8a34d404d0cfa977f52a65bbf5f24f",
    );
}

#[test]
fn suite_get_header_value_trim() {
    suite(
        "GET",
        "/",
        &[],
        &[("My-Header1", " value1"), ("My-Header2", " \"a   b   c\"")],
        "",
        "GET\n/\n\nhost:example.amazonaws.com\nmy-header1:value1\nmy-header2:\"a b c\"\nx-amz-date:20150830T123600Z\n\nhost;my-header1;my-header2;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "acc3ed3afb60bb290fc8d2dd0098b9911fcaa05412b367055dee359757a9c736",
    );
}

#[test]
fn suite_post_x_www_form_urlencoded_signs_the_body() {
    let body = "Param1=value1";
    suite(
        "POST",
        "/",
        &[],
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Content-Length", "13"),
            ("X-Amz-Content-Sha256", &sha256_hex(body)),
        ],
        body,
        "POST\n/\n\ncontent-length:13\ncontent-type:application/x-www-form-urlencoded\nhost:example.amazonaws.com\nx-amz-content-sha256:9095672bbd1f56dfc5b65f3e153adc8731a4a654192329106275f4c7b24d0b6e\nx-amz-date:20150830T123600Z\n\ncontent-length;content-type;host;x-amz-content-sha256;x-amz-date\n9095672bbd1f56dfc5b65f3e153adc8731a4a654192329106275f4c7b24d0b6e",
        "d3875051da38690788ef43de4db0d8f280229d82040bfac253562e56c3f20e0b",
    );
}

fn s3(method: &str, path: &str, query: &str, headers: &[(&str, &str)], payload: &str) -> String {
    authorization(
        &Request {
            method,
            path,
            query,
            headers: &self::headers(headers),
            payload_sha256: payload,
        },
        &s3_credentials(),
        "us-east-1",
        "s3",
    )
}

#[test]
fn s3_get_object_example() {
    let authorization = s3(
        "GET",
        &encode_path("/test.txt"),
        "",
        &[
            ("host", "examplebucket.s3.amazonaws.com"),
            ("range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ],
        EMPTY_SHA256,
    );
    assert_eq!(
        authorization,
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
}

#[test]
fn s3_put_object_example() {
    let payload = sha256_hex("Welcome to Amazon S3.");
    let path = encode_path("/test$file.text");
    assert_eq!(path, "/test%24file.text");
    let authorization = s3(
        "PUT",
        &path,
        "",
        &[
            ("date", "Fri, 24 May 2013 00:00:00 GMT"),
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", &payload),
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ],
        &payload,
    );
    assert_eq!(
        signature(&authorization),
        "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
    );
}

#[test]
fn s3_list_objects_example() {
    let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
    assert_eq!(query, "max-keys=2&prefix=J");
    let authorization = s3(
        "GET",
        "/",
        &query,
        &[
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ],
        EMPTY_SHA256,
    );
    assert_eq!(
        signature(&authorization),
        "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
    );
}

#[test]
fn query_values_are_encoded_once() {
    assert_eq!(
        canonical_query(&[("prefix", "state/"), ("list-type", "2")]),
        "list-type=2&prefix=state%2F"
    );
    assert_eq!(encode_path("/a b/c+d"), "/a%20b/c%2Bd");
}

#[test]
fn utc_times_format_as_amz_dates() {
    // Expected values from Python's `datetime` in UTC.
    for (seconds, expected) in [
        (0, "19700101T000000Z"),
        (951_868_800, "20000301T000000Z"),
        (1_369_353_600, "20130524T000000Z"),
        (1_440_938_160, "20150830T123600Z"),
        (1_709_251_199, "20240229T235959Z"),
        (4_133_923_750, "21001231T080910Z"),
    ] {
        assert_eq!(
            amz_date(UNIX_EPOCH + Duration::from_secs(seconds)),
            expected
        );
    }
}

#[test]
fn listings_are_parsed_and_truncation_is_refused() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>rhythm-state</Name><IsTruncated>false</IsTruncated><Contents><Key>state/20261117T000000Z.sqlite3.zst</Key><Size>1</Size></Contents><Contents><Key>state/a&amp;b</Key></Contents></ListBucketResult>"#;
    assert_eq!(
        parse_list(xml).unwrap(),
        ["state/20261117T000000Z.sqlite3.zst", "state/a&b"]
    );
    let empty = r#"<ListBucketResult><IsTruncated>false</IsTruncated><KeyCount>0</KeyCount></ListBucketResult>"#;
    assert!(parse_list(empty).unwrap().is_empty());
    assert!(parse_list(&xml.replace(">false<", ">true<")).is_err());
    assert!(parse_list("<Error><Code>AccessDenied</Code></Error>").is_err());
}

#[test]
fn keys_and_account_ids_are_checked_before_any_request() {
    assert!(validate_key("state/20261117T000000Z.sqlite3.zst").is_ok());
    for key in ["", "/state/a", "state//a", "state/a b", "state/../a?x"] {
        assert!(validate_key(key).is_err(), "{key}");
    }
    let credentials = || Credentials {
        access_key_id: "id".into(),
        secret_access_key: "secret".into(),
    };
    assert!(R2::new(
        "0123456789abcdef0123456789abcdef",
        "rhythm-state",
        credentials()
    )
    .is_ok());
    assert!(R2::new("evil.example.com/", "rhythm-state", credentials()).is_err());
    assert!(R2::new(
        "0123456789abcdef0123456789abcdef",
        "Bad_Bucket",
        credentials()
    )
    .is_err());
    assert_eq!(format!("{:?}", credentials()), "Credentials { .. }");
}
