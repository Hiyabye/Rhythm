//! Native views share the controller's memory. No display operation fetches
//! school pages, enables automatic checks or persists seat results.
use super::{controller::Controller, model::Snapshot};
use crate::{
    archive::sync::SharedStore, operation_gate::AdmissionGate, storage::studium::MiniGeometry,
    AppState,
};
use chrono::Timelike;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tokio::sync::{oneshot, Notify};

pub(crate) const MINI_WINDOW: &str = "studium-mini";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MiniStatus {
    pub open: bool,
    pub always_on_top: bool,
}

struct WindowSession {
    physical: u64,
    finished: AtomicBool,
    done: Notify,
}
impl WindowSession {
    async fn wait(&self) {
        loop {
            let done = self.done.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if self.finished.load(Ordering::Acquire) {
                return;
            }
            done.await;
        }
    }
    fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        self.done.notify_waiters();
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CloseTicket(u64);

#[derive(Default)]
struct WindowState {
    generation: u64,
    open: bool,
    on_top: bool,
    session: Option<Arc<WindowSession>>,
    closing: bool,
}
impl WindowState {
    fn reserve_open(&mut self) -> Result<u64, String> {
        if self.closing {
            return Err("미니 창을 닫고 있습니다. 잠시 후 다시 열어 주세요.".into());
        }
        self.generation = self.generation.checked_add(1).ok_or("미니 창 번호 오류")?;
        if !self.open {
            self.on_top = false;
        }
        self.open = true;
        Ok(self.generation)
    }
    fn status(&self) -> MiniStatus {
        MiniStatus {
            open: self.open,
            always_on_top: self.open && self.on_top,
        }
    }
    fn ticket(&self) -> CloseTicket {
        CloseTicket(self.generation)
    }
    fn matches(&self, ticket: CloseTicket) -> bool {
        self.generation == ticket.0
    }
    fn rollback_open(&mut self, generation: u64) {
        if self.generation == generation && self.session.is_none() {
            self.open = false;
            self.on_top = false;
        }
    }
    fn destroyed(&mut self, physical: u64) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| session.physical == physical)
        {
            self.session = None;
            self.open = false;
            self.on_top = false;
            self.closing = false;
        }
    }
}

struct Worker {
    cancel: AtomicBool,
    finished: AtomicBool,
    done: Notify,
}
impl Worker {
    async fn wait(&self) {
        loop {
            let done = self.done.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if self.finished.load(Ordering::Acquire) {
                return;
            }
            done.await;
        }
    }
}

pub(crate) struct Views {
    store: SharedStore,
    controller: Weak<Controller>,
    gate: Arc<AdmissionGate>,
    app: AppHandle,
    windows: Mutex<WindowState>,
    worker: Mutex<Option<Arc<Worker>>>,
    wake: Notify,
    shutdown: AtomicBool,
}

