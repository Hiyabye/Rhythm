use super::*;

pub(super) fn validate_generation(
    output: &str,
    source: &str,
    passages: &[&str],
    images: &[SourceImage],
) -> Result<Analysis, String> {
    if output.len() > MAX_OUTPUT_BYTES {
        return Err("AI 응답이 너무 깁니다.".into());
    }
    let mut value: serde_json::Value = serde_json::from_str(output)
        .map_err(|_| "AI가 올바른 JSON을 반환하지 않았습니다.".to_owned())?;
    let facts = value
        .get_mut("facts")
        .and_then(|value| value.as_array_mut())
        .ok_or("추출 정보는 배열이어야 합니다.")?;
    if facts.len() > 6 {
        return Err("중요한 추출 정보는 최대 6개만 포함해 주세요.".into());
    }
    let mut identities = HashSet::new();
    let mut image_facts = 0usize;
    for fact in facts {
        let compact = fact.as_object_mut().ok_or("추출 정보는 객체여야 합니다.")?;
        if compact.len() != 4
            || !["kind", "label", "value", "evidence_id"]
                .iter()
                .all(|key| compact.contains_key(*key))
        {
            return Err("추출 정보에는 kind, label, value, evidence_id만 포함해 주세요.".into());
        }
        let index = compact
            .remove("evidence_id")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or("원문 근거 번호는 정수여야 합니다.")?;
        // Text passages keep their literal grounding contract. An image passage
        // has no quotation to compare, so it becomes a labelled, non-literal
        // evidence instead of a silently invented source string.
        let (evidence, image) = match passages.get(index) {
            Some(passage) => ((*passage).to_owned(), false),
            None => {
                let image_index = index
                    .checked_sub(passages.len())
                    .ok_or("원문 근거 번호가 올바르지 않습니다.")?;
                let image = images.get(image_index).ok_or_else(|| {
                    format!("evidence_id={index}: 원문 근거 번호가 올바르지 않습니다.")
                })?;
                (image.marker.clone(), true)
            }
        };
        let kind = compact
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or("추출 종류가 올바르지 않습니다.")?;
        let extracted = compact
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or("추출 값이 올바르지 않습니다.")?;
        if !identities.insert((kind.to_owned(), extracted.trim().to_owned())) {
            return Err("같은 추출 정보를 반복하지 마세요.".into());
        }
        if image {
            count_image_fact(&mut image_facts)?;
            let literal_date = if matches!(kind, "deadline" | "event_date") {
                // Only the model's own value can be checked; pixels are not text.
                normalize_date(extracted, extracted)?
            } else {
                None
            };
            mark_image_fact(compact, literal_date);
        } else {
            let grounded = normalize_date(extracted, &evidence);
            match grounded {
                Ok(literal_date) => {
                    let date = if matches!(kind, "deadline" | "event_date") {
                        literal_date
                    } else {
                        None
                    };
                    compact.insert(
                        "uncertainty".into(),
                        json!(if date.is_some() { "clear" } else { "unknown" }),
                    );
                    compact.insert("date".into(), json!(date));
                    compact.insert("origin".into(), json!("text"));
                }
                // Small models often read a poster correctly but cite a text
                // passage anyway. With vision active, the fact is kept only as
                // unverified image evidence: its date must still be literal in
                // its own value, and it is capped and marked ambiguous. Without
                // attached images the old strict rejection applies.
                Err(error) if !images.is_empty() => {
                    count_image_fact(&mut image_facts)?;
                    let literal_date = if matches!(kind, "deadline" | "event_date") {
                        normalize_date(extracted, extracted).map_err(|_| {
                            format!(
                                "evidence_id={index}: {error} 해당 id의 text만 근거로 사용하세요."
                            )
                        })?
                    } else {
                        None
                    };
                    mark_image_fact(compact, literal_date);
                    compact.insert("evidence".into(), json!(IMAGE_EVIDENCE));
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "evidence_id={index}: {error} 해당 id의 text만 근거로 사용하세요."
                    ));
                }
            }
        }
        compact.insert("evidence".into(), json!(evidence));
    }
    // Stored schema-1 history has a separate validator; only generation ingress
    // requires compact source-owned references, never arbitrary generated quotes.
    validate_output(&value.to_string(), source)
}
/// Image-derived facts are never "clear": a date is only self-consistent with
/// the model's own value, not verified against visible pixels.
pub(super) fn mark_image_fact(
    compact: &mut serde_json::Map<String, serde_json::Value>,
    literal_date: Option<String>,
) {
    compact.insert(
        "uncertainty".into(),
        json!(if literal_date.is_some() {
            "ambiguous"
        } else {
            "unknown"
        }),
    );
    compact.insert("date".into(), json!(literal_date));
    compact.insert("origin".into(), json!("image"));
}
pub(super) fn count_image_fact(count: &mut usize) -> Result<(), String> {
    *count += 1;
    if *count > MAX_IMAGE_FACTS {
        return Err("이미지에서 읽은 정보는 최대 4개만 포함해 주세요.".into());
    }
    Ok(())
}

