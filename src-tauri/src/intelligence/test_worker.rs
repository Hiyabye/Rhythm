//! Controlled worker executables for process-boundary tests.
//!
//! Each fixture is compiled once per test process from fixed source and launched
//! once before timed protocol requests. This separates executable startup from
//! the request deadline, including on machines with slow security scanning.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Compiles `source` into `directory/name` and completes its first launch.
/// The fixture must exit promptly when stdin is closed and no arguments are given.
pub fn compile(directory: &Path, name: &str, source: &str) -> PathBuf {
    let source_path = directory.join(format!("{name}.rs"));
    std::fs::write(&source_path, source).unwrap();
    let executable = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new("rustc")
        .args(["--edition=2021", "--crate-name", &name.replace('-', "_")])
        .arg(&source_path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    admit_first_launch(&executable);
    executable
}

fn admit_first_launch(executable: &Path) {
    let mut child = Command::new(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // A bounded failure here reports the OS launch boundary directly instead of
    // surfacing later as a misleading protocol timeout.
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "the operating system did not start test fixture {} within 120 seconds",
                executable.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A controlled stand-in for upstream `llama-server`: the same flags, log
/// lines, Unix-socket/loopback listener, API-key check and response shapes
/// the supervisor relies on. Behavior comes from files beside the invoked
/// executable, so restarts keep it: `mode` selects a failure, and the analysis
/// and command tests use the marker/control files documented in `FAKE_SERVER`.
pub fn fake_llama_server() -> &'static Path {
    static BUILT: std::sync::OnceLock<(tempfile::TempDir, PathBuf)> = std::sync::OnceLock::new();
    let (_, executable) = BUILT.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap();
        let executable = compile(directory.path(), "fake-llama-server", FAKE_SERVER);
        (directory, executable)
    });
    executable
}

/// Links the fake server into `directory` as `llama-server` with `mode`.
pub fn link_engine(directory: &Path, mode: &str) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let executable = directory.join(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
    std::fs::hard_link(fake_llama_server(), &executable).unwrap();
    std::fs::write(directory.join("mode"), mode).unwrap();
    executable
}

/// Like `link_engine`, also returning a model file the fake can load.
pub fn fake_engine(directory: &Path, mode: &str) -> (PathBuf, PathBuf) {
    let executable = link_engine(directory, mode);
    let model = directory.join("model.gguf");
    std::fs::write(&model, b"GGUF").unwrap();
    (executable, model)
}

