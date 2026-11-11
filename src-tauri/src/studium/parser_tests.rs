//! Generated, synthetic examples of the legacy page contract. No captured
//! school HTML, user data, network requests or notification sends are used.
use super::*;

const CLOCK: &str = "2026-10-17 오전 12:26:13 현재";
const NOW: i64 = 1_792_164_373; // 2026-10-16T15:26:13Z

fn status(clock: &str) -> String {
    let rows = rooms::all().iter().map(|room| format!(
        "<tr><td>1</td><td><a href=\"roomview5.asp?room_no={}\">{}</a></td><td>{} (4)</td><td>{}</td><td>2</td></tr>",
        room.id, room.name, room.capacity, room.capacity - 2
    )).collect::<String>();
    format!("<table><tr><td>{clock}</td></tr><tr><th>순번</th><th>열람실명</th><th>전체 좌석수</th><th>사용 좌석수</th><th>잔여 좌석수</th></tr>{rows}</table>")
}

fn map(room_id: &str, available: &[u32], clock: &str) -> String {
    let room = rooms::get(room_id).unwrap();
    let positions = (1..=room.capacity)
        .map(|number| {
            format!(
        "<div id=\"Layer{number}\"><table><tr><td bgcolor=\"{}\">{number}</td></tr></table></div>",
        if available.contains(&number) { "gray" } else { "red" }
    )
        })
        .collect::<String>();
    format!("<table><tr><td>{}</td><td>총 좌석수 : {}</td><td>사용 좌석수 : {}</td><td>잔여 좌석수 : {}</td><td>{clock}</td></tr></table><table><tr><td bgcolor=\"red\">사용중</td><td bgcolor=\"gray\">미사용</td></tr></table>{positions}",
        room.name, room.capacity, room.capacity - available.len() as u32, available.len())
}

fn now() -> i64 {
    timestamp_seconds("2026-10-16T15:26:13Z").unwrap()
}

#[test]
fn all_seven_rooms_have_isolated_counts_numbers_and_identity() {
    assert_eq!(NOW, now());
    let summary = parse_status(&status(CLOCK), now()).unwrap();
    assert_eq!(summary.len(), 7);
    for room in rooms::all() {
        let main = summary[room.id].as_ref().unwrap();
        assert_eq!(main.total, room.capacity);
        assert_eq!(main.remaining, 2);
        assert_eq!(main.seat_numbers, None);
        assert_eq!(main.source_time, "2026-10-16T15:26:13Z");
        let detailed =
            parse_seats(&map(room.id, &[1, room.capacity], CLOCK), room.id, now()).unwrap();
        assert_eq!(detailed.seat_numbers, Some(vec![1, room.capacity]));
        assert_eq!(detailed.total, room.capacity);
        assert_eq!(detailed.used + detailed.remaining, room.capacity);
        assert_eq!(detailed.source_url, room.url());
    }
}

#[test]
fn encoding_is_strict_utf8_bom_then_cp949() {
    let page = status(CLOCK);
    assert_eq!(decode_page(page.as_bytes()).unwrap(), page);
    let bom = format!("\u{feff}{page}");
    assert_eq!(decode_page(bom.as_bytes()).unwrap(), page);
    let (encoded, _, errors) = encoding_rs::EUC_KR.encode(&page);
    assert!(!errors);
    assert_eq!(decode_page(&encoded).unwrap(), page);
    assert!(decode_page(&[0xff]).is_err());
    assert!(decode_page(&vec![b'x'; MAX_PAGE_BYTES + 1]).is_err());
}

#[test]
fn a_bad_room_row_does_not_destroy_other_room_results() {
    let html = status(CLOCK).replace("113 (4)", "114 (4)");
    let parsed = parse_status(&html, now()).unwrap();
    assert!(parsed["19"].is_err());
    assert!(parsed["5"].is_ok());
    let html = status(CLOCK).replace("room_no=19", "room_no=5");
    let parsed = parse_status(&html, now()).unwrap();
    assert!(parsed["19"].is_err());
    assert!(parsed["5"].is_err());
    assert!(parsed["6"].is_ok());
}

