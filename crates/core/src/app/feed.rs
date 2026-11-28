//! The signed notice feed of the `sogang_notices` extension (공지 피드 in the
//! 게시판 pane); the sync itself is [`crate::feed`].

use super::App;
use crate::{
    extensions::NOTICES,
    feed::transport::HttpsTransport,
    storage::{feed::FeedStatus, with_store},
};

const OFF: &str = "서강대학교 공지 기능이 꺼져 있습니다.";

pub struct Feed<'a>(pub(super) &'a App);

impl Feed<'_> {
    /// The feed's age, how the last sync ended and the boards the server keeps
    /// failing on.
    pub async fn status(&self) -> Result<FeedStatus, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        if !with_store(&app.store, |s| s.enabled(NOTICES))? {
            return Err(OFF.into());
        }
        app.feed.status(&app.store)
    }

    /// 지금 받기: syncs now, or waits for the sync already running.
    pub async fn sync(&self) -> Result<FeedStatus, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        if !with_store(&app.store, |s| s.enabled(NOTICES))? {
            return Err(OFF.into());
        }
        if !app
            .feed
            .run(
                &app.store,
                &app.data_root,
                &HttpsTransport::new(app.user_agent),
            )
            .await?
        {
            app.feed.wait().await;
        }
        app.feed.status(&app.store)
    }
}
