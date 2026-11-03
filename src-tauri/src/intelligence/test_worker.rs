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