impl Views {
    pub(crate) fn new(
        store: SharedStore,
        controller: Weak<Controller>,
        gate: Arc<AdmissionGate>,
        app: AppHandle,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            controller,
            gate,
            app,
            windows: Mutex::new(WindowState::default()),
            worker: Mutex::new(None),
            wake: Notify::new(),
            shutdown: AtomicBool::new(false),
        })
    }
    pub(crate) fn refresh(&self) {
        self.wake.notify_one();
    }
    pub(crate) fn capture_close(&self) -> Option<CloseTicket> {
        self.windows.lock().ok().map(|state| state.ticket())
    }
    fn status(&self) -> Result<MiniStatus, String> {
        self.windows
            .lock()
            .map(|state| state.status())
            .map_err(|_| "미니 창 상태 잠금 오류".into())
    }
    pub(crate) fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.shutdown.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut worker = self.worker.lock().map_err(|_| "열람실 표시 잠금 오류")?;
        if worker
            .as_ref()
            .is_some_and(|worker| !worker.finished.load(Ordering::Acquire))
        {
            return Ok(());
        }
        let run = Arc::new(Worker {
            cancel: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            done: Notify::new(),
        });
        *worker = Some(run.clone());
        let owner = self.clone();
        let task = async move {
            let _finish = WorkerFinish(run.clone());
            owner.run(run).await;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(task);
            }
            Err(_) => {
                tauri::async_runtime::spawn(task);
            }
        }
        self.refresh();
        Ok(())
    }
    pub(crate) async fn quiesce(&self) {
        let run = self.worker.lock().ok().and_then(|worker| worker.clone());
        if let Some(run) = run {
            run.cancel.store(true, Ordering::Release);
            self.wake.notify_waiters();
            run.wait().await;
        }
    }
    pub(crate) async fn resume_after_quiesce(self: &Arc<Self>) {
        let run = self.worker.lock().ok().and_then(|worker| worker.clone());
        if let Some(run) = run.filter(|run| run.cancel.load(Ordering::Acquire)) {
            run.wait().await;
        }
        if let Err(error) = self.start() {
            eprintln!("Studium: 표시 재개 실패: {error}");
        }
    }
    pub(crate) async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.quiesce().await;
    }
    async fn on_main<T: Send + 'static>(
        self: &Arc<Self>,
        action: impl FnOnce(Arc<Self>) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (send, receive) = oneshot::channel();
        let owner = self.clone();
        self.app
            .run_on_main_thread(move || {
                let result = if owner.shutdown.load(Ordering::Acquire) {
                    Err("Rhythm을 종료하고 있습니다.".into())
                } else {
                    action(owner)
                };
                let _ = send.send(result);
            })
            .map_err(|_| "미니 창 작업을 전달하지 못했습니다.")?;
        receive
            .await
            .map_err(|_| "미니 창 작업이 중단되었습니다.".to_string())?
    }
    async fn run(self: Arc<Self>, run: Arc<Worker>) {
        loop {
            if run.cancel.load(Ordering::Acquire) {
                break;
            }
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let Some(controller) = self.controller.upgrade() else {
                break;
            };
            tokio::select! {
                biased;
                _ = super::cancelled(&run.cancel) => break,
                result = self.update(run.clone()) => if let Err(error) = result { eprintln!("Studium: 표시 갱신 실패: {error}"); },
            }
            tokio::select! {
                _ = super::cancelled(&run.cancel) => break,
                _ = &mut notified => {},
                _ = controller.wait_for_view_change() => {},
                _ = tokio::time::sleep(Duration::from_secs(if self.gate.ensure_open().is_ok() { 30 } else { 1 })) => {},
            }
        }
    }
    async fn update(self: &Arc<Self>, run: Arc<Worker>) -> Result<(), String> {
        let _lease = match self.gate.shared().await {
            Ok(lease) => lease,
            Err(_) => return Ok(()),
        };
        self.on_main(move |owner| {
            if run.cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            // Re-read after reaching the main thread. A new lookup or a
            // preference edit may have happened while this action was queued.
            // Native application shares Store with admission/publication, so
            // an older generation never overwrites a newer controller state.
            let store = owner.store.lock().map_err(|_| "설정 잠금 오류")?;
            owner.gate.ensure_open()?;
            let snapshot = if store.enabled(crate::extensions::STUDIUM)? {
                owner
                    .controller
                    .upgrade()
                    .map(|controller| controller.snapshot(store.studium_preferences()?))
                    .transpose()?
            } else {
                None
            };
            let clock_format = store
                .settings()
                .map_err(|_| "시간 표시 설정을 읽지 못했습니다.")?
                .clock_format;
            let rows = snapshot
                .as_ref()
                .map(|snapshot| {
                    summary_rows(snapshot, chrono::Utc::now().timestamp(), &clock_format)
                })
                .unwrap_or_default();
            let tooltip = tray_tooltip(&rows);
            crate::core::lifecycle::update_studium_tray(&owner.app, &rows, &tooltip)
                .map_err(|_| "트레이 현황을 갱신하지 못했습니다.")?;
            drop(store);
            let _ = owner.app.emit("studium://state-changed", ());
            Ok(())
        })
        .await
    }

    fn enabled(&self) -> Result<(), String> {
        self.gate.ensure_open()?;
        if self.shutdown.load(Ordering::Acquire) {
            return Err("Rhythm을 종료하고 있습니다.".into());
        }
        let store = self.store.lock().map_err(|_| "설정 잠금 오류")?;
        if !store.enabled(crate::extensions::STUDIUM)? {
            return Err("Studium 확장 기능이 꺼져 있습니다.".into());
        }
        Ok(())
    }

    async fn open(self: &Arc<Self>) -> Result<MiniStatus, String> {
        let _lease = self.gate.shared().await?;
        self.enabled()?;
        let closing = {
            let state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            state.session.as_ref().filter(|_| state.closing).cloned()
        };
        if let Some(closing) = closing {
            tokio::time::timeout(Duration::from_secs(2), closing.wait())
                .await
                .map_err(|_| "미니 창을 닫고 있습니다. 잠시 후 다시 열어 주세요.")?;
        }
        let generation = {
            let store = self.store.lock().map_err(|_| "설정 잠금 오류")?;
            self.gate.ensure_open()?;
            if !store.enabled(crate::extensions::STUDIUM)? {
                return Err("Studium 확장 기능이 꺼져 있습니다.".into());
            }
            self.windows
                .lock()
                .map_err(|_| "미니 창 상태 잠금 오류")?
                .reserve_open()?
        };
        let result = self.on_main(move |owner| owner.open_now(generation)).await;
        if result.is_err() {
            if let Ok(mut state) = self.windows.lock() {
                // Only a failed creation request can roll back its reservation;
                // a later explicit open or an existing native window survives.
                state.rollback_open(generation);
            }
        }
        result
    }

    fn open_now(self: &Arc<Self>, generation: u64) -> Result<MiniStatus, String> {
        let geometry = {
            let store = self.store.lock().map_err(|_| "설정 잠금 오류")?;
            self.gate.ensure_open()?;
            if !store.enabled(crate::extensions::STUDIUM)? {
                return Err("Studium 확장 기능이 꺼져 있습니다.".into());
            }
            let state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            if state.generation != generation || !state.open {
                return Err("이전 미니 창 요청을 무시했습니다.".into());
            }
            store.studium_mini_geometry()?
        };
        if let Some(window) = self.app.get_webview_window(MINI_WINDOW) {
            let store = self.store.lock().map_err(|_| "설정 잠금 오류")?;
            self.gate.ensure_open()?;
            if !store.enabled(crate::extensions::STUDIUM)? {
                return Err("Studium 확장 기능이 꺼져 있습니다.".into());
            }
            {
                let state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
                if !state.matches(CloseTicket(generation)) {
                    return Err("이전 미니 창 요청을 무시했습니다.".into());
                }
                if state.session.is_none() || state.closing {
                    return Err("미니 창을 닫고 있습니다. 잠시 후 다시 열어 주세요.".into());
                }
            }
            window
                .show()
                .map_err(|_| "미니 창을 표시하지 못했습니다.")?;
            window
                .unminimize()
                .map_err(|_| "미니 창을 표시하지 못했습니다.")?;
            window
                .set_focus()
                .map_err(|_| "미니 창에 포커스를 주지 못했습니다.")?;
            return self.status();
        }
        let screens = self
            .app
            .available_monitors()
            .map_err(|_| "화면 배치를 확인하지 못했습니다.")?
            .into_iter()
            .map(|monitor| Screen {
                x: monitor.work_area().position.x,
                y: monitor.work_area().position.y,
                width: monitor.work_area().size.width,
                height: monitor.work_area().size.height,
                scale: monitor.scale_factor(),
            })
            .collect::<Vec<_>>();
        let placement = placement(geometry.as_ref(), &screens);
        let mut builder = WebviewWindowBuilder::new(
            &self.app,
            MINI_WINDOW,
            WebviewUrl::App("index.html?studium-mini=1".into()),
        )
        .title("Rhythm · 열람실")
        .inner_size(f64::from(placement.width), f64::from(placement.height))
        .min_inner_size(300.0, 260.0)
        .max_inner_size(1200.0, 1400.0)
        .visible(false)
        .always_on_top(false);
        if placement.position.is_none() {
            builder = builder.center();
        }
        let window = match builder.build() {
            Ok(window) => window,
            Err(_) => {
                if let Ok(mut state) = self.windows.lock() {
                    if state.generation == generation {
                        state.open = false;
                        state.on_top = false;
                    }
                }
                return Err("미니 창을 만들지 못했습니다.".into());
            }
        };
        let session = Arc::new(WindowSession {
            physical: generation,
            finished: AtomicBool::new(false),
            done: Notify::new(),
        });
        let store = self.store.lock().map_err(|_| "설정 잠금 오류")?;
        if self.gate.ensure_open().is_err()
            || !store.enabled(crate::extensions::STUDIUM)?
            || !self
                .windows
                .lock()
                .map_err(|_| "미니 창 상태 잠금 오류")?
                .matches(CloseTicket(generation))
        {
            let _ = window.destroy();
            return Err("미니 창 요청이 취소되었습니다.".into());
        }
        {
            let mut state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            state.session = Some(session.clone());
            state.closing = false;
            state.on_top = false;
        }
        let owner = Arc::downgrade(self);
        window.on_window_event(move |event| {
            if let Some(owner) = owner.upgrade() {
                match event {
                    tauri::WindowEvent::CloseRequested { .. } => owner.native_close(&session),
                    tauri::WindowEvent::Destroyed => owner.native_destroyed(&session),
                    _ => {}
                }
            } else if matches!(event, tauri::WindowEvent::Destroyed) {
                session.finish();
            }
        });
        if let Some((x, y)) = placement.position {
            window
                .set_position(PhysicalPosition::new(x, y))
                .map_err(|_| "미니 창 위치를 복원하지 못했습니다.")?;
        }
        window
            .show()
            .map_err(|_| "미니 창을 표시하지 못했습니다.")?;
        window
            .set_focus()
            .map_err(|_| "미니 창에 포커스를 주지 못했습니다.")?;
        self.refresh();
        self.status()
    }

    fn native_close(&self, session: &Arc<WindowSession>) {
        let ticket = if let Ok(mut state) = self.windows.lock() {
            if state
                .session
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, session))
            {
                state.open = false;
                state.on_top = false;
                state.closing = true;
                Some(state.ticket())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(ticket) = ticket {
            self.save_geometry(ticket);
        }
        self.refresh();
    }

    fn native_destroyed(&self, session: &Arc<WindowSession>) {
        finish_destroyed_state(&self.windows, session);
        self.refresh();
    }

    /// Executed on the main thread while the native window still exists.
    fn save_geometry(&self, ticket: CloseTicket) {
        let Some(window) = self.app.get_webview_window(MINI_WINDOW) else {
            return;
        };
        let Some(geometry) = geometry(&window) else {
            return;
        };
        if let Ok(store) = self.store.lock() {
            if self.gate.ensure_open().is_err() {
                return;
            }
            let current = self.windows.lock().is_ok_and(|state| state.matches(ticket));
            if current {
                if let Err(error) = store.studium_save_mini_geometry(&geometry) {
                    eprintln!("Studium: 미니 창 위치 저장 실패: {error}");
                }
            }
        }
    }

    /// Exit is delivered on the main thread. Capture synchronously before its
    /// event loop stops; shutdown never waits for a queued main-thread action.
    pub(crate) fn save_exit_geometry(&self) {
        if let Some(ticket) = self.capture_close() {
            self.save_geometry(ticket);
        }
    }

    pub(crate) async fn disable(self: &Arc<Self>, ticket: CloseTicket) -> Result<(), String> {
        let session = self.on_main(move |owner| owner.close_now(ticket)).await?;
        if let Some(session) = session {
            tokio::time::timeout(Duration::from_secs(2), session.wait())
                .await
                .map_err(|_| "미니 창 닫기를 확인하지 못했습니다.")?;
        }
        self.refresh();
        Ok(())
    }

    fn close_now(&self, ticket: CloseTicket) -> Result<Option<Arc<WindowSession>>, String> {
        {
            let state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            if !state.matches(ticket) {
                return Ok(None);
            }
            if state.closing {
                return Ok(state.session.clone());
            }
        }
        self.save_geometry(ticket);
        let window = self.app.get_webview_window(MINI_WINDOW);
        let session = {
            let mut state = self.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            if !state.matches(ticket) {
                return Ok(None);
            }
            state.open = false;
            state.on_top = false;
            state.closing = true;
            state.session.clone()
        };
        if let Some(window) = window {
            if window.destroy().is_err() {
                let _ = window.set_always_on_top(false);
                if let Ok(mut state) = self.windows.lock() {
                    if state.matches(ticket) {
                        state.open = true;
                        state.closing = false;
                    }
                }
                return Err("미니 창을 닫지 못했습니다.".into());
            }
        } else {
            if let Ok(mut state) = self.windows.lock() {
                state.closing = false;
                state.session = None;
            }
            if let Some(session) = &session {
                session.finish();
            }
        }
        Ok(session)
    }

    async fn set_on_top(self: &Arc<Self>, enabled: bool) -> Result<MiniStatus, String> {
        let _lease = self.gate.shared().await?;
        self.enabled()?;
        self.on_main(move |owner| {
            let store = owner.store.lock().map_err(|_| "설정 잠금 오류")?;
            owner.gate.ensure_open()?;
            if !store.enabled(crate::extensions::STUDIUM)? {
                return Err("Studium 확장 기능이 꺼져 있습니다.".into());
            }
            if !owner
                .windows
                .lock()
                .map_err(|_| "미니 창 상태 잠금 오류")?
                .open
            {
                return Err("미니 창이 열려 있지 않습니다.".into());
            }
            let window = owner
                .app
                .get_webview_window(MINI_WINDOW)
                .ok_or("미니 창이 열려 있지 않습니다.")?;
            window
                .set_always_on_top(enabled)
                .map_err(|_| "항상 위 표시를 바꾸지 못했습니다.")?;
            let mut state = owner.windows.lock().map_err(|_| "미니 창 상태 잠금 오류")?;
            if !state.open {
                return Err("미니 창이 열려 있지 않습니다.".into());
            }
            state.on_top = enabled;
            let result = state.status();
            drop(state);
            owner.refresh();
            Ok(result)
        })
        .await
    }
}

