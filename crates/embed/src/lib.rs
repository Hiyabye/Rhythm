//! The notice embedding contract shared by the desktop app and the collector:
//! the instruction prefixes, the bounded notice input, the authored Korean
//! category prototypes and nearest-prototype scoring. Vectors from either side
//! are comparable only when both use these exact strings. Nothing here runs a
//! model or performs I/O.
//!
//! Each category has authored Korean prototypes, embedded with the same
//! instruction prefix as the notices; the nearest prototype wins. A score
//! below [`MIN_SCORE`] becomes `기타` with a low-confidence marker. Prototype
//! text, scoring and threshold are part of [`config_hash`], so changing them
//! reclassifies stored vectors without re-embedding and leaves old rows as
//! history.
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// Bumped only for a contract change the literal hash below cannot express.
const CONFIG_VERSION: &str = "notice-classification-v2";
const BODY_CHARS: usize = 1000;
const MAX_INPUT_BYTES: usize = 7 * 1024;
/// Calibrated on 1,073 hand-labeled archived notices: below this about half of
/// the top matches were wrong, so the match is only a weak hint
/// (docs/AI_EVALUATION.md).
pub const MIN_SCORE: f32 = 0.68;

/// Stored notice vectors carry this exact prefix and search compares against
/// them, so it keeps the original category list: changing it would require
/// re-embedding every notice, and the newer list measured no better.
pub const INSTRUCTION: &str = "Instruct: Classify the given Korean university notice into exactly one category: 학사, 장학, 수업·교육, 연구, 공모전·대회, 인턴·채용, 행사·활동, 기타.\nQuery: ";
/// Search queries are compared with the stored classifier vectors above. Of
/// the instructions tried on the synthetic search fixtures, Qwen3's own web
/// search instruction ranked relevant notices highest (docs/AI_EVALUATION.md).
pub const SEARCH_INSTRUCTION: &str =
    "Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery: ";

/// The closed category set. `기타` has no prototypes: it is the fallback for
/// a best score below [`MIN_SCORE`].
pub const CATEGORIES: [&str; 11] = [
    "학사",
    "수업",
    "장학",
    "연구",
    "학과 소식",
    "공모전·대회",
    "인턴·채용",
    "교육·프로그램",
    "행사·활동",
    "생활·시설",
    "기타",
];

