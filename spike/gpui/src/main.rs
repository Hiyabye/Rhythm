//! Throwaway GPUI feasibility spike for #340. Each section probes one risk:
//! Hangul IME, the sanitized-HTML reader, tokio interop, tray + always-on-top
//! mini window, close-to-background, fonts and OS appearance.

mod async_probe;
mod tray;

use std::{borrow::Cow, cell::Cell, sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme as _, StyledExt as _, Theme,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    text::TextView,
};
use gpui_kit::*;

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/PretendardVariable.ttf"),
    include_bytes!("../assets/Pretendard-Regular.otf"),
    include_bytes!("../assets/Pretendard-Medium.otf"),
    include_bytes!("../assets/Pretendard-SemiBold.otf"),
    include_bytes!("../assets/Pretendard-Bold.otf"),
];
/// Static Pretendard weights, the variable font, and the system font.
const FAMILIES: [&str; 3] = ["Pretendard", "Pretendard Variable", ".SystemUIFont"];
const SAMPLE_PNG: &[u8] = include_bytes!("../assets/sample.png");

/// A notice body in the shape Rhythm's sanitizer emits: block text, a list,
/// a table, a verified local image (`rhythm-resource:` scheme), an external
/// link, plus a remote image and a script that must never load or run.
const SAMPLE_NOTICE: &str = r#"
<h2>2026학년도 2학기 국가근로장학생 추가 모집</h2>
<p>신청 기간: <strong>2026. 11. 20.(금) ~ 11. 27.(금) 17:00</strong>까지</p>
<p>아래 내용을 확인하고 <a href="https://www.sogang.ac.kr/">서강대학교 홈페이지</a>에서 신청하시기 바랍니다.</p>
<ul><li>지원 자격: 재학생 (휴학생 제외)</li><li>근로 시간: 주 20시간 이내</li></ul>
<ol><li>신청서 제출</li><li>면접</li><li>최종 선발</li></ol>
<table>
  <tr><th>구분</th><th>시급</th><th>비고</th></tr>
  <tr><td>교내</td><td>10,030원</td><td>최저임금 기준</td></tr>
  <tr><td>교외</td><td>12,220원</td><td>기관 사정에 따라 상이</td></tr>
</table>
<p>로컬 이미지:</p>
<p><img src="rhythm-resource:sample" alt="첨부 이미지" width="64" height="64"></p>
<p>원격 이미지 (불러오면 안 됨):</p>
<p><img src="https://example.com/tracker.png" alt="원격 이미지"></p>
<script>alert('x')</script>
<p>문의: 학생지원팀 (02-705-8114)</p>
"#;

actions!(spike, [Quit]);

thread_local! {
    static MAIN_OPEN: Cell<bool> = const { Cell::new(false) };
}

struct Spike {
    input: Entity<InputState>,
    textarea: Entity<TextareaState>,
    input_log: Vec<String>,
    link_log: Vec<String>,
    async_probe: Entity<async_probe::AsyncProbe>,
    appearance: WindowAppearance,
    font_index: usize,
    _subscriptions: Vec<Subscription>,
}

