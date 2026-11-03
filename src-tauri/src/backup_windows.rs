//! Windows restore primitives. These provide process-interruption recovery on
//! a local fixed NTFS volume; they do not claim power-loss atomicity.
use std::{
    ffi::OsStr,
    fs,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
};

const REPARSE_POINT: u32 = 0x400;
const DRIVE_FIXED: u32 = 3;
const MOVEFILE_WRITE_THROUGH: u32 = 8;
// HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT), winerror.h.
const NOT_UNDER_SYNC_ROOT: i32 = 0x80070186u32 as i32;

#[link(name = "kernel32")]
extern "system" {
    fn GetVolumePathNameW(path: *const u16, volume: *mut u16, length: u32) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        filesystem: *mut u16,
        filesystem_len: u32,
    ) -> i32;
    fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
}

#[link(name = "CldApi")]
extern "system" {
    fn CfGetSyncRootInfoByPath(
        path: *const u16,
        info_class: i32,
        buffer: *mut std::ffi::c_void,
        buffer_len: u32,
        returned_len: *mut u32,
    ) -> i32;
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

pub(super) fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_reparse_attribute(metadata.file_attributes())
}

fn is_reparse_attribute(attributes: u32) -> bool {
    attributes & REPARSE_POINT != 0
}

fn cloud_root_status(status: i32) -> Result<(), String> {
    if status == NOT_UNDER_SYNC_ROOT {
        Ok(())
    } else if status >= 0 {
        Err("동기화 폴더에서는 복원할 수 없습니다. 로컬 앱 데이터 폴더를 사용해 주세요.".into())
    } else {
        Err(format!(
            "복원 폴더의 클라우드 동기화 상태를 확인하지 못했습니다. (HRESULT: 0x{:08X})",
            status as u32
        ))
    }
}

/// Reject reparse points in all existing ancestors so path lookup cannot move
/// restoration work to another volume or traverse a junction.
pub(super) fn check_path(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(
                    "복원 경로에 링크 또는 리파스 지점이 있어 안전하게 사용할 수 없습니다.".into(),
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("복원 경로의 안전성을 확인하지 못했습니다.".into()),
        }
    }
    Ok(())
}

pub(super) fn validate_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() || !root.is_dir() {
        return Err("복원 데이터 폴더를 확인할 수 없습니다.".into());
    }
    check_path(root)?;
    let path = wide(root.as_os_str());
    let mut volume = vec![0u16; 32768];
    // SAFETY: inputs are NUL-terminated UTF-16, output buffers are valid and
    // their lengths are passed in UTF-16 code units.
    if unsafe { GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err("복원 데이터가 저장된 드라이브를 확인하지 못했습니다.".into());
    }
    // SAFETY: GetVolumePathNameW succeeded and wrote a terminated volume path.
    if unsafe { GetDriveTypeW(volume.as_ptr()) } != DRIVE_FIXED {
        return Err("Windows 복원은 로컬 고정 NTFS 드라이브에서만 지원합니다.".into());
    }
    let mut filesystem = [0u16; 64];
    // SAFETY: optional outputs are null; filesystem points to a writable buffer.
    if unsafe {
        GetVolumeInformationW(
            volume.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    } == 0
    {
        return Err("복원 드라이브의 파일 시스템을 확인하지 못했습니다.".into());
    }
    let end = filesystem
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(filesystem.len());
    if !String::from_utf16_lossy(&filesystem[..end]).eq_ignore_ascii_case("NTFS") {
        return Err("Windows 복원은 로컬 고정 NTFS 드라이브에서만 지원합니다.".into());
    }
    // A normal, fully hydrated directory may still belong to a Cloud Files
    // sync root. Query the path itself, not just its reparse attributes.
    let mut info = [0u64; 1024];
    let mut returned = 0u32;
    // SAFETY: path is NUL-terminated and info is an aligned writable buffer;
    // class 0 is CF_SYNC_ROOT_INFO_BASIC. Any failure other than the documented
    // Win32 not-under-sync-root condition fails closed, including a short buffer.
    let status = unsafe {
        CfGetSyncRootInfoByPath(
            path.as_ptr(),
            0,
            info.as_mut_ptr().cast(),
            std::mem::size_of_val(&info) as u32,
            &mut returned,
        )
    };
    cloud_root_status(status)
}

/// Source and destination are within the validated root and never cross a
/// volume. Do not allow replacement or a copy-and-delete fallback.
pub(super) fn rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    let from = wide(source.as_os_str());
    let to = wide(destination.as_os_str());
    // SAFETY: both paths are NUL-terminated and remain alive across the call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_root_status_accepts_only_known_outside_error() {
        assert!(cloud_root_status(NOT_UNDER_SYNC_ROOT).is_ok());
        assert!(cloud_root_status(0).is_err());
        for status in [
            0x8007018bu32, // cloud file access denied (including restricted tokens)
            0x8007007a,    // short buffer
            0x80070002,    // missing path
            0x80070032,    // operation not supported (e.g. unavailable filter)
            0x80070178,    // not a cloud file does not establish its parent's status
        ] {
            let error = cloud_root_status(status as i32).unwrap_err();
            assert!(error.contains(&format!("0x{status:08X}")), "{error}");
        }
    }

    #[test]
    fn local_restore_root_passes_native_cloud_filter_query() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("한글 복원 폴더");
        fs::create_dir(&root).unwrap();
        // Exercise the real API, including the verbatim path spelling produced
        // by Windows canonicalize. This is also an early CI environment check:
        // an unavailable Cloud Files filter must not silently permit restores.
        validate_root(&root).unwrap();
        validate_root(&root.canonicalize().unwrap()).unwrap();
    }

    #[test]
    fn missing_restore_root_and_reparse_attributes_are_rejected() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(validate_root(&temporary.path().join("missing")).is_err());
        assert!(is_reparse_attribute(REPARSE_POINT));
        assert!(!is_reparse_attribute(0));
    }
}
