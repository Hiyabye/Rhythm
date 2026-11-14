pub mod classification;
pub mod commands;
#[cfg(test)]
mod embedding_evaluation;
#[cfg(test)]
mod evaluation;
#[cfg(all(test, windows))]
mod gpu_tests;
pub(crate) mod model_storage;
pub mod models;
mod server;
#[cfg(test)]
mod test_worker;

use serde::Serialize;
use serde_json::{json, Value};
use server::{Launch, Server};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use tokio::process::Child;
use tokio::time::Duration;

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

/// Parses `/v1/embeddings` into `count` unit-length vectors in input order.
/// A malformed, duplicate, non-finite or wrong-length vector rejects the batch
/// instead of producing a silently misaligned classification.
fn parse_embeddings(value: &Value, count: usize) -> Result<Vec<Vec<f32>>, String> {
    let data = value["data"]
        .as_array()
        .ok_or("AI 엔진의 임베딩 응답이 올바르지 않습니다.")?;
    if data.len() != count {
        return Err("AI 엔진이 요청한 수와 다른 임베딩을 반환했습니다.".into());
    }
    let mut ordered: Vec<Option<Vec<f32>>> = vec![None; count];
    let mut dimension = None;
    for item in data {
        let index = item["index"]
            .as_u64()
            .filter(|index| (*index as usize) < count)
            .ok_or("AI 엔진의 임베딩 순서가 올바르지 않습니다.")? as usize;
        let raw = item["embedding"]
            .as_array()
            .filter(|values| !values.is_empty() && values.len() <= MAX_EMBED_DIM)
            .ok_or("AI 엔진의 임베딩 벡터가 올바르지 않습니다.")?;
        if dimension.is_some_and(|dimension| dimension != raw.len()) {
            return Err("AI 엔진의 임베딩 차원이 일치하지 않습니다.".into());
        }
        dimension = Some(raw.len());
        let mut vector = Vec::with_capacity(raw.len());
        let mut norm = 0f64;
        for value in raw {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or("AI 엔진의 임베딩 값이 올바르지 않습니다.")?;
            norm += value * value;
            vector.push(value as f32);
        }
        let norm = norm.sqrt();
        if norm <= 1e-12 {
            return Err("AI 엔진이 빈 임베딩을 반환했습니다.".into());
        }
        for value in &mut vector {
            *value = (*value as f64 / norm) as f32;
        }
        if ordered[index].replace(vector).is_some() {
            return Err("AI 엔진의 임베딩 순서가 중복되었습니다.".into());
        }
    }
    ordered
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "AI 엔진의 임베딩 응답이 완전하지 않습니다.".into())
}

pub(crate) fn valid_execution_mode(mode: &str) -> bool {
    matches!(mode, "auto" | "cpu" | "gpu")
}

fn backend_name(backend: Option<&str>) -> &str {
    match backend {
        Some("cpu") => "CPU",
        Some("metal") => "Metal",
        Some("cuda") => "CUDA",
        Some("vulkan") => "NVIDIA GPU (Vulkan)",
        _ => "미확인",
    }
}

#[cfg(windows)]
struct WorkerJob(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl WorkerJob {
    fn new() -> Result<Self, String> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::HANDLE,
                System::JobObjects::{
                    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
                    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
            },
        };
        // SAFETY: null security attributes make the unnamed job handle non-inheritable.
        let raw = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| format!("AI 작업 보호를 설정하지 못했습니다: {error}"))?;
        // SAFETY: CreateJobObjectW returned a new handle; ownership moves to OwnedHandle.
        let job = Self(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw.0) });
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: the valid structure and handle live throughout this synchronous call.
        unsafe {
            SetInformationJobObject(
                HANDLE(job.0.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| format!("AI 작업 보호를 설정하지 못했습니다: {error}"))?;
        Ok(job)
    }

    fn assign(&self, child: &Child) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::AssignProcessToJobObject};
        let process = child
            .raw_handle()
            .ok_or("AI 작업 프로세스가 이미 종료되었습니다.")?;
        // SAFETY: both handles remain owned by their respective objects for this call.
        unsafe { AssignProcessToJobObject(HANDLE(self.0.as_raw_handle()), HANDLE(process)) }
            .map_err(|error| format!("AI 작업 보호를 적용하지 못했습니다: {error}"))
    }
}

