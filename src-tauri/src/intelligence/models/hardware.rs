use super::*;

#[derive(Clone, Serialize)]
pub struct Hardware {
    pub os: &'static str,
    pub arch: &'static str,
    pub ram_bytes: u64,
    pub disk_available_bytes: u64,
    pub backend: &'static str,
}
pub fn hardware(root: &Path) -> Hardware {
    Hardware {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        ram_bytes: ram_bytes(),
        disk_available_bytes: available_space(root).unwrap_or(0),
        backend: BACKEND_LABEL,
    }
}
/// Refresh free disk without repeating the physical-RAM platform probe.
/// Call from a blocking task, outside SQLite/worker locks.
pub fn refresh_disk_space(mut hardware: Hardware, root: &Path) -> Hardware {
    hardware.disk_available_bytes = available_space(root).unwrap_or(0);
    hardware
}
// Main-process features cannot prove which backend the bundled worker uses;
// on Windows the runtime status separately confirms the device actually used.
#[cfg(windows)]
pub(super) const BACKEND_LABEL: &str =
    "CPU · NVIDIA GPU (Vulkan) · 실제 실행 장치는 엔진 상태에서 확인";
#[cfg(not(windows))]
pub(super) const BACKEND_LABEL: &str = "작업 프로세스 빌드에 따름 · 가속 사용 여부 미확인";
#[cfg(target_os = "macos")]
pub(super) fn ram_bytes() -> u64 {
    std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
#[cfg(windows)]
pub(super) fn ram_bytes() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut info = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: info has the required size and remains valid for the call.
    unsafe { GlobalMemoryStatusEx(&mut info) }
        .map(|()| info.ullTotalPhys)
        .unwrap_or(0)
}
#[cfg(not(any(target_os = "macos", windows)))]
pub(super) fn ram_bytes() -> u64 {
    0
}
pub(crate) fn available_space(root: &Path) -> Result<u64, String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // SAFETY: wide is NUL-terminated and available is a valid output pointer.
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) }
            .map_err(|_| "사용 가능한 저장 공간을 확인하지 못했습니다.")?;
        Ok(available)
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("df")
            .arg("-Pk")
            .arg(root)
            .output()
            .map_err(|_| "사용 가능한 저장 공간을 확인하지 못했습니다.")?;
        if !output.status.success() {
            return Err("저장 공간을 확인하지 못했습니다.".into());
        }
        let text =
            String::from_utf8(output.stdout).map_err(|_| "저장 공간 응답이 올바르지 않습니다.")?;
        let bytes = text
            .lines()
            .last()
            .and_then(|s| s.split_whitespace().nth(3))
            .and_then(|s| s.parse::<u64>().ok())
            .and_then(|v| v.checked_mul(1024));
        bytes.ok_or_else(|| "사용 가능한 저장 공간을 읽지 못했습니다.".into())
    }
}
pub(super) fn require_space(free: u64, remaining: u64) -> Result<(), String> {
    // Leave room for SQLite/WAL and the OS; never consume the last free bytes.
    if free < remaining.saturating_add(1024 * 1024 * 1024) {
        return Err(
            "모델 설치 공간이 부족합니다. 모델 크기 외에 1 GiB 이상의 여유 공간이 필요합니다."
                .into(),
        );
    }
    Ok(())
}
