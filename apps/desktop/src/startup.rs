//! The window shown instead of the app when core could not open. Native
//! dialogs come with #345; until then a small window carries the message.

use gpui_kit::{
    component::{button::Button, ActiveTheme as _, StyledExt as _},
    *,
};
use rhythm_core::app::OpenError;

struct StartupError {
    title: &'static str,
    message: SharedString,
}

impl Render for StartupError {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .p_5()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().font_weight(FontWeight::BOLD).child(self.title))
            .child(div().flex_1().text_sm().child(self.message.clone()))
            .child(
                div().h_flex().justify_end().child(
                    Button::new("quit")
                        .label("닫기")
                        .on_click(|_, _, _| refused()),
                ),
            )
    }
}

/// Ends a launch that did not open, with status 1 like the Tauri app. No
/// core app exists yet, so there is nothing to shut down.
fn refused() -> ! {
    std::process::exit(1)
}

/// Shows why the app did not open; closing it quits.
pub(crate) fn show(error: OpenError, cx: &mut App) {
    // The notes match the Tauri app's startup dialogs.
    let (title, message) = match error {
        OpenError::Locked(error) => ("Rhythm을 시작할 수 없습니다", error),
        OpenError::Restore(error) => (
            "백업 복원을 마무리하지 못했습니다",
            format!("{error}\n\n기존 파일은 그대로 두었습니다. 원인을 확인한 뒤 다시 실행해 주세요."),
        ),
        OpenError::Failed(error) => (
            "Rhythm을 시작하지 못했습니다",
            format!("{error}\n\n저장된 데이터는 그대로 두었습니다. 원인을 확인한 뒤 다시 실행해 주세요."),
        ),
    };
    eprintln!("{title}: {message}");
    let bounds = Bounds::centered(None, size(px(440.), px(220.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Rhythm".into()),
            ..Default::default()
        }),
        is_resizable: false,
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.on_window_should_close(cx, |_, _| refused());
        cx.new(|_| StartupError {
            title,
            message: message.into(),
        })
    });
    if opened.is_err() {
        refused();
    }
    cx.activate(true);
}
