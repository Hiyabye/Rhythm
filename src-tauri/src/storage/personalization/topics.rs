//! Topic extraction, canonical names, aliases and the sensitive-term denylist.
use super::*;

pub(super) fn canonical_topic(value: &str) -> String {
    match folded(value).as_str() {
        "ai" | "인공지능" => "ai".into(),
        "장학" | "장학금" | "scholarship" | "scholarships" => "scholarship".into(),
        "인턴" | "인턴십" | "internship" | "internships" => "internship".into(),
        "연구"
        | "research"
        | "연구 기회"
        | "연구기회"
        | "research opportunity"
        | "research opportunities" => "research".into(),
        "분산 시스템"
        | "분산 컴퓨팅"
        | "분산컴퓨팅"
        | "distributed systems"
        | "distributed computing" => "distributed systems".into(),
        "시스템 프로그래밍" | "시스템프로그래밍" | "system programming" | "systems programming" => {
            "systems programming".into()
        }
        "퀀트" | "퀀트 금융" | "퀀트금융" | "quant finance" | "quantitative finance" => {
            "quantitative finance".into()
        }
        "인공지능 인프라" | "ai 인프라" | "ai infra" | "ai infrastructure" => {
            "ai infrastructure".into()
        }
        "공모전" | "competition" | "competitions" => "competition".into(),
        other => other.into(),
    }
}

pub(super) fn display_topic(topic: &str) -> (&str, &'static [&'static str]) {
    match topic {
        "ai" => ("인공지능", &["AI"]),
        "scholarship" => ("장학금", &["장학", "scholarship", "scholarships"]),
        "internship" => ("인턴십", &["인턴", "internship", "internships"]),
        "research" => (
            "연구",
            &[
                "research",
                "연구 기회",
                "연구기회",
                "research opportunity",
                "research opportunities",
            ],
        ),
        "distributed systems" => (
            "분산 시스템",
            &[
                "distributed systems",
                "distributed computing",
                "분산 컴퓨팅",
                "분산컴퓨팅",
            ],
        ),
        "systems programming" => (
            "시스템 프로그래밍",
            &[
                "systems programming",
                "system programming",
                "시스템프로그래밍",
            ],
        ),
        "quantitative finance" => (
            "퀀트 금융",
            &["quantitative finance", "quant finance", "퀀트금융"],
        ),
        "ai infrastructure" => (
            "AI 인프라",
            &[
                "AI infrastructure",
                "AI infra",
                "인공지능 인프라",
                "ai 인프라",
            ],
        ),
        "competition" => ("공모전", &["competition", "competitions"]),
        other => (other, &[]),
    }
}

pub(super) fn interest_key(value: &str) -> String {
    canonical_topic(value)
}

pub(super) fn known_aliases(value: &str) -> &'static [&'static str] {
    match canonical_topic(value).as_str() {
        "ai" => &["AI", "인공지능"],
        "scholarship" => &["장학금", "장학", "scholarship", "scholarships"],
        "internship" => &["인턴십", "인턴", "internship", "internships"],
        "research" => &[
            "연구",
            "research",
            "연구 기회",
            "연구기회",
            "research opportunity",
            "research opportunities",
        ],
        "distributed systems" => &[
            "분산 시스템",
            "분산 컴퓨팅",
            "분산컴퓨팅",
            "distributed systems",
            "distributed computing",
        ],
        "systems programming" => &[
            "시스템 프로그래밍",
            "시스템프로그래밍",
            "systems programming",
            "system programming",
        ],
        "quantitative finance" => &[
            "퀀트 금융",
            "퀀트금융",
            "quantitative finance",
            "quant finance",
        ],
        "ai infrastructure" => &[
            "AI 인프라",
            "인공지능 인프라",
            "AI infrastructure",
            "AI infra",
            "ai 인프라",
        ],
        "competition" => &["공모전", "competition", "competitions"],
        _ => &[],
    }
}

