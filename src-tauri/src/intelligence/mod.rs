use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{timeout, Duration},
};

#[derive(Clone, Serialize)]
pub struct WorkerStatus {
    pub state: String,
    pub runtime_available: bool,
    pub model: Option<String>,
    pub detail: Option<String>,
}
impl WorkerStatus {
    pub fn unavailable(detail: &str) -> Self {
        Self {
            state: "unavailable".into(),
            runtime_available: false,
            model: None,
            detail: Some(detail.into()),
        }
    }
}
#[derive(Deserialize)]
struct StatusPayload {
    engine: String,
    runtime_available: bool,
    // `model` is nullable, but must still be present in a protocol-v1 status.
    #[serde(deserialize_with = "Deserialize::deserialize")]
    model: Option<String>,
}

async fn drain_diagnostics(mut reader: impl AsyncRead + Unpin, mut log: impl FnMut(&[u8])) {
    let mut buffer = [0; 4096];
    while let Ok(count) = reader.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        // Diagnostic output is not a framed protocol. Bound allocation even if
        // upstream emits an arbitrarily long line or invalid UTF-8, and keep draining.
        log(&buffer[..count]);
    }
}

pub struct Worker {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    next_id: u64,
    status: WorkerStatus,
}
impl Worker {
    pub fn launch(path: &Path) -> Self {
        Self::launch_command(Command::new(path))
    }
    fn launch_command(mut command: Command) -> Self {
        let mut worker = Self {
            child: None,
            input: None,
            output: None,
            next_id: 0,
            status: WorkerStatus::unavailable(
                "AI 엔진을 시작하지 못했습니다. 앱을 다시 실행해 주세요.",
            ),
        };
        // A GUI parent must not create a separate console for the bundled worker.
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        match command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stderr) = child.stderr.take() {
                    // Give GUI-only parents a valid diagnostics handle and drain it so
                    // upstream logging can never block the worker's protocol pipe.
                    tokio::spawn(async move {
                        drain_diagnostics(stderr, |bytes| {
                            eprintln!("AI worker: {}", String::from_utf8_lossy(bytes));
                        })
                        .await;
                    });
                }
                worker.input = child.stdin.take();
                worker.output = child.stdout.take().map(BufReader::new);
                worker.child = Some(child);
            }
            Err(e) => eprintln!("Worker launch: {e}"),
        };
        worker
    }
    async fn request(&mut self, method: &str) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let operation = async {
            let input = self.input.as_mut().ok_or("AI 엔진이 실행 중이 아닙니다.")?;
            input
                .write_all(format!("{}\n", json!({"version":1,"id":id,"method":method})).as_bytes())
                .await
                .map_err(|_| "AI 엔진에 요청을 보내지 못했습니다.")?;
            input
                .flush()
                .await
                .map_err(|_| "AI 엔진 연결이 끊어졌습니다.")?;
            let mut line = Vec::new();
            let count = self
                .output
                .as_mut()
                .ok_or("AI 엔진에 연결할 수 없습니다.")?
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| "AI 엔진 응답을 읽지 못했습니다.")?;
            if count == 0 {
                return Err("AI 엔진이 예기치 않게 종료되었습니다.".into());
            }
            if line.len() > 1024 * 1024 || line.last() != Some(&b'\n') {
                return Err("AI 엔진 응답이 너무 크거나 완전하지 않습니다.".into());
            }
            let response: Value = serde_json::from_slice(&line)
                .map_err(|_| "AI 엔진 응답 형식이 올바르지 않습니다.")?;
            if response["version"] != 1 || response["id"] != id || response["ok"] != true {
                return Err("AI 엔진 요청을 처리하지 못했습니다.".into());
            }
            Ok(response["result"].clone())
        };
        timeout(Duration::from_secs(8), operation)
            .await
            .map_err(|_| "AI 엔진 응답 시간이 초과되었습니다.".to_string())?
    }
    pub async fn refresh(&mut self) -> WorkerStatus {
        let result = self.request("status").await.and_then(|value| {
            let payload: StatusPayload = serde_json::from_value(value)
                .map_err(|_| "AI 엔진 상태 응답 형식이 올바르지 않습니다.".to_string())?;
            if payload.engine != "mistral.rs" {
                return Err("AI 엔진 상태 응답의 엔진 정보가 올바르지 않습니다.".into());
            }
            Ok(payload)
        });
        match result {
            Ok(result) => {
                self.status = WorkerStatus {
                    state: "ready".into(),
                    runtime_available: result.runtime_available,
                    model: result.model,
                    detail: None,
                };
            }
            Err(error) => {
                self.terminate().await;
                self.status = WorkerStatus {
                    state: "failed".into(),
                    runtime_available: false,
                    model: None,
                    detail: Some(error),
                };
            }
        }
        self.status.clone()
    }
    pub fn observe_exit(&mut self) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => (),
                Ok(Some(_)) | Err(_) => {
                    self.input.take();
                    self.output.take();
                    self.status = WorkerStatus {
                        state: "failed".into(),
                        runtime_available: false,
                        model: None,
                        detail: Some(
                            "AI 엔진이 예기치 않게 종료되었습니다. 앱을 다시 실행해 주세요.".into(),
                        ),
                    };
                }
            }
        }
    }
    pub fn status(&mut self) -> WorkerStatus {
        self.observe_exit();
        self.status.clone()
    }
    async fn terminate(&mut self) {
        self.input.take();
        self.output.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
    pub async fn shutdown(&mut self) {
        if self.child.is_some() {
            let _ = self.request("shutdown").await;
            self.input.take();
            if let Some(child) = self.child.as_mut() {
                if timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                    self.terminate().await;
                }
            }
            self.child.take();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn diagnostic_chunks_are_bounded_and_invalid_utf8_does_not_stop_drainage() {
        let mut bytes = vec![0xff; 2 * 1024 * 1024];
        bytes.extend_from_slice(b"\nfinal diagnostic\n");
        let mut received = 0;
        let mut last = Vec::new();
        drain_diagnostics(bytes.as_slice(), |chunk| {
            assert!(chunk.len() <= 4096);
            received += chunk.len();
            last = chunk.to_vec();
            // This is the production logger conversion; invalid bytes are harmless.
            assert!(String::from_utf8_lossy(chunk).len() <= 4096 * 3);
        })
        .await;
        assert_eq!(received, bytes.len());
        assert!(last.ends_with(b"final diagnostic\n"));
    }
    fn fake_worker(mode: &str) -> Worker {
        static FIXTURE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let directory = FIXTURE.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join("worker.rs");
            std::fs::write(&source, r###"
use std::io::{self, BufRead, Write};
fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let mut request = String::new();
    io::stdin().lock().read_line(&mut request).unwrap();
    let result = match mode.as_str() {
        "empty" => "{}",
        "null" => "null",
        "missing-runtime" => r#"{"engine":"mistral.rs","model":null}"#,
        "loaded" => r#"{"engine":"mistral.rs","runtime_available":true,"model":"/local/model.gguf"}"#,
        "missing-model" => r#"{"engine":"mistral.rs","runtime_available":true}"#,
        "wrong-engine" => r#"{"engine":"other","runtime_available":true,"model":null}"#,
        "wrong-runtime" => r#"{"engine":"mistral.rs","runtime_available":"yes","model":null}"#,
        "wrong-model" => r#"{"engine":"mistral.rs","runtime_available":true,"model":42}"#,
        _ => r#"{"engine":"mistral.rs","runtime_available":false,"model":null}"#,
    };
    let mut output = io::stdout().lock();
    match mode.as_str() {
        "eof" => return,
        "incomplete" => write!(output, "{{\"version\":1}}"),
        "oversized" => output.write_all(&vec![b'x'; 1024 * 1024 + 1]),
        "malformed" => writeln!(output, "not-json"),
        "wrong-version" => writeln!(output, "{{\"version\":2,\"id\":1,\"ok\":true,\"result\":{result}}}"),
        "wrong-id" => writeln!(output, "{{\"version\":1,\"id\":2,\"ok\":true,\"result\":{result}}}"),
        _ => writeln!(output, "{{\"version\":1,\"id\":1,\"ok\":true,\"result\":{result}}}"),
    }.unwrap();
    output.flush().unwrap();
    // EOF/incomplete framing needs the pipe closed; other failures must make
    // the parent kill and reap a still-running process, not merely observe exit.
    if mode != "incomplete" { std::thread::sleep(std::time::Duration::from_secs(60)); }
}
"###).unwrap();
            let executable = directory.path().join(format!("fake-worker{}", std::env::consts::EXE_SUFFIX));
            let result = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-name", "fake_worker"])
                .arg(&source).arg("-o").arg(&executable).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            directory
        });
        let mut command = Command::new(
            directory
                .path()
                .join(format!("fake-worker{}", std::env::consts::EXE_SUFFIX)),
        );
        command.arg(mode);
        Worker::launch_command(command)
    }
    #[tokio::test]
    async fn malformed_status_is_failed_and_child_is_reaped() {
        for mode in [
            "empty",
            "null",
            "missing-runtime",
            "missing-model",
            "wrong-engine",
            "wrong-runtime",
            "wrong-model",
        ] {
            let mut worker = fake_worker(mode);
            let status = timeout(Duration::from_secs(3), worker.refresh())
                .await
                .unwrap();
            assert_eq!(status.state, "failed", "mode: {mode}");
            assert!(status.detail.is_some());
            assert!(
                worker.child.is_none(),
                "failed worker must be reaped: {mode}"
            );
            assert!(worker.input.is_none() && worker.output.is_none());
        }
    }
    #[tokio::test]
    async fn invalid_protocol_frames_fail_and_reap_child() {
        for mode in [
            "eof",
            "incomplete",
            "oversized",
            "malformed",
            "wrong-version",
            "wrong-id",
        ] {
            let mut worker = fake_worker(mode);
            assert_eq!(
                timeout(Duration::from_secs(3), worker.refresh())
                    .await
                    .unwrap()
                    .state,
                "failed",
                "mode: {mode}"
            );
            assert!(worker.child.is_none(), "mode: {mode}");
        }
    }
    #[tokio::test]
    async fn lifecycle_only_status_remains_valid() {
        let mut worker = fake_worker("valid");
        let status = worker.refresh().await;
        assert_eq!(status.state, "ready");
        assert!(!status.runtime_available);
        assert!(status.model.is_none());
        worker.terminate().await;
        let mut worker = fake_worker("loaded");
        let status = worker.refresh().await;
        assert_eq!(status.state, "ready");
        assert!(status.runtime_available);
        assert_eq!(status.model.as_deref(), Some("/local/model.gguf"));
        worker.terminate().await;
    }
    #[tokio::test]
    async fn missing_worker_does_not_crash_core() {
        let mut worker = Worker::launch(Path::new("/nonexistent/rhythm-ai-worker"));
        assert_eq!(worker.refresh().await.state, "failed");
        worker.shutdown().await;
    }
    #[tokio::test]
    async fn real_worker_lifecycle_and_crash_isolation() {
        let Some(path) = std::env::var_os("RHYTHM_WORKER_TEST_BIN") else {
            eprintln!("SKIPPED real worker integration: set RHYTHM_WORKER_TEST_BIN to the built worker executable");
            return;
        };
        let mut worker = Worker::launch(Path::new(&path));
        assert_eq!(worker.refresh().await.state, "ready");
        worker.child.as_mut().unwrap().kill().await.unwrap();
        assert_eq!(worker.status().state, "failed");
        let mut worker = Worker::launch(Path::new(&path));
        assert_eq!(worker.refresh().await.state, "ready");
        worker.shutdown().await;
        assert!(worker.child.is_none());
    }
}