/// Authored Korean prototypes, deliberately separate from the evaluation
/// fixtures in `evaluation.json`. The first entry of each category names its
/// scope; the rest are typical notice titles. 벨라르미노 학사 is the dormitory,
/// so its notices need their own 생활·시설 prototypes to stay out of 학사.
pub const CATEGORY_PROTOTYPES: [(&str, &[&str]); 10] = [
    ("학사", &[
        "학사 행정 안내: 휴학, 복학, 재입학, 졸업, 학적, 전공 신청, 등록금 납부, 학위 수여",
        "2026학년도 2학기 휴학 및 복학 신청 안내",
        "졸업예정 처리와 조기졸업 신청 안내",
        "전공 신청 및 변경, 다전공과 마이크로전공 신청 안내",
        "학부 등록금 납부 기간과 분할 납부 안내",
        "학칙 개정 공고와 시행세칙 변경 안내",
        "신입생 학번 안내와 학생증 발급 안내",
        "성적증명서와 졸업증명서 발급 일정 안내",
        "대학원 신입생 모집 요강과 입학 설명회 안내",
        "학과 요람 변경과 졸업 요건 안내",
    ]),
    ("수업", &[
        "수업과 수강신청 안내: 개설 교과목, 수강정원, 폐강, 휴강, 보강, 시험, 성적, 강의평가",
        "2026학년도 1학기 수강신청 일정과 개설 교과목 안내",
        "중간고사 일정과 시험 장소 안내",
        "이번 주 수업 휴강 및 보강 안내",
        "과제 제출 마감일과 실습 채점 결과 안내",
        "출석과 좌석표, 강의실 변경 안내",
        "하계 계절수업 개설 과목과 타 대학 학점교류 수강 안내",
        "강의평가 실시 안내",
        "강의계획서와 수업 운영 방식 안내",
        "교과목 증원과 강의 언어 변경 안내",
    ]),
    ("장학", &[
        "장학금 안내: 교내외 장학금, 국가장학금, 장학생 선발, 학자금 대출, 생활비 지원",
        "2026년 장학생 모집 공고와 지원 자격",
        "기업 장학생 선발 안내",
        "성적우수 장학금 선발 결과 안내",
        "학자금 대출 신청 안내",
        "재단 장학생 모집과 제출 서류 안내",
    ]),
    ("연구", &[
        "연구 참여 안내: 학부 연구생 모집, 연구실 인턴, 연구 과제, 연구비 지원 사업",
        "학부생 연구 참여 프로그램 모집",
        "연구실 안전교육 이수 안내",
        "연구 지원 사업 공모와 연구비 신청 안내",
        "박사학위 논문심사 공개 발표 안내",
        "학술대회 논문 모집과 투고 안내",
    ]),
    ("학과 소식", &[
        "학과 소식: 교수 연구팀 논문 채택, 수상 소식, 졸업생 임용, 언론 보도, 소식지",
        "교수 연구팀 최우수 국제학술대회 정규 논문 채택",
        "우리 학과 학생 논문상 수상을 축하합니다",
        "연구실 졸업생 대학 교수 임용 소식",
        "학과 언론 보도 기사",
        "대학 소식지 발간",
    ]),
    ("공모전·대회", &[
        "공모전과 경진대회 참가자 모집: 아이디어 공모, 영상 공모전, 데이터 분석 대회, 해커톤",
        "AI 영상 콘텐츠 공모전 참가 안내",
        "공공데이터 활용 아이디어 경진대회",
        "해커톤 참가팀 모집",
        "프로그래밍 대회와 해킹방어대회 참가 신청",
        "창업 경진대회 참가 모집",
    ]),
    ("인턴·채용", &[
        "채용과 인턴 모집 공고: 신입사원 공개채용, 채용연계형 인턴, 체험형 인턴, 현장실습",
        "2026년 하반기 신입사원 공개채용 안내",
        "채용연계형 인턴십 모집",
        "기업 채용설명회와 잡페어 안내",
        "학과사무실 근로학생과 조교 모집",
        "교과목 실습 멘토 모집",
        "전문연구요원 채용 공고",
        "교수 초빙 공고",
    ]),
    ("교육·프로그램", &[
        "외부 교육 프로그램 교육생 모집: 부트캠프, 아카데미, 국비지원 교육과정, 인재 양성 과정",
        "K-디지털 트레이닝 국비지원 부트캠프 교육생 모집",
        "AI 개발자 양성과정 수강생 모집, 교육비 전액 무료",
        "기업 아카데미 기수 교육생 선발",
        "자격증 취득 과정과 자격 시험 접수 안내",
        "해외 대학 AI 연수 프로그램 참가자 모집",
        "필수 법정 교육 이수 안내",
        "청년취업사관학교 캠퍼스 교육생 모집",
        "보안 아카데미 직무 트랙 교육생 모집",
    ]),
    ("행사·활동", &[
        "행사와 활동 안내: 세미나, 콜로퀴엄, 특강, 컨퍼런스, 포럼, 설명회, 동아리, 대외활동",
        "학과 콜로퀴엄 초청 강연 안내",
        "기술 세미나와 컨퍼런스 사전 등록 안내",
        "포럼 개최 안내",
        "동아리 신입 회원 모집",
        "대학생 대외활동과 서포터즈 모집",
        "축제와 입학식, 오리엔테이션 행사 안내",
        "기관 견학 투어 참가자 모집",
        "기술 엑스포와 서밋, 네트워킹 행사 참가 안내",
    ]),
    ("생활·시설", &[
        "생활과 시설 안내: 기숙사(벨라르미노 학사) 생활, 식당 식단, 시설 공사, 건물과 공간 이용, 정전, 서버 점검",
        "벨라르미노 학사 기숙사 사생 모집과 입사 안내",
        "기숙사 입사자 발표와 기숙사비 납부 안내",
        "주간 식단표",
        "아침 점심 저녁 메뉴: 밥, 국, 반찬, 김치",
        "건물 보수공사로 인한 시설 이용 제한 안내",
        "실습실 사용 불가와 새 공간 조성 안내",
        "기숙사 택배 수령과 생활 수칙, 운영 시간 안내",
    ]),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Classified {
    pub category: &'static str,
    pub score: f32,
    pub margin: f32,
    pub low_confidence: bool,
}

/// Stable identity for the prototype set, scoring, threshold and input shape.
pub fn config_hash() -> &'static str {
    static HASH: LazyLock<String> = LazyLock::new(|| {
        let mut hasher = Sha256::new();
        hasher.update(CONFIG_VERSION);
        hasher.update(INSTRUCTION);
        hasher.update(BODY_CHARS.to_le_bytes());
        hasher.update(MIN_SCORE.to_le_bytes());
        for category in CATEGORIES {
            hasher.update(category);
        }
        for (category, prototypes) in CATEGORY_PROTOTYPES {
            hasher.update(category);
            for prototype in prototypes {
                hasher.update([0]);
                hasher.update(prototype);
            }
        }
        format!("{:x}", hasher.finalize())
    });
    &HASH
}