#[test]
fn status_schema_time_and_count_failures_are_not_zero_seats() {
    assert!(parse_status(&status(CLOCK).replace("잔여 좌석수", "예약 좌석수"), now()).is_err());
    assert!(parse_status(&status("시각 없음"), now()).is_err());
    for value in ["-2", "많음", "999999999999999999999"] {
        let html = status(CLOCK).replace("<td>2</td>", &format!("<td>{value}</td>"));
        assert!(parse_status(&html, now())
            .unwrap()
            .values()
            .all(Result::is_err));
    }
    assert!(
        parse_status(&status(CLOCK).replace("인벤툼 II", "다른 방"), now()).unwrap()["5"].is_err()
    );
}

#[test]
fn layout_table_rows_do_not_duplicate_the_nested_room_rows() {
    let page = status(CLOCK);
    let nested = format!(
        "<table><tr><td>열람실 현황<table><tr><td>{page}</td></tr></table></td></tr></table>"
    );
    let parsed = parse_status(&nested, now()).unwrap();
    assert_eq!(parsed.len(), 7);
    for room in rooms::all() {
        assert_eq!(parsed[room.id].as_ref().unwrap().remaining, 2);
    }

    // Actual duplicate data rows still fail instead of choosing one table.
    let duplicated = format!("<table><tr><td>{page}{page}</td></tr></table>");
    assert!(parse_status(&duplicated, now())
        .unwrap()
        .values()
        .all(Result::is_err));
}

#[test]
fn exact_freshness_boundaries_are_inclusive_and_utc_normalized() {
    for age in [-60, 0, 300] {
        assert!(parse_status(&status(CLOCK), now() + age).is_ok());
    }
    for age in [-61, 301] {
        assert!(parse_status(&status(CLOCK), now() + age).is_err());
    }
}

#[test]
fn korean_noon_midnight_and_invalid_calendar_dates_are_checked() {
    let midnight = parse_time("2026-10-17 오전 12:00:00 현재", now()).err();
    assert!(midnight.is_some()); // Old for this clock, not silently corrected.
    let t = timestamp_seconds("2026-10-16T15:00:00Z").unwrap();
    assert_eq!(
        parse_time("2026-10-17 오전 12:00:00 현재", t).unwrap(),
        "2026-10-16T15:00:00Z"
    );
    let t = timestamp_seconds("2026-10-17T03:00:00Z").unwrap();
    assert_eq!(
        parse_time("2026-10-17 오후 12:00:00 현재", t).unwrap(),
        "2026-10-17T03:00:00Z"
    );
    for clock in [
        "0000-01-01 오전 1:00:00 현재",
        "2026-02-30 오전 1:00:00 현재",
        "2026-10-17 오전 0:26:13 현재",
        "2026-10-17 오전 13:26:13 현재",
        "2026-10-17 오전 1:60:13 현재",
        "2026-10-17 오전 1:00:60 현재",
    ] {
        assert!(parse_time(clock, now()).is_err(), "{clock}");
    }
}

#[test]
fn verified_full_and_all_available_maps_have_distinct_valid_numbers() {
    let full = parse_seats(&map("5", &[], CLOCK), "5", now()).unwrap();
    assert_eq!((full.remaining, full.seat_numbers), (0, Some(vec![])));
    let numbers: Vec<u32> = (1..=102).collect();
    let empty = parse_seats(&map("5", &numbers, CLOCK), "5", now()).unwrap();
    assert_eq!(empty.remaining, 102);
    assert_eq!(empty.seat_numbers, Some(numbers));
}

#[test]
fn map_identity_missing_duplicate_and_mislabeled_numbers_fail() {
    let html = map("5", &[1], CLOCK);
    assert!(parse_seats(&html, "19", now()).is_err());
    assert!(parse_seats(
        &html.replace("id=\"Layer102\"", "id=\"Layer1\""),
        "5",
        now()
    )
    .is_err());
    assert!(parse_seats(
        &html.replace("id=\"Layer102\"", "id=\"Other102\""),
        "5",
        now()
    )
    .is_err());
    assert!(parse_seats(
        &html.replace("id=\"Layer102\"", "id=\"Layerx\""),
        "5",
        now()
    )
    .is_err());
    assert!(parse_seats(&html.replace(">102</td>", ">101</td>"), "5", now()).is_err());
}