const LOAD_DEADLINE: Duration = Duration::from_secs(180);
/// Embedding a bounded batch is prefill-only; a slow disk load dominates.
const EMBED_DEADLINE: Duration = Duration::from_secs(120);
/// One classification batch. Larger batches lose cancellation granularity and
/// make a dropped future strand more work in the serial server.
const MAX_EMBED_BATCH: usize = 32;
const MAX_EMBED_INPUT_BYTES: usize = 8 * 1024;
const MAX_EMBED_DIM: usize = 8192;
const STOPPED: &str = "AI 엔진이 중지되었습니다. 다음 요청 때 다시 시작합니다.";
/// Classification input is bounded, so a large context only reserves memory.
/// 8K matches the pinned embedding model's trained context.
const EMBED_CONTEXT: u32 = 8192;

/// Bundled `llama-server` builds below the `llama/` resource directory.
/// macOS ships one Metal build that can also run entirely on CPU; Windows
/// ships a Vulkan build for NVIDIA GPUs and an independent CPU build.
fn bundled(llama: &Path, pooling: &str) -> (Option<Launch>, Launch) {
    let name = format!("llama-server{}", std::env::consts::EXE_SUFFIX);
    let launch = |executable: PathBuf, backend| Launch {
        executable,
        backend,
        context: EMBED_CONTEXT,
        pooling: pooling.to_owned(),
    };
    if cfg!(windows) {
        (
            Some(launch(llama.join("vulkan").join(&name), "vulkan")),
            launch(llama.join("cpu").join(&name), "cpu"),
        )
    } else {
        let executable = llama.join("metal").join(&name);
        (
            Some(launch(executable.clone(), "metal")),
            launch(executable, "cpu"),
        )
    }
}

/// Owns the local embedding process. Requests are serial under the owning
/// Tokio mutex; the server starts on the first request for a model.
pub struct Worker {
    gpu: Option<Launch>,
    cpu: Launch,
    runtime: PathBuf,
    execution_mode: String,
    active_gpu: bool,
    fallback_reason: Option<String>,
    server: Option<Server>,
    status: WorkerStatus,
}
impl Worker {
    /// `llama` is the bundled resource directory; `runtime` is the private
    /// app-data directory that holds the socket or access key.
    pub fn configured(llama: &Path, runtime: &Path, mode: &str, pooling: &str) -> Self {
        let (gpu, cpu) = bundled(llama, pooling);
        Self::with_launches(gpu, cpu, runtime, mode)
    }

    fn with_launches(gpu: Option<Launch>, cpu: Launch, runtime: &Path, mode: &str) -> Self {
        let mut worker = Self {
            gpu,
            cpu,
            runtime: runtime.to_owned(),
            execution_mode: mode.to_owned(),
            active_gpu: false,
            fallback_reason: None,
            server: None,
            status: WorkerStatus::unavailable(STOPPED),
        };
        worker.select_initial_backend();
        worker
    }

    /// Windows lets the user pick the device. Other platforms ship one build
    /// that tries Metal first and falls back to CPU, so the mode does not apply.
    fn mode(&self) -> &str {
        if cfg!(windows) {
            &self.execution_mode
        } else {
            "auto"
        }
    }

    fn select_initial_backend(&mut self) {
        let gpu_file = self
            .gpu
            .as_ref()
            .is_some_and(|launch| launch.executable.is_file());
        self.active_gpu = match self.mode() {
            "cpu" => false,
            "gpu" => true,
            _ => gpu_file,
        };
        if self.mode() == "auto" && cfg!(windows) && !gpu_file {
            self.fallback_reason = Some("GPU 작업 파일이 없어 CPU를 사용합니다.".into());
        }
    }