struct WorkerFinish(Arc<Worker>);
impl Drop for WorkerFinish {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.done.notify_waiters();
    }
}

fn finish_destroyed_state(windows: &Mutex<WindowState>, session: &WindowSession) {
    if let Ok(mut state) = windows.lock() {
        state.destroyed(session.physical);
    }
    // Waking an opener is the final step: it must observe a fully cleaned
    // native session rather than the old `closing` flag.
    session.finish();
}

fn summary_rows(snapshot: &Snapshot, now: i64, clock_format: &str) -> Vec<String> {
    snapshot
        .preferences
        .room_ids
        .iter()
        .filter_map(|id| {
            let room = snapshot.rooms.iter().find(|room| room.id == id)?;
            let value = snapshot.results.iter().find(|value| &value.room_id == id);
            let status = match value {
                Some(value) if value.status == "loading" => "확인 중".into(),
                Some(value) if matches!(value.status.as_str(), "ready" | "count_only") => {
                    match &value.result {
                        Some(data) => {
                            let source =
                                chrono::DateTime::parse_from_rfc3339(&data.source_time).ok();
                            let stale = source.as_ref().is_none_or(|time| {
                                !(-60..=300).contains(&now.saturating_sub(time.timestamp()))
                            });
                            let time = source
                                .map(|time| {
                                    let local = time.with_timezone(
                                        &chrono::FixedOffset::east_opt(9 * 3600).unwrap(),
                                    );
                                    if clock_format == "24h" {
                                        local.format("%m/%d %H:%M").to_string()
                                    } else {
                                        format!(
                                            "{} {} {:02}:{:02}",
                                            local.format("%m/%d"),
                                            if local.hour() < 12 {
                                                "오전"
                                            } else {
                                                "오후"
                                            },
                                            (local.hour() + 11) % 12 + 1,
                                            local.minute()
                                        )
                                    }
                                })
                                .unwrap_or_else(|| "기준 시각 미확인".into());
                            format!(
                                "{}빈 {}석 · {}{}",
                                if stale { "오래됨 · " } else { "" },
                                data.remaining,
                                time,
                                if value.status == "count_only" {
                                    " · 번호 미확인"
                                } else {
                                    ""
                                }
                            )
                        }
                        None => "미조회".into(),
                    }
                }
                Some(value) if value.status == "failed" => "조회 실패".into(),
                Some(value) if value.status == "cancelled" => "취소됨".into(),
                _ => "미조회".into(),
            };
            Some(format!("{} · {status}", room.label))
        })
        .collect()
}

