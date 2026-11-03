//! Reveal one verified archive item through the documented single-item Shell API.
use std::{
    ffi::{c_void, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const ERROR: &str = "저장 폴더를 열지 못했습니다. 데이터 저장 위치를 확인해 주세요.";

#[link(name = "shell32")]
extern "system" {
    fn SHParseDisplayName(
        name: *const u16,
        context: *mut c_void,
        item: *mut *mut c_void,
        attributes: u32,
        result: *mut u32,
    ) -> i32;
    fn SHOpenFolderAndSelectItems(
        item: *const c_void,
        count: u32,
        children: *const *const c_void,
        flags: u32,
    ) -> i32;
    fn ILFree(item: *const c_void);
}

pub(super) fn resource_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut pieces = relative.split('/');
    if pieces.next() != Some("resources") {
        return Err("로컬 자료 경로가 안전하지 않습니다.".into());
    }
    let mut path = root.join("resources");
    let mut count = 0;
    for piece in pieces {
        if piece.is_empty() || piece == "." || piece == ".." || piece.contains(['\\', ':', '\0']) {
            return Err("로컬 자료 경로가 안전하지 않습니다.".into());
        }
        path.push(piece);
        count += 1;
    }
    if count == 0 {
        return Err("로컬 자료 경로가 안전하지 않습니다.".into());
    }
    Ok(path)
}

fn shell_path(canonical: &Path) -> PathBuf {
    let raw: Vec<u16> = canonical.as_os_str().encode_wide().collect();
    const PREFIX: &[u16] = &[92, 92, 63, 92]; // \\?\
    const UNC_PREFIX: &[u16] = &[92, 92, 63, 92, 85, 78, 67, 92]; // \\?\UNC\
    if raw.starts_with(UNC_PREFIX) && raw.len() - 6 < 260 {
        let mut normal = vec![92, 92];
        normal.extend_from_slice(&raw[UNC_PREFIX.len()..]);
        return PathBuf::from(OsString::from_wide(&normal));
    }
    if raw.starts_with(PREFIX)
        && raw.len() > 6
        && ((b'A' as u16..=b'Z' as u16).contains(&raw[4])
            || (b'a' as u16..=b'z' as u16).contains(&raw[4]))
        && raw[5] == b':' as u16
        && raw.len() - PREFIX.len() < 260
    {
        return PathBuf::from(OsString::from_wide(&raw[PREFIX.len()..]));
    }
    canonical.to_owned()
}

fn resolved_resource(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(root.join("resources")).map_err(|_| ERROR.to_string())?;
    let target = std::fs::canonicalize(path).map_err(|_| ERROR.to_string())?;
    if target == base || !target.starts_with(&base) || !target.is_file() {
        return Err("로컬 자료 경로가 안전하지 않습니다.".into());
    }
    Ok(shell_path(&target))
}

pub(super) fn reveal(root: &Path, path: &Path) -> Result<(), String> {
    // Shell may run in a different file-system namespace than this process.
    let path = resolved_resource(root, path)?;
    // A fresh thread has no incompatible pre-existing COM apartment, and Shell
    // parsing is kept off both the UI and async executor threads.
    std::thread::Builder::new()
        .name("rhythm-reveal".into())
        .spawn(move || reveal_on_thread(&path))
        .map_err(|_| ERROR.to_string())?
        .join()
        .map_err(|_| ERROR.to_string())?
}

fn reveal_on_thread(path: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    struct Pidl(*mut c_void);
    impl Drop for Pidl {
        fn drop(&mut self) {
            unsafe { ILFree(self.0) };
        }
    }
    // SAFETY: this is a new thread; success is balanced by the guard.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|_| ERROR.to_string())?;
    let _com = Com;
    let name: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut item = std::ptr::null_mut();
    // SAFETY: the terminated path and out pointer remain live throughout parsing.
    if unsafe {
        SHParseDisplayName(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut item,
            0,
            std::ptr::null_mut(),
        )
    } < 0
        || item.is_null()
    {
        return Err(ERROR.into());
    }
    let item = Pidl(item);
    // SAFETY: count zero and a null child array select this fully qualified item.
    if unsafe { SHOpenFolderAndSelectItems(item.0, 0, std::ptr::null(), 0) } < 0 {
        return Err(ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_path_converts_archive_separators_and_rejects_escape() {
        let root = Path::new(r"C:\자료 보관");
        assert_eq!(
            resource_path(root, "resources/한글/공지.hwp").unwrap(),
            root.join("resources").join("한글").join("공지.hwp")
        );
        for invalid in [
            "resources/../escape",
            "resources/a\\b",
            "resources//a",
            "resources/C:evil",
            "other/a",
            "resources",
        ] {
            assert!(resource_path(root, invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn reveal_resolves_physical_path_and_rechecks_resource_containment() {
        let root = tempfile::tempdir().unwrap();
        let resources = root.path().join("resources");
        std::fs::create_dir(&resources).unwrap();
        let target = resources.join("한글.hwp");
        std::fs::write(&target, b"test").unwrap();
        let path = resolved_resource(root.path(), &target).unwrap();
        assert_eq!(path.canonicalize().unwrap(), target.canonicalize().unwrap());
        let outside = root.path().join("outside.hwp");
        std::fs::write(&outside, b"outside").unwrap();
        assert!(resolved_resource(root.path(), &outside).is_err());
    }

    #[test]
    fn short_verbatim_paths_are_converted_for_shell_without_loss() {
        assert_eq!(
            shell_path(Path::new(r"\\?\C:\자료\공지.hwp")),
            PathBuf::from(r"C:\자료\공지.hwp")
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\server\share\공지.hwp")),
            PathBuf::from(r"\\server\share\공지.hwp")
        );
        let long = PathBuf::from(format!(r"\\?\C:\{}", "a".repeat(260)));
        assert_eq!(shell_path(&long), long);
    }
}
