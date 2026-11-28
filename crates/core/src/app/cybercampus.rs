//! The `cybercampus` extension's eClass login (사이버캠퍼스 pane), sidebar
//! courses and 과제·마감 page; the connector itself is
//! [`crate::archive::cybercampus`].

use super::App;
use crate::{
    archive::cybercampus::{client, credentials, Status},
    extensions::CYBERCAMPUS,
    storage::{
        cybercampus::{CybercampusCourse, CybercampusTask},
        with_store, Store,
    },
};
use std::sync::atomic::AtomicBool;

const OFF: &str = "사이버캠퍼스 확장 기능이 꺼져 있습니다.";

pub struct Cybercampus<'a>(pub(super) &'a App);

impl Cybercampus<'_> {
    /// The saved login's ID (never the password), whether eClass rejected it
    /// and the synced courses.
    pub async fn status(&self) -> Result<Status, String> {
        let _lease = self.0.operation_gate.shared().await?;
        self.on()?;
        self.read_status()
    }

    /// Checks the login against eClass and stores it only when the site
    /// accepts it.
    pub async fn save_login(&self, id: &str, password: &str) -> Result<Status, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        let credentials = credentials::Credentials::new(id, password)?;
        if !with_store(&app.store, |s| s.enabled(CYBERCAMPUS))? {
            return Err("사이버캠퍼스 확장 기능을 먼저 켜 주세요.".into());
        }
        if app.archive.status()?.running {
            return Err("동기화가 끝난 뒤 다시 시도해 주세요.".into());
        }
        app.archive
            .cybercampus
            .save_login(
                &client::Reqwest::new(app.user_agent),
                &app.store,
                &credentials,
                &AtomicBool::new(false),
            )
            .await?;
        self.read_status()
    }

    /// Deletes the stored login and the in-memory session. Archived notices
    /// and deadlines stay, like any other source's archive. Never refused
    /// while the extension is off: removing a credential always works.
    pub async fn forget_login(&self) -> Result<Status, String> {
        let app = self.0;
        let _lease = app.operation_gate.shared().await?;
        app.archive.cybercampus.forget_login(&app.store).await?;
        self.read_status()
    }

    /// The synced term's courses with their unread counts, for the sidebar.
    /// Unlike [`Self::status`] it does not read the credential store.
    pub async fn courses(&self) -> Result<Vec<CybercampusCourse>, String> {
        self.with_on(Store::cybercampus_courses).await
    }

    pub async fn tasks(&self) -> Result<Vec<CybercampusTask>, String> {
        self.with_on(Store::cybercampus_tasks).await
    }

    fn on(&self) -> Result<(), String> {
        if with_store(&self.0.store, |s| s.enabled(CYBERCAMPUS))? {
            Ok(())
        } else {
            Err(OFF.into())
        }
    }

    fn read_status(&self) -> Result<Status, String> {
        Status::read(&self.0.archive.cybercampus, &self.0.store)
    }

    async fn with_on<T>(
        &self,
        read: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, String> {
        let _lease = self.0.operation_gate.shared().await?;
        with_store(&self.0.store, |s| {
            if !s.enabled(CYBERCAMPUS)? {
                return Err(OFF.into());
            }
            read(s)
        })
    }
}