#[test]
fn changed_map_legend_color_totals_and_summary_fail() {
    let html = map("5", &[1], CLOCK);
    for broken in [
        html.replace("미사용", "예약 가능"),
        html.replace("bgcolor=\"gray\">1<", "bgcolor=\"blue\">1<"),
        html.replace("잔여 좌석수 : 1", "잔여 좌석수 : 2"),
        html.replace("총 좌석수 : 102", "총 좌석수 : 999"),
        html.replace("사용 좌석수 : 101", "사용 좌석수 : 102"),
        html.replace("2026-10-17 오전 12:26:13 현재", "기준 시각 없음"),
        html.replace("잔여 좌석수", "예약 좌석수"),
    ] {
        assert!(parse_seats(&broken, "5", now()).is_err());
    }
}

#[test]
fn positions_and_forecast_do_not_affect_actual_seat_result() {
    let html = map("5", &[1, 2], CLOCK).replace(
        "id=\"Layer1\"",
        "id=\"Layer1\" style=\"left:broken;top:9999999px\"",
    );
    let with_forecast = format!(
        "{html}<table><tr><td>좌석 해지 예상 시간표</td><td>60분 이내</td><td>0</td></tr></table>"
    );
    let result = parse_seats(&with_forecast, "5", now()).unwrap();
    assert_eq!(result.remaining, 2);
    assert_eq!(result.seat_numbers, Some(vec![1, 2]));
}

#[test]
fn more_recent_detail_can_differ_from_common_counts() {
    let common = parse_status(&status(CLOCK), now()).unwrap();
    let detail = parse_seats(
        &map("5", &[84], "2026-10-17 오전 12:26:14 현재"),
        "5",
        now(),
    )
    .unwrap();
    assert_eq!(common["5"].as_ref().unwrap().remaining, 2);
    assert_eq!((detail.remaining, detail.seat_numbers), (1, Some(vec![84])));
    assert!(
        timestamp_seconds(&detail.source_time).unwrap()
            > timestamp_seconds(&common["5"].as_ref().unwrap().source_time).unwrap()
    );
}

#[test]
fn unknown_rooms_and_non_html_error_responses_fail() {
    assert!(rooms::get("unknown").is_err());
    assert!(parse_seats(&map("5", &[], CLOCK), "unknown", now()).is_err());
    for body in [
        "접근이 거절되었습니다",
        "<html>로그인이 필요합니다</html>",
        "\0",
    ] {
        assert!(parse_status(body, now()).is_err());
        assert!(parse_seats(body, "5", now()).is_err());
    }
}

const FIRST_ORIGIN: &str = "position:absolute;left:100.125px;top:200.250px";

/// Synthetic coordinates only; these positions do not reproduce a school map.
fn positioned_map(room_id: &str, available: &[u32], clock: &str) -> String {
    let mut html = map(room_id, available, clock);
    for number in 1..=rooms::get(room_id).unwrap().capacity {
        let left = 100 + ((number - 1) % 20) * 30;
        let top = 200 + ((number - 1) / 20) * 40;
        html = html.replace(
            &format!("<div id=\"Layer{number}\">"),
            &format!("<div id=\"Layer{number}\" style=\"position:absolute;left:{left}.125px;top:{top}.250px\">"),
        );
    }
    html
}

fn assert_optional_layout_failure(html: &str) {
    let data = parse_seats(html, "5", now()).expect("counts/numbers remain valid");
    assert_eq!(data.remaining, 2);
    assert_eq!(data.seat_numbers, Some(vec![1, 102]));
    assert_eq!(data.source_time, "2026-10-16T15:26:13Z");
    assert_eq!(data.source_url, rooms::get("5").unwrap().url());
    assert!(data.layout.is_none());
    assert!(data.layout_error.is_some());
}

