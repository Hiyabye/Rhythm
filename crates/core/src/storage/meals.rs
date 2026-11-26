//! The Bellarmine dormitory menu weeks mirrored from the feed (#318). Display
//! only: menus never feed ranking, recommendations or notifications.
use super::Store;
use rhythm_feed::record::{Meal, MealDay, MealStatus, MealWeek};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// The 식단 page: the current week and, once published, the next one.
#[derive(Debug, Serialize, PartialEq)]
pub struct Meals {
    /// Today in Korea, `YYYY-MM-DD`; the page opens on it.
    pub today: String,
    /// At most two weeks, the earliest first, none before today's week. A
    /// missing week is simply absent: no other week stands in for it.
    pub weeks: Vec<MealWeek>,
}

#[derive(Deserialize)]
struct StoredMenu {
    days: Vec<MealDay>,
    breakfast_common: Meal,
    drinks: Meal,
}

impl Store {
    pub fn meal_weeks(&self, today: &str) -> Result<Meals, String> {
        if super::calendar::day_number(today).is_none() {
            return Err("날짜 형식이 올바르지 않습니다.".into());
        }
        let weeks = self
            .connection
            .prepare(
                "SELECT week,source_id,external_id,image_url,low_resolution,menu FROM meal_weeks
                 WHERE week >= date(?1,'-6 days','weekday 1') ORDER BY week LIMIT 2",
            )
            .and_then(|mut statement| {
                statement
                    .query_map(params![today], |r| {
                        Ok((
                            MealWeek {
                                week: r.get(0)?,
                                source_id: r.get(1)?,
                                external_id: r.get(2)?,
                                image_url: r.get(3)?,
                                status: MealStatus::Unreadable,
                                low_resolution: r.get(4)?,
                                days: Vec::new(),
                                breakfast_common: None,
                                drinks: None,
                            },
                            r.get::<_, Option<String>>(5)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| e.to_string())?;
        let weeks = weeks
            .into_iter()
            .map(|(mut week, menu)| {
                if let Some(menu) = menu {
                    let menu: StoredMenu =
                        serde_json::from_str(&menu).map_err(|e| e.to_string())?;
                    week.status = MealStatus::Ok;
                    week.days = menu.days;
                    week.breakfast_common = Some(menu.breakfast_common);
                    week.drinks = Some(menu.drinks);
                }
                Ok(week)
            })
            .collect::<Result<_, String>>()?;
        Ok(Meals {
            today: today.to_owned(),
            weeks,
        })
    }
}
