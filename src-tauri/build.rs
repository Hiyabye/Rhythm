fn main() {
    assert_eq!(
        std::env::var("TARGET").as_deref(),
        Ok("x86_64-pc-windows-msvc"),
        "Rhythm supports Windows 11 x64 with the MSVC toolchain."
    );
    tauri_build::build()
}
