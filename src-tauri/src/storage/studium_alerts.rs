//! Seat observations/candidates stay in memory. Durable state is limited to
//! rules, rearm latches, hashed activation capabilities and delivery accounting.
use super::{proactive, Store};
use crate::{
    extensions,
    studium::{
        model::{AlertCandidate, AlertRule, AlertRuleInput, RoomState, SeatData},
        parser, rooms,
    },
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

const MAX_TOKENS: i64 = 64;

fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn validate_input(room_id: &str, input: &AlertRuleInput) -> Result<(), String> {
    let room = rooms::get(room_id)?;
    if !matches!(input.kind.as_str(), "remaining" | "preferred")
        || !(1..=room.capacity).contains(&input.minimum)
        || (input.kind == "preferred" && input.minimum != 1)
    {
        return Err("좌석 알림 조건을 확인해 주세요.".into());
    }
    Ok(())
}

fn read_rule(connection: &Connection, room_id: &str) -> Result<(AlertRule, bool), String> {
    let (rule, armed) = connection
        .query_row(
            "SELECT enabled,kind,minimum,version,armed FROM studium_alert_rules WHERE room_id=?1",
            [room_id],
            |row| {
                Ok((
                    AlertRule {
                        enabled: row.get(0)?,
                        kind: row.get(1)?,
                        minimum: row.get(2)?,
                        version: row.get(3)?,
                    },
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    validate_input(
        room_id,
        &AlertRuleInput {
            enabled: rule.enabled,
            kind: rule.kind.clone(),
            minimum: rule.minimum,
        },
    )?;
    if rule.version < 1 {
        return Err("저장된 좌석 알림 설정을 확인해 주세요.".into());
    }
    Ok((rule, armed))
}

fn valid_fresh_data(room_id: &str, data: &SeatData) -> bool {
    let Ok(room) = rooms::get(room_id) else {
        return false;
    };
    let Ok(source_time) = parser::timestamp_seconds(&data.source_time) else {
        return false;
    };
    if !(-60..=300).contains(&chrono::Utc::now().timestamp().saturating_sub(source_time))
        || data.total != room.capacity
        || data.used.checked_add(data.remaining) != Some(room.capacity)
        || data.source_url
            != if data.seat_numbers.is_some() {
                room.url()
            } else {
                rooms::STATUS_URL.to_owned()
            }
    {
        return false;
    }
    if let Some(numbers) = &data.seat_numbers {
        if numbers.len() != data.remaining as usize
            || numbers
                .iter()
                .any(|number| *number == 0 || *number > room.capacity)
            || numbers.iter().copied().collect::<HashSet<_>>().len() != numbers.len()
        {
            return false;
        }
    }
    true
}

impl Store {
    pub(crate) fn studium_alert_rules(&self) -> Result<BTreeMap<String, AlertRule>, String> {
        rooms::all()
            .into_iter()
            .map(|room| {
                read_rule(&self.connection, room.id).map(|(rule, _)| (room.id.to_owned(), rule))
            })
            .collect()
    }

    pub(crate) fn studium_save_alert_rule(
        &self,
        room_id: &str,
        input: &AlertRuleInput,
    ) -> Result<BTreeMap<String, AlertRule>, String> {
        if !self.enabled(extensions::STUDIUM)? {
            return Err("열람실 확장 기능이 꺼져 있습니다.".into());
        }
        validate_input(room_id, input)?;
        if input.enabled
            && input.kind == "preferred"
            && self.studium_favorites()?[room_id].is_empty()
        {
            return Err("선호 좌석을 먼저 저장한 뒤 알림을 켜 주세요.".into());
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let (current, _) = read_rule(&transaction, room_id)?;
        if (current.enabled, current.kind.as_str(), current.minimum)
            != (input.enabled, input.kind.as_str(), input.minimum)
        {
            transaction.execute(
                "UPDATE studium_alert_rules SET enabled=?2,kind=?3,minimum=?4,version=version+1,armed=1 WHERE room_id=?1",
                params![room_id, input.enabled, input.kind, input.minimum],
            ).map_err(|e| e.to_string())?;
            transaction
                .execute(
                    "DELETE FROM studium_alert_tokens WHERE room_id=?1",
                    [room_id],
                )
                .map_err(|e| e.to_string())?;
        }
        let rules = self.studium_alert_rules()?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(rules)
    }

    /// Unknown observations preserve the latch. Only a fresh successful
    /// non-match rearms it; a match merely returns a transient candidate.
    pub(crate) fn studium_alert_observe(
        &self,
        job_id: u64,
        state: &RoomState,
    ) -> Result<Option<AlertCandidate>, String> {
        if job_id == 0
            || !matches!(state.status.as_str(), "ready" | "count_only")
            || !self.enabled(extensions::STUDIUM)?
        {
            return Ok(None);
        }
        let Some(data) = state
            .result
            .as_ref()
            .filter(|data| valid_fresh_data(&state.room_id, data))
        else {
            return Ok(None);
        };
        if (state.status == "ready") != data.seat_numbers.is_some() {
            return Ok(None);
        }
        let (rule, armed) = read_rule(&self.connection, &state.room_id)?;
        if !rule.enabled
            || !self
                .studium_preferences()?
                .room_ids
                .contains(&state.room_id)
        {
            return Ok(None);
        }
        if rule.kind == "preferred" && state.status == "count_only" {
            return Ok(None);
        }
        match self.alert_condition(&state.room_id, &rule, data)? {
            None => Ok(None),
            Some(false) => {
                self.connection
                    .execute(
                        "UPDATE studium_alert_rules SET armed=1 WHERE room_id=?1 AND version=?2",
                        params![state.room_id, rule.version],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
            Some(true) if armed => Ok(Some(AlertCandidate {
                room_id: state.room_id.clone(),
                version: rule.version,
                job_id,
                data: data.clone(),
            })),
            Some(true) => Ok(None),
        }
    }

    fn alert_condition(
        &self,
        room_id: &str,
        rule: &AlertRule,
        data: &SeatData,
    ) -> Result<Option<bool>, String> {
        if rule.kind == "remaining" {
            return Ok(Some(data.remaining >= rule.minimum));
        }
        let Some(numbers) = &data.seat_numbers else {
            return Ok(None);
        };
        let favorites = self.studium_favorites()?;
        Ok(Some(
            favorites[room_id]
                .iter()
                .any(|number| numbers.contains(number)),
        ))
    }

    /// Called under the same Store boundary as the final OS handoff. Runtime
    /// additionally fences current controller generation/cancellation/permission.
    /// Reservation consumes the latch even when the handoff result is unknown.
    pub(crate) fn studium_alert_reserve(
        &self,
        candidate: &AlertCandidate,
        token: &str,
    ) -> Result<bool, String> {
        if !valid_token(token) {
            return Err("알림 활성화 정보가 올바르지 않습니다.".into());
        }
        if candidate.job_id == 0
            || !self.enabled(extensions::STUDIUM)?
            || !self
                .studium_preferences()?
                .room_ids
                .contains(&candidate.room_id)
            || !valid_fresh_data(&candidate.room_id, &candidate.data)
        {
            return Ok(false);
        }
        let settings = self.proactive_effective_settings()?;
        if !settings.notifications_enabled
            || settings
                .quiet_hours
                .contains(self.proactive_local_minutes()?)
        {
            return Ok(false);
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let (rule, armed) = read_rule(&transaction, &candidate.room_id)?;
        if !rule.enabled
            || !armed
            || rule.version != candidate.version
            || self.alert_condition(&candidate.room_id, &rule, &candidate.data)? != Some(true)
        {
            return Ok(false);
        }
        let watermark = proactive::advance_delivery_clock(&transaction)?;
        transaction
            .execute(
                "DELETE FROM studium_alert_tokens WHERE expires_at<=?1",
                [&watermark],
            )
            .map_err(|e| e.to_string())?;
        let token_count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM studium_alert_tokens", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        if token_count >= MAX_TOKENS
            || proactive::delivery_attempt_count(&transaction, &watermark, 600)?
                >= i64::from(proactive::MAX_DELIVERY_ATTEMPTS)
        {
            transaction.commit().map_err(|e| e.to_string())?;
            return Ok(false);
        }
        transaction.execute("INSERT INTO studium_alert_tokens(token_hash,room_id,rule_version,expires_at,created_at) VALUES(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ',?4,'+1800 seconds'),?4)", params![digest(token), candidate.room_id, candidate.version, watermark]).map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO studium_alert_attempts(attempted_at) VALUES(?1)",
                [&watermark],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "UPDATE studium_alert_rules SET armed=0 WHERE room_id=?1 AND version=?2",
                params![candidate.room_id, candidate.version],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub(crate) fn studium_alert_invalidate_tokens(&self) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM studium_alert_tokens", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// One-use capabilities never expand the selected lookup targets or fetch.
    /// Invalidated rules, disabled extensions and old/unknown tokens fail closed.
    pub(crate) fn studium_alert_consume(&self, token: &str) -> Result<Option<String>, String> {
        if !valid_token(token) {
            return Ok(None);
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let watermark = proactive::advance_delivery_clock(&transaction)?;
        transaction
            .execute(
                "DELETE FROM studium_alert_tokens WHERE expires_at<=?1",
                [&watermark],
            )
            .map_err(|e| e.to_string())?;
        let target: Option<(String, i64)> = transaction
            .query_row(
                "SELECT room_id,rule_version FROM studium_alert_tokens WHERE token_hash=?1",
                [digest(token)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "DELETE FROM studium_alert_tokens WHERE token_hash=?1",
                [digest(token)],
            )
            .map_err(|e| e.to_string())?;
        let valid = if let Some((room_id, version)) = target {
            let (rule, _) = read_rule(&transaction, &room_id)?;
            (rule.enabled
                && rule.version == version
                && self.enabled(extensions::STUDIUM)?
                && self.proactive_effective_settings()?.notifications_enabled
                && self.studium_preferences()?.room_ids.contains(&room_id))
            .then_some(room_id)
        } else {
            None
        };
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(valid)
    }
}

#[cfg(test)]
mod tests;
