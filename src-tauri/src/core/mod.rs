/// Own the application-data directory before recovering interrupted jobs. OS file
/// locks are released on process termination, so a crash leaves no stale lease.
pub fn lock_data_dir(root: &std::path::Path) -> Result<std::fs::File, String> {
    std::fs::create_dir_all(root).map_err(|_| "로컬 저장 폴더를 만들지 못했습니다.")?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("rhythm.lock"))
        .map_err(|_| "앱 실행 잠금 파일을 열지 못했습니다.")?;
    file.try_lock().map_err(|_| {
        "Rhythm이 이미 실행 중이거나 데이터 폴더를 잠글 수 없습니다. 기존 창을 확인해 주세요."
    })?;
    Ok(file)
}

#[cfg(target_os = "macos")]
use tauri::{
    menu::{Menu, PredefinedMenuItem as Item, Submenu},
    AppHandle,
};

#[cfg(not(target_os = "macos"))]
pub fn install_menu(_app: &tauri::AppHandle) -> tauri::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn install_menu(app: &AppHandle) -> tauri::Result<()> {
    let application = Submenu::with_items(
        app,
        "Rhythm",
        true,
        &[
            &Item::hide(app, Some("Rhythm 가리기"))?,
            &Item::hide_others(app, Some("다른 앱 가리기"))?,
            &Item::separator(app)?,
            &Item::quit(app, Some("Rhythm 종료"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "편집",
        true,
        &[
            &Item::undo(app, Some("실행 취소"))?,
            &Item::redo(app, Some("다시 실행"))?,
            &Item::separator(app)?,
            &Item::cut(app, Some("오려두기"))?,
            &Item::copy(app, Some("복사하기"))?,
            &Item::paste(app, Some("붙여넣기"))?,
            &Item::select_all(app, Some("모두 선택"))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "윈도우",
        true,
        &[
            &Item::minimize(app, Some("최소화"))?,
            &Item::fullscreen(app, Some("전체 화면"))?,
            &Item::close_window(app, Some("윈도우 닫기"))?,
        ],
    )?;
    app.set_menu(Menu::with_items(app, &[&application, &edit, &window])?)?;
    Ok(())
}

#[cfg(test)]
mod ownership_tests {
    #[test]
    fn second_owner_is_rejected_and_lock_is_released_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let first = super::lock_data_dir(directory.path()).unwrap();
        assert!(super::lock_data_dir(directory.path()).is_err());
        drop(first);
        assert!(super::lock_data_dir(directory.path()).is_ok());
    }
}
