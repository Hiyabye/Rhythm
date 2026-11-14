//! Synthetic Korean evaluation notices shared by the opt-in embedding
//! evaluation and the date extractor tests. Not university content.
use crate::{
    archive::model::{ArticleInput, SourceDefinition},
    storage::Store,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Case {
    pub(super) id: String,
    pub(super) title: String,
    text: String,
    pub(super) category: String,
    #[serde(default)]
    repeat_text: String,
    #[serde(default)]
    repeat_count: usize,
    #[serde(default)]
    suffix: String,
}
impl Case {
    pub(super) fn body(&self) -> String {
        format!(
            "{}{}{}",
            self.text,
            self.repeat_text.repeat(self.repeat_count),
            self.suffix
        )
    }
    pub(super) fn article(&self) -> ArticleInput {
        let body = self.body();
        ArticleInput {
            external_id: self.id.clone(),
            canonical_url: format!("https://www.sogang.ac.kr/evaluation/{}", self.id),
            title: self.title.clone(),
            author: Some("합성 평가 자료".into()),
            category: None,
            published_at: "2026-01-01".into(),
            modified_at: None,
            original: body.clone(),
            unsupported_body: false,
            body_html: "<p>합성 평가 자료</p>".into(),
            body_text: body,
            resources: vec![],
            etag: None,
            last_modified: None,
        }
    }
}
pub(super) fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("evaluation.json")).expect("valid evaluation fixtures")
}
pub(super) fn seed(store: &Store) {
    store
        .catalog(&[SourceDefinition {
            id: "ai-evaluation".into(),
            name: "합성 한국어 평가".into(),
            organization: "Rhythm".into(),
            description: "외부 수집이 아닌 결정적 합성 자료".into(),
            url: "https://www.sogang.ac.kr/".into(),
            family: "main".into(),
            supported: true,
            group: None,
        }])
        .unwrap();
}
#[cfg(unix)]
pub(super) fn worker_rss_kib(pid: Option<u32>) -> Option<u64> {
    let pid = pid?;
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}
#[cfg(windows)]
pub(super) fn worker_rss_kib(pid: Option<u32>) -> Option<u64> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
    };
    // SAFETY: The process handle is closed once and the output structure is valid.
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid?,
        )
        .ok()?;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let size = counters.cb;
        let result = GetProcessMemoryInfo(handle, &mut counters, size)
            .ok()
            .map(|()| counters.WorkingSetSize as u64 / 1024);
        let _ = CloseHandle(handle);
        result
    }
}
#[cfg(not(any(unix, windows)))]
pub(super) fn worker_rss_kib(_pid: Option<u32>) -> Option<u64> {
    None
}
