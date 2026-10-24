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
