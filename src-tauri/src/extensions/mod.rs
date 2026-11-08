use crate::storage::Store;
use serde::Serialize;

pub const NOTICES: &str = "sogang_notices";
pub const CYBERCAMPUS: &str = "cybercampus";
pub const SEARCH: &str = "search";
pub const SAVED: &str = "saved";
pub const RECOMMENDATIONS: &str = "recommendations";
pub const WEEK: &str = "week";
pub const AI: &str = "ai";
pub const PROACTIVE: &str = "proactive";

#[derive(Serialize)]
pub struct ExtensionInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub available: bool,
    pub enabled: bool,
}

/// Every feature beyond the core shell is a built-in extension, listed in the
/// order the UI shows them. Nothing is loaded from outside the app.
const BUILT_IN: [(&str, &str, &str); 8] = [
    (
        NOTICES,
        "서강대학교 공지",
        "구독한 학교 공지의 원문, 첨부 자료와 변경 이력을 이 기기에 보관합니다.",
    ),
    (
        CYBERCAMPUS,
        "사이버캠퍼스",
        "eClass에 로그인해 수강 과목의 공지와 과제·마감을 가져옵니다.",
    ),
    (SEARCH, "탐색", "보관한 공지를 검색하고 조건으로 걸러 봅니다."),
    (SAVED, "보관함", "저장한 공지를 한곳에 모아 봅니다."),
    (
        RECOMMENDATIONS,
        "나를 위한 소식",
        "관심사와 반응을 바탕으로 오늘 볼 만한 공지를 골라 줍니다.",
    ),
    (
        WEEK,
        "앞으로 7일",
        "다가오는 일주일의 공지와 마감을 날짜별로 보여 줍니다.",
    ),
    (
        AI,
        "AI 분석",
        "공지를 요약하고 날짜와 핵심 정보를 뽑아냅니다. 기기 안의 모델이나 직접 설정한 서버를 씁니다.",
    ),
    (
        PROACTIVE,
        "알림·백그라운드",
        "앱이 켜져 있는 동안 새 공지를 확인하고 알림을 보냅니다. 로그인 시 실행과 트레이도 여기서 정합니다.",
    ),
];

/// The extension that owns a catalog source family.
pub fn owner(family: &str) -> &'static str {
    if family == crate::archive::cybercampus::FAMILY {
        CYBERCAMPUS
    } else {
        NOTICES
    }
}

pub struct Registry {
    jobs: std::sync::Arc<crate::archive::sync::Controller>,
}
impl Registry {
    pub fn new(jobs: std::sync::Arc<crate::archive::sync::Controller>) -> Self {
        Self { jobs }
    }
    pub fn list(&self, store: &Store) -> Result<Vec<ExtensionInfo>, String> {
        BUILT_IN
            .iter()
            .map(|&(id, display_name, description)| {
                Ok(ExtensionInfo {
                    id,
                    display_name,
                    description,
                    available: true,
                    enabled: store.enabled(id)?,
                })
            })
            .collect()
    }
    /// Records the choice. Disabling a source extension signals archive
    /// cancellation under the caller's store lock, the same lock job admission
    /// takes, so no job of its sources can start or continue afterwards. Work
    /// that needs to await (AI, lifecycle) is stopped by the calling command.
    pub fn set(&self, store: &Store, id: &str, enabled: bool) -> Result<(), String> {
        if !BUILT_IN.iter().any(|&(known, ..)| known == id) {
            return Err("등록되지 않은 확장 기능입니다.".into());
        }
        if !enabled && matches!(id, NOTICES | CYBERCAMPUS) {
            self.jobs.cancel();
        }
        store.set_enabled(id, enabled)
    }
    pub fn stop_all(&self) {
        self.jobs.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_extension_preserves_preferences_without_automatic_networking() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("rhythm.db")).unwrap();
        let registry = Registry::new(crate::archive::sync::Controller::new());
        let listed = registry.list(&store).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.id).collect::<Vec<_>>(),
            [
                NOTICES,
                CYBERCAMPUS,
                SEARCH,
                SAVED,
                RECOMMENDATIONS,
                WEEK,
                AI,
                PROACTIVE
            ]
        );
        // A new profile starts with every extension off.
        assert!(listed.iter().all(|e| !e.enabled && e.available));
        registry.set(&store, NOTICES, true).unwrap();
        registry.set(&store, AI, true).unwrap();
        let enabled = registry.list(&store).unwrap();
        assert!(enabled[0].enabled && enabled[6].enabled);
        assert!(!enabled[1].enabled);
        registry.set(&store, NOTICES, false).unwrap();
        assert!(!registry.list(&store).unwrap()[0].enabled);
        assert_eq!(store.settings().unwrap().theme, "system");
        assert!(registry.set(&store, "unknown", true).is_err());
    }

    #[test]
    fn cybercampus_family_has_its_own_extension() {
        assert_eq!(owner(crate::archive::cybercampus::FAMILY), CYBERCAMPUS);
        assert_eq!(owner("engineering"), NOTICES);
    }
}
