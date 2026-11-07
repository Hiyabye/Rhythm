//! Deterministic candidate retrieval, scoring and recommendation reasons.
use super::*;

pub(super) fn matches_term(document: &str, term: &str) -> bool {
    matches_folded(&folded(document), term)
}

pub(super) fn matches_folded(document: &str, term: &str) -> bool {
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

pub(super) fn date_days(date: &str) -> Option<i64> {
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

pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(super) struct Candidate {
    item: DocumentItem,
    feedback: Option<String>,
    feedback_updated_at: Option<String>,
    source_subscribed: bool,
    source_organization: String,
    last_revision_at: Option<String>,
    full_text_matches: u32,
}

pub(super) const ACTIVE_SOURCES_SQL: &str = "SELECT s.id FROM sources s WHERE
                    NOT EXISTS(SELECT 1 FROM source_extensions se WHERE se.source_id=s.id)
                    OR EXISTS(
                        SELECT 1 FROM source_extensions se
                        JOIN extension_preferences ep ON ep.extension_id=se.extension_id
                        WHERE se.source_id=s.id AND ep.enabled=1
                    )";

pub(super) const CANDIDATE_COLUMNS: &str = "d.id,d.source_id,s.name,d.title,d.author,d.category,d.published_at,d.collected_at,d.historical,
                    COALESCE(u.saved,0),COALESCE(u.is_read,0),substr(d.body_text,1,320),f.value,
                    CASE WHEN EXISTS(SELECT 1 FROM resources r WHERE r.document_id=d.id
                         AND r.revision_id=(SELECT MAX(rv.id) FROM document_revisions rv WHERE rv.document_id=d.id)
                         AND r.status<>'downloaded') THEN 'missing_resources' ELSE 'archived' END,
                    f.updated_at,s.subscribed,s.organization,d.last_revision_at";

pub(super) fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Terms shorter than the FTS trigram minimum are matched with `LIKE` instead.
pub(super) fn is_fts_term(term: &str) -> bool {
    term.chars().count() >= 3
}

pub(super) fn term_parameter(term: &str) -> String {
    if is_fts_term(term) {
        fts_phrase(term)
    } else {
        like_contains(term)
    }
}

/// `EXISTS` probes that feed `Candidate::full_text_matches`, sharing the FTS
/// index with the retrieval query. `first_parameter` is the SQL placeholder
/// number of the first term (the recommendation query reserves `?1` for the
/// combined FTS query; the per-document query starts right after its id list).
pub(super) fn matched_terms_sql(terms: &[String], first_parameter: usize) -> String {
    terms
        .iter()
        .enumerate()
        .map(|(offset, term)| {
            let parameter = first_parameter + offset;
            if is_fts_term(term) {
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
        .collect()
}

pub(super) fn map_candidate(
    row: &rusqlite::Row<'_>,
    term_count: usize,
) -> rusqlite::Result<Candidate> {
    let feedback = row.get(12)?;
    let feedback_updated_at = row.get(14)?;
    let mut full_text_matches = 0u32;
    for index in 0..term_count {
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
}

/// The preference snapshot plus the match terms a ranking pass needs. Built
/// once per call so recommendation ranking and notification scoring cannot
/// drift apart.
pub(super) struct ScoringContext {
    snapshot: PersonalizationSnapshot,
    terms: Vec<String>,
    term_indices: HashMap<String, usize>,
    has_preferences: bool,
}

pub(super) struct ScoredCandidate {
    score: i32,
    reasons: Vec<String>,
    /// List-ordering explanation, not a score signal. Recommendations append it
    /// after the real reasons; notification scoring omits it.
    ranking_note: Option<String>,
    positive_interest_matches: Vec<String>,
    explicit_interest_match: bool,
    negative_interest_matches: Vec<String>,
    positive_feedback: bool,
    negative_feedback: bool,
}

/// The single scoring routine behind both recommendation ranking and
/// per-document notification scores. It is intentionally pure: the same
/// candidate with the same context always produces the same score and reasons.
pub(super) fn score_candidate(context: &ScoringContext, candidate: &Candidate) -> ScoredCandidate {
    let item = &candidate.item;
    let feedback = candidate.feedback.as_deref();
    let feedback_age = candidate.feedback_updated_at.as_deref().and_then(date_days);
    let last_revision_age = candidate.last_revision_at.as_deref().and_then(date_days);
    let mut score = 0i32;
    let mut reasons = Vec::new();
    let mut positive_interest_matches = Vec::new();
    let mut explicit_interest_match = false;
    let mut negative_interest_matches = Vec::new();
    let searchable = format!(
        "{} {} {} {} {} {}",
        item.title,
        item.category.as_deref().unwrap_or_default(),
        item.source_name,
        candidate.source_organization,
        item.snippet,
        item.author.as_deref().unwrap_or_default()
    );
    let searchable_folded = folded(&searchable);
    for interest in &context.snapshot.interests {
        let matched = std::iter::once(interest.name.as_str())
            .chain(interest.aliases.iter().map(String::as_str))
            .any(|term| {
                matches_folded(&searchable_folded, term)
                    || context
                        .term_indices
                        .get(term)
                        .is_some_and(|index| candidate.full_text_matches & (1u32 << *index) != 0)
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
                negative_interest_matches.push(interest.name.clone());
            } else {
                score += amount;
                reasons.push(format!("관심 분야 ‘{}’에 맞는 공지", interest.name));
                explicit_interest_match |= interest.origin == "explicit";
                positive_interest_matches.push(interest.name.clone());
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
    let positive_feedback = feedback == Some("interested") && positive_feedback_bonus > 0;
    if positive_feedback {
        score += positive_feedback_bonus;
        reasons.push("관심 표시한 공지".into());
    }
    let negative_feedback = feedback == Some("not_interested") && negative_feedback_penalty > 0;
    if negative_feedback {
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
        (&context.snapshot.profile.university, 4),
        (&context.snapshot.profile.department, 8),
        (&context.snapshot.profile.study_level, 2),
    ] {
        if !value.is_empty()
            && (matches_folded(&searchable_folded, value)
                || context
                    .term_indices
                    .get(value.as_str())
                    .is_some_and(|index| candidate.full_text_matches & (1u32 << *index) != 0))
        {
            score += amount;
            reasons.push("프로필 정보와 일치".into());
        }
    }
    if negative_feedback {
        // Recent corrections lower this item without permanently hiding it.
        score = score.min(-negative_feedback_penalty);
    }
    let ranking_note = if context.has_preferences && reasons.is_empty() {
        Some("관심사 일치 신호가 없어 게시일 기준으로 보조 정렬".into())
    } else {
        None
    };
    if !context.has_preferences {
        reasons.clear();
    }
    ScoredCandidate {
        score,
        reasons,
        ranking_note,
        positive_interest_matches,
        explicit_interest_match,
        negative_interest_matches,
        positive_feedback,
        negative_feedback,
    }
}

impl Store {
    /// Builds the shared preference/term state. Both the recommendation list and
    /// per-document notification scores go through this, so matching rules,
    /// `has_preferences`, and term caps stay identical.
    pub(super) fn scoring_context(&self) -> Result<ScoringContext, String> {
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
        let term_indices = terms
            .iter()
            .enumerate()
            .map(|(index, term)| (term.clone(), index))
            .collect::<HashMap<_, _>>();
        Ok(ScoringContext {
            snapshot,
            terms,
            term_indices,
            has_preferences,
        })
    }
    pub fn personalization_recommendations(
        &self,
        limit: u32,
    ) -> Result<PersonalizationRecommendations, String> {
        let limit = limit.clamp(1, 100) as usize;
        let context = self.scoring_context()?;
        let snapshot = &context.snapshot;
        let has_preferences = context.has_preferences;
        let mut source_terms = Vec::new();
        let mut seen_source_terms = HashSet::new();
        for interest in &context.snapshot.interests {
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
        let mut term_parameters = Vec::with_capacity(context.terms.len());
        for term in &context.terms {
            let parameter = term_parameter(term);
            if is_fts_term(term) {
                fts_terms.push(parameter.clone());
            }
            term_parameters.push(parameter);
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
        let short_conditions = context
            .terms
            .iter()
            .enumerate()
            .filter(|(_, term)| !is_fts_term(term))
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
        let term_matches_sql = matched_terms_sql(&context.terms, 2);
        let sql = format!(
            "WITH active_sources(source_id) AS ({ACTIVE_SOURCES_SQL}), candidates(id) AS (
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
             SELECT {CANDIDATE_COLUMNS}{term_matches_sql}
             FROM candidates c JOIN documents d ON d.id=c.id JOIN sources s ON s.id=d.source_id
             LEFT JOIN document_state u ON u.document_id=d.id
             LEFT JOIN personalization_feedback f ON f.document_id=d.id
             ORDER BY d.published_at DESC,d.id DESC"
        );
        let mut statement = self.connection.prepare(&sql).map_err(|e| e.to_string())?;
        let term_count = context.terms.len();
        let candidates = if context.terms.is_empty() {
            statement
                .query_map([], |row| map_candidate(row, term_count))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        } else {
            let query_values = std::iter::once(fts_query.as_str())
                .chain(term_parameters.iter().map(String::as_str));
            statement
                .query_map(rusqlite::params_from_iter(query_values), |row| {
                    map_candidate(row, term_count)
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut ranked = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let signals = score_candidate(&context, &candidate);
            let mut reasons = signals.reasons;
            if let Some(note) = signals.ranking_note {
                reasons.push(note);
            }
            ranked.push((signals.score, candidate.item, reasons, candidate.feedback));
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

    /// Scores one stored document for a notification decision.
    ///
    /// This is the same scoring routine that ranks
    /// [`Self::personalization_recommendations`] (same interest amounts, same
    /// profile matching, same feedback windows, same source subscription bonus),
    /// but the document is looked up by its stable id outside the bounded
    /// recommendation candidate pools and without the top-N cut or the
    /// list-level source diversity pass. A notification scheduler can therefore
    /// evaluate an event that no recommendation list would reach.
    ///
    /// Returns `Ok(None)` when the document does not exist or its source is
    /// currently toggled off, matching the recommendation source-filter
    /// semantics. Recommendations are still filtered to active sources, so a
    /// disabled source never produces a notification score.
    ///
    /// Suggested conservative policy inputs:
    /// - `score` — ranking score, comparable with other documents.
    /// - `reasons` — only reasons that changed points; empty when no signal
    ///   matched or when `has_preferences` is `false` (cold start).
    /// - `positive_interest_matches` / `negative_interest_matches` — interest
    ///   names that matched, so an "interest" policy can require a positive
    ///   interest match and skip documents matching a negative interest.
    /// - `negative_feedback` — recent "not interested" still suppressing the
    ///   document; a conservative policy should not notify it.
    /// - `has_preferences` — `false` means the score only reflects recency
    ///   defaults, so importance thresholds cannot rely on interest matching.
    pub fn personalization_notification_score(
        &self,
        document_id: i64,
    ) -> Result<Option<PersonalizationNotificationScore>, String> {
        Ok(self
            .personalization_notification_scores(&[document_id])?
            .into_iter()
            .next())
    }

    /// Scores up to the per-call notification bound (256) of distinct document ids.
    ///
    /// Results follow the input order after de-duplication; ids that are missing
    /// or belong to a disabled source are omitted. Ids beyond the bound are
    /// ignored rather than widening the scan, and the call never applies a
    /// recommendation list limit, so every requested eligible document is
    /// scored.
    pub fn personalization_notification_scores(
        &self,
        document_ids: &[i64],
    ) -> Result<Vec<PersonalizationNotificationScore>, String> {
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for document_id in document_ids.iter().take(MAX_NOTIFICATION_SCORE_DOCUMENTS) {
            if seen.insert(*document_id) {
                unique.push(*document_id);
            }
        }
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        let context = self.scoring_context()?;
        let id_placeholders = (1..=unique.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let term_matches_sql = matched_terms_sql(&context.terms, unique.len() + 1);
        let sql = format!(
            "WITH active_sources(source_id) AS ({ACTIVE_SOURCES_SQL})
             SELECT {CANDIDATE_COLUMNS}{term_matches_sql}
             FROM documents d
             JOIN sources s ON s.id=d.source_id
             JOIN active_sources a ON a.source_id=d.source_id
             LEFT JOIN document_state u ON u.document_id=d.id
             LEFT JOIN personalization_feedback f ON f.document_id=d.id
             WHERE d.id IN ({id_placeholders})"
        );
        let mut parameters: Vec<Value> = unique.iter().map(|id| Value::Integer(*id)).collect();
        parameters.extend(
            context
                .terms
                .iter()
                .map(|term| Value::Text(term_parameter(term))),
        );
        let mut statement = self.connection.prepare(&sql).map_err(|e| e.to_string())?;
        let term_count = context.terms.len();
        let mut scored = HashMap::with_capacity(unique.len());
        for row in statement
            .query_map(rusqlite::params_from_iter(parameters), |row| {
                map_candidate(row, term_count)
            })
            .map_err(|e| e.to_string())?
        {
            let candidate = row.map_err(|e| e.to_string())?;
            let document_id = candidate.item.id;
            let signals = score_candidate(&context, &candidate);
            scored.insert(document_id, (candidate.feedback, signals));
        }
        Ok(unique
            .into_iter()
            .filter_map(|document_id| {
                let (feedback, signals) = scored.remove(&document_id)?;
                Some(PersonalizationNotificationScore {
                    document_id,
                    score: signals.score,
                    reasons: signals.reasons,
                    has_preferences: context.has_preferences,
                    positive_interest_matches: signals.positive_interest_matches,
                    explicit_interest_match: signals.explicit_interest_match,
                    negative_interest_matches: signals.negative_interest_matches,
                    feedback,
                    positive_feedback: signals.positive_feedback,
                    negative_feedback: signals.negative_feedback,
                })
            })
            .collect())
    }
}
