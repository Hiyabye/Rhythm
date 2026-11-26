//! Model destinations selected locally by the user. Restored databases must not
//! retain custom managed-file deletion authority (see backup reconciliation).
use crate::storage::intelligence::AiModel;
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Serialize)]
pub struct Status {
    pub directory: String,
    pub custom: bool,
    pub available_bytes: Option<u64>,
    pub error: Option<String>,
}

fn unsafe_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// Rejects a model directory that is, or on Windows sits below, a link or
/// junction, so a swapped link cannot redirect managed deletion. macOS keeps
/// system links such as `/var -> private/var` and linked home volumes in its
/// ordinary paths, so there only the directory itself is checked and the
/// caller canonicalizes the rest.
fn check_ancestors(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let checked = path.ancestors();
    #[cfg(not(windows))]
    let checked = std::iter::once(path);
    for ancestor in checked {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if !metadata.is_dir() || unsafe_link(&metadata) => {
                return Err(
                    "모델 폴더 경로에 파일이나 링크가 있습니다. 실제 폴더를 선택해 주세요.".into(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(
                    "모델 폴더에 접근하지 못했습니다. 연결 상태와 권한을 확인해 주세요.".into(),
                )
            }
        }
    }
    Ok(())
}

fn checked_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|s| s.len() > 4096 || s.contains('\0'))
        || path.components().any(|p| matches!(p, Component::ParentDir))
    {
        return Err("모델 설치 폴더의 전체 경로를 확인해 주세요.".into());
    }
    check_ancestors(path)?;
    let canonical = path.canonicalize().map_err(|_| {
        "모델 설치 폴더가 없습니다. 드라이브를 연결하거나 폴더를 다시 선택해 주세요."
    })?;
    if !canonical.is_dir() {
        return Err("모델 설치 위치로 폴더를 선택해 주세요.".into());
    }
    check_ancestors(&canonical)?;
    Ok(canonical)
}

pub fn display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else {
        path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
    }
}

/// Selection never moves existing models. Probe only a newly created random file.
pub fn prepare_selection(path: &Path) -> Result<String, String> {
    let directory = checked_directory(path)?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| "설치 폴더 쓰기 검사를 준비하지 못했습니다.")?;
    let name = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let probe = directory.join(format!(".rhythm-write-check-{name}"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| "선택한 폴더에 파일을 저장할 수 없습니다. 쓰기 권한을 확인해 주세요.")?;
    let result = file
        .write_all(b"Rhythm model storage check")
        .and_then(|()| file.sync_all());
    drop(file);
    let cleanup = fs::remove_file(&probe);
    result.map_err(|_| "선택한 폴더에 파일을 저장하지 못했습니다. 공간과 권한을 확인해 주세요.")?;
    cleanup.map_err(|_| "폴더 검사 파일을 정리하지 못했습니다. 폴더 권한을 확인해 주세요.")?;
    Ok(directory.to_string_lossy().into_owned())
}

pub fn status(root: &Path, custom: Option<&str>) -> Status {
    let directory = custom
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("models"));
    // A default directory is created only when the first installation begins.
    let capacity = if custom.is_none() && !directory.exists() {
        root
    } else {
        &directory
    };
    let result = checked_directory(capacity).and_then(|path| super::models::available_space(&path));
    let (available_bytes, error) = match result {
        Ok(bytes) => (Some(bytes), None),
        Err(error) => (None, Some(error)),
    };
    Status {
        directory: display_path(&directory),
        custom: custom.is_some(),
        available_bytes,
        error,
    }
}

