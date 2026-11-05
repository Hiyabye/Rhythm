//! Supervises one upstream `llama-server` child process for the loaded model.
//!
//! The server listens only on a Unix socket inside a private `0700` directory.
//! Where that is unavailable (Windows, or a socket path beyond the OS limit),
//! it listens on 127.0.0.1 on an OS-chosen port and every request needs a
//! random per-launch API key. The model is fixed for the life of the process;
//! loading another model, unloading and cancelling all stop it.

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::{sleep, timeout, Duration, Instant},
};

/// Stable part of the context-overflow error. It rejects the input, not the
/// device, so auto mode retries only that request on CPU.
pub(super) const INPUT_LIMIT: &str = "토큰 한도를 넘습니다";
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_LOG_LINE: u64 = 4096;
// sockaddr_un.sun_path is 104 bytes on macOS (108 on Linux), including the NUL.
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 103;

/// One way to start the bundled server.
#[derive(Clone, Debug)]
pub(super) struct Launch {
    pub executable: PathBuf,
    /// Backend this launch targets: `metal`, `vulkan` or `cpu`.
    pub backend: &'static str,
    pub context: u32,
    /// Some only for the dedicated embedding process. Generation launches keep
    /// the chat flags and never receive `--embedding`.
    pub embedding: Option<EmbeddingLaunch>,
}
impl Launch {
    fn gpu(&self) -> bool {
        self.backend != "cpu"
    }
}

/// Model-declared pooling mode. The vector is normalized by the server, so
/// callers only need cosine similarity.
#[derive(Clone, Debug)]
pub(super) struct EmbeddingLaunch {
    pub pooling: String,
}

enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    Tcp {
        port: u16,
        key: String,
    },
}

#[derive(Default)]
struct Observed {
    port: Option<u16>,
    offloaded: Option<(u32, u32)>,
    device: Option<String>,
    /// Upstream confirms the multimodal encoder actually loaded.
    multimodal: bool,
}

pub(super) struct Server {
    child: Child,
    endpoint: Endpoint,
    runtime: PathBuf,
    pub model: PathBuf,
    /// Projector this process was started with; None means text-only.
    pub projector: Option<PathBuf>,
    /// True only when upstream logged the multimodal encoder as loaded.
    pub vision: bool,
    /// True only for a dedicated embedding process.
    pub embedding: bool,
    pub loaded_backend: &'static str,
    pub load_ms: u64,
    #[cfg(windows)]
    _job: super::WorkerJob,
}