#[test]
fn normalized_layouts_cover_all_seven_rooms_and_keep_the_same_response_identity() {
    for room in rooms::all() {
        let data = parse_seats(
            &positioned_map(room.id, &[1, room.capacity], CLOCK),
            room.id,
            now(),
        )
        .unwrap();
        let layout = data.layout.as_ref().unwrap();
        assert!(data.layout_error.is_none());
        assert_eq!(layout.points.len(), room.capacity as usize);
        assert_eq!(layout.width, 570_000);
        assert_eq!(layout.height, ((room.capacity - 1) / 20) * 40_000);
        for (index, point) in layout.points.iter().enumerate() {
            assert_eq!(point.number, index as u32 + 1);
            assert_eq!(point.x, (index as u32 % 20) * 30_000);
            assert_eq!(point.y, (index as u32 / 20) * 40_000);
        }
        assert_eq!(data.seat_numbers, Some(vec![1, room.capacity]));
        assert_eq!(data.total, room.capacity);
        assert_eq!(data.source_time, "2026-10-16T15:26:13Z");
        assert_eq!(data.source_url, room.url());
    }
    let common = parse_status(&status(CLOCK), now()).unwrap();
    assert!(common.values().all(|result| result
        .as_ref()
        .is_ok_and(|data| data.layout.is_none() && data.layout_error.is_none())));
}

#[test]
fn full_and_all_available_layouts_still_contain_every_numbered_marker() {
    let room = rooms::get("5").unwrap();
    for available in [vec![], (1..=room.capacity).collect::<Vec<_>>()] {
        let data = parse_seats(&positioned_map("5", &available, CLOCK), "5", now()).unwrap();
        assert_eq!(data.remaining, available.len() as u32);
        assert_eq!(data.seat_numbers, Some(available));
        assert_eq!(data.layout.unwrap().points.len(), room.capacity as usize);
    }
}

#[test]
fn exact_decimal_and_maximum_coordinates_do_not_round_or_overflow() {
    let close_points = positioned_map("5", &[1, 102], CLOCK).replace(
        "left:130.125px;top:200.250px",
        "left:100.126px;top:200.250px",
    );
    let close_layout = parse_seats(&close_points, "5", now())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!((close_layout.points[0].x, close_layout.points[1].x), (0, 1));
    let html = positioned_map("5", &[1, 102], CLOCK)
        .replace(FIRST_ORIGIN, "position:absolute;left:.001px;top:0.002px")
        .replace("left:130.125px;top:400.250px", "left:8192px;top:8192.000px");
    let data = parse_seats(&html, "5", now()).unwrap();
    let layout = data.layout.unwrap();
    assert_eq!((layout.width, layout.height), (8_191_999, 8_191_998));
    assert_eq!((layout.points[0].x, layout.points[0].y), (0, 0));
    assert_eq!((layout.points[1].x, layout.points[1].y), (130_124, 200_248));
    assert_eq!(
        (layout.points[101].x, layout.points[101].y),
        (layout.width, layout.height)
    );
}

#[test]
fn geometry_units_expressions_precision_and_bounds_fail_only_the_layout() {
    let page = positioned_map("5", &[1, 102], CLOCK);
    for left in [
        "broken",
        "-1px",
        "-0px",
        "+1px",
        "1e3px",
        "1%",
        "1em",
        "0",
        "1.0000px",
        "8192.001px",
        "NaNpx",
        "infpx",
        "calc(1px)",
        "var(--x)",
        "url(https://example.invalid/a)",
        "1 px",
        "1px !important",
        "999999999999999999999px",
        ".px",
    ] {
        let changed = page.replace(
            FIRST_ORIGIN,
            &format!("position:absolute;left:{left};top:200.250px"),
        );
        assert_optional_layout_failure(&changed);
    }
}

