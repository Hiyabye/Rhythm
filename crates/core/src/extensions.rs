use crate::storage::Store;
use serde::Serialize;

pub const NOTICES: &str = "sogang_notices";
pub const CYBERCAMPUS: &str = "cybercampus";
pub const SEARCH: &str = "search";
pub const SAVED: &str = "saved";
pub const RECOMMENDATIONS: &str = "recommendations";
pub const WEEK: &str = "week";
pub const CALENDAR: &str = "calendar";
pub const AI: &str = "ai";
pub const PROACTIVE: &str = "proactive";
pub const STUDIUM: &str = "studium";
pub const MEALS: &str = "meals";

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
pub const BUILT_IN: [(&str, &str, &str); 11] = [
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
    (SEARCH, "검색", "보관한 공지를 검색하고 조건으로 걸러 봅니다."),
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
        CALENDAR,
        "캘린더",
        "과제 마감과 공지 속 날짜를 달력으로 보고, 직접 만든 할 일을 앞으로 7일에도 표시합니다.",
    ),
    (
        AI,
        "AI 분류",
        "작은 분류 모델이 이 기기에서 공지를 주제별로 나누고, 비슷한 공지, 뜻으로 찾는 검색과 마감이 가까운 기회 모아 보기를 켭니다.",
    ),
    (
        PROACTIVE,
        "알림·백그라운드",
        "앱이 켜져 있는 동안 새 공지를 확인하고 알림을 보냅니다. 로그인 시 실행과 트레이도 여기서 정합니다.",
    ),
    (
        STUDIUM,
        "열람실",
        "선택한 열람실의 빈 좌석 수와 번호를 직접 확인합니다.",
    ),
    (
        MEALS,
        "식단",
        "벨라르미노 학사 식당의 주간 메뉴표를 글자로 읽어 요일별로 보여 줍니다.",
    ),
];

/// The extension that owns a catalog source family.
pub fn owner(family: &str) -> &'static str {
    if family == rhythm_sources::catalog::CYBERCAMPUS_FAMILY {
        CYBERCAMPUS
    } else {
        NOTICES
    }
}

/// Every built-in extension with its stored on/off state, in UI order.
pub fn list(store: &Store) -> Result<Vec<ExtensionInfo>, String> {
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

pub fn is_built_in(id: &str) -> bool {
    BUILT_IN.iter().any(|&(known, ..)| known == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cybercampus_family_has_its_own_extension() {
        assert_eq!(
            owner(rhythm_sources::catalog::CYBERCAMPUS_FAMILY),
            CYBERCAMPUS
        );
        assert_eq!(owner("engineering"), NOTICES);
    }
}