impl Server {
    /// Starts `launch` with `model` and an optional vision projector, then waits
    /// until it serves requests. Every failure path kills and reaps the child.
    pub async fn start(
        launch: &Launch,
        model: &Path,
        projector: Option<&Path>,
        runtime: &Path,
        deadline: Duration,
    ) -> Result<Self, String> {
        let started = Instant::now();
        prepare_runtime(runtime)?;
        let socket = runtime.join("llama.sock");
        let key_file = runtime.join("llama.key");
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_file(&key_file);
        let mut command = Command::new(&launch.executable);
        command.arg("-m").arg(model);
        if let Some(projector) = projector {
            command.arg("--mmproj").arg(projector);
            // Bound per-image cost; the encoder still receives enough tokens to
            // read Korean poster text instead of guessing from layout alone.
            command.args(["--image-min-tokens", "1024", "--image-max-tokens", "2048"]);
        }
        let key = match socket_host(&socket) {
            Some(host) => {
                command.args(["--host", &host]);
                None
            }
            None => {
                let key = random_key()?;
                write_private(&key_file, key.as_bytes())?;
                command.args(["--host", "127.0.0.1", "--port", "0", "--api-key-file"]);
                command.arg(&key_file);
                Some(key)
            }
        };
        let context = launch.context.to_string();
        command.args([
            "--no-webui",
            "--no-slots",
            "-np",
            "1",
            "-c",
            &context,
            // Level 4 logs the selected device and offloaded layer count.
            "-lv",
            "4",
        ]);
        match &launch.embedding {
            Some(embedding) => {
                // Dedicated embedding models have no chat template and reject
                // chat-only flags. Request a unit-length vector from upstream.
                // Without a large physical batch one long notice exceeds the
                // default 512-token ubatch and the server returns HTTP 500.
                command.args([
                    "--embedding",
                    "--pooling",
                    &embedding.pooling,
                    "--embd-normalize",
                    "2",
                    "--batch-size",
                    "2048",
                    "--ubatch-size",
                    "2048",
                ]);
            }
            None => {
                command.args([
                    "--jinja",
                    // Analysis is deterministic JSON, never a reasoning task. This also
                    // stops Gemma 4's optional thought channel from consuming the whole
                    // output budget before the JSON begins.
                    "--reasoning-budget",
                    "0",
                ]);
            }
        }
        if launch.backend == "vulkan" {
            let device = nvidia_device(&launch.executable).await?;
            command.args(["-ngl", "99", "--device", &device]);
        } else if launch.gpu() {
            command.args(["-ngl", "99"]);
        } else {
            command.args(["-ngl", "0", "--device", "none"]);
        }
        let observed = Arc::new(Mutex::new(Observed::default()));
        let (mut child, job) = spawn(command, observed.clone())?;
        let ready =
            wait_until_ready(&mut child, key.as_deref(), &socket, &observed, deadline).await;
        let _ = std::fs::remove_file(&key_file);
        let endpoint = match ready {
            Ok(endpoint) => endpoint,
            Err(error) => {
                reap(&mut child).await;
                return Err(error);
            }
        };
        let multimodal = observed
            .lock()
            .map(|facts| facts.multimodal)
            .unwrap_or(false);
        let (loaded_backend, complete) = {
            let facts = observed
                .lock()
                .map_err(|_| "AI 엔진 상태를 읽지 못했습니다.")?;
            let on_gpu = facts.offloaded.is_some_and(|(done, _)| done > 0);
            let backend = match (on_gpu, facts.device.as_deref()) {
                (true, Some(device)) if device.starts_with("MTL") => "metal",
                (true, Some(device)) if device.starts_with("Vulkan") => "vulkan",
                (true, _) => launch.backend,
                (false, _) => "cpu",
            };
            // A GPU result is reported only after the server observed every
            // layer on the device; a silent CPU load cannot satisfy a GPU launch.
            (
                backend,
                facts
                    .offloaded
                    .is_some_and(|(done, total)| done > 0 && done == total),
            )
        };
        if launch.backend == "vulkan" && !complete {
            reap(&mut child).await;
            return Err("GPU 작업 프로세스의 실제 실행 장치를 확인하지 못했습니다.".into());
        }
        #[cfg(not(windows))]
        let () = job;
        Ok(Self {
            child,
            endpoint,
            runtime: runtime.to_owned(),
            model: model.to_owned(),
            projector: projector.map(Path::to_owned),
            vision: multimodal && projector.is_some(),
            embedding: launch.embedding.is_some(),
            loaded_backend,
            load_ms: started.elapsed().as_millis() as u64,
            #[cfg(windows)]
            _job: job,
        })
    }

    /// Sends one chat completion (a JSON request body) and returns the parsed response.
    pub async fn chat(&mut self, body: &str, deadline: Duration) -> Result<Value, String> {
        let (status, response) = timeout(
            deadline,
            request(
                &self.endpoint,
                "POST",
                "/v1/chat/completions",
                body.as_bytes(),
            ),
        )
        .await
        .map_err(|_| "AI 엔진 응답 시간이 초과되었습니다.".to_string())??;
        let value: Value = serde_json::from_slice(&response)
            .map_err(|_| "AI 엔진 응답 형식이 올바르지 않습니다.".to_string())?;
        if status == 200 {
            return value
                .is_object()
                .then_some(value)
                .ok_or("AI 엔진 응답 내용이 올바르지 않습니다.".into());
        }
        if value["error"]["type"] == "exceed_context_size_error" {
            return Err(format!(
                "분석 입력과 출력이 모델의 {INPUT_LIMIT}. 본문 길이를 줄여 주세요."
            ));
        }
        eprintln!(
            "AI engine request failed with HTTP {status}: {}",
            String::from_utf8_lossy(&response)
        );
        Err("로컬 추론에 실패했습니다. AI 엔진 로그를 확인해 주세요.".into())
    }

