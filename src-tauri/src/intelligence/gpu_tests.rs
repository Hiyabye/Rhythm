use super::*;
use std::sync::OnceLock;

fn fixture(mode: &str) -> tempfile::TempDir {
    static BUILT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let built = BUILT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        test_worker::compile(dir.path(), "worker", r#"
use std::io::{self,BufRead,Write};
fn main() {
 let exe=std::env::current_exe().unwrap();
 let gpu=exe.file_stem().unwrap().to_string_lossy().ends_with("-gpu");
 let mode=std::fs::read_to_string(exe.parent().unwrap().join("gpu-mode")).unwrap_or_default();
 let mut loaded=false;
 for line in io::stdin().lock().lines() {
  let line=line.unwrap();
  let id=line.split("\"id\":").nth(1).unwrap().split(|c:char| !c.is_ascii_digit()).next().unwrap();
  if line.contains("\"shutdown\"") { break; }
  if gpu && (mode=="fail" || mode=="legacy-cpu") && !line.contains("\"status\"") {
   println!("{{\"version\":1,\"id\":{id},\"ok\":false,\"error\":\"GPU memory exhausted\"}}");
  } else if gpu && mode=="oversize" && line.contains("\"infer\"") && line.contains("긴 공지") {
   println!("{{\"version\":1,\"id\":{id},\"ok\":false,\"error\":\"GPU 분석 입력과 출력이 8192 토큰 한도를 넘습니다. 본문 길이를 줄이거나 CPU를 사용해 주세요.\"}}");
  } else if line.contains("\"infer\"") {
   let backend=if gpu {"vulkan"} else {"cpu"};
   println!("{{\"version\":1,\"id\":{id},\"ok\":true,\"result\":{{\"text\":\"한국어 결과\",\"loaded_backend\":\"{backend}\"}}}}");
  } else {
   if line.contains("\"load\"") { loaded=true; }
   let engine=if (gpu && mode=="misreport") || (!gpu && mode=="legacy-cpu") {"mistral.rs"} else {"llama.cpp"};
   let backend=if gpu && mode!="misreport" {"vulkan"} else {"cpu"};
   let model=if loaded {"\"test.gguf\""} else {"null"};
   println!("{{\"version\":1,\"id\":{id},\"ok\":true,\"result\":{{\"engine\":\"{engine}\",\"runtime_available\":true,\"analysis_json_v1\":true,\"model\":{model},\"build_backend\":\"{backend}\",\"loaded_backend\":\"{backend}\"}}}}");
  }
  io::stdout().flush().unwrap();
 }
}
"#);
        dir
    });
    let dir = tempfile::tempdir().unwrap();
    for name in ["rhythm-ai-worker.exe", "rhythm-ai-worker-gpu.exe"] {
        std::fs::hard_link(built.path().join("worker.exe"), dir.path().join(name)).unwrap();
    }
    std::fs::write(dir.path().join("gpu-mode"), mode).unwrap();
    dir
}

#[tokio::test]
async fn gpu_auto_selects_gpu_and_cpu_override_is_respected() {
    let dir = fixture("ok");
    for (mode, backend) in [("auto", "vulkan"), ("gpu", "vulkan"), ("cpu", "cpu")] {
        let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), mode);
        assert_eq!(worker.refresh().await.state, "ready");
        let result = worker
            .infer_analysis(Path::new("test.gguf"), "공지 분석", 32)
            .await
            .unwrap();
        assert_eq!(result.metrics.loaded_backend.as_deref(), Some(backend));
        assert!(worker.set_execution_mode("cpu").is_err());
        worker.stop_for_inventory().await.unwrap();
        worker.refresh().await;
        assert!(
            worker.process_id().is_none(),
            "status refresh must not undo an intentional stop"
        );
        worker.set_execution_mode("cpu").unwrap();
        assert!(worker.set_execution_mode("invalid").is_err());
    }
}

