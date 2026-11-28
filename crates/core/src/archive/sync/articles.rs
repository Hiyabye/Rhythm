//! Fetching a notice page under the robots policy, and recording a source's
//! sync progress.
use super::*;

pub(super) fn persist(
    store: &SharedStore,
    source: &SourceInfo,
    status: SourceStatus,
    error: Option<&str>,
    success: bool,
) -> Result<(), String> {
    with_store(store, |s| {
        s.source_progress(
            &source.definition.id,
            status,
            source.next_page,
            source.history_complete,
            source.discovered,
            source.archived,
            source.failed,
            error,
            success,
        )
    })
}
pub(super) async fn fetch<T: Transport>(
    transport: &T,
    url: &Url,
    validators: Option<(Option<String>, Option<String>)>,
    limit: usize,
    cancel: &AtomicBool,
    policy: &Robots,
    access: network::Access,
) -> Result<network::Fetch, String> {
    let mut target = url.clone();
    for redirects in 0..=3 {
        // Apply robots to every hop before any request, not only the original URL.
        // Origin equality also prevents sending validators to another source host.
        if !policy.permits(&target) {
            return Err("학교의 robots.txt 정책에 따라 수집을 중단했습니다.".into());
        }
        if policy.delay > 0.65 {
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs_f64(policy.delay))=>{},_=wait_cancel(cancel)=>return Err("동기화를 중단했습니다.".into())}
        }
        match transport
            .fetch(&target, validators.clone(), limit, cancel, access)
            .await?
        {
            network::Fetch::Redirect(next) => {
                if redirects == 3 {
                    return Err("학교 서버의 이동 경로가 너무 많습니다.".into());
                }
                network::validate(&next, access)?;
                target = next;
            }
            response => return Ok(response),
        }
    }
    unreachable!()
}
pub(super) async fn wait_cancel(cancel: &AtomicBool) {
    crate::job::cancelled(cancel, std::time::Duration::from_millis(25)).await;
}