    /// Sends one OpenAI-style embeddings request. The server returns one vector
    /// per input; `index` is honored because batching order is not guaranteed.
    pub async fn embeddings(&mut self, body: &str, deadline: Duration) -> Result<Value, String> {
        let (status, response) = timeout(
            deadline,
            request(&self.endpoint, "POST", "/v1/embeddings", body.as_bytes()),
        )
        .await
        .map_err(|_| "AI 엔진 응답 시간이 초과되었습니다.".to_string())??;
        let value: Value = serde_json::from_slice(&response)
            .map_err(|_| "AI 엔진 응답 형식이 올바르지 않습니다.".to_string())?;
        if status == 200 && value["data"].is_array() {
            return Ok(value);
        }
        eprintln!(
            "AI engine embedding request failed with HTTP {status}: {}",
            String::from_utf8_lossy(&response)
        );
        Err("로컬 임베딩 계산에 실패했습니다. AI 엔진 로그를 확인해 주세요.".into())
    }

    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    #[cfg(test)]
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    #[cfg(test)]
    pub async fn kill_for_test(&mut self) {
        let _ = self.child.kill().await;
    }

    /// Kills and reaps the process: the reliable boundary for releasing model
    /// memory and for abandoning an in-flight request.
    pub async fn stop(mut self) {
        reap(&mut self.child).await;
        #[cfg(unix)]
        if let Endpoint::Unix(socket) = &self.endpoint {
            let _ = std::fs::remove_file(socket);
        }
        let _ = std::fs::remove_file(self.runtime.join("llama.key"));
    }

    /// Like `stop`, but reports whether the exit was confirmed.
    pub async fn stop_confirmed(mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|_| "AI 엔진 종료 상태를 확인하지 못했습니다.")?
            .is_none()
        {
            if self.child.start_kill().is_err()
                && self
                    .child
                    .try_wait()
                    .map_err(|_| "AI 엔진 종료 상태를 확인하지 못했습니다.")?
                    .is_none()
            {
                return Err("AI 엔진을 안전하게 종료하지 못했습니다.".into());
            }
            self.child
                .wait()
                .await
                .map_err(|_| "AI 엔진 종료를 확인하지 못했습니다.")?;
        }
        self.stop().await;
        Ok(())
    }
}

/// Creates the private runtime directory: only this user may enter it, so only
/// this user can reach a socket inside it.
fn prepare_runtime(runtime: &Path) -> Result<(), String> {
    std::fs::create_dir_all(runtime).map_err(|_| "AI 엔진 실행 폴더를 만들지 못했습니다.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(runtime, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "AI 엔진 실행 폴더 권한을 설정하지 못했습니다.")?;
    }
    Ok(())
}

/// The `--host` value for a Unix socket, or None where loopback is required.
fn socket_host(socket: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        // llama-server treats a host ending in `.sock` as a Unix socket path.
        let host = socket.to_str()?;
        (host.len() <= MAX_SOCKET_PATH).then(|| host.to_owned())
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
        None
    }
}

fn random_key() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "AI 엔진 접근 키를 만들지 못했습니다.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|_| "AI 엔진 접근 키를 저장하지 못했습니다.".into())
}

/// Picks the first NVIDIA device the Vulkan build reports. Other vendors are
/// not a supported GPU target.
async fn nvidia_device(executable: &Path) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .arg("--list-devices")
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| "GPU 장치 목록을 확인하지 못했습니다.")?
        .map_err(|_| "GPU 작업 파일을 실행하지 못했습니다.")?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().split_once(": "))
        .find(|(name, description)| name.starts_with("Vulkan") && description.contains("NVIDIA"))
        .map(|(name, _)| name.to_owned())
        .ok_or("NVIDIA GPU를 찾지 못했습니다.".into())
}

#[cfg(windows)]
type Job = super::WorkerJob;
#[cfg(not(windows))]
type Job = ();

