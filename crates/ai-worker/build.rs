fn main() {
    let host = std::env::var("HOST").expect("Cargo must provide HOST");
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    if host != "x86_64-pc-windows-msvc" || target != host {
        panic!("Rhythm AI worker requires a native Windows x64 MSVC build");
    }
    // Match the application's release linkage: ship the VC runtime statically
    // while retaining the Windows-provided Universal CRT dynamically.
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        static_vcruntime::metabuild();
    }
}
