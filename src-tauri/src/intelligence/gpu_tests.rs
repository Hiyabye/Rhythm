use super::*;

/// The bundled layout with the fake server as both builds: `vulkan/` uses
/// `gpu_mode`, `cpu/` always succeeds. Both answer single-input requests.
fn fixture(gpu_mode: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    test_worker::link_engine(&dir.path().join("vulkan"), gpu_mode);
    let (_, model) = test_worker::fake_engine(&dir.path().join("cpu"), "ok");
    for build in ["vulkan", "cpu"] {
        std::fs::write(dir.path().join(build).join("embed-one"), b"").unwrap();
    }
    (dir, model)
}

fn configured(dir: &tempfile::TempDir, mode: &str) -> Worker {
    Worker::configured(dir.path(), &test_runtime(), mode, "last")
}

fn loaded(worker: &Worker) -> Option<&'static str> {
    worker.server.as_ref().map(|server| server.loaded_backend)
}

fn one() -> Vec<String> {
    vec!["공지".into()]
}

#[tokio::test]
async fn gpu_auto_selects_gpu_and_cpu_override_is_respected() {
    let (dir, model) = fixture("ok");
    for (mode, backend) in [("auto", "vulkan"), ("gpu", "vulkan"), ("cpu", "cpu")] {
        let mut worker = configured(&dir, mode);
        worker.embed(&model, &one()).await.unwrap();
        assert_eq!(loaded(&worker), Some(backend));
        assert!(worker.set_execution_mode("cpu").is_err());
        worker.stop_confirmed().await.unwrap();
        assert!(worker.process_id().is_none());
        worker.set_execution_mode("cpu").unwrap();
        assert!(worker.set_execution_mode("invalid").is_err());
    }
}

#[tokio::test]
async fn gpu_failure_falls_back_once_only_in_auto_mode() {
    for gpu_mode in ["gpu-fails", "silent-cpu", "no-nvidia"] {
        let (dir, model) = fixture(gpu_mode);
        let mut worker = configured(&dir, "auto");
        worker.embed(&model, &one()).await.unwrap();
        assert_eq!(loaded(&worker), Some("cpu"), "{gpu_mode}");
        assert!(!worker.active_gpu, "{gpu_mode}");
        assert!(worker.status().detail.unwrap().contains("CPU 전환"));
        worker.shutdown().await;
        worker.embed(&model, &one()).await.unwrap();
        assert!(!worker.active_gpu, "auto keeps CPU for the session");
        worker.shutdown().await;
        // Forced GPU reports the failure instead of silently using the CPU.
        let mut worker = configured(&dir, "gpu");
        assert!(worker.embed(&model, &one()).await.is_err(), "{gpu_mode}");
        assert!(worker.process_id().is_none());
        assert!(worker.fallback_reason.is_none());
    }
}

#[tokio::test]
async fn gpu_missing_binary_is_reported_and_auto_can_still_run() {
    let (dir, model) = fixture("ok");
    std::fs::remove_file(dir.path().join("vulkan").join("llama-server.exe")).unwrap();
    let mut auto = configured(&dir, "auto");
    assert!(auto.fallback_reason.is_some());
    auto.embed(&model, &one()).await.unwrap();
    assert!(auto.status().detail.unwrap().contains("CPU 전환"));
    auto.shutdown().await;
    let mut gpu = configured(&dir, "gpu");
    assert!(gpu.embed(&model, &one()).await.is_err());
    assert!(gpu.process_id().is_none());
}

#[tokio::test]
async fn gpu_unlaunchable_binary_falls_back() {
    let (dir, model) = fixture("ok");
    let gpu = dir.path().join("vulkan").join("llama-server.exe");
    // Detach the shared hard link before replacing only this test's GPU file.
    std::fs::remove_file(&gpu).unwrap();
    std::fs::write(&gpu, b"not an executable").unwrap();
    let mut worker = configured(&dir, "auto");
    worker.embed(&model, &one()).await.unwrap();
    assert!(!worker.active_gpu);
    worker.shutdown().await;
    assert!(worker.process_id().is_none());
}