pub(super) const TOPIC_STOPWORDS: &[&str] = &[
    "공지",
    "공지사항",
    "안내",
    "모집",
    "신청",
    "신청서",
    "기간",
    "대학",
    "학부",
    "학생",
    "서강",
    "소식",
    "학사",
    "학년도",
    "일반",
    "대상",
    "지원",
    "제출",
    "접수",
    "공고",
    "작성",
    "문의",
    "학교",
    "관련",
    "내용",
    "정보",
    "참고",
    "필수",
    "자격",
    "시행",
    "선발",
    "참가",
    "활동",
    "사업",
    "운영",
    "교육",
    "수업",
    "교내",
    "행사",
    "계획",
    "변경",
    "결과",
    "확인",
    "등록",
    "장소",
    "첨부",
    "파일",
    "연락",
    "담당",
    "부서",
    "교원",
    "교수",
    "재학생",
    "졸업생",
    "신입생",
    "대상자",
    "시스템",
    "notice",
    "announcement",
    "university",
    "news",
    "student",
    "students",
    "department",
    "campus",
    "application",
    "deadline",
    "schedule",
    "submit",
    "document",
    "eligibility",
    "opportunity",
    "opportunities",
    "program",
    "information",
    "recruitment",
];

pub(super) const SENSITIVE_TOPIC_MARKERS: &[&str] = &[
    "장애",
    "질병",
    "질환",
    "병력",
    "우울증",
    "불안장애",
    "정신질환",
    "성별",
    "성적",
    "여성",
    "남성",
    "성소수자",
    "동성애",
    "종교",
    "신앙",
    "인종",
    "민족",
    "국적",
    "외국인",
    "난민",
    "정치",
    "disability",
    "disabled",
    "disease",
    "disorder",
    "diagnosis",
    "depression",
    "anxiety",
    "mental",
    "gender",
    "sexual",
    "lgbt",
    "religion",
    "faith",
    "race",
    "ethnic",
    "nationality",
    "immigrant",
    "refugee",
    "political",
    "women",
    "female",
    "male",
];

pub(super) fn is_sensitive_topic(topic: &str) -> bool {
    SENSITIVE_TOPIC_MARKERS
        .iter()
        .any(|marker| topic.contains(marker))
}

pub(crate) fn topics(title: &str, category: Option<&str>) -> BTreeSet<String> {
    let text = format!("{title} {}", category.unwrap_or_default());
    let mut out = BTreeSet::new();
    for raw in text.split(|c: char| !(c.is_alphanumeric() || ('가'..='힣').contains(&c))) {
        let token = folded(raw);
        if out.len() < MAX_EVIDENCE_TOPICS_PER_ACTION
            && token.chars().count() >= 2
            && token.chars().count() <= 24
            && !token.chars().all(char::is_numeric)
            && !TOPIC_STOPWORDS.contains(&token.as_str())
            && !is_sensitive_topic(&token)
        {
            out.insert(canonical_topic(&token));
        }
    }
    let lower = text.to_lowercase();
    for (phrases, topic) in [
        (
            &[
                "분산 시스템",
                "분산 컴퓨팅",
                "distributed systems",
                "distributed computing",
            ][..],
            "distributed systems",
        ),
        (
            &[
                "시스템 프로그래밍",
                "시스템프로그래밍",
                "systems programming",
                "system programming",
            ][..],
            "systems programming",
        ),
        (
            &[
                "퀀트 금융",
                "퀀트금융",
                "quantitative finance",
                "quant finance",
            ][..],
            "quantitative finance",
        ),
        (
            &[
                "인공지능 인프라",
                "ai 인프라",
                "ai infrastructure",
                "ai infra",
            ][..],
            "ai infrastructure",
        ),
        (
            &[
                "research opportunity",
                "research opportunities",
                "연구 기회",
                "연구기회",
            ][..],
            "research",
        ),
    ] {
        if out.len() < MAX_EVIDENCE_TOPICS_PER_ACTION
            && phrases.iter().any(|phrase| matches_term(&lower, phrase))
        {
            out.insert(topic.into());
        }
    }
    for special in ["c++", "c#"] {
        if out.len() < MAX_EVIDENCE_TOPICS_PER_ACTION && lower.contains(special) {
            out.insert(special.into());
        }
    }
    out
}
