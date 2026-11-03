fn main() {
    println!("cargo:rerun-if-env-changed=RHYTHM_WORKER_BUILD_ID");
    let build_id = std::env::var("RHYTHM_WORKER_BUILD_ID").unwrap_or_else(|_| "development".into());
    assert!(
        build_id == "development"
            || (build_id.len() == 64 && build_id.bytes().all(|b| b.is_ascii_hexdigit())),
        "invalid worker build identity"
    );
    println!("cargo:rustc-env=RHYTHM_WORKER_BUILD_ID={build_id}");
    // Windows release workers link the VC runtime statically while keeping the
    // Windows-provided Universal CRT dynamic, matching the application.
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    if target == "x86_64-pc-windows-msvc" && std::env::var("PROFILE").as_deref() == Ok("release") {
        static_vcruntime::metabuild();
    }
}