fn spawn(mut command: Command, observed: Arc<Mutex<Observed>>) -> Result<(Child, Job), String> {
    #[cfg(windows)]
    let job = super::WorkerJob::new()?;
    // A GUI application must not flash a console each time the server starts.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            eprintln!("AI engine launch: {error}");
            "AI 엔진을 시작하지 못했습니다. 앱 설치 상태를 확인해 주세요.".to_string()
        })?;
    #[cfg(windows)]
    if let Err(error) = job.assign(&child) {
        // Never leave an unprotected server running when assignment fails.
        let _ = child.start_kill();
        tokio::spawn(async move {
            let _ = child.kill().await;
            let _ = child.wait().await;
        });
        return Err(error);
    }
    if let Some(stderr) = child.stderr.take() {
        // Drain diagnostics so upstream logging can never block the server.
        tokio::spawn(drain_log(stderr, move |line| {
            if let Ok(mut facts) = observed.lock() {
                observe(&mut facts, line);
            }
            eprintln!("AI engine: {}", line.trim_end());
        }));
    }
    #[cfg(windows)]
    return Ok((child, job));
    #[cfg(not(windows))]
    Ok((child, ()))
}

/// Reads bounded lines; invalid UTF-8 and overlong lines never stop drainage.
pub(super) async fn drain_log(reader: impl AsyncRead + Unpin, mut handle: impl FnMut(&str)) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut reader)
            .take(MAX_LOG_LINE)
            .read_until(b'\n', &mut line)
            .await
        {
            Ok(0) | Err(_) => return,
            Ok(_) => handle(&String::from_utf8_lossy(&line)),
        }
    }
}

fn observe(facts: &mut Observed, line: &str) {
    if line.contains("loaded multimodal model") {
        facts.multimodal = true;
    }
    if let Some(rest) = line.split("listening on http://127.0.0.1:").nth(1) {
        facts.port = rest.trim().parse().ok();
    }
    if let Some(rest) = line.split("using device ").nth(1) {
        facts.device = rest.split_whitespace().next().map(str::to_owned);
    }
    if let Some((before, _)) = line.split_once(" layers to GPU") {
        if let Some((done, total)) = before
            .rsplit(' ')
            .next()
            .and_then(|count| count.split_once('/'))
        {
            if let (Ok(done), Ok(total)) = (done.parse(), total.parse()) {
                facts.offloaded = Some((done, total));
            }
        }
    }
}