/// Modes: `ok`, `exit` (fails to load), `gpu-fails` (fails only with GPU
/// layers), `silent-cpu` (never offloads), `no-nvidia` (no Vulkan device),
/// `overflow` (context error for "긴 공지" on GPU), `bad-json`, `bad-metrics`,
/// `mmproj-fails` (never logs the multimodal encoder as loaded),
/// `crash-on-chat`. Control files: `load-delay`/`chat-delay` hang only the
/// first load/request; `delay` hangs every request while it exists; each
/// request writes `entered` and appends a line to `requests`; `response`
/// replaces the generated text; `malformed` returns unstructured text
/// (`repair-once` clears it after one request).
const FAKE_SERVER: &str = r####"
use std::io::{BufRead, BufReader, Read, Write};
trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 { return; }
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_owned();
    let mode = std::fs::read_to_string(dir.join("mode")).unwrap_or_else(|_| "ok".into());
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    if args.iter().any(|a| a == "--list-devices") {
        println!("Available devices:");
        if mode != "no-nvidia" { println!("  Vulkan0: NVIDIA GeForce GTX 1650 (4096 MiB, 3500 MiB free)"); }
        return;
    }
    let gpu = value("-ngl").as_deref() != Some("0");
    if mode == "exit" || (mode == "gpu-fails" && gpu) { eprintln!("error: failed to load model"); std::process::exit(1); }
    let device = if value("--device").is_some_and(|d| d.starts_with("Vulkan")) { "Vulkan0" } else { "MTL0" };
    eprintln!("I llama_prepare_model_devices: using device {device} (Fake) - 4096 MiB free");
    eprintln!("I load_tensors: offloaded {}/4 layers to GPU", if gpu && mode != "silent-cpu" { 4 } else { 0 });
    if let Some(projector) = value("--mmproj") {
        if mode != "mmproj-fails" {
            eprintln!("I srv    load_model: loaded multimodal model, '{projector}'");
        }
    }
    if dir.join("load-delay").exists() && !dir.join("entered").exists() {
        std::fs::write(dir.join("entered"), b"entered").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    let host = value("--host").unwrap();
    let key = value("--api-key-file").map(|file| std::fs::read_to_string(file).unwrap());
    let accept: Box<dyn FnMut() -> Box<dyn Stream>> = if host.ends_with(".sock") {
        #[cfg(unix)]
        {
            let listener = std::os::unix::net::UnixListener::bind(&host).unwrap();
            Box::new(move || Box::new(listener.accept().unwrap().0) as Box<dyn Stream>)
        }
        #[cfg(not(unix))]
        panic!("no Unix sockets")
    } else {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        eprintln!("I srv  server_on_li: listening on http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        Box::new(move || Box::new(listener.accept().unwrap().0) as Box<dyn Stream>)
    };
    let mut accept = accept;
    loop {
        let mut stream = accept();
        let mut reader = BufReader::new(&mut stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 { continue; }
        let mut length = 0;
        let mut authorized = key.is_none();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() { break; }
            if let Some(v) = line.strip_prefix("Content-Length: ") { length = v.trim().parse().unwrap(); }
            if let (Some(v), Some(key)) = (line.strip_prefix("Authorization: Bearer "), &key) { authorized = v.trim() == key; }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        let (status, response) = if request_line.starts_with("GET /health") {
            (200, r#"{"status":"ok"}"#.to_string())
        } else if !authorized {
            (401, r#"{"error":{"code":401,"type":"authentication_error"}}"#.to_string())
        } else if request_line.starts_with("POST /v1/embeddings") {
            embeddings(&dir, &body)
        } else {
            chat(&dir, &mode, gpu, &body)
        };
        let reason = if status == 200 { "OK" } else { "Error" };
        let _ = write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}", response.len());
    }
}
fn embeddings(dir: &std::path::Path, _body: &str) -> (u16, String) {
    let mut requests = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("requests")).unwrap();
    writeln!(requests, "embedding").unwrap();
    // Two deterministic vectors, normalized by the parent. The fixture is
    // compiled without third-party crates, so it does not parse the input.
    (200, r#"{"data":[{"index":0,"embedding":[3.0,4.0]},{"index":1,"embedding":[0.0,5.0]}]}"#.to_string())
}
fn chat(dir: &std::path::Path, mode: &str, gpu: bool, body: &str) -> (u16, String) {
    let mut requests = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("requests")).unwrap();
    writeln!(requests, "{}", if body.contains("\"response_format\"") { "analysis" } else { "plain" }).unwrap();
    let images = body.matches("\"type\":\"image_url\"").count();
    if images > 0 {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("images")).unwrap();
        writeln!(file, "{images}").unwrap();
    }
    let first = !dir.join("entered").exists();
    std::fs::write(dir.join("entered"), b"entered").unwrap();
    if dir.join("delay").exists() || (dir.join("chat-delay").exists() && first) {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
    match mode {
        "crash-on-chat" => std::process::exit(1),
        "bad-json" => return (200, "not-json".into()),
        "overflow" if gpu && body.contains("긴 공지") => return (400, r#"{"error":{"code":400,"message":"request (9000 tokens) exceeds the available context size (8192 tokens)","type":"exceed_context_size_error"}}"#.into()),
        _ => (),
    }
    let text = if dir.join("malformed").exists() {
        if dir.join("repair-once").exists() { std::fs::remove_file(dir.join("malformed")).unwrap(); }
        "not structured JSON".to_string()
    } else if let Ok(text) = std::fs::read_to_string(dir.join("response")) {
        text
    } else if body.contains("\"response_format\"") {
        "analysis output".into()
    } else {
        "plain output".into()
    };
    let timings = if mode == "bad-metrics" { r#"{"prompt_n":"twenty"}"# } else { r#"{"prompt_n":20,"prompt_ms":1000.0,"predicted_n":10,"predicted_ms":1000.0}"# };
    let text = format!("{text:?}");
    (200, format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":{text}}},"finish_reason":"stop"}}],"timings":{timings}}}"#))
}
"####;
