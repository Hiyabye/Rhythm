//! The opened core app and the tokio runtime its async methods run on. GPUI
//! drives its own executor on the UI thread, so each core call runs on the
//! runtime and the UI awaits only its result.

use gpui_kit::Global;
use rhythm_core::app::App as Rhythm;
use std::{future::Future, sync::Arc};
use tokio::runtime::Runtime;

pub(crate) struct Backend {
    pub(crate) app: Arc<Rhythm>,
    pub(crate) runtime: Arc<Runtime>,
}

impl Global for Backend {}

impl Backend {
    /// Runs `call` on the runtime. The future it returns needs no runtime of
    /// its own, so a GPUI task can await it.
    pub(crate) fn run<T, F, Fut>(
        &self,
        call: F,
    ) -> impl Future<Output = Result<T, String>> + 'static
    where
        F: FnOnce(Arc<Rhythm>) -> Fut,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
    {
        let task = self.runtime.spawn(call(self.app.clone()));
        async move { task.await.map_err(|error| error.to_string())? }
    }
}