pub(super) fn managed_path(
    root: &Path,
    relative: &str,
    custom: Option<&str>,
    create: bool,
) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.components().count() != 2
        || relative.parent() != Some(Path::new("models"))
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("Rhythm 모델 저장 경로가 올바르지 않습니다.".into());
    }
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("모델 파일 이름이 없습니다.")?;
    let hash = name
        .strip_suffix(".gguf")
        .ok_or("모델 파일 이름이 올바르지 않습니다.")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("모델 파일 이름이 무결성 식별자와 일치하지 않습니다.".into());
    }
    let directory = if let Some(custom) = custom {
        checked_directory(Path::new(custom))?
    } else {
        let root = checked_directory(root)?;
        let directory = root.join("models");
        check_ancestors(&directory)?;
        if create && !directory.exists() {
            fs::create_dir(&directory).map_err(|_| "모델 저장 폴더를 만들지 못했습니다.")?;
        }
        if directory.exists() {
            checked_directory(&directory)?
        } else {
            directory
        }
    };
    let full = directory.join(name);
    for file in [&full, &full.with_extension("part")] {
        match fs::symlink_metadata(file) {
            Ok(metadata) if unsafe_link(&metadata) || !metadata.is_file() => {
                return Err("모델 파일의 외부 경로 연결을 차단했습니다.".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("모델 파일에 접근하지 못했습니다. 권한을 확인해 주세요.".into()),
        }
    }
    Ok(full)
}

pub fn resolve_model_path(root: &Path, model: &AiModel) -> Result<PathBuf, String> {
    if model.ownership == "managed" {
        managed_path(root, &model.path, model.storage_directory.as_deref(), false)
    } else if model.ownership == "external" && Path::new(&model.path).is_absolute() {
        Ok(PathBuf::from(&model.path))
    } else {
        Err("모델 등록 경로가 올바르지 않습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_checks_a_korean_folder_without_touching_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("한글 모델 저장 위치");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("사용자 파일.txt"), "보존").unwrap();
        let selected = prepare_selection(&directory).unwrap();
        assert!(Path::new(&selected).is_absolute());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert!(status(root.path(), Some(&selected))
            .available_bytes
            .is_some());
        assert!(prepare_selection(Path::new("relative")).is_err());
        assert!(prepare_selection(&directory.join("사용자 파일.txt")).is_err());
        assert!(prepare_selection(&directory.join("missing")).is_err());
    }
    #[test]
    fn reading_missing_paths_does_not_create_folders_or_redirect_to_default() {
        let root = tempfile::tempdir().unwrap();
        let relative = format!("models/{}.gguf", "a".repeat(64));
        assert!(managed_path(root.path(), &relative, None, false).is_ok());
        assert!(!root.path().join("models").exists());
        let missing = root.path().join("removed drive folder");
        assert!(managed_path(root.path(), &relative, missing.to_str(), false).is_err());
        assert!(!missing.exists());
        assert!(managed_path(root.path(), "models/../outside", None, true).is_err());
        assert!(managed_path(root.path(), "models/user-file.gguf", None, true).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn linked_parent_is_canonicalized_but_a_linked_model_folder_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("실제 볼륨");
        fs::create_dir_all(real.join("모델")).unwrap();
        let linked_parent = root.path().join("연결된 볼륨");
        std::os::unix::fs::symlink(&real, &linked_parent).unwrap();
        let selected = prepare_selection(&linked_parent.join("모델")).unwrap();
        assert_eq!(
            Path::new(&selected),
            real.join("모델").canonicalize().unwrap()
        );

        let linked_folder = root.path().join("모델 링크");
        std::os::unix::fs::symlink(real.join("모델"), &linked_folder).unwrap();
        assert!(prepare_selection(&linked_folder).is_err());
        let relative = format!("models/{}.gguf", "c".repeat(64));
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap();
        std::os::unix::fs::symlink(real.join("모델"), data.join("models")).unwrap();
        assert!(managed_path(&data, &relative, None, true).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn junction_destinations_and_ancestors_are_rejected_without_touching_target() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("실제 모델 폴더");
        fs::create_dir(&target).unwrap();
        fs::create_dir(target.join("하위 폴더")).unwrap();
        let relative = format!("models/{}.gguf", "b".repeat(64));
        let target_file = target.join(format!("{}.gguf", "b".repeat(64)));
        fs::write(&target_file, b"preserve target bytes").unwrap();
        let junction = root.path().join("models");
        // Junctions do not require Developer Mode or administrator privileges.
        // Only these test-owned paths enter the fixed script through environment values.
        let created = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$ErrorActionPreference = 'Stop'\nNew-Item -ItemType Junction -Path $env:RHYTHM_TEST_LINK -Target $env:RHYTHM_TEST_TARGET | Out-Null",
            ])
            .env("RHYTHM_TEST_LINK", &junction)
            .env("RHYTHM_TEST_TARGET", &target)
            .creation_flags(0x0800_0000)
            .output()
            .expect("create isolated Windows junction");
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let metadata_is_link = unsafe_link(&fs::symlink_metadata(&junction).unwrap());
        let selection_rejected = prepare_selection(&junction).is_err();
        let ancestor_rejected = prepare_selection(&junction.join("하위 폴더")).is_err();
        let default_rejected = managed_path(root.path(), &relative, None, true).is_err();
        let custom_rejected =
            managed_path(root.path(), &relative, junction.to_str(), true).is_err();
        // Remove only the junction, never the target directory or its contents.
        fs::remove_dir(&junction).unwrap();
        assert!(metadata_is_link);
        assert!(selection_rejected && ancestor_rejected && default_rejected && custom_rejected);
        assert_eq!(fs::read(&target_file).unwrap(), b"preserve target bytes");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
    }
}