/// Bounded classifier input: instruction, title, then the first 1000 body
/// characters, capped so one long notice cannot exceed the embedding batch.
pub fn input(title: &str, body: &str) -> String {
    let body = body.chars().take(BODY_CHARS).collect::<String>();
    let text = format!("{INSTRUCTION}{title}\n{body}");
    if text.len() <= MAX_INPUT_BYTES {
        return text;
    }
    let mut end = MAX_INPUT_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Each prototype with its category, embedded like a notice: without the
/// shared instruction prefix every notice scored in a narrow band and the
/// nearest prototype was close to arbitrary.
pub fn prototype_inputs() -> Vec<(&'static str, String)> {
    CATEGORY_PROTOTYPES
        .iter()
        .flat_map(|(category, entries)| {
            entries
                .iter()
                .map(move |entry| (*category, format!("{INSTRUCTION}{entry}")))
        })
        .collect()
}

/// Classifies against already-embedded prototypes. Both sides are unit length,
/// so the dot product is cosine similarity.
pub fn classify_with(vector: &[f32], prototypes: &[(&'static str, Vec<f32>)]) -> Classified {
    let mut best = ("기타", f32::NEG_INFINITY);
    let mut second = f32::NEG_INFINITY;
    for (category, prototype) in prototypes {
        let score = cosine(vector, prototype);
        if score > best.1 {
            second = best.1;
            best = (category, score);
        } else if score > second {
            second = score;
        }
    }
    let low_confidence = best.1 < MIN_SCORE || !best.1.is_finite();
    Classified {
        category: if low_confidence { "기타" } else { best.0 },
        score: if best.1.is_finite() { best.1 } else { -1.0 },
        margin: if best.1.is_finite() && second.is_finite() {
            best.1 - second
        } else {
            0.0
        },
        low_confidence,
    }
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return f32::NEG_INFINITY;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_but_other_has_prototypes_embedded_like_notices() {
        let inputs = prototype_inputs();
        for category in CATEGORIES {
            let count = inputs
                .iter()
                .filter(|(owner, _)| *owner == category)
                .count();
            assert_eq!(count == 0, category == "기타", "{category}: {count}");
        }
        assert!(inputs.iter().all(|(owner, _)| CATEGORIES.contains(owner)));
        assert!(inputs
            .iter()
            .all(|(_, text)| text.starts_with(INSTRUCTION) && text.len() > INSTRUCTION.len()));
    }

    #[test]
    fn config_hash_is_stable_and_hex() {
        let hash = config_hash();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(hash, config_hash());
    }

    #[test]
    fn input_is_bounded_and_keeps_the_title() {
        let body = "가".repeat(50_000);
        let text = input("제목", &body);
        assert!(text.starts_with(INSTRUCTION));
        assert!(text.len() <= MAX_INPUT_BYTES);
        assert!(text.contains("제목"));
        assert!(text.chars().count() < 4_000);
    }

    #[test]
    fn threshold_falls_back_to_other_and_keeps_the_score() {
        let prototypes = vec![("장학", vec![0.0, 1.0]), ("학사", vec![1.0, 0.0])];
        let near = classify_with(&[1.0, 0.0], &prototypes);
        assert_eq!(near.category, "학사");
        assert!(!near.low_confidence);
        assert!(near.score > 0.9);
        // Orthogonal to every prototype: below the threshold.
        let far = classify_with(&[0.0, 0.0], &prototypes);
        assert_eq!(far.category, "기타");
        assert!(far.low_confidence);
    }

    #[test]
    fn margin_is_the_gap_to_the_second_category() {
        let prototypes = vec![("장학", vec![1.0, 0.0]), ("학사", vec![0.0, 1.0])];
        let result = classify_with(&[0.8, 0.2], &prototypes);
        assert_eq!(result.category, "장학");
        assert!((result.margin - 0.6).abs() < 1e-6, "{result:?}");
    }
}
