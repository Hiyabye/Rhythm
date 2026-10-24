use serde::Serialize;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
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
pub struct Worker {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    next_id: u64,
    status: WorkerStatus,
}
impl Worker {
    pub fn launch(path: &Path) -> Self {
        let mut worker = Self {
            child: None,
            input: None,
            output: None,
            next_id: 0,
            status: WorkerStatus::unavailable(
                "AI 엔진을 시작하지 못했습니다. 앱을 다시 실행해 주세요.",
            ),
        };
        let mut command = Command::new(path);
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
                        let mut lines = BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            eprintln!("AI worker: {line}");
                        }
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
        match self.request("status").await {
            Ok(result) => {
                self.status = WorkerStatus {
                    state: "ready".into(),
                    runtime_available: result["runtime_available"].as_bool().unwrap_or(false),
                    model: result["model"].as_str().map(str::to_owned),
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
    async fn missing_worker_does_not_crash_core() {
        let mut worker = Worker::launch(Path::new("/nonexistent/rhythm-ai-worker"));
        assert_eq!(worker.refresh().await.state, "failed");
        worker.shutdown().await;
    }
    #[tokio::test]
    async fn real_worker_lifecycle_and_crash_isolation() {
        let Some(path) = std::env::var_os("RHYTHM_WORKER_TEST_BIN") else {
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