    /// The build in use: the Windows executable chosen for the current mode,
    /// or the single Metal-capable build elsewhere, even when it ran on CPU.
    fn build_backend(&self) -> &'static str {
        match (&self.gpu, cfg!(windows)) {
            (Some(gpu), false) => gpu.backend,
            (Some(gpu), true) if self.active_gpu => gpu.backend,
            _ => self.cpu.backend,
        }
    }

    fn selected_launch(&self) -> Result<Launch, String> {
        if self.active_gpu {
            self.gpu.clone().ok_or("GPU 작업 파일이 없습니다.".into())
        } else {
            Ok(self.cpu.clone())
        }
    }

    pub fn execution_mode(&self) -> &str {
        &self.execution_mode
    }

    pub fn set_execution_mode(&mut self, mode: &str) -> Result<(), String> {
        if !valid_execution_mode(mode) {
            return Err("AI 실행 방식이 올바르지 않습니다.".into());
        }
        if self.server.is_some() {
            return Err("실행 방식을 바꾸기 전에 AI 작업을 종료해야 합니다.".into());
        }
        self.execution_mode = mode.into();
        self.fallback_reason = None;
        self.select_initial_backend();
        Ok(())
    }

    fn add_fallback_detail(&mut self) {
        if let Some(reason) = &self.fallback_reason {
            let detail = self.status.detail.take().unwrap_or_default();
            self.status.detail = Some(format!("CPU 전환: {reason} · {detail}"));
        }
    }

    fn fall_back_to_cpu(&mut self, reason: &str) {
        self.active_gpu = false;
        // Error strings come from this module or bounded server responses.
        self.fallback_reason = Some(reason.chars().take(240).collect());
    }

    fn can_fall_back(&self) -> bool {
        self.active_gpu && self.mode() == "auto"
    }

    /// Starts the server for `path` unless it is already running.
    async fn ensure_server(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_file()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("gguf"))
        {
            return Err("로컬 GGUF 모델 파일을 찾을 수 없습니다.".into());
        }
        self.observe_exit();
        if self
            .server
            .as_ref()
            .is_some_and(|server| server.model == path)
        {
            return Ok(());
        }
        self.stop_server().await;
        let launch = self.selected_launch()?;
        let server = match Server::start(&launch, path, &self.runtime, LOAD_DEADLINE).await {
            Ok(server) => server,
            Err(error) if self.can_fall_back() => {
                self.fall_back_to_cpu(&error);
                Server::start(&self.cpu.clone(), path, &self.runtime, LOAD_DEADLINE).await?
            }
            Err(error) => return Err(error),
        };
        self.server = Some(server);
        Ok(())
    }

    async fn stop_server(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop().await;
        }
    }

    /// Reaps the process, then records the failure for the status surface.
    async fn fail(&mut self, error: &str) {
        self.stop_server().await;
        self.status.state = "failed".into();
        self.status.detail = Some(error.to_owned());
    }

    /// Embeds a bounded batch in order. Vectors are unit length, so callers
    /// only need a dot product. Owner contract: on cancellation, drop the
    /// future, then call `shutdown`.
    pub async fn embed(&mut self, path: &Path, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if inputs.is_empty()
            || inputs.len() > MAX_EMBED_BATCH
            || inputs
                .iter()
                .any(|text| text.trim().is_empty() || text.len() > MAX_EMBED_INPUT_BYTES)
        {
            return Err("임베딩 입력 범위를 확인해 주세요.".into());
        }
        let body = json!({"input": inputs, "model": "rhythm-embedding"}).to_string();
        self.status.state = "busy".into();
        let mut result = self.embed_once(path, &body, inputs.len()).await;
        if let Err(error) = &result {
            if self.can_fall_back() {
                let error = error.clone();
                self.stop_server().await;
                self.fall_back_to_cpu(&error);
                result = self.embed_once(path, &body, inputs.len()).await;
            }
        }
        match &result {
            Ok(_) => {
                self.status = WorkerStatus {
                    state: "ready".into(),
                    runtime_available: true,
                    model: Some(path.to_string_lossy().into_owned()),
                    detail: Some(format!(
                        "빌드 백엔드: {} · 실제 불러온 백엔드: {}",
                        backend_name(Some(self.build_backend())),
                        backend_name(self.server.as_ref().map(|server| server.loaded_backend))
                    )),
                };
                self.add_fallback_detail();
            }
            Err(error) => self.fail(error).await,
        }
        result
    }

    async fn embed_once(
        &mut self,
        path: &Path,
        body: &str,
        count: usize,
    ) -> Result<Vec<Vec<f32>>, String> {
        self.ensure_server(path).await?;
        let server = self
            .server
            .as_mut()
            .ok_or("AI 엔진이 실행 중이 아닙니다.")?;
        let response = server.embeddings(body, EMBED_DEADLINE).await?;
        parse_embeddings(&response, count)
    }

    /// A model file may be removed or replaced next, or an installer may
    /// replace the executable. Do not proceed until the current process is
    /// known to have exited and been reaped.
    pub async fn stop_confirmed(&mut self) -> Result<(), String> {
        if let Some(server) = self.server.take() {
            server.stop_confirmed().await?;
        }
        self.status = WorkerStatus::unavailable(STOPPED);
        Ok(())
    }
    pub fn observe_exit(&mut self) {
        if self.server.as_mut().is_some_and(Server::exited) {
            self.server = None;
            self.status = WorkerStatus {
                state: "failed".into(),
                runtime_available: false,
                model: None,
                detail: Some(
                    "AI 엔진이 예기치 않게 종료되었습니다. 다음 요청 때 다시 시작합니다.".into(),
                ),
            };
        }
    }
    pub fn status(&mut self) -> WorkerStatus {
        self.observe_exit();
        self.status.clone()
    }
    /// Test-only: select the CPU launch directly, the same `-ngl 0 --device none`
    /// command the macOS build falls back to on a GPU failure.
    #[cfg(test)]
    pub(super) fn force_cpu(&mut self) {
        self.gpu = None;
        self.active_gpu = false;
        self.fallback_reason = None;
    }
    #[cfg(test)]
    pub(super) fn process_id(&self) -> Option<u32> {
        self.server.as_ref().and_then(Server::process_id)
    }
    pub async fn shutdown(&mut self) {
        self.stop_server().await;
        self.status = WorkerStatus::unavailable(STOPPED);
    }
}