#[tokio::test]
async fn gpu_failure_reaps_and_falls_back_once_only_in_auto_mode() {
    let dir = fixture("fail");
    for method in ["load", "infer"] {
        let mut worker =
            Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "auto");
        worker.refresh().await;
        let gpu_pid = worker.process_id();
        if method == "load" {
            assert_eq!(
                worker.load(Path::new("test.gguf")).await.unwrap().state,
                "ready"
            );
        } else {
            assert_eq!(
                worker
                    .infer_analysis(Path::new("test.gguf"), "공지", 32)
                    .await
                    .unwrap()
                    .metrics
                    .loaded_backend
                    .as_deref(),
                Some("cpu")
            );
        }
        assert_ne!(worker.process_id(), gpu_pid);
        assert!(!worker.active_gpu);
        assert!(worker.status().runtime_available);
        assert!(worker.status().detail.unwrap().contains("CPU 전환"));
        worker.restart().await.unwrap();
        assert!(!worker.active_gpu);
        worker.shutdown().await;
    }
    let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "gpu");
    worker.refresh().await;
    assert!(worker.load(Path::new("test.gguf")).await.is_err());
    assert!(worker.process_id().is_none());
    assert!(worker.fallback_reason.is_none());
}

#[tokio::test]
async fn gpu_missing_binary_is_reported_and_auto_can_still_run() {
    let dir = fixture("ok");
    std::fs::remove_file(dir.path().join("rhythm-ai-worker-gpu.exe")).unwrap();
    let mut auto = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "auto");
    assert_eq!(auto.refresh().await.state, "ready");
    assert!(auto.status().detail.unwrap().contains("CPU 전환"));
    auto.shutdown().await;
    let mut gpu = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "gpu");
    assert_ne!(gpu.refresh().await.state, "ready");
    assert!(gpu.process_id().is_none());
}

#[tokio::test]
async fn gpu_wrong_engine_cannot_satisfy_forced_gpu_and_auto_recovers() {
    let dir = fixture("misreport");
    for mode in ["auto", "gpu"] {
        let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), mode);
        let status = worker.refresh().await;
        if mode == "auto" {
            assert_eq!(status.state, "ready");
            assert!(!worker.active_gpu);
            assert!(status.detail.unwrap().contains("CPU 전환"));
        } else {
            assert_eq!(status.state, "failed");
            assert!(worker.process_id().is_none());
        }
        worker.shutdown().await;
    }
}

#[tokio::test]
async fn auto_does_not_retry_inference_on_a_previous_cpu_engine() {
    let dir = fixture("legacy-cpu");
    let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "auto");
    assert_eq!(worker.refresh().await.state, "ready");
    let error = worker
        .infer_analysis(Path::new("test.gguf"), "공지", 32)
        .await
        .unwrap_err();
    assert!(error.contains("llama.cpp"));
    assert!(worker.process_id().is_none());
}

#[tokio::test]
async fn gpu_unlaunchable_binary_falls_back_without_starting_after_stop() {
    let dir = fixture("ok");
    let gpu = dir.path().join("rhythm-ai-worker-gpu.exe");
    // Detach the fixture hard link before replacing only this test's GPU file.
    std::fs::remove_file(&gpu).unwrap();
    std::fs::write(&gpu, b"not an executable").unwrap();
    let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "auto");
    assert_eq!(worker.refresh().await.state, "ready");
    assert!(!worker.active_gpu);
    worker.stop().await;
    worker.refresh().await;
    assert!(worker.process_id().is_none());
}

#[tokio::test]
async fn gpu_input_overflow_runs_that_request_on_cpu_then_returns_to_gpu() {
    let dir = fixture("oversize");
    let mut worker = Worker::launch_configured(&dir.path().join("rhythm-ai-worker.exe"), "auto");
    assert_eq!(worker.refresh().await.state, "ready");
    let long = worker
        .infer_analysis(Path::new("test.gguf"), "긴 공지", 32)
        .await
        .unwrap();
    assert_eq!(long.metrics.loaded_backend.as_deref(), Some("cpu"));
    let next = worker
        .infer_analysis(Path::new("test.gguf"), "짧은 공지", 32)
        .await
        .unwrap();
    assert_eq!(next.metrics.loaded_backend.as_deref(), Some("vulkan"));
    worker.shutdown().await;
}
