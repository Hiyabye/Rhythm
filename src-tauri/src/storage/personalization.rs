use super::Store;
use crate::archive::model::DocumentItem;
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

const EXPLICIT_INTEREST_LIMIT: i64 = 50;
const LEARNED_INTEREST_LIMIT: i64 = 50;
const RECENT_CANDIDATE_LIMIT: i64 = 500;
const RECENT_COLLECTION_LIMIT: i64 = 250;
const SAVED_CANDIDATE_LIMIT: i64 = 100;
const FEEDBACK_CANDIDATE_LIMIT: i64 = 100;
const FTS_CANDIDATE_LIMIT: i64 = 250;
const SOURCE_CANDIDATE_LIMIT: i64 = 250;
const EVIDENCE_RETENTION_DAYS: i64 = 180;
const MAX_EVIDENCE_TOPICS_PER_ACTION: usize = 32;
const MAX_MATCH_TERMS: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizationProfile {
    pub university: String,
    pub department: String,
    pub study_level: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalInterest {
    pub id: i64,
    pub name: String,
    pub aliases: Vec<String>,
    pub priority: u8,
    pub polarity: String,
    pub origin: String,
    pub evidence_count: u32,
    pub evidence_summary: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizationSnapshot {
    pub profile: PersonalizationProfile,
    pub interests: Vec<PersonalInterest>,
    pub learning_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizedRecommendation {
    pub item: DocumentItem,
    pub reasons: Vec<String>,
    pub feedback: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizationRecommendations {
    pub items: Vec<PersonalizedRecommendation>,
    pub has_preferences: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalInterestInput {
    pub id: Option<i64>,
    pub name: String,
    pub aliases: Vec<String>,
    pub priority: u8,
    #[serde(default = "positive_polarity")]
    pub polarity: String,
}

fn folded(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn like_contains(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len() + 2);
    pattern.push('%');
    for character in folded(value).chars() {
        if matches!(character, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}
fn positive_polarity() -> String {
    "positive".into()
}

fn canonical_topic(value: &str) -> String {
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

fn display_topic(topic: &str) -> (&str, &'static [&'static str]) {
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

fn interest_key(value: &str) -> String {
    canonical_topic(value)
}

fn known_aliases(value: &str) -> &'static [&'static str] {
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

const TOPIC_STOPWORDS: &[&str] = &[
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

const SENSITIVE_TOPIC_MARKERS: &[&str] = &[
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

fn is_sensitive_topic(topic: &str) -> bool {
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

pub(crate) fn next_activity_sequence(tx: &Transaction<'_>) -> Result<i64, String> {
    tx.execute(
        "UPDATE personalization_activity SET sequence=sequence+1 WHERE id=1",
        [],
    )
    .map_err(|e| e.to_string())?;
    tx.query_row(
        "SELECT sequence FROM personalization_activity WHERE id=1",
        [],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

fn insert_evidence(
    tx: &Transaction<'_>,
    document_id: i64,
    kind: &str,
    sequence: i64,
    title: &str,
    category: Option<&str>,
) -> Result<(), String> {
    for topic in topics(title, category) {
        tx.execute(
            "INSERT OR IGNORE INTO personalization_evidence(topic,document_id,kind,activity_sequence)
             VALUES(?1,?2,?3,?4)",
            params![topic, document_id, kind, sequence],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn matches_term(document: &str, term: &str) -> bool {
    matches_folded(&folded(document), term)
}

fn matches_folded(document: &str, term: &str) -> bool {
    let term = folded(term);
    if term.is_empty() || !document.contains(&term) {
        return false;
    }
    if !term
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
    {
        return true;
    }
    document.match_indices(&term).any(|(index, _)| {
        let before = document[..index].chars().next_back();
        let after = document[index + term.len()..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

fn date_days(date: &str) -> Option<i64> {
    let date = date.get(..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let day = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs()
        / 86_400;
    Some(now as i64 - days_from_civil(year, month, day))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

struct Candidate {
    item: DocumentItem,
    feedback: Option<String>,
    feedback_updated_at: Option<String>,
    source_subscribed: bool,
    source_organization: String,
    last_revision_at: Option<String>,
    full_text_matches: u32,
}

impl Store {
    pub fn personalization_snapshot(&self) -> Result<PersonalizationSnapshot, String> {
        self.recompute_learned()?;
        let (profile, learning_enabled) = self
            .connection
            .query_row(
                "SELECT university,department,study_level,learning_enabled
                 FROM personalization_profile WHERE id=1",
                [],
                |row| {
                    Ok((
                        PersonalizationProfile {
                            university: row.get(0)?,
                            department: row.get(1)?,
                            study_level: row.get(2)?,
                        },
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let mut statement = self.connection.prepare(
            "SELECT id,name,aliases_json,priority,polarity,origin,evidence_count,evidence_summary,updated_at
             FROM personalization_interests
             ORDER BY CASE origin WHEN 'explicit' THEN 0 ELSE 1 END,priority DESC,normalized",
        ).map_err(|e| e.to_string())?;
        let interests = statement
            .query_map([], |row| {
                let aliases: String = row.get(2)?;
                Ok(PersonalInterest {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    aliases: serde_json::from_str(&aliases).unwrap_or_default(),
                    priority: row.get::<_, i64>(3)? as u8,
                    polarity: row.get(4)?,
                    origin: row.get(5)?,
                    evidence_count: row.get::<_, i64>(6)? as u32,
                    evidence_summary: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(PersonalizationSnapshot {
            profile,
            interests,
            learning_enabled,
        })
    }

    pub fn personalization_save_profile(
        &self,
        profile: &PersonalizationProfile,
    ) -> Result<PersonalizationSnapshot, String> {
        let clean = |value: &str| -> Result<String, String> {
            let value = value.trim();
            if value.chars().count() > 120 {
                Err("프로필 항목은 120자 이내여야 합니다.".into())
            } else {
                Ok(value.into())
            }
        };
        self.connection
            .execute(
                "UPDATE personalization_profile SET university=?1,department=?2,study_level=?3 WHERE id=1",
                params![clean(&profile.university)?, clean(&profile.department)?, clean(&profile.study_level)?],
            )
            .map_err(|e| e.to_string())?;
        self.personalization_snapshot()
    }

    pub fn personalization_save_interest(
        &self,
        input: &PersonalInterestInput,
    ) -> Result<PersonalizationSnapshot, String> {
        let name = input.name.trim();
        if name.is_empty()
            || name.chars().count() > 80
            || !(1..=5).contains(&input.priority)
            || !["positive", "negative"].contains(&input.polarity.as_str())
            || input.aliases.len() > 12
        {
            return Err("관심 분야를 확인해 주세요.".into());
        }
        let key = interest_key(name);
        let mut aliases = Vec::new();
        for alias in input
            .aliases
            .iter()
            .map(String::as_str)
            .chain(known_aliases(name).iter().copied())
        {
            let alias = alias.trim();
            if alias.is_empty() || alias.chars().count() > 80 {
                return Err("관심 분야 별칭을 확인해 주세요.".into());
            }
            let alias_folded = folded(alias);
            if alias_folded != folded(name)
                && !aliases
                    .iter()
                    .any(|stored: &String| folded(stored) == alias_folded)
            {
                aliases.push(alias.to_string());
            }
        }
        let alias_json = serde_json::to_string(&aliases).map_err(|e| e.to_string())?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let explicit_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM personalization_interests WHERE origin='explicit'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let wanted: HashSet<String> = std::iter::once(key.clone())
            .chain(aliases.iter().map(|alias| interest_key(alias)))
            .collect();
        let mut statement = tx
            .prepare("SELECT id,name,aliases_json FROM personalization_interests")
            .map_err(|e| e.to_string())?;
        let stored = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        let matches = stored
            .into_iter()
            .filter_map(|(id, stored_name, raw_aliases)| {
                let same = std::iter::once(stored_name)
                    .chain(serde_json::from_str::<Vec<String>>(&raw_aliases).unwrap_or_default())
                    .any(|term| wanted.contains(&interest_key(&term)));
                same.then_some(id)
            })
            .collect::<Vec<_>>();
        if input.id.is_none() && matches.len() > 1 {
            return Err("별칭이 겹치는 관심 분야를 먼저 정리해 주세요.".into());
        }
        let existing_id = match input.id {
            Some(id) if !matches.contains(&id) => {
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM personalization_interests WHERE id=?1)",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !exists {
                    return Err("수정할 관심 분야를 찾을 수 없습니다.".into());
                }
                if !matches.is_empty() {
                    return Err("이미 등록된 관심 분야와 겹치는 별칭이 있습니다.".into());
                }
                Some(id)
            }
            Some(id) => {
                if matches.iter().any(|matched| *matched != id) {
                    return Err("이미 등록된 관심 분야와 겹치는 별칭이 있습니다.".into());
                }
                Some(id)
            }
            None => matches.first().copied(),
        };
        let is_new_explicit = match existing_id {
            Some(id) => tx
                .query_row(
                    "SELECT origin<>'explicit' FROM personalization_interests WHERE id=?1",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?,
            None => true,
        };
        if is_new_explicit && explicit_count >= EXPLICIT_INTEREST_LIMIT {
            return Err("직접 설정할 수 있는 관심 분야는 최대 50개입니다.".into());
        }
        let mut renamed_topic = None;
        if let Some(id) = existing_id {
            let old_topic: String = tx
                .query_row(
                    "SELECT normalized FROM personalization_interests WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if old_topic != key {
                let sequence: i64 = tx
                    .query_row(
                        "SELECT sequence FROM personalization_activity WHERE id=1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                renamed_topic = Some(old_topic.clone());
                tx.execute(
                    "INSERT INTO personalization_suppressed(topic,baseline_sequence) VALUES(?1,?2)
                     ON CONFLICT(topic) DO UPDATE SET baseline_sequence=excluded.baseline_sequence",
                    params![old_topic, sequence],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute(
                "UPDATE personalization_interests SET name=?2,normalized=?3,aliases_json=?4,priority=?5,polarity=?6,
                 origin='explicit',evidence_count=0,evidence_summary=NULL,
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",
                params![id, name, key, alias_json, input.priority, input.polarity],
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute(
                "INSERT INTO personalization_interests(name,normalized,aliases_json,priority,polarity,origin)
                 VALUES(?1,?2,?3,?4,?5,'explicit')",
                params![name, key, alias_json, input.priority, input.polarity],
            )
            .map_err(|e| e.to_string())?;
        }
        for topic in wanted {
            if renamed_topic.as_deref() == Some(topic.as_str()) {
                continue;
            }
            tx.execute(
                "DELETE FROM personalization_suppressed WHERE topic=?1",
                [topic],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.personalization_snapshot()
    }

    pub fn personalization_remove_interest(
        &self,
        id: i64,
    ) -> Result<PersonalizationSnapshot, String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let interest: Option<(String, String)> = tx
            .query_row(
                "SELECT normalized,aliases_json FROM personalization_interests WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((topic, aliases_json)) = interest {
            let sequence: i64 = tx
                .query_row(
                    "SELECT sequence FROM personalization_activity WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let mut topics = HashSet::from([topic]);
            topics.extend(
                serde_json::from_str::<Vec<String>>(&aliases_json)
                    .unwrap_or_default()
                    .iter()
                    .map(|alias| interest_key(alias))
                    .filter(|topic| !topic.is_empty()),
            );
            for topic in topics {
                tx.execute(
                    "INSERT INTO personalization_suppressed(topic,baseline_sequence) VALUES(?1,?2)
                     ON CONFLICT(topic) DO UPDATE SET baseline_sequence=MAX(baseline_sequence,excluded.baseline_sequence)",
                    params![topic, sequence],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute("DELETE FROM personalization_interests WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.personalization_snapshot()
    }

    pub fn personalization_set_learning(
        &self,
        enabled: bool,
    ) -> Result<PersonalizationSnapshot, String> {
        self.connection
            .execute(
                "UPDATE personalization_profile SET learning_enabled=?1 WHERE id=1",
                [enabled],
            )
            .map_err(|e| e.to_string())?;
        self.personalization_snapshot()
    }

    // Reset personalization only. The archive, saved/read state, resources, and AI results remain.
    pub fn personalization_reset(&self) -> Result<PersonalizationSnapshot, String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE personalization_profile SET university='',department='',study_level='',learning_enabled=1 WHERE id=1",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("UPDATE settings SET interests='' WHERE id=1", [])
            .map_err(|e| e.to_string())?;
        for table in [
            "personalization_interests",
            "personalization_feedback",
            "personalization_evidence",
            "personalization_suppressed",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])
                .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE personalization_activity SET sequence=0,recomputed_sequence=0,last_recomputed_at=NULL WHERE id=1",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.personalization_snapshot()
    }

    pub fn personalization_feedback(&self, id: i64) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM personalization_feedback WHERE document_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn personalization_set_feedback(
        &self,
        id: i64,
        value: Option<&str>,
    ) -> Result<Option<String>, String> {
        if value.is_some_and(|value| !matches!(value, "interested" | "not_interested")) {
            return Err("피드백 값을 확인해 주세요.".into());
        }
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM documents WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("보관된 항목을 찾을 수 없습니다.".into());
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT value FROM personalization_feedback WHERE document_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if previous.as_deref() == value {
            return Ok(previous);
        }
        let sequence = next_activity_sequence(&tx)?;
        match value {
            Some("interested") => {
                tx.execute(
                    "INSERT INTO personalization_feedback(document_id,value) VALUES(?1,'interested')
                     ON CONFLICT(document_id) DO UPDATE SET value='interested',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                    [id],
                )
                .map_err(|e| e.to_string())?;
                let learning_enabled: bool = tx
                    .query_row(
                        "SELECT learning_enabled FROM personalization_profile WHERE id=1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if learning_enabled {
                    let (title, category): (String, Option<String>) = tx
                        .query_row(
                            "SELECT title,category FROM documents WHERE id=?1",
                            [id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .map_err(|e| e.to_string())?;
                    insert_evidence(&tx, id, "feedback", sequence, &title, category.as_deref())?;
                }
            }
            Some("not_interested") => {
                tx.execute(
                    "INSERT INTO personalization_feedback(document_id,value) VALUES(?1,'not_interested')
                     ON CONFLICT(document_id) DO UPDATE SET value='not_interested',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
                    [id],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "DELETE FROM personalization_evidence WHERE document_id=?1 AND kind='feedback'",
                    [id],
                )
                .map_err(|e| e.to_string())?;
            }
            None => {
                tx.execute(
                    "DELETE FROM personalization_feedback WHERE document_id=?1",
                    [id],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "DELETE FROM personalization_evidence WHERE document_id=?1 AND kind='feedback'",
                    [id],
                )
                .map_err(|e| e.to_string())?;
            }
            _ => unreachable!(),
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.recompute_learned()?;
        self.personalization_feedback(id)
    }

    pub(crate) fn recompute_learned(&self) -> Result<(), String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let recompute_due: bool = tx
            .query_row(
                "SELECT sequence<>recomputed_sequence OR last_recomputed_at IS NULL
                    OR last_recomputed_at<strftime('%Y-%m-%dT%H:%M:%fZ','now','-1 day')
                 FROM personalization_activity WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !recompute_due {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(());
        }
        tx.execute(
            "DELETE FROM personalization_evidence
             WHERE created_at<strftime('%Y-%m-%dT%H:%M:%fZ','now','-'||?1||' days')",
            [EVIDENCE_RETENTION_DAYS],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM personalization_suppressed
             WHERE NOT EXISTS(
                 SELECT 1 FROM personalization_evidence e
                 WHERE e.topic=personalization_suppressed.topic
             )",
            [],
        )
        .map_err(|e| e.to_string())?;
        let explicit_aliases = {
            let mut statement = tx
                .prepare(
                    "SELECT aliases_json FROM personalization_interests WHERE origin='explicit'",
                )
                .map_err(|e| e.to_string())?;
            let stored = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            stored
                .into_iter()
                .flat_map(|raw| serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default())
                .map(|alias| interest_key(&alias))
                .collect::<HashSet<_>>()
        };
        let mut statement = tx
            .prepare(
                "SELECT e.topic,COUNT(DISTINCT e.document_id),MAX(e.activity_sequence)
                 FROM personalization_evidence e
                 JOIN document_state s ON s.document_id=e.document_id
                 LEFT JOIN personalization_feedback f ON f.document_id=e.document_id
                 WHERE e.created_at>=strftime('%Y-%m-%dT%H:%M:%fZ','now','-'||?1||' days')
                   AND ((e.kind='feedback' AND f.value='interested')
                    OR (e.kind='saved' AND s.saved=1))
                   AND NOT EXISTS(
                       SELECT 1 FROM personalization_interests p
                       WHERE p.normalized=e.topic AND p.origin='explicit'
                   )
                 GROUP BY e.topic
                 HAVING COUNT(DISTINCT e.document_id)>=2
                    AND MAX(e.activity_sequence)>COALESCE(
                        (SELECT baseline_sequence FROM personalization_suppressed WHERE topic=e.topic),0
                    )
                 ORDER BY COUNT(DISTINCT e.document_id) DESC,MAX(e.activity_sequence) DESC,e.topic
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(
                params![EVIDENCE_RETENTION_DAYS, LEARNED_INTEREST_LIMIT],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        let mut active = HashSet::new();
        for (topic, count, latest_sequence) in rows {
            // An explicit alias already represents this topic; do not create a duplicate learned row.
            if explicit_aliases.contains(&topic) {
                continue;
            }
            let baseline: Option<i64> = tx
                .query_row(
                    "SELECT baseline_sequence FROM personalization_suppressed WHERE topic=?1",
                    [&topic],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let suppressed = baseline.is_some_and(|sequence| latest_sequence <= sequence);
            if suppressed {
                continue;
            }
            if baseline.is_some() {
                tx.execute(
                    "DELETE FROM personalization_suppressed WHERE topic=?1",
                    [&topic],
                )
                .map_err(|e| e.to_string())?;
            }
            let explicit: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM personalization_interests WHERE normalized=?1 AND origin='explicit')",
                    [&topic],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if explicit {
                continue;
            }
            let (name, aliases) = display_topic(&topic);
            let aliases_json = serde_json::to_string(aliases).map_err(|e| e.to_string())?;
            let summary =
                format!("서로 다른 {count}개 공지에서 저장하거나 관심 표시한 행동을 근거로 제안");
            tx.execute(
                "INSERT INTO personalization_interests(name,normalized,aliases_json,priority,origin,evidence_count,evidence_summary)
                 VALUES(?1,?2,?3,1,'learned',?4,?5)
                 ON CONFLICT(normalized) DO UPDATE SET evidence_count=excluded.evidence_count,
                 evidence_summary=excluded.evidence_summary,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE personalization_interests.origin='learned'
                   AND (personalization_interests.evidence_count<>excluded.evidence_count
                        OR personalization_interests.evidence_summary<>excluded.evidence_summary)",
                params![name, topic, aliases_json, count, summary],
            )
            .map_err(|e| e.to_string())?;
            active.insert(topic);
        }
        let mut existing = tx
            .prepare("SELECT normalized FROM personalization_interests WHERE origin='learned'")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        existing.retain(|topic| !active.contains(topic));
        for topic in existing {
            tx.execute(
                "DELETE FROM personalization_interests WHERE origin='learned' AND normalized=?1",
                [topic],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "UPDATE personalization_activity
             SET recomputed_sequence=sequence,last_recomputed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id=1",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn personalization_recommendations(
        &self,
        limit: u32,
    ) -> Result<PersonalizationRecommendations, String> {
        let limit = limit.clamp(1, 100) as usize;
        let snapshot = self.personalization_snapshot()?;
        let interaction_or_subscription: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM document_state WHERE saved=1)
                    OR EXISTS(SELECT 1 FROM personalization_feedback)
                    OR EXISTS(
                        SELECT 1 FROM sources s WHERE s.subscribed=1 AND (
                            NOT EXISTS(SELECT 1 FROM source_extensions se WHERE se.source_id=s.id)
                            OR EXISTS(
                                SELECT 1 FROM source_extensions se
                                JOIN extension_preferences ep ON ep.extension_id=se.extension_id
                                WHERE se.source_id=s.id AND ep.enabled=1
                            )
                        )
                    )",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let has_preferences = interaction_or_subscription
            || !snapshot.profile.university.is_empty()
            || !snapshot.profile.department.is_empty()
            || !snapshot.profile.study_level.is_empty()
            || !snapshot.interests.is_empty();
        let mut terms = Vec::new();
        let mut seen_terms = HashSet::new();
        for interest in &snapshot.interests {
            for term in std::iter::once(&interest.name).chain(interest.aliases.iter()) {
                let key = folded(term);
                if !key.is_empty() && seen_terms.insert(key) {
                    terms.push(term.clone());
                }
                if terms.len() >= MAX_MATCH_TERMS {
                    break;
                }
            }
            if terms.len() >= MAX_MATCH_TERMS {
                break;
            }
        }
        for term in [
            &snapshot.profile.department,
            &snapshot.profile.university,
            &snapshot.profile.study_level,
        ] {
            if !term.is_empty() && seen_terms.insert(folded(term)) && terms.len() < MAX_MATCH_TERMS
            {
                terms.push(term.clone());
            }
        }
        let mut source_terms = Vec::new();
        let mut seen_source_terms = HashSet::new();
        for interest in &snapshot.interests {
            for term in std::iter::once(&interest.name).chain(interest.aliases.iter()) {
                if seen_source_terms.insert(folded(term)) {
                    source_terms.push(term.clone());
                }
            }
        }
        for term in [
            &snapshot.profile.department,
            &snapshot.profile.university,
            &snapshot.profile.study_level,
        ] {
            if !term.is_empty() && seen_source_terms.insert(folded(term)) {
                source_terms.push(term.clone());
            }
        }
        let source_rows = self
            .connection
            .prepare("SELECT id,name,organization FROM sources ORDER BY id")
            .map_err(|e| e.to_string())?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let source_ids = source_rows
            .into_iter()
            .filter_map(|(id, name, organization)| {
                let text = format!("{name} {organization}");
                source_terms
                    .iter()
                    .any(|term| matches_term(&text, term))
                    .then_some(id)
            })
            .collect::<Vec<_>>();
        let source_union = if source_ids.is_empty() {
            String::new()
        } else {
            let ids = source_ids
                .iter()
                .map(|id| format!("'{}'", id.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "UNION SELECT id FROM (
                    SELECT d.id FROM documents d JOIN active_sources a ON a.source_id=d.source_id
                    WHERE d.source_id IN ({ids})
                    ORDER BY d.published_at DESC,d.id DESC LIMIT {SOURCE_CANDIDATE_LIMIT}
                )"
            )
        };
        let updated_union = format!(
            "UNION SELECT id FROM (
                SELECT d.id FROM documents d JOIN active_sources a ON a.source_id=d.source_id
                WHERE d.last_revision_at IS NOT NULL
                ORDER BY d.last_revision_at DESC,d.id DESC LIMIT {RECENT_COLLECTION_LIMIT}
            )"
        );
        let category_union = if has_preferences
            && (!snapshot.interests.is_empty()
                || !snapshot.profile.university.is_empty()
                || !snapshot.profile.department.is_empty()
                || !snapshot.profile.study_level.is_empty())
        {
            "UNION SELECT id FROM (
                SELECT d.id FROM documents d JOIN active_sources a ON a.source_id=d.source_id
                WHERE d.category COLLATE NOCASE IN (
                    SELECT name FROM personalization_interests
                    UNION SELECT j.value FROM personalization_interests p,json_each(p.aliases_json) j
                    UNION SELECT university FROM personalization_profile WHERE university<>''
                    UNION SELECT department FROM personalization_profile WHERE department<>''
                    UNION SELECT study_level FROM personalization_profile WHERE study_level<>''
                )
                ORDER BY d.published_at DESC,d.id DESC LIMIT 250
            )"
                .to_string()
        } else {
            String::new()
        };
        let mut fts_terms = Vec::new();
        let mut term_parameters = Vec::with_capacity(terms.len());
        for term in &terms {
            if term.chars().count() >= 3 {
                let query = format!("\"{}\"", term.replace('"', "\"\""));
                fts_terms.push(query.clone());
                term_parameters.push(query);
            } else {
                term_parameters.push(like_contains(term));
            }
        }
        let fts_query = fts_terms.join(" OR ");
        let fts_union = if fts_query.is_empty() {
            String::new()
        } else {
            format!(
                "UNION SELECT id FROM (
                    SELECT d.id FROM documents_fts
                    JOIN documents d ON d.id=documents_fts.rowid
                    JOIN active_sources a ON a.source_id=d.source_id
                    WHERE documents_fts MATCH ?1
                    ORDER BY d.published_at DESC,d.id DESC LIMIT {FTS_CANDIDATE_LIMIT}
                )"
            )
        };
        let short_conditions = terms
            .iter()
            .enumerate()
            .filter(|(_, term)| term.chars().count() < 3)
            .map(|(index, _)| {
                let parameter = index + 2;
                format!(
                    "(documents_fts.title LIKE ?{parameter} ESCAPE '\\' \
                     OR documents_fts.body_text LIKE ?{parameter} ESCAPE '\\')"
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let short_union = if short_conditions.is_empty() {
            String::new()
        } else {
            format!(
                "UNION SELECT id FROM (
                    SELECT d.id FROM documents_fts
                    JOIN documents d ON d.id=documents_fts.rowid
                    JOIN active_sources a ON a.source_id=d.source_id
                    WHERE {short_conditions}
                    ORDER BY d.published_at DESC,d.id DESC LIMIT {FTS_CANDIDATE_LIMIT}
                )"
            )
        };
        let matched_terms_sql = terms
            .iter()
            .enumerate()
            .map(|(index, term)| {
                let parameter = index + 2;
                if term.chars().count() >= 3 {
                    format!(
                        ",EXISTS(SELECT 1 FROM documents_fts WHERE documents_fts.rowid=d.id \
                         AND documents_fts MATCH ?{parameter})"
                    )
                } else {
                    format!(
                        ",EXISTS(SELECT 1 FROM documents_fts WHERE documents_fts.rowid=d.id \
                         AND (documents_fts.title LIKE ?{parameter} ESCAPE '\\' \
                              OR documents_fts.body_text LIKE ?{parameter} ESCAPE '\\'))"
                    )
                }
            })
            .collect::<String>();
        let sql = format!(
            "WITH active_sources(source_id) AS (
                SELECT s.id FROM sources s WHERE
                    NOT EXISTS(SELECT 1 FROM source_extensions se WHERE se.source_id=s.id)
                    OR EXISTS(
                        SELECT 1 FROM source_extensions se
                        JOIN extension_preferences ep ON ep.extension_id=se.extension_id
                        WHERE se.source_id=s.id AND ep.enabled=1
                    )
             ), candidates(id) AS (
                SELECT id FROM (
                    SELECT d.id FROM documents d JOIN active_sources a ON a.source_id=d.source_id
                    ORDER BY d.published_at DESC,d.id DESC LIMIT {RECENT_CANDIDATE_LIMIT}
                )
                UNION SELECT id FROM (
                    SELECT d.id FROM documents d JOIN active_sources a ON a.source_id=d.source_id
                    WHERE d.historical=0 ORDER BY d.collected_at DESC,d.id DESC LIMIT {RECENT_COLLECTION_LIMIT}
                )
                {updated_union}
                UNION SELECT document_id FROM (
                    SELECT u.document_id FROM document_state u
                    JOIN documents d ON d.id=u.document_id JOIN active_sources a ON a.source_id=d.source_id
                    WHERE u.saved=1 ORDER BY u.saved_at DESC,u.document_id DESC LIMIT {SAVED_CANDIDATE_LIMIT}
                )
                UNION SELECT document_id FROM (
                    SELECT f.document_id FROM personalization_feedback f
                    JOIN documents d ON d.id=f.document_id JOIN active_sources a ON a.source_id=d.source_id
                    ORDER BY f.updated_at DESC,f.document_id DESC LIMIT {FEEDBACK_CANDIDATE_LIMIT}
                )
                {source_union}
                {category_union}
                {fts_union}
                {short_union}
             )
             SELECT d.id,d.source_id,s.name,d.title,d.author,d.category,d.published_at,d.collected_at,d.historical,
                    COALESCE(u.saved,0),COALESCE(u.is_read,0),substr(d.body_text,1,320),f.value,
                    CASE WHEN EXISTS(SELECT 1 FROM resources r WHERE r.document_id=d.id
                         AND r.revision_id=(SELECT MAX(rv.id) FROM document_revisions rv WHERE rv.document_id=d.id)
                         AND r.status<>'downloaded') THEN 'missing_resources' ELSE 'archived' END,
                    f.updated_at,s.subscribed,s.organization,d.last_revision_at{matched_terms_sql}
             FROM candidates c JOIN documents d ON d.id=c.id JOIN sources s ON s.id=d.source_id
             LEFT JOIN document_state u ON u.document_id=d.id
             LEFT JOIN personalization_feedback f ON f.document_id=d.id
             ORDER BY d.published_at DESC,d.id DESC"
        );
        let term_indices = terms
            .iter()
            .enumerate()
            .map(|(index, term)| (term.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut statement = self.connection.prepare(&sql).map_err(|e| e.to_string())?;
        let map_candidate = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Candidate> {
            let feedback = row.get(12)?;
            let feedback_updated_at = row.get(14)?;
            let mut full_text_matches = 0u32;
            for (index, _) in terms.iter().enumerate() {
                if row.get::<_, bool>(18 + index)? {
                    full_text_matches |= 1u32 << index;
                }
            }
            Ok(Candidate {
                item: DocumentItem {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    source_name: row.get(2)?,
                    title: row.get(3)?,
                    author: row.get(4)?,
                    category: row.get(5)?,
                    published_at: row.get(6)?,
                    collected_at: row.get(7)?,
                    historical: row.get(8)?,
                    saved: row.get(9)?,
                    is_read: row.get(10)?,
                    snippet: row.get(11)?,
                    content_status: row.get(13)?,
                },
                feedback,
                source_subscribed: row.get(15)?,
                source_organization: row.get(16)?,
                last_revision_at: row.get(17)?,
                full_text_matches,
                feedback_updated_at,
            })
        };
        let candidates = if terms.is_empty() {
            statement
                .query_map([], map_candidate)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        } else {
            let query_values = std::iter::once(fts_query.as_str())
                .chain(term_parameters.iter().map(String::as_str));
            statement
                .query_map(rusqlite::params_from_iter(query_values), map_candidate)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut ranked = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let full_text_matches = candidate.full_text_matches;
            let item = candidate.item;
            let feedback = candidate.feedback;
            let feedback_age = candidate.feedback_updated_at.as_deref().and_then(date_days);
            let source_organization = candidate.source_organization;
            let last_revision_age = candidate.last_revision_at.as_deref().and_then(date_days);
            let mut score = 0i32;
            let mut reasons = Vec::new();
            let searchable = format!(
                "{} {} {} {} {} {}",
                item.title,
                item.category.as_deref().unwrap_or_default(),
                item.source_name,
                source_organization,
                item.snippet,
                item.author.as_deref().unwrap_or_default()
            );
            let searchable_folded = folded(&searchable);
            for interest in &snapshot.interests {
                let matched = std::iter::once(interest.name.as_str())
                    .chain(interest.aliases.iter().map(String::as_str))
                    .any(|term| {
                        matches_folded(&searchable_folded, term)
                            || term_indices
                                .get(term)
                                .is_some_and(|index| full_text_matches & (1u32 << *index) != 0)
                    });
                if matched {
                    let amount = if interest.origin == "explicit" {
                        12 + i32::from(interest.priority) * 3
                    } else {
                        (i32::from(interest.priority) * 3).min(6)
                    };
                    if interest.polarity == "negative" {
                        score -= amount;
                        reasons.push(format!("덜 보고 싶은 분야 ‘{}’ 관련 공지", interest.name));
                    } else {
                        score += amount;
                        reasons.push(format!("관심 분야 ‘{}’에 맞는 공지", interest.name));
                    }
                }
            }
            if candidate.source_subscribed {
                score += 2;
                reasons.push("구독 중인 정보원".into());
            }
            let positive_feedback_bonus = match feedback_age {
                Some(0..=30) => 24,
                Some(31..=180) => 12,
                Some(181..=365) => 6,
                _ => 0,
            };
            let negative_feedback_penalty = match feedback_age {
                Some(0..=180) => 40,
                Some(181..=365) => 20,
                _ => 0,
            };
            if feedback.as_deref() == Some("interested") && positive_feedback_bonus > 0 {
                score += positive_feedback_bonus;
                reasons.push("관심 표시한 공지".into());
            }
            if feedback.as_deref() == Some("not_interested") && negative_feedback_penalty > 0 {
                reasons.push("관심 없음 표시한 공지".into());
            }
            if item.saved {
                score += 8;
                reasons.push("저장한 공지".into());
            }
            let published_age = date_days(&item.published_at);
            if !item.historical {
                if let Some(age) = published_age.filter(|age| *age >= 0) {
                    let amount = (30 - age.min(30)) as i32 / 5;
                    score += amount;
                    if amount > 0 {
                        reasons.push("최근 게시".into());
                    }
                } else if published_age.is_none()
                    && date_days(&item.collected_at).is_some_and(|age| (0..=3).contains(&age))
                {
                    score += 2;
                    reasons.push("최근 보관".into());
                }
                if published_age.is_some_and(|age| age > 30)
                    && date_days(&item.collected_at).is_some_and(|age| (0..=3).contains(&age))
                {
                    score += 2;
                    reasons.push("최근 보관된 오래된 공지".into());
                }
            }
            if last_revision_age.is_some_and(|age| (0..=3).contains(&age))
                && (item.historical || published_age.is_none_or(|age| age > 30))
            {
                score += 4;
                reasons.push("최근 내용이 갱신된 공지".into());
            }
            for (value, amount) in [
                (&snapshot.profile.university, 4),
                (&snapshot.profile.department, 8),
                (&snapshot.profile.study_level, 2),
            ] {
                if !value.is_empty()
                    && (matches_folded(&searchable_folded, value)
                        || term_indices
                            .get(value.as_str())
                            .is_some_and(|index| full_text_matches & (1u32 << *index) != 0))
                {
                    score += amount;
                    reasons.push("프로필 정보와 일치".into());
                }
            }
            if feedback.as_deref() == Some("not_interested") && negative_feedback_penalty > 0 {
                // Recent corrections lower this item without permanently hiding it.
                score = score.min(-negative_feedback_penalty);
            }
            if has_preferences && reasons.is_empty() {
                reasons.push("관심사 일치 신호가 없어 게시일 기준으로 보조 정렬".into());
            }
            if !has_preferences {
                reasons.clear();
            }
            ranked.push((score, item, reasons, feedback));
        }
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.published_at.cmp(&a.1.published_at))
                .then_with(|| b.1.id.cmp(&a.1.id))
        });
        let source_limit = limit.div_ceil(2).max(1);
        let mut selected: Vec<(i32, DocumentItem, Vec<String>, Option<String>)> =
            Vec::with_capacity(limit);
        let mut deferred: Vec<(i32, DocumentItem, Vec<String>, Option<String>)> = Vec::new();
        let mut per_source = std::collections::HashMap::<String, usize>::new();
        for (score, item, mut reasons, feedback) in ranked {
            if selected.len() == limit {
                break;
            }
            let count = per_source.get(&item.source_id).copied().unwrap_or_default();
            if count < source_limit {
                if !deferred.is_empty()
                    && selected
                        .iter()
                        .any(|(_, selected_item, _, _)| selected_item.source_id != item.source_id)
                    && has_preferences
                {
                    reasons.push("출처 편중을 줄이기 위해 함께 표시".into());
                }
                *per_source.entry(item.source_id.clone()).or_default() += 1;
                selected.push((score, item, reasons, feedback));
            } else {
                deferred.push((score, item, reasons, feedback));
            }
        }
        for (score, item, mut reasons, feedback) in deferred {
            if selected.len() == limit {
                break;
            }
            if selected
                .iter()
                .any(|(_, selected_item, _, _)| selected_item.source_id != item.source_id)
                && has_preferences
            {
                reasons.push("출처 편중을 줄이기 위해 함께 표시".into());
            }
            selected.push((score, item, reasons, feedback));
        }
        let items = selected
            .into_iter()
            .map(|(_, item, reasons, feedback)| PersonalizedRecommendation {
                item,
                reasons,
                feedback,
            })
            .collect();
        Ok(PersonalizationRecommendations {
            items,
            has_preferences,
        })
    }
}