impl Spike {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("여기에 한글을 입력하세요"));
        let textarea = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("여러 줄 입력 (Enter로 줄바꿈)")
        });
        let subscriptions = vec![
            cx.subscribe(&input, |this, state, event: &InputEvent, cx| {
                this.log_input("input", state.read(cx).value(), event);
                cx.notify();
            }),
            cx.subscribe(&textarea, |this, state, event: &InputEvent, cx| {
                this.log_input("textarea", state.read(cx).value(), event);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.appearance = window.appearance();
                // gpui-component themes do not follow the OS on their own.
                Theme::sync_system_appearance(Some(window), cx);
                eprintln!("[appearance] {:?}", this.appearance);
                cx.notify();
            }),
        ];
        Self {
            input,
            textarea,
            input_log: Vec::new(),
            link_log: Vec::new(),
            async_probe: cx.new(|_| async_probe::AsyncProbe::default()),
            appearance: window.appearance(),
            font_index: 0,
            _subscriptions: subscriptions,
        }
    }

    fn log_input(&mut self, source: &str, value: SharedString, event: &InputEvent) {
        let kind = match event {
            InputEvent::Change => "change",
            InputEvent::PressEnter { .. } => "enter",
            InputEvent::Focus => "focus",
            InputEvent::Blur => "blur",
        };
        let line = format!("{source} {kind}: {value:?}");
        eprintln!("[ime] {line}");
        self.input_log.push(line);
        if self.input_log.len() > 12 {
            self.input_log.remove(0);
        }
    }

    fn section(title: &'static str) -> Div {
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .rounded_md()
            .child(div().text_lg().font_weight(FontWeight::BOLD).child(title))
    }
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sample_image = Arc::new(Image::from_bytes(ImageFormat::Png, SAMPLE_PNG.to_vec()));
        let link_target = cx.entity().downgrade();

        let reader = TextView::html("notice", SAMPLE_NOTICE)
            .selectable(true)
            // The host owns resource policy: only verified local resources
            // resolve; anything else becomes an empty source and never loads.
            .image_source(move |uri| {
                if uri.as_ref() == "rhythm-resource:sample" {
                    ImageSource::Image(sample_image.clone())
                } else {
                    eprintln!("[reader] blocked image: {uri}");
                    ImageSource::Image(Arc::new(Image::from_bytes(ImageFormat::Png, Vec::new())))
                }
            })
            .on_link_click(move |url, _, _, cx| {
                let url = url.to_string();
                eprintln!("[reader] link: {url}");
                if let Some(this) = link_target.upgrade() {
                    this.update(cx, |this, cx| {
                        this.link_log.push(url.clone());
                        cx.notify();
                    });
                }
                if url.starts_with("https://") {
                    cx.open_url(&url);
                }
            });

        let font = FAMILIES[self.font_index];

        div()
            .id("spike-root")
            .size_full()
            .overflow_y_scroll()
            .font_family(font)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .v_flex()
            .gap_3()
            .p_4()
            .child(
                Self::section("1. 한글 입력 (IME)")
                    .child(Input::new(&self.input))
                    .child(Textarea::new(&self.textarea).h(px(80.)))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("현재 값: {:?}", self.input.read(cx).value())),
                    )
                    .children(
                        self.input_log.iter().map(|line| div().text_xs().child(line.clone())),
                    ),
            )
            .child(
                Self::section("2. 공지 본문 (sanitized HTML)").child(reader).children(
                    self.link_log
                        .iter()
                        .map(|url| div().text_xs().child(format!("링크 클릭: {url}"))),
                ),
            )
            .child(
                Self::section("3. 비동기 (tokio + reqwest + rusqlite)")
                    .child(self.async_probe.clone()),
            )
            .child(
                Self::section("4. 창 · 트레이 · 모양")
                    .child(format!("OS 모양: {:?}", self.appearance))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("mini")
                                    .label("미니 창 열기")
                                    .on_click(|_, _, cx| open_mini(cx)),
                            )
                            .child(Button::new("font").label(format!("글꼴: {font}")).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.font_index = (this.font_index + 1) % FAMILIES.len();
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Button::new("quit")
                                    .danger()
                                    .label("종료")
                                    .on_click(|_, _, cx| cx.quit()),
                            ),
                    )
                    .child(div().text_sm().child(
                        "창을 닫으면 앱은 트레이에 남습니다. 트레이 메뉴의 '열기'로 다시 엽니다.",
                    )),
            )
    }
}

struct Mini;

impl Render for Mini {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .justify_center()
            .items_center()
            .font_family("Pretendard")
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().font_weight(FontWeight::BOLD).child("로욜라 1열람실"))
            .child("남은 좌석 42 / 120")
    }
}

pub(crate) fn open_main(cx: &mut App) {
    if MAIN_OPEN.with(Cell::get) {
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(900.), px(800.)), cx);
    let result = open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Rhythm GPUI spike".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        cx,
        |window, cx| {
            // Close-to-background: GPUI has no per-window hide, so closing
            // drops the window; the app keeps running for the tray to reopen.
            window.on_window_should_close(cx, |_, _| {
                MAIN_OPEN.with(|open| open.set(false));
                true
            });
            cx.new(|cx| Spike::new(window, cx))
        },
    );
    match result {
        Ok(_) => MAIN_OPEN.with(|open| open.set(true)),
        Err(err) => eprintln!("open main window failed: {err:#}"),
    }
    cx.activate(true);
}

pub(crate) fn open_mini(cx: &mut App) {
    let bounds = Bounds::new(point(px(40.), px(40.)), size(px(260.), px(120.)));
    if let Err(err) = open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            kind: WindowKind::PopUp,
            titlebar: Some(TitlebarOptions {
                title: Some("열람실".into()),
                ..Default::default()
            }),
            focus: false,
            ..Default::default()
        },
        cx,
        |_, cx| cx.new(|_| Mini),
    ) {
        eprintln!("open mini window failed: {err:#}");
    }
}

fn main() {
    let app = application().with_quit_mode(QuitMode::Explicit);
    app.on_reopen(open_main);
    app.run(|cx: &mut App| {
        gpui_kit::init(cx);
        Theme::sync_system_appearance(None, cx);
        if let Err(err) = cx.text_system().add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
            eprintln!("loading Pretendard failed: {err:#}");
        }
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Rhythm".into(),
            items: vec![MenuItem::action("종료", Quit)],
            disabled: false,
        }]);

        tray::install(cx);
        open_main(cx);

        // A heartbeat makes a stalled UI thread visible in the log.
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(30)).await;
                eprintln!("[heartbeat] ui alive");
            }
        })
        .detach();
    });
}
