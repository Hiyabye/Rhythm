//! Tokio interop: Rhythm's network/storage code runs on tokio, GPUI has its own
//! executor. A shared multi-thread runtime runs the futures; GPUI awaits the
//! join handle, and dropping the GPUI task aborts the tokio task.

use std::{sync::LazyLock, time::Duration, time::Instant};

use gpui_kit::component::{StyledExt as _, button::{Button, ButtonVariants as _}};
use gpui_kit::*;
use tokio::{io::AsyncWriteExt as _, net::TcpListener};

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime")
});

/// Aborts the tokio task when GPUI drops the awaiting task (cancellation).
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Default)]
pub struct AsyncProbe {
    status: String,
    ticks: u32,
    task: Option<Task<()>>,
    ticker: Option<Task<()>>,
}

impl AsyncProbe {
    fn start(&mut self, cx: &mut Context<Self>) {
        self.status = "요청 중… (서버가 2초 뒤 응답)".into();
        self.ticks = 0;
        let started = Instant::now();
        let handle = AbortOnDrop(RUNTIME.spawn(probe()));
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut handle = handle;
            let result = (&mut handle.0).await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(Ok(text)) => format!("완료 {:.1}s: {text}", started.elapsed().as_secs_f32()),
                    Ok(Err(err)) => format!("실패: {err}"),
                    Err(err) => format!("tokio 작업 종료: {err}"),
                };
                this.ticker = None;
                cx.notify();
            });
        }));
        // UI responsiveness: this counter must keep ticking while the request runs.
        self.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            if this.update(cx, |this, cx| { this.ticks += 1; cx.notify(); }).is_err() {
                break;
            }
        }));
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.task.take().is_some() {
            self.status = "취소됨 (tokio 작업 abort)".into();
        }
        self.ticker = None;
        cx.notify();
    }
}

/// A loopback HTTP server that answers after 2 s, a reqwest GET against it and
/// a rusqlite query. No external network.
async fn probe() -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let body = "안녕하세요";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let text = reqwest::get(format!("http://{addr}/"))
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let count = tokio::task::spawn_blocking(|| -> rusqlite::Result<i64> {
        let db = rusqlite::Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE n(title TEXT); INSERT INTO n VALUES ('공지1'),('공지2');")?;
        db.query_row("SELECT count(*) FROM n", [], |row| row.get(0))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(format!("HTTP {text:?}, SQLite rows {count}"))
}

impl Render for AsyncProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(Button::new("start").primary().label("요청 시작").on_click(cx.listener(|this, _, _, cx| this.start(cx))))
                    .child(Button::new("cancel").label("취소").on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))),
            )
            .child(format!("상태: {}", self.status))
            .child(format!("UI 틱: {}", self.ticks))
    }
}