pub(super) fn valid_text(text: &str, min: usize, max: usize) -> bool {
    let length = text.trim().chars().count();
    if length < min
        || length > max
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return false;
    }
    if [
        "<think",
        "</think",
        "<|",
        "|>",
        "[INST]",
        "[/INST]",
        "```",
        "죄송하지만",
        "도와드릴 수 없",
        "분석할 수 없",
    ]
    .iter()
    .any(|token| text.contains(token))
    {
        return false;
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.len() >= 10 && words.iter().copied().collect::<HashSet<_>>().len() * 3 < words.len() {
        return false;
    }
    let mut last = None;
    let mut repeated = 0;
    for character in text.chars() {
        repeated = if last == Some(character) {
            repeated + 1
        } else {
            1
        };
        if repeated > 20 {
            return false;
        }
        last = Some(character);
    }
    true
}
pub(super) fn uncertainty(value: &str) -> bool {
    ["clear", "ambiguous", "unknown"].contains(&value)
}

/// Parse strictly, then validate exact evidence against the actual bounded source.
/// This is syntactic/grounding validation, not a claim of semantic model accuracy.
pub fn validate_output(output: &str, source: &str) -> Result<Analysis, String> {
    if output.len() > MAX_OUTPUT_BYTES || output.trim().is_empty() {
        return Err("AI 응답이 비어 있거나 너무 깁니다. 다시 분석해 주세요.".into());
    }
    let analysis: Analysis = serde_json::from_str(output).map_err(|_| {
        "AI가 올바른 구조의 결과를 반환하지 않았습니다. 다시 분석해 주세요.".to_owned()
    })?;
    if !valid_text(&analysis.short_summary, 4, 180)
        || !valid_text(&analysis.summary, 4, 1600)
        || !uncertainty(&analysis.uncertainty)
        || analysis.facts.len() > 16
    {
        return Err("AI 요약이 검증 기준에 맞지 않습니다. 다시 분석해 주세요.".into());
    }
    let mut facts = HashSet::new();
    for fact in &analysis.facts {
        if ![
            "deadline",
            "event_date",
            "eligibility",
            "documents",
            "application_method",
            "link",
            "action",
        ]
        .contains(&fact.kind.as_str())
            || !valid_text(&fact.label, 1, 40)
            || !valid_text(&fact.value, 1, 500)
            // Source-owned quotations may legitimately repeat boilerplate or
            // contain instruction-like text; model-output style rules do not apply.
            || !(4..=600).contains(&fact.evidence.trim().chars().count())
            || fact.evidence.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
            || !uncertainty(&fact.uncertainty)
            || !["text", "image"].contains(&fact.origin.as_str())
            || (fact.origin == "text" && !source.contains(&fact.evidence))
            || !facts.insert((&fact.kind, &fact.value, &fact.evidence))
        {
            return Err("추출 정보의 원문 근거를 확인하지 못했습니다. 다시 분석해 주세요.".into());
        }
        if let Some(date) = &fact.date {
            let image = fact.origin == "image";
            // Image facts are never "clear": the date is only self-consistent
            // with the model's own value, not verified against visible pixels.
            let shape = if image {
                fact.uncertainty == "ambiguous"
                    && literal_dates(&fact.value)
                        .iter()
                        .any(|literal| &literal.date == date)
            } else {
                fact.uncertainty == "clear" && grounded_date(date, &fact.evidence)
            };
            if !["deadline", "event_date"].contains(&fact.kind.as_str()) || !shape {
                return Err("추출 날짜가 원문과 일치하지 않습니다. 다시 분석해 주세요.".into());
            }
        }
        if fact.kind == "link" {
            let link =
                url::Url::parse(&fact.value).map_err(|_| "추출 링크가 올바르지 않습니다.")?;
            if !["https", "http"].contains(&link.scheme())
                || link.host_str().is_none()
                || !link.username().is_empty()
                || link.password().is_some()
                || (fact.origin == "text" && !fact.evidence.contains(&fact.value))
            {
                return Err("추출 링크의 원문 근거를 확인하지 못했습니다.".into());
            }
        }
    }
    Ok(analysis)
}
