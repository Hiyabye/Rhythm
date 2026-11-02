fn main() {
    // Windows release workers link the VC runtime statically while keeping the
    // Windows-provided Universal CRT dynamic, matching the application.
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    if target == "x86_64-pc-windows-msvc" && std::env::var("PROFILE").as_deref() == Ok("release") {
        static_vcruntime::metabuild();
    }
}
