//! Where Rhythm keeps its data and finds its bundled files, resolved without
//! the desktop shell (#341). The results must stay identical to what Tauri 2
//! resolved for v2.x (`app_data_dir`/`app_local_data_dir`, `resource_dir`), or
//! an update would open an empty profile next to the user's real one.

use std::path::{Path, PathBuf};

/// The bundle identifier of release builds. Validation builds pass their own.
pub const IDENTIFIER: &str = "io.github.hiyabye.rhythm";

/// Validation builds (identifier ending in `.validation`) may point their data
/// at an absolute directory through this variable; release builds ignore it.
pub const VALIDATION_DATA_DIR: &str = "RHYTHM_VALIDATION_DATA_DIR";

/// The data directory for `identifier`: Application Support on macOS, the
/// non-roaming `%LOCALAPPDATA%` on Windows (archive and model weights stay
/// out of the roaming profile).
pub fn data_root(identifier: &str) -> Result<PathBuf, String> {
    let default = default_data_root(identifier)?;
    Ok(validation_override(
        identifier,
        std::env::var_os(VALIDATION_DATA_DIR).map(PathBuf::from),
    )
    .unwrap_or(default))
}

fn default_data_root(identifier: &str) -> Result<PathBuf, String> {
    // The same `dirs` calls Tauri's path resolver makes.
    #[cfg(windows)]
    let base = dirs::data_local_dir();
    #[cfg(not(windows))]
    let base = dirs::data_dir();
    base.map(|base| base.join(identifier))
        .ok_or_else(|| "데이터 폴더 위치를 찾지 못했습니다.".into())
}

fn validation_override(identifier: &str, value: Option<PathBuf>) -> Option<PathBuf> {
    value.filter(|path| identifier.ends_with(".validation") && path.is_absolute())
}

pub fn database(data_root: &Path) -> PathBuf {
    data_root.join("rhythm.sqlite3")
}

/// The private directory that holds the embedding server's socket or
/// per-launch key and, on Unix, its pid file. Backup and restore never read it.
pub fn embedding_runtime(data_root: &Path) -> PathBuf {
    data_root.join("run").join("embedding")
}

/// Bundled resources of the running executable.
pub fn resource_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    resource_dir_from(&exe).map_err(|error| format!("앱 리소스 폴더를 찾지 못했습니다: {error}"))
}

/// The bundled llama.cpp build inside the resource directory.
pub fn llama(resources: &Path) -> PathBuf {
    resources.join("llama")
}

/// Tauri 2's rule: Windows installs and Cargo output directories keep
/// resources beside the executable; a macOS bundle keeps them in
/// `Contents/Resources`.
fn resource_dir_from(exe: &Path) -> std::io::Result<PathBuf> {
    let directory = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("executable has no parent directory"))?;
    if cfg!(windows) || is_cargo_output(directory) {
        return Ok(directory.to_path_buf());
    }
    directory.join("../Resources").canonicalize()
}

/// `target/<profile>` or `target/<triple>/<profile>` with Cargo's lock file.
fn is_cargo_output(directory: &Path) -> bool {
    let parts: Vec<_> = directory.components().collect();
    let target = |back: usize| {
        parts
            .len()
            .checked_sub(back)
            .is_some_and(|index| parts[index].as_os_str() == "target")
    };
    (target(2) || target(3)) && directory.join(".cargo-lock").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_data_root_is_application_support() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
        assert_eq!(
            default_data_root(IDENTIFIER).unwrap(),
            home.join("Library/Application Support/io.github.hiyabye.rhythm")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_data_root_is_non_roaming_local_app_data() {
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA is set"));
        assert_eq!(
            default_data_root(IDENTIFIER).unwrap(),
            local.join("io.github.hiyabye.rhythm")
        );
        let roaming = PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA is set"));
        assert!(!default_data_root(IDENTIFIER).unwrap().starts_with(roaming));
    }

    #[test]
    fn only_validation_builds_take_an_absolute_override() {
        let absolute = std::env::temp_dir().join("rhythm-validation");
        let validation = "io.github.hiyabye.rhythm.validation";
        assert_eq!(
            validation_override(validation, Some(absolute.clone())),
            Some(absolute.clone())
        );
        assert_eq!(validation_override(IDENTIFIER, Some(absolute)), None);
        assert_eq!(
            validation_override(validation, Some(PathBuf::from("relative"))),
            None
        );
        assert_eq!(validation_override(validation, None), None);
    }

    #[test]
    fn data_files_keep_their_v2_names() {
        let root = Path::new("data");
        assert_eq!(database(root), root.join("rhythm.sqlite3"));
        assert_eq!(embedding_runtime(root), root.join("run").join("embedding"));
        assert_eq!(llama(Path::new("res")), Path::new("res").join("llama"));
    }

    #[test]
    fn cargo_output_keeps_resources_beside_the_executable() {
        let workspace = tempfile::tempdir().unwrap();
        for profile in [
            workspace.path().join("target").join("debug"),
            workspace
                .path()
                .join("target")
                .join("x86_64-pc-windows-msvc")
                .join("release"),
        ] {
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(profile.join(".cargo-lock"), b"").unwrap();
            assert_eq!(resource_dir_from(&profile.join("rhythm")).unwrap(), profile);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bundle_resources_are_in_contents_resources() {
        let workspace = tempfile::tempdir().unwrap();
        let contents = workspace.path().join("Rhythm.app").join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        assert_eq!(
            resource_dir_from(&contents.join("MacOS").join("rhythm")).unwrap(),
            contents.join("Resources").canonicalize().unwrap()
        );
        // A `target` directory without Cargo's lock file is not Cargo output.
        let fake = workspace.path().join("target").join("debug");
        std::fs::create_dir_all(&fake).unwrap();
        assert!(resource_dir_from(&fake.join("rhythm")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_resources_are_beside_the_executable() {
        let install = Path::new(r"C:\Users\u\AppData\Local\Rhythm");
        assert_eq!(
            resource_dir_from(&install.join("rhythm.exe")).unwrap(),
            install
        );
    }
}