fn tray_tooltip(rows: &[String]) -> String {
    match rows.first() {
        None => "Rhythm".into(),
        Some(first) => {
            let text = format!(
                "Rhythm · {first}{}",
                if rows.len() > 1 {
                    format!(" 외 {}개 열람실", rows.len() - 1)
                } else {
                    String::new()
                }
            );
            text.chars()
                .scan(0_usize, |count, character| {
                    *count += character.len_utf16();
                    (*count <= 120).then_some(character)
                })
                .collect()
        }
    }
}

#[derive(Clone, Copy)]
struct Screen {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
}
#[derive(Debug, PartialEq, Eq)]
struct Placement {
    width: u32,
    height: u32,
    position: Option<(i32, i32)>,
}

fn placement(saved: Option<&MiniGeometry>, screens: &[Screen]) -> Placement {
    let width = saved.map_or(380, |saved| saved.width).clamp(300, 1200);
    let height = saved.map_or(480, |saved| saved.height).clamp(260, 1400);
    let usable = |screen: &&Screen| {
        screen.scale.is_finite() && screen.scale > 0.0 && screen.width > 0 && screen.height > 0
    };
    if let Some(saved) = saved {
        for screen in screens.iter().filter(usable) {
            let (x, y) = (f64::from(saved.x), f64::from(saved.y));
            if x >= f64::from(screen.x)
                && y >= f64::from(screen.y)
                && x + f64::from(width) * screen.scale
                    <= f64::from(screen.x) + f64::from(screen.width)
                && y + f64::from(height) * screen.scale
                    <= f64::from(screen.y) + f64::from(screen.height)
            {
                return Placement {
                    width,
                    height,
                    position: Some((saved.x, saved.y)),
                };
            }
        }
    }
    let fit = screens.iter().find(usable);
    Placement {
        width: fit.map_or(width, |screen| {
            width
                .min((f64::from(screen.width) / screen.scale) as u32)
                .max(300)
        }),
        height: fit.map_or(height, |screen| {
            height
                .min((f64::from(screen.height) / screen.scale) as u32)
                .max(260)
        }),
        position: None,
    }
}

