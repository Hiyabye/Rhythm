use super::Store;
use crate::archive::model::DocumentItem;
use rusqlite::types::Value;
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

mod ranking;
mod topics;
use ranking::*;
pub(crate) use topics::*;

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
// One notification batch is bounded so a scheduler cannot turn a single call into
// an unbounded scan. Larger batches are truncated to the first distinct ids.
const MAX_NOTIFICATION_SCORE_DOCUMENTS: usize = 256;

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

/// Per-document personalization score for notification decisions.
///
/// The score is produced by the same scoring routine that orders
/// [`PersonalizationRecommendations`], but the document is fetched by its stable
/// id instead of through the bounded recommendation candidate pools, so a
/// scheduler can evaluate an event that no recommendation list would reach.
/// Only score-bearing reasons are reported (never the ranking-only note about
/// the ordering fallback or the list-level source diversity note).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizationNotificationScore {
    /// Stable document id the score was requested for.
    pub document_id: i64,
    /// Score shared with the recommendation ranking for the same document.
    pub score: i32,
    /// Reasons that actually added or removed points, in scoring order.
    pub reasons: Vec<String>,
    /// Whether any personalization signal exists at all. When `false`, the score
    /// only reflects recency/saved/subscription-free defaults and `reasons` is
    /// empty, matching cold-start ranking output.
    pub has_preferences: bool,
    /// Positive interests whose terms matched this document.
    pub positive_interest_matches: Vec<String>,
    /// Whether at least one of those matches was a user-configured (explicit)
    /// interest rather than a learned one. A conservative interest policy can
    /// require this instead of trusting behavioral guesses.
    pub explicit_interest_match: bool,
    /// Negative interests whose terms matched this document.
    pub negative_interest_matches: Vec<String>,
    /// Current stored feedback value, if the user rated this document.
    pub feedback: Option<String>,
    /// Whether an "interested" rating contributed points within its freshness window.
    pub positive_feedback: bool,
    /// Whether a "not interested" rating is still suppressing this document.
    pub negative_feedback: bool,
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

fn current_activity_sequence(tx: &Transaction<'_>) -> Result<i64, String> {
    tx.query_row(
        "SELECT sequence FROM personalization_activity WHERE id=1",
        [],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

pub(crate) fn next_activity_sequence(tx: &Transaction<'_>) -> Result<i64, String> {
    tx.execute(
        "UPDATE personalization_activity SET sequence=sequence+1 WHERE id=1",
        [],
    )
    .map_err(|e| e.to_string())?;
    current_activity_sequence(tx)
}

pub(crate) fn insert_evidence(
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
                let sequence = current_activity_sequence(&tx)?;
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
            let sequence = current_activity_sequence(&tx)?;
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
            // The query above already excludes topics that are explicit interests; an explicit
            // alias can still represent this topic, so do not create a duplicate learned row.
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
}
