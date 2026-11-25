//! Tray icon through the `tray-icon` crate (the one Tauri uses). It must be
//! created on the main thread after the platform loop starts; GPUI's loop
//! pumps the OS messages it needs. Menu events are polled from a GPUI task.

use std::time::Duration;

use gpui_kit::*;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

pub fn install(cx: &mut App) {
    let menu = Menu::new();
    let open = MenuItem::new("열기", true, None);
    let mini = MenuItem::new("미니 창", true, None);
    let quit = MenuItem::new("종료", true, None);
    if let Err(err) = menu.append_items(&[&open, &mini, &quit]) {
        eprintln!("[tray] menu failed: {err}");
        return;
    }
    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Rhythm GPUI spike")
        .with_icon(icon())
        .build()
    {
        Ok(tray) => tray,
        Err(err) => {
            eprintln!("[tray] build failed: {err}");
            return;
        }
    };
    let (open, mini, quit) = (open.id().clone(), mini.id().clone(), quit.id().clone());
    cx.spawn(async move |cx| {
        // The tray lives as long as this task (the whole app).
        let _tray = tray;
        loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let _ = cx.update(|cx| {
                    if event.id == open {
                        crate::open_main(cx);
                    } else if event.id == mini {
                        crate::open_mini(cx);
                    } else if event.id == quit {
                        cx.quit();
                    }
                });
            }
        }
    })
    .detach();
}

/// A 32×32 filled circle, so the spike needs no image decoder.
fn icon() -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let alpha = if dx * dx + dy * dy <= 14.0 * 14.0 { 255 } else { 0 };
            rgba.extend_from_slice(&[0x4f, 0x46, 0xe5, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid icon")
}
