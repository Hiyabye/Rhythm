use super::*;

/// The bundled layout with the fake server as both builds: `vulkan/` uses
/// `gpu_mode`, `cpu/` always succeeds.
fn fixture(gpu_mode: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    test_worker::link_engine(&dir.path().join("vulkan"), gpu_mode);
    let (_, model) = test_worker::fake_engine(&dir.path().join("cpu"), "ok");
    (dir, model)
}

fn configured(dir: &tempfile::TempDir, mode: &str) -> Worker {
    Worker::launch_configured(dir.path(), &test_runtime(), mode)
}

#[tokio::test]
async fn gpu_auto_selects_gpu_and_cpu_override_is_respected() {
    let (dir, model) = fixture("ok");
    for (mode, backend) in [("auto", "vulkan"), ("gpu", "vulkan"), ("cpu", "cpu")] {
        let mut worker = configured(&dir, mode);
        assert_eq!(worker.refresh().await.state, "ready");
        let result = worker
            .infer_analysis(&model, "공지 분석", 32)
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
async fn gpu_failure_falls_back_once_only_in_auto_mode() {
    for gpu_mode in ["gpu-fails", "silent-cpu", "no-nvidia"] {
        let (dir, model) = fixture(gpu_mode);
        for method in ["load", "infer"] {
            let mut worker = configured(&dir, "auto");
            if method == "load" {
                assert_eq!(worker.load(&model).await.unwrap().state, "ready");
            } else {
                let result = worker.infer_analysis(&model, "공지", 32).await.unwrap();
                assert_eq!(result.metrics.loaded_backend.as_deref(), Some("cpu"));
            }
            assert!(!worker.active_gpu, "{gpu_mode}");
            assert!(worker.status().detail.unwrap().contains("CPU 전환"));
            worker.restart().await.unwrap();
            assert!(!worker.active_gpu, "auto keeps CPU for the session");
            worker.shutdown().await;
        }
        // Forced GPU reports the failure instead of silently using the CPU.
        let mut worker = configured(&dir, "gpu");
        assert!(worker.load(&model).await.is_err(), "{gpu_mode}");
        assert!(worker.process_id().is_none());
        assert!(worker.fallback_reason.is_none());
    }
}

#[tokio::test]
async fn gpu_missing_binary_is_reported_and_auto_can_still_run() {
    let (dir, model) = fixture("ok");
    std::fs::remove_file(dir.path().join("vulkan").join("llama-server.exe")).unwrap();
    let mut auto = configured(&dir, "auto");
    assert_eq!(auto.refresh().await.state, "ready");
    assert!(auto.status().detail.unwrap().contains("CPU 전환"));
    assert_eq!(
        auto.infer(&model, "공지", 32).await.unwrap(),
        "plain output"
    );
    auto.shutdown().await;
    let mut gpu = configured(&dir, "gpu");
    assert_ne!(gpu.refresh().await.state, "ready");
    assert!(gpu.load(&model).await.is_err());
    assert!(gpu.process_id().is_none());
}

#[tokio::test]
async fn gpu_unlaunchable_binary_falls_back_without_starting_after_stop() {
    let (dir, model) = fixture("ok");
    let gpu = dir.path().join("vulkan").join("llama-server.exe");
    // Detach the shared hard link before replacing only this test's GPU file.
    std::fs::remove_file(&gpu).unwrap();
    std::fs::write(&gpu, b"not an executable").unwrap();
    let mut worker = configured(&dir, "auto");
    assert_eq!(worker.load(&model).await.unwrap().state, "ready");
    assert!(!worker.active_gpu);
    worker.stop().await;
    worker.refresh().await;
    assert!(worker.process_id().is_none());
}

#[tokio::test]
async fn gpu_input_overflow_runs_that_request_on_cpu_then_returns_to_gpu() {
    let (dir, model) = fixture("overflow");
    let mut worker = configured(&dir, "auto");
    assert_eq!(worker.refresh().await.state, "ready");
    let long = worker.infer_analysis(&model, "긴 공지", 32).await.unwrap();
    assert_eq!(long.metrics.loaded_backend.as_deref(), Some("cpu"));
    let next = worker
        .infer_analysis(&model, "짧은 공지", 32)
        .await
        .unwrap();
    assert_eq!(next.metrics.loaded_backend.as_deref(), Some("vulkan"));
    worker.shutdown().await;
}