async fn wait_until_ready(
    child: &mut Child,
    key: Option<&str>,
    socket: &Path,
    observed: &Mutex<Observed>,
    deadline: Duration,
) -> Result<Endpoint, String> {
    let until = Instant::now() + deadline;
    loop {
        if !matches!(child.try_wait(), Ok(None)) {
            return Err(
                "모델을 불러오지 못했습니다. 모델 파일과 AI 엔진 로그를 확인해 주세요.".into(),
            );
        }
        let endpoint = match key {
            Some(key) => observed
                .lock()
                .map_err(|_| "AI 엔진 상태를 읽지 못했습니다.")?
                .port
                .map(|port| Endpoint::Tcp {
                    port,
                    key: key.to_owned(),
                }),
            #[cfg(unix)]
            None => Some(Endpoint::Unix(socket.to_owned())),
            #[cfg(not(unix))]
            None => {
                let _ = socket;
                None
            }
        };
        if let Some(endpoint) = endpoint {
            // 503 while loading; 200 once the model serves requests.
            let healthy = timeout(
                Duration::from_secs(2),
                request(&endpoint, "GET", "/health", b""),
            )
            .await
            .is_ok_and(|result| matches!(result, Ok((200, _))));
            if healthy {
                return Ok(endpoint);
            }
        }
        if Instant::now() >= until {
            return Err("모델을 불러오는 시간이 초과되었습니다.".into());
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn request(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let unreachable = |_| "AI 엔진에 연결할 수 없습니다.".to_string();
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(socket) => {
            let stream = tokio::net::UnixStream::connect(socket)
                .await
                .map_err(unreachable)?;
            exchange(stream, method, path, None, body).await
        }
        Endpoint::Tcp { port, key } => {
            let stream = tokio::net::TcpStream::connect(("127.0.0.1", *port))
                .await
                .map_err(unreachable)?;
            exchange(stream, method, path, Some(key), body).await
        }
    }
}

/// One HTTP/1.1 request per connection. The response must carry an exact
/// Content-Length; anything else is rejected rather than guessed.
async fn exchange(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    method: &str,
    path: &str,
    key: Option<&str>,
    body: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let authorization = key
        .map(|key| format!("Authorization: Bearer {key}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{authorization}\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|_| "AI 엔진에 요청을 보내지 못했습니다.")?;
    stream
        .write_all(body)
        .await
        .map_err(|_| "AI 엔진에 요청을 보내지 못했습니다.")?;
    stream
        .flush()
        .await
        .map_err(|_| "AI 엔진 연결이 끊어졌습니다.")?;
    let mut response = Vec::new();
    (&mut stream)
        .take(MAX_RESPONSE as u64 + 1)
        .read_to_end(&mut response)
        .await
        .map_err(|_| "AI 엔진 응답을 읽지 못했습니다.")?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let invalid = || "AI 엔진 응답이 너무 크거나 완전하지 않습니다.".to_string();
    if response.len() > MAX_RESPONSE {
        return Err(invalid());
    }
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(invalid)?;
    let head = std::str::from_utf8(&response[..split]).map_err(|_| invalid())?;
    let body = &response[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.strip_prefix("HTTP/1.1 "))
        .and_then(|rest| rest.get(..3)?.parse().ok())
        .ok_or_else(invalid)?;
    let mut length = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(invalid)?;
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.trim().parse::<usize>().map_err(|_| invalid())?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(invalid());
        }
    }
    (length == Some(body.len()))
        .then(|| (status, body.to_vec()))
        .ok_or_else(invalid)
}

async fn reap(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_require_an_exact_length_and_no_chunking() {
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap(),
            (200, b"{}".to_vec())
        );
        for response in [
            &b"HTTP/1.1 200 OK\r\n\r\n{}"[..],
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}",
            b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}",
            b"not http",
        ] {
            assert!(
                parse_response(response).is_err(),
                "{}",
                String::from_utf8_lossy(response)
            );
        }
        let mut oversized = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n".to_vec();
        oversized.resize(MAX_RESPONSE + 1, b'x');
        assert!(parse_response(&oversized).is_err());
    }

    #[test]
    fn log_lines_reveal_port_device_and_offload() {
        let mut facts = Observed::default();
        for line in [
            "0.00.302.102 I llama_prepare_model_devices: using device MTL0 (Apple M1 Pro) (unknown id) - 12123 MiB free",
            "0.00.503.404 I load_tensors: offloaded 25/25 layers to GPU",
            "0.06.910.903 I srv  server_on_li: listening on http://127.0.0.1:58961",
        ] {
            observe(&mut facts, line);
        }
        assert_eq!(facts.device.as_deref(), Some("MTL0"));
        assert_eq!(facts.offloaded, Some((25, 25)));
        assert_eq!(facts.port, Some(58961));
    }

    #[tokio::test]
    async fn log_lines_are_bounded_and_invalid_utf8_does_not_stop_drainage() {
        let mut bytes = vec![0xff; 2 * 1024 * 1024];
        bytes.extend_from_slice(b"\nfinal diagnostic\n");
        let mut received = 0;
        let mut last = String::new();
        drain_log(bytes.as_slice(), |line| {
            assert!(line.len() <= MAX_LOG_LINE as usize * 3);
            received += 1;
            last = line.to_owned();
        })
        .await;
        assert!(received > 1);
        assert_eq!(last, "final diagnostic\n");
    }

    #[cfg(unix)]
    #[test]
    fn long_socket_paths_fall_back_to_loopback() {
        assert!(socket_host(Path::new("/tmp/run/llama.sock")).is_some());
        let long = format!("/{}/llama.sock", "a".repeat(MAX_SOCKET_PATH));
        assert!(socket_host(Path::new(&long)).is_none());
    }
}
