//! App updates are intentionally manual in the Windows distribution.
const MANUAL: &str = "이 Windows 버전은 새 설치 파일로 업데이트합니다. 보관한 데이터는 유지됩니다.";
#[tauri::command]
pub fn updater_configured() -> bool {
    false
}
#[tauri::command]
pub fn updater_check() -> Result<(), String> {
    Err(MANUAL.into())
}
#[tauri::command]
pub fn updater_install() -> Result<(), String> {
    Err(MANUAL.into())
}
#[tauri::command]
pub fn updater_restart() -> Result<(), String> {
    Err(MANUAL.into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn updater_is_disabled_regardless_of_environment() {
        assert!(!super::updater_configured());
        assert!(super::updater_install().is_err());
        assert!(super::updater_restart().is_err());
    }
}
