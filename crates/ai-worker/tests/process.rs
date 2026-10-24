use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

#[test]
fn executable_protocol_survives_bad_requests_and_exits_cleanly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rhythm-ai-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input, "not-json").unwrap();
    for request in [
        json!({"version": 1, "id": 1, "method": "ping"}),
        json!({"version": 1, "id": 2, "method": "status"}),
        json!({"version": 2, "id": 3, "method": "shutdown"}),
        json!({"version": 1, "id": 4, "method": "unknown"}),
        json!({"version": 1, "id": 5, "method": "shutdown"}),
        json!({"version": 1, "id": 6, "method": "ping"}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 6, "shutdown must stop processing");
    assert_eq!(responses[0]["ok"], false);
    assert_eq!(responses[0]["id"], 0);
    assert_eq!(responses[1]["result"]["healthy"], true);
    assert_eq!(responses[2]["result"]["engine"], "mistral.rs");
    assert_eq!(responses[2]["result"]["model"], Value::Null);
    assert_eq!(responses[3]["ok"], false);
    assert_eq!(responses[4]["ok"], false);
    assert_eq!(responses[5]["result"]["shutdown"], true);
}

#[test]
#[ignore = "requires RHYTHM_TEST_MODEL pointing to an existing compatible GGUF"]
fn actual_local_inference() {
    let model = std::env::var("RHYTHM_TEST_MODEL").expect("set RHYTHM_TEST_MODEL to a local GGUF");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rhythm-ai-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"version":1,"id":1,"method":"infer","params":{
            "model_path":model,"prompt":"Say hello in one short sentence.","max_tokens":32
        }})
    )
    .unwrap();
    writeln!(input, "{}", json!({"version":1,"id":2,"method":"shutdown"})).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses.len(),
        2,
        "stdout must contain only protocol responses"
    );
    assert_eq!(responses[1]["result"]["shutdown"], true);
    let response = &responses[0];
    assert_eq!(response["ok"], true, "{response}");
    assert!(!response["result"]["text"]
        .as_str()
        .unwrap()
        .trim()
        .is_empty());
    println!("{}", response["result"]["text"]);
}

#[test]
fn executable_exits_on_parent_stdin_close() {
    let output = Command::new(env!("CARGO_BIN_EXE_rhythm-ai-worker"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
