//! Menu images and weeks (migration 009, #318). `meals.rs` decides what to
//! fetch and validates results; this only reads and records.
use super::State;
use rhythm_feed::{manifest::MealHealth, record::MealWeek};
use rusqlite::{params, OptionalExtension};

/// A live notice of a menu board, with its image links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MealNotice {
    pub external_id: String,
    pub title: String,
    pub published_at: String,
    pub first_seen_at: String,
    pub images: Vec<String>,
}

/// An image offered to the OCR step, waiting for its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    pub image_url: String,
    pub source_id: String,
    pub external_id: String,
    pub week: String,
    pub published_at: String,
    pub image_sha256: String,
}

impl State {
    /// The board's notices that are not removed, newest first.
    pub fn meal_notices(&self, source_id: &str) -> Result<Vec<MealNotice>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT external_id,title,published_at,first_seen_at,
                   (SELECT json_group_array(json_extract(value,'$.url')) FROM json_each(resources)
                    WHERE json_extract(value,'$.kind')='image')
                 FROM notices WHERE source_id=?1 AND removed_at IS NULL
                 ORDER BY published_at DESC,first_seen_at DESC,external_id DESC",
            )?
            .query_map([source_id], |row| {
                let images: String = row.get(4)?;
                Ok(MealNotice {
                    external_id: row.get(0)?,
                    title: row.get(1)?,
                    published_at: row.get(2)?,
                    first_seen_at: row.get(3)?,
                    images: serde_json::from_str(&images).unwrap_or_default(),
                })
            })?
            .collect()
        })
    }

    /// Whether `image_url` already has a result of `ocr_version`.
    pub fn meal_image_read(&self, image_url: &str, ocr_version: &str) -> Result<bool, String> {
        self.with(|c| {
            c.query_row(
                "SELECT 1 FROM meal_images
                 WHERE image_url=?1 AND ocr_version=?2 AND status!='pending'",
                params![image_url, ocr_version],
                |_| Ok(()),
            )
            .optional()
        })
        .map(|found| found.is_some())
    }

    /// Records an image handed to the OCR step (again, if an earlier offer got
    /// no result or an older OCR version read it).
    pub fn meal_offer(&self, image: &Offered, ocr_version: &str, now: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "INSERT INTO meal_images(image_url,source_id,external_id,week,published_at,
                   image_sha256,ocr_version,status,offered_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8)
                 ON CONFLICT(image_url) DO UPDATE SET source_id=excluded.source_id,
                   external_id=excluded.external_id,week=excluded.week,
                   published_at=excluded.published_at,image_sha256=excluded.image_sha256,
                   ocr_version=excluded.ocr_version,status='pending',reason=NULL,
                   offered_at=excluded.offered_at,processed_at=NULL",
                params![
                    image.image_url,
                    image.source_id,
                    image.external_id,
                    image.week,
                    image.published_at,
                    image.image_sha256,
                    ocr_version,
                    now
                ],
            )
        })
        .map(drop)
    }

    /// Images of `ocr_version` waiting for a result.
    pub fn meal_offered(&self, ocr_version: &str) -> Result<Vec<Offered>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT image_url,source_id,external_id,week,published_at,image_sha256
                 FROM meal_images WHERE status='pending' AND ocr_version=?1
                 ORDER BY week,published_at,image_url",
            )?
            .query_map([ocr_version], |row| {
                Ok(Offered {
                    image_url: row.get(0)?,
                    source_id: row.get(1)?,
                    external_id: row.get(2)?,
                    week: row.get(3)?,
                    published_at: row.get(4)?,
                    image_sha256: row.get(5)?,
                })
            })?
            .collect()
        })
    }

    /// Records an image's result and makes its week current unless a newer
    /// post of that week already set it. `reason` is `Some` for `unreadable`.
    /// Returns whether the week changed.
    pub fn meal_record(
        &self,
        image: &Offered,
        week: &MealWeek,
        reason: Option<&str>,
        now: &str,
    ) -> Result<bool, String> {
        let record = serde_json::to_string(week).map_err(|e| e.to_string())?;
        self.with(|c| {
            let transaction = c.unchecked_transaction()?;
            transaction.execute(
                "UPDATE meal_images SET status=?2,reason=?3,processed_at=?4 WHERE image_url=?1",
                params![
                    image.image_url,
                    if reason.is_some() { "unreadable" } else { "ok" },
                    reason,
                    now
                ],
            )?;
            // A later post replaces the week, whatever its result (#318: never
            // show a menu the dormitory replaced); an older one never does.
            let changed = transaction.execute(
                "INSERT INTO meal_weeks(week,source_id,external_id,published_at,record,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(week) DO UPDATE SET source_id=excluded.source_id,
                   external_id=excluded.external_id,published_at=excluded.published_at,
                   record=excluded.record,updated_at=excluded.updated_at
                 WHERE (excluded.published_at,excluded.external_id)
                       >=(meal_weeks.published_at,meal_weeks.external_id)
                   AND excluded.record!=meal_weeks.record",
                params![
                    week.week,
                    image.source_id,
                    image.external_id,
                    image.published_at,
                    record,
                    now
                ],
            )?;
            transaction.commit()?;
            Ok(changed > 0)
        })
    }

    /// The menu OCR's health for the manifest: the last image read and the
    /// unreadable ones since.
    pub fn feed_meals_health(&self) -> Result<Vec<MealHealth>, String> {
        self.with(|c| {
            c.prepare(
                "SELECT source_id,
                   MAX(CASE WHEN status='ok' THEN processed_at END) AS last_ok,
                   (SELECT COUNT(*) FROM meal_images later WHERE later.source_id=m.source_id
                      AND later.status='unreadable'
                      AND later.processed_at>COALESCE(
                        (SELECT MAX(processed_at) FROM meal_images ok
                         WHERE ok.source_id=m.source_id AND ok.status='ok'),''))
                 FROM meal_images m WHERE status!='pending' GROUP BY source_id ORDER BY source_id",
            )?
            .query_map([], |row| {
                Ok(MealHealth {
                    source_id: row.get(0)?,
                    last_success_at: row.get(1)?,
                    consecutive_failures: row.get(2)?,
                })
            })?
            .collect()
        })
    }
}
