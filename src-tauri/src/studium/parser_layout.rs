//! Optional, bounded marker origins from the already validated detail DOM.
//! No CSS or source markup is returned or executed, and box sizes are ignored.
use crate::studium::model::{SeatLayout, SeatPoint};
use scraper::{ElementRef, Html, Selector};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

static IDENTIFIED: LazyLock<Selector> = LazyLock::new(|| Selector::parse("[id]").unwrap());
const MAX_MILLI_PX: u32 = 8192 * 1000;
const BAD_GEOMETRY: &str = "좌석의 상대 위치를 확인하지 못했습니다. 번호 목록을 이용해 주세요.";

/// Decimal CSS px becomes exact fixed-point integers, with no floating-point
/// truncation/rounding. Other units, signs, expressions and exponents fail.
fn pixels(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.len() > 24 {
        return Err(BAD_GEOMETRY.into());
    }
    let normalized = value.to_ascii_lowercase();
    let numeric = normalized.strip_suffix("px").ok_or(BAD_GEOMETRY)?;
    let (whole, fraction) = numeric
        .split_once('.')
        .map_or((numeric, None), |(whole, fraction)| (whole, Some(fraction)));
    if (whole.is_empty() && fraction.is_none()) || !whole.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(BAD_GEOMETRY.into());
    }
    let whole: u32 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| BAD_GEOMETRY)?
    };
    let fraction = match fraction {
        Some(fraction)
            if !fraction.is_empty()
                && fraction.len() <= 3
                && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            fraction.parse::<u32>().map_err(|_| BAD_GEOMETRY)?
                * 10_u32.pow(3 - fraction.len() as u32)
        }
        Some(_) => return Err(BAD_GEOMETRY.into()),
        None => 0,
    };
    let number = whole
        .checked_mul(1000)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or(BAD_GEOMETRY)?;
    if number > MAX_MILLI_PX {
        return Err(BAD_GEOMETRY.into());
    }
    Ok(number)
}

fn origin(element: ElementRef<'_>) -> Result<(u32, u32), String> {
    let style = element.value().attr("style").ok_or(BAD_GEOMETRY)?;
    if style.len() > 8192 || style.contains('\\') || style.contains("/*") || style.contains("*/") {
        return Err(BAD_GEOMETRY.into());
    }
    let (mut position, mut left, mut top) = (None, None, None);
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let slot = match name.trim().to_ascii_lowercase().as_str() {
            "position" => &mut position,
            "left" => &mut left,
            "top" => &mut top,
            // These would change the meaning of the source left/top origin.
            // Do not approximate a new CSS layout with the old marker contract.
            "transform" | "translate" | "rotate" | "scale" | "zoom" | "inset" | "right"
            | "bottom" | "margin" | "margin-left" | "margin-top" | "margin-right"
            | "margin-bottom" => return Err(BAD_GEOMETRY.into()),
            // Width/height describe legacy wrappers, not physical seats.
            _ => continue,
        };
        if slot.replace(value.trim()).is_some() {
            return Err(BAD_GEOMETRY.into());
        }
    }
    if !position.is_some_and(|position| position.eq_ignore_ascii_case("absolute")) {
        return Err(BAD_GEOMETRY.into());
    }
    Ok((
        pixels(left.ok_or(BAD_GEOMETRY)?)?,
        pixels(top.ok_or(BAD_GEOMETRY)?)?,
    ))
}

pub(super) fn parse(page: &Html, capacity: u32) -> Result<SeatLayout, String> {
    let mut points = BTreeMap::new();
    let mut coordinates = BTreeSet::new();
    let mut parent = None;
    for element in page.select(&IDENTIFIED) {
        let id = element.value().attr("id").unwrap_or_default();
        let Some(number) = id.strip_prefix("Layer") else {
            continue;
        };
        if element.value().name() != "div" {
            return Err(BAD_GEOMETRY.into());
        }
        let number: u32 = number.parse().map_err(|_| BAD_GEOMETRY)?;
        if number == 0
            || number > capacity
            || id != format!("Layer{number}")
            || points.contains_key(&number)
        {
            return Err(BAD_GEOMETRY.into());
        }
        let direct_parent = element.parent().ok_or(BAD_GEOMETRY)?.id();
        match parent {
            Some(parent) if parent != direct_parent => return Err(BAD_GEOMETRY.into()),
            None => parent = Some(direct_parent),
            _ => {}
        }
        let (x, y) = origin(element)?;
        if !coordinates.insert((x, y)) {
            return Err(BAD_GEOMETRY.into());
        }
        points.insert(number, SeatPoint { number, x, y });
    }
    if capacity == 0 || points.len() != capacity as usize {
        return Err(BAD_GEOMETRY.into());
    }
    let min_x = points
        .values()
        .map(|point| point.x)
        .min()
        .ok_or(BAD_GEOMETRY)?;
    let min_y = points
        .values()
        .map(|point| point.y)
        .min()
        .ok_or(BAD_GEOMETRY)?;
    let width = points
        .values()
        .map(|point| point.x)
        .max()
        .ok_or(BAD_GEOMETRY)?
        - min_x;
    let height = points
        .values()
        .map(|point| point.y)
        .max()
        .ok_or(BAD_GEOMETRY)?
        - min_y;
    if width == 0 || height == 0 {
        return Err(BAD_GEOMETRY.into());
    }
    Ok(SeatLayout {
        width,
        height,
        points: points
            .into_values()
            .map(|mut point| {
                point.x -= min_x;
                point.y -= min_y;
                point
            })
            .collect(),
    })
}