#[test]
fn missing_duplicate_and_geometry_shifting_styles_fail_only_the_layout() {
    let page = positioned_map("5", &[1, 102], CLOCK);
    for style in [
        "left:100.125px;top:200.250px",
        "position:relative;left:100.125px;top:200.250px",
        "position:absolute;top:200.250px",
        "position:absolute;left:100.125px",
        "position:absolute;left:100.125px;LEFT:100.125px;top:200.250px",
        "position:absolute;position:absolute;left:100.125px;top:200.250px",
    ] {
        assert_optional_layout_failure(&page.replace(FIRST_ORIGIN, style));
    }
    for property in [
        "transform",
        "translate",
        "rotate",
        "scale",
        "zoom",
        "inset",
        "right",
        "bottom",
        "margin-left",
    ] {
        assert_optional_layout_failure(
            &page.replace(FIRST_ORIGIN, &format!("{FIRST_ORIGIN};{property}:1px")),
        );
    }
    for suffix in [";/*transform*/transform:scale(2)", ";trans\\form:scale(2)"] {
        assert_optional_layout_failure(
            &page.replace(FIRST_ORIGIN, &format!("{FIRST_ORIGIN}{suffix}")),
        );
    }
}

#[test]
fn mixed_parent_duplicate_positions_and_noncanonical_ids_fail_only_the_layout() {
    let page = positioned_map("5", &[1, 102], CLOCK);
    let start = page.find("<div id=\"Layer1\"").unwrap();
    let end = start + page[start..].find("</div>").unwrap() + "</div>".len();
    let mixed_parent = format!(
        "{}<section>{}</section>{}",
        &page[..start],
        &page[start..end],
        &page[end..]
    );
    assert_optional_layout_failure(&mixed_parent);
    assert_optional_layout_failure(&page.replace(
        "left:130.125px;top:200.250px",
        "left:100.125px;top:200.250px",
    ));
    assert_optional_layout_failure(&page.replace("id=\"Layer1\"", "id=\"Layer01\""));
    assert_optional_layout_failure(&format!("{page}<span id=\"Layer1\">추가 요소</span>"));
    let no_horizontal_extent = (1..=102).fold(page.clone(), |page, number| {
        let x = 100 + ((number - 1) % 20) * 30;
        let y = 200 + ((number - 1) / 20) * 40;
        page.replace(
            &format!("left:{x}.125px;top:{y}.250px"),
            &format!("left:100px;top:{number}px"),
        )
    });
    assert_optional_layout_failure(&no_horizontal_extent);
    let no_vertical_extent = (1..=102).fold(page.clone(), |page, number| {
        let x = 100 + ((number - 1) % 20) * 30;
        let y = 200 + ((number - 1) / 20) * 40;
        page.replace(
            &format!("left:{x}.125px;top:{y}.250px"),
            &format!("left:{number}px;top:100px"),
        )
    });
    assert_optional_layout_failure(&no_vertical_extent);
}

#[test]
fn wrapper_dimensions_are_not_exported_as_seat_sizes() {
    let page = positioned_map("5", &[1, 102], CLOCK);
    let reference = parse_seats(&page, "5", now()).unwrap();
    let changed = page.replace(FIRST_ORIGIN, &format!("{FIRST_ORIGIN};width:1px;height:999999px;background:url(https://example.invalid/never-requested)"));
    let parsed = parse_seats(&changed, "5", now()).unwrap();
    assert_eq!(parsed.layout, reference.layout);
    let serialized = serde_json::to_string(parsed.layout.as_ref().unwrap()).unwrap();
    assert!(
        !serialized.contains("background")
            && !serialized.contains("https")
            && !serialized.contains("style")
    );
}

#[test]
fn optional_layout_never_softens_room_time_counts_or_number_validation() {
    let page = positioned_map("5", &[1, 102], CLOCK);
    assert!(parse_seats(&page, "6", now()).is_err());
    assert!(parse_seats(&page, "5", now() + 301).is_err());
    assert!(parse_seats(
        &page.replace("잔여 좌석수 : 2", "잔여 좌석수 : 3"),
        "5",
        now()
    )
    .is_err());
    assert!(parse_seats(
        &page.replace("id=\"Layer102\"", "id=\"Layer1\""),
        "5",
        now()
    )
    .is_err());
    assert!(parse_seats(&page.replace(">102</td>", ">101</td>"), "5", now()).is_err());
}