#[cfg(test)]
impl Worker {
    /// A CPU-only embedding worker against one test executable.
    pub(super) fn embedding(executable: &Path) -> Self {
        let cpu = Launch {
            executable: executable.to_owned(),
            backend: "cpu",
            context: 4096,
            pooling: "last".into(),
        };
        Self::with_launches(None, cpu, &test_runtime(), "auto")
    }
}

/// A fresh runtime directory; short enough for a Unix socket path.
#[cfg(test)]
fn test_runtime() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("rhythm-run-{}-{id}", std::process::id()))
}

/// The staged real build (`node scripts/stage-llama.mjs`), if present.
#[cfg(test)]
pub(crate) fn staged_llama() -> Option<PathBuf> {
    let directory = std::env::var_os("RHYTHM_LLAMA_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries/llama"));
    directory.is_dir().then_some(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    fn fake(mode: &str) -> (tempfile::TempDir, Worker, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let (executable, model) = test_worker::fake_engine(directory.path(), mode);
        (directory, Worker::embedding(&executable), model)
    }

    fn one() -> Vec<String> {
        vec!["공지".into()]
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn job_parent_fixture() {
        let Some(marker) = std::env::var_os("RHYTHM_JOB_TEST_MARKER") else {
            return;
        };
        let directory = std::path::PathBuf::from(std::env::var_os("RHYTHM_JOB_TEST_DIR").unwrap());
        let mut worker = Worker::embedding(&directory.join("llama-server.exe"));
        worker
            .embed(&directory.join("model.gguf"), &["공지".into()])
            .await
            .unwrap();
        let pid = worker.process_id().expect("protected test server started");
        let marker = std::path::PathBuf::from(marker);
        let temporary = marker.with_extension("tmp");
        std::fs::write(&temporary, pid.to_string()).unwrap();
        std::fs::rename(temporary, marker).unwrap();
        // Keep the job handle and server alive until the outer test kills this parent.
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            std::hint::black_box(&worker);
        }
    }

    #[cfg(windows)]
    #[test]
    fn killing_parent_terminates_protected_server() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use std::process::{Child as StdChild, Command as StdCommand, Stdio as StdStdio};
        use std::time::Instant;
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        struct ParentGuard(StdChild);
        impl Drop for ParentGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let directory = tempfile::tempdir().unwrap();
        test_worker::fake_engine(directory.path(), "ok");
        // The fixture embeds a single input.
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        let marker = directory.path().join("server-pid");
        let parent = StdCommand::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "intelligence::tests::job_parent_fixture",
                "--nocapture",
            ])
            .env("RHYTHM_JOB_TEST_MARKER", &marker)
            .env("RHYTHM_JOB_TEST_DIR", directory.path())
            .stdin(StdStdio::null())
            .stdout(StdStdio::null())
            .stderr(StdStdio::null())
            .spawn()
            .unwrap();
        let mut parent = ParentGuard(parent);
        let started = Instant::now();
        while !marker.exists() {
            assert!(
                parent.0.try_wait().unwrap().is_none(),
                "parent fixture exited before protecting the server"
            );
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "parent fixture did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let pid: u32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
        // SAFETY: OpenProcess returns an owned, non-inherited handle to this test's child.
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.unwrap();
        // SAFETY: ownership of the returned handle transfers to OwnedHandle.
        let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
        // SAFETY: process is a live owned process handle and timeouts are bounded.
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 0) },
            WAIT_TIMEOUT
        );
        parent.0.kill().unwrap();
        parent.0.wait().unwrap();
        // The server serves forever; only the job can end it.
        assert_eq!(
            unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 10_000) },
            WAIT_OBJECT_0
        );
    }

    #[tokio::test]
    async fn missing_engine_fails_without_crashing() {
        let directory = tempfile::tempdir().unwrap();
        let mut worker = Worker::embedding(&directory.path().join("missing-llama-server"));
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        assert!(worker.embed(&model, &one()).await.is_err());
        assert!(worker.process_id().is_none());
        assert_eq!(worker.status().state, "failed");
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn non_gguf_or_missing_models_are_rejected_before_launch() {
        let (directory, mut worker, _) = fake("ok");
        let text = directory.path().join("model.txt");
        std::fs::write(&text, b"GGUF").unwrap();
        for path in [text, directory.path().join("missing.gguf")] {
            assert!(worker.embed(&path, &one()).await.is_err());
            assert!(worker.process_id().is_none());
        }
    }

    #[tokio::test]
    async fn requests_reuse_the_process_and_shutdown_reaps_it() {
        let (directory, mut worker, model) = fake("ok");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        worker.embed(&model, &one()).await.unwrap();
        let status = worker.status();
        assert_eq!(status.state, "ready");
        assert_eq!(status.model.as_deref(), model.to_str());
        assert!(status.detail.unwrap().contains("실제 불러온 백엔드: CPU"));
        let pid = worker.process_id().unwrap();
        worker.embed(&model, &one()).await.unwrap();
        assert_eq!(worker.process_id(), Some(pid));
        worker.shutdown().await;
        assert!(worker.process_id().is_none());
        assert!(worker.status().model.is_none());
    }

    #[tokio::test]
    async fn invalid_inputs_fail_before_starting_a_process() {
        let (_directory, mut worker, model) = fake("ok");
        for inputs in [
            Vec::new(),
            vec![" ".to_string()],
            vec!["가".repeat(3 * 1024)],
            vec!["공지".to_string(); MAX_EMBED_BATCH + 1],
        ] {
            assert!(worker.embed(&model, &inputs).await.is_err());
        }
        assert!(worker.process_id().is_none());
    }

    #[tokio::test]
    async fn failed_loads_and_malformed_responses_reap_the_process() {
        for mode in ["exit", "bad-json", "crash-on-embed"] {
            let (directory, mut worker, model) = fake(mode);
            timeout(Duration::from_secs(30), worker.embed(&model, &one()))
                .await
                .unwrap()
                .unwrap_err();
            assert!(worker.process_id().is_none(), "mode: {mode}");
            assert_eq!(worker.status().state, "failed", "mode: {mode}");
            // A later request starts a fresh process without restarting the app.
            std::fs::write(directory.path().join("mode"), "ok").unwrap();
            std::fs::write(directory.path().join("embed-one"), b"").unwrap();
            worker.embed(&model, &one()).await.unwrap();
            worker.shutdown().await;
        }
    }

    #[tokio::test]
    async fn cancelled_load_or_request_is_reaped_before_reuse() {
        for control in ["load-delay", "embed-delay"] {
            let (directory, mut worker, model) = fake("ok");
            std::fs::write(directory.path().join(control), b"").unwrap();
            std::fs::write(directory.path().join("embed-one"), b"").unwrap();
            let inputs = one();
            tokio::select! {
                result = worker.embed(&model, &inputs) => panic!("finished before cancellation: {result:?}"),
                _ = async {
                    while !directory.path().join("entered").exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => (),
            }
            worker.shutdown().await;
            assert!(worker.process_id().is_none(), "{control}");
            // The delay applies once; a fresh process serves the next request.
            assert_eq!(worker.embed(&model, &one()).await.unwrap().len(), 1);
            worker.shutdown().await;
        }
    }

    #[tokio::test]
    async fn unexpected_exit_is_observed_and_the_next_request_restarts() {
        let (directory, mut worker, model) = fake("ok");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        worker.embed(&model, &one()).await.unwrap();
        worker.server.as_mut().unwrap().kill_for_test().await;
        assert_eq!(worker.status().state, "failed");
        assert!(worker.process_id().is_none());
        assert_eq!(worker.embed(&model, &one()).await.unwrap().len(), 1);
        worker.shutdown().await;
        assert!(worker.process_id().is_none());
    }

    // macOS ships one Metal-capable build, reported as the build even when it
    // runs on CPU. Windows reports the build in use (`gpu_tests`).
    #[cfg(not(windows))]
    #[tokio::test]
    async fn failed_metal_load_retries_on_cpu_and_reports_both_backends() {
        let directory = tempfile::tempdir().unwrap();
        let (executable, model) = test_worker::fake_engine(directory.path(), "gpu-fails");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        let launch = |backend| Launch {
            executable: executable.clone(),
            backend,
            context: 4096,
            pooling: "last".into(),
        };
        let mut worker = Worker::with_launches(
            Some(launch("metal")),
            launch("cpu"),
            &test_runtime(),
            "auto",
        );
        worker.embed(&model, &one()).await.unwrap();
        let detail = worker.status().detail.unwrap();
        assert!(detail.contains("빌드 백엔드: Metal"), "{detail}");
        assert!(detail.contains("실제 불러온 백엔드: CPU"), "{detail}");
        assert!(detail.contains("CPU 전환"), "{detail}");
        worker.shutdown().await;

        std::fs::write(directory.path().join("mode"), "ok").unwrap();
        let mut worker = Worker::with_launches(
            Some(launch("metal")),
            launch("cpu"),
            &test_runtime(),
            "auto",
        );
        worker.embed(&model, &one()).await.unwrap();
        let detail = worker.status().detail.unwrap();
        assert!(detail.contains("실제 불러온 백엔드: Metal"), "{detail}");
        worker.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn socket_lives_in_a_private_directory() {
        use std::os::unix::fs::PermissionsExt;
        let (directory, mut worker, model) = fake("ok");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        worker.embed(&model, &one()).await.unwrap();
        let mode = std::fs::metadata(&worker.runtime)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(worker.runtime.join("llama.sock").exists());
        worker.shutdown().await;
        assert!(!worker.runtime.join("llama.sock").exists());
    }

    #[tokio::test]
    async fn loopback_requires_the_per_launch_key_and_removes_it_after_start() {
        let directory = tempfile::tempdir().unwrap();
        let (executable, model) = test_worker::fake_engine(directory.path(), "ok");
        std::fs::write(directory.path().join("embed-one"), b"").unwrap();
        // Beyond the Unix socket path limit, the server must use loopback.
        let runtime = test_runtime().join("r".repeat(120));
        let cpu = Launch {
            executable,
            backend: "cpu",
            context: 4096,
            pooling: "last".into(),
        };
        let mut worker = Worker::with_launches(None, cpu, &runtime, "auto");
        assert_eq!(worker.embed(&model, &one()).await.unwrap().len(), 1);
        assert!(!runtime.join("llama.key").exists());
        assert!(!runtime.join("llama.sock").exists());
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn real_server_lifecycle_without_model_weights() {
        let Some(llama) = staged_llama() else {
            eprintln!(
                "SKIPPED real llama-server lifecycle: run `node scripts/stage-llama.mjs` first"
            );
            return;
        };
        let directory = tempfile::tempdir().unwrap();
        let mut worker = Worker::configured(&llama, &test_runtime(), "auto", "last");
        // An invalid model makes the real server exit; it must be reaped.
        let bogus = directory.path().join("bogus.gguf");
        std::fs::write(&bogus, b"not a model").unwrap();
        assert!(
            timeout(Duration::from_secs(60), worker.embed(&bogus, &one()))
                .await
                .unwrap()
                .is_err()
        );
        assert!(worker.process_id().is_none());
        assert_eq!(worker.status().state, "failed");
        worker.shutdown().await;
    }

    #[tokio::test]
    async fn embedding_requests_use_the_embeddings_endpoint_and_normalize() {
        let (directory, mut worker, model) = fake("ok");
        let vectors = worker
            .embed(&model, &["첫 공지".into(), "둘째 공지".into()])
            .await
            .unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 2);
        assert!((vectors[0][0] - 0.6).abs() < 1e-6);
        assert!((vectors[0][1] - 0.8).abs() < 1e-6);
        assert!((vectors[1][1] - 1.0).abs() < 1e-6);
        let requests = std::fs::read_to_string(directory.path().join("requests")).unwrap();
        assert!(requests.contains("embedding"), "{requests}");
        worker.shutdown().await;
    }

    #[test]
    fn embedding_responses_reject_malformed_vectors() {
        let valid =
            json!({"data":[{"index":0,"embedding":[3.0,4.0]},{"index":1,"embedding":[0.0,5.0]}]});
        let vectors = parse_embeddings(&valid, 2).unwrap();
        assert!((vectors[0][0] - 0.6).abs() < 1e-6);
        assert!((vectors[1][1] - 1.0).abs() < 1e-6);
        for invalid in [
            json!({"data":[]}),
            json!({"data":[{"index":0,"embedding":[1.0,0.0]}]}),
            json!({"data":[{"index":1,"embedding":[1.0,0.0]},{"index":1,"embedding":[1.0,0.0]}]}),
            json!({"data":[{"index":0,"embedding":[]},{"index":1,"embedding":[1.0]}]}),
            json!({"data":[{"index":0,"embedding":[0.0,0.0]},{"index":1,"embedding":[1.0,0.0]}]}),
            json!({"data":[{"index":0,"embedding":["x"]},{"index":1,"embedding":[1.0]}]}),
        ] {
            assert!(parse_embeddings(&invalid, 2).is_err(), "{invalid}");
        }
    }
}