fn geometry(window: &WebviewWindow) -> Option<MiniGeometry> {
    let position = window.outer_position().ok()?;
    let scale = window.scale_factor().ok()?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let size = window.inner_size().ok()?;
    Some(MiniGeometry {
        x: position.x.clamp(-100_000, 100_000),
        y: position.y.clamp(-100_000, 100_000),
        width: (f64::from(size.width) / scale).round().clamp(300.0, 1200.0) as u32,
        height: (f64::from(size.height) / scale)
            .round()
            .clamp(260.0, 1400.0) as u32,
    })
}

#[tauri::command]
pub(crate) async fn studium_open_mini(
    state: tauri::State<'_, AppState>,
) -> Result<MiniStatus, String> {
    state.studium_views.open().await
}
#[tauri::command]
pub(crate) async fn studium_close_mini(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    state.studium_views.enabled()?;
    if let Some(ticket) = state.studium_views.capture_close() {
        state.studium_views.disable(ticket).await?;
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn studium_mini_status(
    state: tauri::State<'_, AppState>,
) -> Result<MiniStatus, String> {
    let _lease = state.operation_gate.shared().await?;
    state.studium_views.status()
}
#[tauri::command]
pub(crate) async fn studium_mini_set_on_top(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<MiniStatus, String> {
    state.studium_views.set_on_top(enabled).await
}
#[tauri::command]
pub(crate) async fn studium_mini_show_main(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _lease = state.operation_gate.shared().await?;
    state.studium_views.enabled()?;
    state
        .studium_views
        .on_main(|owner| {
            owner.enabled()?;
            crate::core::lifecycle::show_main_window(&owner.app)
                .map_err(|_| "Rhythm 창을 열지 못했습니다.".into())
        })
        .await
}

#[cfg(test)]
mod tests;
