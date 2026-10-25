use std::io::{Read, Write};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn capture_pipe(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes).unwrap();
        }
        bytes
    })
}

fn wait_with_deadline(mut child: Child, limit: Duration) -> Output {
    // Drain both pipes while waiting: a verbose worker must not block on its
    // diagnostics pipe before the parent can observe exit or enforce the deadline.
    let stdout = capture_pipe(child.stdout.take());
    let stderr = capture_pipe(child.stderr.take());
    let deadline = Instant::now() + limit;
    let mut timed_out = false;
    let mut wait_error = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                timed_out = matches!(result, Ok(None));
                wait_error = result.err();
                let _ = child.kill();
                break child
                    .wait()
                    .expect("reap worker after wait failure/deadline");
            }
        }
    };
    let output = Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    };
    assert!(wait_error.is_none(), "worker wait failed: {wait_error:?}");
    assert!(
        !timed_out,
        "worker exceeded {limit:?}; killed and reaped: {output:?}"
    );
    output
}

#[test]
#[should_panic(expected = "killed and reaped")]
fn hanging_worker_is_killed_at_test_deadline() {
    let child = Command::new(env!("CARGO_BIN_EXE_rhythm-ai-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Keep stdin open without a request, so a healthy worker deliberately waits.
    wait_with_deadline(child, Duration::from_millis(50));
}

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
    let output = wait_with_deadline(child, Duration::from_secs(15));
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
    assert_eq!(responses[2]["result"]["loaded_backend"], Value::Null);
    assert!(matches!(
        responses[2]["result"]["build_backend"].as_str(),
        Some("cpu" | "metal" | "cuda")
    ));
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
    for request in [
        json!({"version":1,"id":1,"method":"load","params":{"model_path":model}}),
        json!({"version":1,"id":2,"method":"status"}),
        json!({"version":1,"id":3,"method":"infer","params":{
            "model_path":model,"prompt":"한국어로 한 문장의 짧은 인사를 해 주세요.","max_tokens":32
        }}),
        json!({"version":1,"id":4,"method":"unload"}),
        json!({"version":1,"id":5,"method":"status"}),
        json!({"version":1,"id":6,"method":"shutdown"}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = wait_with_deadline(child, Duration::from_secs(180));
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses.len(),
        6,
        "stdout must contain only protocol responses"
    );
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(response["id"], index + 1);
        assert_eq!(response["ok"], true, "{response}");
    }
    assert!(responses[0]["result"]["model"].is_string());
    let backend = responses[0]["result"]["loaded_backend"].as_str().unwrap();
    assert!(matches!(backend, "cpu" | "metal" | "cuda" | "unknown"));
    if !cfg!(any(feature = "metal", feature = "cuda")) {
        assert_eq!(backend, "cpu");
    }
    assert_eq!(
        responses[0]["result"]["loaded_backend"],
        responses[2]["result"]["loaded_backend"]
    );
    assert_eq!(
        responses[0]["result"]["model"],
        responses[1]["result"]["model"]
    );
    assert!(responses[3]["result"]["model"].is_null());
    assert!(responses[3]["result"]["loaded_backend"].is_null());
    assert!(responses[4]["result"]["model"].is_null());
    assert!(responses[4]["result"]["loaded_backend"].is_null());
    assert_eq!(responses[5]["result"]["shutdown"], true);
    let response = &responses[2];
    assert_eq!(response["ok"], true, "{response}");
    assert!(!response["result"]["text"]
        .as_str()
        .unwrap()
        .trim()
        .is_empty());
    println!("{}", response["result"]);
}

#[test]
fn executable_exits_on_parent_stdin_close() {
    let child = Command::new(env!("CARGO_BIN_EXE_rhythm-ai-worker"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = wait_with_deadline(child, Duration::from_secs(15));
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
