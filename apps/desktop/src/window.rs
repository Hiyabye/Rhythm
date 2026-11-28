//! The main window: a sidebar built from the extension registry and one page
//! at a time. Pages not yet ported to GPUI are placeholders (#347–#351).

use crate::{
    backend::Backend,
    nav::{self, Page, Pane},
};
use gpui_kit::{
    base::Disableable as _,
    component::{switch::Switch, ActiveTheme as _, StyledExt as _},
    prelude::FluentBuilder as _,
    *,
};
use rhythm_core::{app::Snapshot, shell::Event};

/// The open main window, if any. GPUI keeps no per-window hide, so a closed
/// window is gone and [`show_main`] opens a new one.
struct MainWindowRef {
    window: AnyWindowHandle,
    view: WeakEntity<MainWindow>,
}

impl Global for MainWindowRef {}

/// Focuses the main window, opening it if it was closed.
pub(crate) fn show_main(cx: &mut App) {
    // A Dock click after a refused open finds no core app to show.
    if !cx.has_global::<Backend>() {
        return;
    }
    let open = cx
        .try_global::<MainWindowRef>()
        .filter(|main| main.view.upgrade().is_some())
        .map(|main| main.window);
    if let Some(window) = open {
        if window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            cx.activate(true);
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(1180.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(540.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Rhythm".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    }) {
        Ok((window, view)) => cx.set_global(MainWindowRef {
            window,
            view: view.downgrade(),
        }),
        Err(error) => eprintln!("메인 창을 열지 못했습니다: {error:#}"),
    }
    cx.activate(true);
}

/// Applies a core event to the open main window.
pub(crate) fn apply_event(event: Event, cx: &mut App) {
    let Some(view) = cx
        .try_global::<MainWindowRef>()
        .and_then(|main| main.view.upgrade())
    else {
        return;
    };
    view.update(cx, |this, cx| {
        let Some(snapshot) = this.snapshot.as_mut() else {
            return;
        };
        match event {
            Event::ThemeChanged(theme) => snapshot.settings.theme = theme,
            Event::ClockFormatChanged(format) => snapshot.settings.clock_format = format,
            // Core shows the main window itself; routing a click to its
            // notice, day or room comes with #345.
            Event::NoticeClicked(_) | Event::ReminderClicked(_) | Event::StudiumClicked(_) => {
                return
            }
        }
        cx.notify();
    });
}

pub(crate) struct MainWindow {
    page: Page,
    snapshot: Option<Snapshot>,
    /// Notices in 휴지통, which keep its page reachable.
    trashed: u64,
    error: Option<SharedString>,
    /// An extension change is running.
    busy: bool,
    _appearance: Subscription,
}

impl MainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            // gpui-component themes do not follow the OS on their own.
            component::Theme::sync_system_appearance(Some(window), cx);
            cx.notify();
        });
        let mut this = Self {
            page: Page::Today,
            snapshot: None,
            trashed: 0,
            error: None,
            busy: false,
            _appearance: appearance,
        };
        this.load(None, cx);
        this
    }

    fn enabled(&self) -> Vec<&'static str> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            snapshot
                .extensions
                .iter()
                .filter(|extension| extension.enabled)
                .map(|extension| extension.id)
                .collect()
        })
    }

    /// Reads the settings snapshot and the 휴지통 count, after turning
    /// `change` (an extension ID and its new state) on or off.
    fn load(&mut self, change: Option<(&'static str, bool)>, cx: &mut Context<Self>) {
        self.busy = change.is_some();
        let task = cx.global::<Backend>().run(move |app| async move {
            let snapshot = match change {
                Some((id, enabled)) => app.settings().set_extension(id, enabled).await?,
                None => app.settings().snapshot().await?,
            };
            let trashed = app.archive().summary().await?.trashed;
            Ok((snapshot, trashed))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((snapshot, trashed)) => {
                        this.snapshot = Some(snapshot);
                        this.trashed = trashed;
                        this.error = None;
                        if !nav::shown(this.page, &this.enabled(), trashed) {
                            this.page = Page::Today;
                        }
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn nav_item(
        &self,
        id: impl Into<ElementId>,
        label: &'static str,
        target: Page,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_3()
            .py_1p5()
            .rounded_md()
            .cursor_pointer()
            .when(active, |item| {
                item.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .hover(|item| item.bg(cx.theme().accent))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.page = target;
                cx.notify();
            }))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.enabled();
        let items: Vec<_> = nav::primary(&enabled, self.trashed)
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                self.nav_item(
                    ("nav", index),
                    item.label,
                    item.id,
                    self.page == item.id,
                    cx,
                )
            })
            .collect();
        let settings = matches!(self.page, Page::Settings(_));
        div()
            .v_flex()
            .w(px(220.))
            .h_full()
            .flex_shrink_0()
            .p_2()
            .gap_1()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .children(items)
            .child(div().flex_1())
            .child(self.nav_item(
                "nav-settings",
                "설정",
                Page::Settings(Pane::Extensions),
                settings,
                cx,
            ))
    }

    fn settings(&self, current: Pane, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.enabled();
        let panes: Vec<_> = nav::panes(&enabled)
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                self.nav_item(
                    ("pane", index),
                    item.label,
                    Page::Settings(item.id),
                    item.id == current,
                    cx,
                )
            })
            .collect();
        let body = match current {
            Pane::Extensions => self.extensions(cx).into_any_element(),
            _ => placeholder(cx).into_any_element(),
        };
        div()
            .h_flex()
            .items_start()
            .gap_6()
            .child(div().v_flex().w(px(180.)).gap_1().children(panes))
            .child(div().v_flex().flex_1().child(body))
    }

    /// The on/off list, enough to drive the sidebar until the real pane (#351).
    fn extensions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .snapshot
            .iter()
            .flat_map(|snapshot| &snapshot.extensions)
            .map(|extension| {
                let id = extension.id;
                div()
                    .h_flex()
                    .justify_between()
                    .gap_4()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(extension.display_name)
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(extension.description),
                            ),
                    )
                    .child(
                        Switch::new(id)
                            .checked(extension.enabled)
                            .disabled(self.busy || !extension.available)
                            .accessibility_label(extension.display_name)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.load(Some((id, *checked)), cx);
                                cx.notify();
                            })),
                    )
            })
            .collect();
        div().v_flex().children(rows)
    }
}

fn placeholder(cx: &App) -> impl IntoElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .child("GPUI 앱에서는 아직 준비 중인 화면입니다.")
}

impl Render for MainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.page {
            Page::Settings(pane) => self.settings(pane, cx).into_any_element(),
            _ => placeholder(cx).into_any_element(),
        };
        div()
            .h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.sidebar(cx))
            .child(
                div()
                    .id("page")
                    .v_flex()
                    .flex_1()
                    .h_full()
                    .overflow_y_scroll()
                    .p_6()
                    .gap_4()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child(nav::heading(self.page)),
                    )
                    .when_some(self.error.clone(), |page, error| {
                        page.child(div().text_color(cx.theme().danger).child(error))
                    })
                    .child(body),
            )
    }
}
