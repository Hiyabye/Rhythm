use super::*;
use rhythm_sources::{
    model::{ArticleInput, ResourceInput},
    transport::FetchError,
};
use std::{collections::HashMap, sync::Mutex};

const HOST: &str = "https://new-cms.sogang.ac.kr";
const NOW: &str = "2026-11-25T03:00:00.000Z";
const TODAY: &str = "2026-11-25";
/// A JPEG signature followed by `name`, so each test image has its own hash.
fn jpeg(name: &str) -> Vec<u8> {
    [&[0xff, 0xd8, 0xff, 0xe0][..], name.as_bytes()].concat()
}

#[derive(Default)]
struct Fake {
    /// Bodies and MIME types by exact URL; anything else is a 404.
    responses: Mutex<HashMap<String, (Vec<u8>, String)>>,
    calls: Mutex<Vec<String>>,
}
impl Fake {
    fn new() -> Self {
        let fake = Self::default();
        fake.serve("/robots.txt", b"User-agent: *\nAllow: /\n", "text/plain");
        fake
    }
    fn serve(&self, path: &str, bytes: &[u8], mime: &str) {
        self.responses
            .lock()
            .unwrap()
            .insert(format!("{HOST}{path}"), (bytes.to_vec(), mime.into()));
    }
    fn image_calls(&self) -> usize {
        let calls = self.calls.lock().unwrap();
        calls
            .iter()
            .filter(|url| url.contains("/CrossEditor/"))
            .count()
    }
}
impl Transport for Fake {
    async fn fetch(
        &self,
        url: &Url,
        _validators: Option<(Option<String>, Option<String>)>,
        _limit: usize,
        _cancel: &AtomicBool,
        access: Access,
    ) -> Result<Fetch, FetchError> {
        assert_eq!(access, Access::Source);
        self.calls.lock().unwrap().push(url.to_string());
        match self.responses.lock().unwrap().get(url.as_str()).cloned() {
            Some((bytes, mime)) => Ok(Fetch::Body {
                bytes,
                etag: None,
                last_modified: None,
                mime: Some(mime),
            }),
            None => Err(FetchError::http(404, "없음".into())),
        }
    }
}

fn image_path(name: &str) -> String {
    format!("/CrossEditor/binary/images/000047/{name}.jpg")
}

/// A menu post with the given title, publication day and image names.
fn post(state: &State, id: u32, title: &str, published: &str, images: &[&str]) {
    state.board(SOURCE_ID).unwrap();
    let article = ArticleInput {
        external_id: id.to_string(),
        canonical_url: format!("{HOST}/bbs/sgdormitory/1648/{id}/artclView.do"),
        title: title.into(),
        author: None,
        category: Some("주간 메뉴".into()),
        published_at: published.into(),
        modified_at: None,
        original: String::new(),
        unsupported_body: false,
        body_html: String::new(),
        body_text: String::new(),
        resources: images
            .iter()
            .map(|name| ResourceInput {
                url: format!("{HOST}{}", image_path(name)),
                filename: format!("{name}.jpg"),
                kind: "image".into(),
            })
            .collect(),
        etag: None,
        last_modified: None,
    };
    state.upsert(SOURCE_ID, &article, false).unwrap();
}

async fn offer_now(state: &State, fake: &Fake, folder: &Path) -> Offers {
    offer(state, fake, folder, TODAY, NOW, &AtomicBool::new(false))
        .await
        .unwrap()
}

fn files(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn titles_name_a_monday_to_sunday_week_near_publication() {
    let day = |text| parse_day(text).unwrap();
    let week = |title, published| title_week(title, day(published)).map(format_day);
    let current = "11월 23일 ~ 11월 29일 식단";
    assert_eq!(week(current, "2026-11-18").as_deref(), Some("2026-11-23"));
    assert_eq!(
        week("3월 12일~18일 식단", "2018-03-08").as_deref(),
        Some("2018-03-12")
    );
    // The year comes from the post: a December post names January's week.
    assert_eq!(
        week("12월 28일 ~ 1월 3일 식단", "2026-12-23").as_deref(),
        Some("2026-12-28")
    );
    assert_eq!(
        week("1월 4일 ~ 1월 10일 식단", "2026-12-30").as_deref(),
        Some("2027-01-04")
    );
    for (title, published) in [
        ("11월 24일 ~ 11월 30일 식단", "2026-11-18"), // a Tuesday
        ("11월 23일 ~ 11월 28일 식단", "2026-11-18"), // not its Sunday
        ("11월 23일 ~ 12월 29일 식단", "2026-11-18"),
        ("11월 23일 식단", "2026-11-18"),
        ("주간 메뉴 안내", "2026-11-18"),
        ("2월 30일 ~ 3월 7일 식단", "2026-02-25"),
        (current, "2026-06-01"), // far from publication
    ] {
        assert_eq!(week(title, published), None, "{title}");
    }
}

#[test]
fn dates_round_trip_and_korea_is_nine_hours_ahead() {
    for text in [
        "1970-01-01",
        "2000-02-29",
        "2026-11-23",
        "2027-01-03",
        "2100-03-01",
    ] {
        assert_eq!(format_day(parse_day(text).unwrap()), text);
    }
    assert_eq!(weekday(parse_day("2026-11-23").unwrap()), 0);
    assert_eq!(weekday(parse_day("2026-11-29").unwrap()), 6);
    assert_eq!(parse_day("2026-02-29"), None);
    assert_eq!(
        korean_day("2026-11-25T14:59:59.999Z").as_deref(),
        Some("2026-11-25")
    );
    assert_eq!(
        korean_day("2026-11-25T15:00:00.000Z").as_deref(),
        Some("2026-11-26")
    );
    assert_eq!(
        korean_day("2026-12-31T20:00:00.000Z").as_deref(),
        Some("2027-01-01")
    );
}

#[test]
fn the_ocr_step_uses_this_version_and_job_shape() {
    let script = include_str!("../../meal-ocr/meal_ocr.py");
    assert!(script.contains(&format!("VERSION = \"{OCR_VERSION}\"")));
    assert!(script.contains(r#"{"version", "image", "image_sha256", "week", "published"}"#));
}

#[tokio::test]
async fn the_newest_post_of_each_current_week_is_offered_once() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    let folder = tempfile::tempdir().unwrap();
    // This week twice (the newer post wins), next week, a past and a far week.
    post(
        &state,
        1,
        "11월 23일 ~ 11월 29일 식단",
        "2026-11-17",
        &["old"],
    );
    post(
        &state,
        2,
        "11월 23일 ~ 11월 29일 식단",
        "2026-11-19",
        &["new"],
    );
    post(
        &state,
        3,
        "11월 30일 ~ 12월 6일 식단",
        "2026-11-24",
        &["next"],
    );
    post(
        &state,
        4,
        "11월 16일 ~ 11월 22일 식단",
        "2026-11-10",
        &["past"],
    );
    post(
        &state,
        5,
        "12월 28일 ~ 1월 3일 식단",
        "2026-11-24",
        &["far"],
    );
    post(&state, 6, "기숙사 식당 안내", "2026-11-24", &["notice"]);
    for name in ["old", "new", "next", "past", "far"] {
        fake.serve(&image_path(name), &jpeg(name), "image/jpeg");
    }
    let offers = offer_now(&state, &fake, folder.path()).await;
    assert_eq!(
        offers,
        Offers {
            offered: 2,
            ..Offers::default()
        }
    );
    let (new, next) = (sha256_hex(jpeg("new")), sha256_hex(jpeg("next")));
    let mut expected = [new.clone(), next.clone()]
        .map(|sha| [format!("{sha}.jpg"), format!("{sha}.json")])
        .concat();
    expected.sort();
    assert_eq!(files(folder.path()), expected);
    let job: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{next}.json"))).unwrap())
            .unwrap();
    assert_eq!(
        job,
        serde_json::json!({"version": OCR_VERSION, "image": format!("{next}.jpg"),
            "image_sha256": next, "week": "2026-11-30", "published": "2026-11-24"})
    );
    assert_eq!(
        std::fs::read(folder.path().join(format!("{new}.jpg"))).unwrap(),
        jpeg("new")
    );
    let pending = state.meal_offered(OCR_VERSION).unwrap();
    let weeks: Vec<(&str, &str)> = pending
        .iter()
        .map(|p| (p.week.as_str(), p.external_id.as_str()))
        .collect();
    assert_eq!(weeks, [("2026-11-23", "2"), ("2026-11-30", "3")]);
    assert_eq!(pending[0].image_sha256, new);

    // A pending image is offered again; a read one never is.
    let again = tempfile::tempdir().unwrap();
    for image in &pending[..1] {
        let week = unreadable(image);
        state
            .meal_record(image, &week, Some("unsupported_horizontal_grid"), NOW)
            .unwrap();
    }
    let before = fake.image_calls();
    let offers = offer_now(&state, &fake, again.path()).await;
    assert_eq!((offers.offered, offers.read), (1, 1));
    assert_eq!(fake.image_calls(), before + 1);
}

#[tokio::test]
async fn a_post_that_is_not_one_image_or_not_an_image_is_never_offered() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    let folder = tempfile::tempdir().unwrap();
    post(
        &state,
        1,
        "11월 23일 ~ 11월 29일 식단",
        "2026-11-19",
        &["a", "b"],
    );
    post(
        &state,
        2,
        "11월 30일 ~ 12월 6일 식단",
        "2026-11-24",
        &["page"],
    );
    fake.serve(&image_path("page"), b"<html>", "text/html");
    let offers = offer_now(&state, &fake, folder.path()).await;
    assert_eq!(
        (offers.offered, offers.skipped, offers.failed.len()),
        (0, 1, 1)
    );
    assert!(files(folder.path()).is_empty());
    assert!(state.meal_offered(OCR_VERSION).unwrap().is_empty());

    // robots.txt applies to the image like to any page.
    let blocked = Fake::new();
    blocked.serve(
        "/robots.txt",
        b"User-agent: *\nDisallow: /CrossEditor/\n",
        "text/plain",
    );
    blocked.serve(&image_path("page"), &jpeg("page"), "image/jpeg");
    let offers = offer_now(&state, &blocked, folder.path()).await;
    assert_eq!(offers.failed.len(), 1);
    assert_eq!(blocked.image_calls(), 0);

    // One image posted for two weeks is offered for the newer post only.
    let twice = State::open_in_memory().unwrap();
    post(
        &twice,
        1,
        "11월 23일 ~ 11월 29일 식단",
        "2026-11-19",
        &["same"],
    );
    post(
        &twice,
        2,
        "11월 30일 ~ 12월 6일 식단",
        "2026-11-24",
        &["copy"],
    );
    let fake = Fake::new();
    fake.serve(&image_path("same"), &jpeg("same"), "image/jpeg");
    fake.serve(&image_path("copy"), &jpeg("same"), "image/jpeg");
    let folder = tempfile::tempdir().unwrap();
    let offers = offer_now(&twice, &fake, folder.path()).await;
    assert_eq!((offers.offered, offers.skipped), (1, 1));
    assert_eq!(
        twice.meal_offered(OCR_VERSION).unwrap()[0].week,
        "2026-11-30"
    );
}

#[tokio::test]
async fn a_post_corrected_to_another_week_is_read_again() {
    let state = State::open_in_memory().unwrap();
    let fake = Fake::new();
    fake.serve(&image_path("menu"), &jpeg("menu"), "image/jpeg");
    // The title names the wrong week; the banner check fails for it.
    post(
        &state,
        1,
        "11월 23일 ~ 11월 29일 식단",
        "2026-11-24",
        &["menu"],
    );
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(offer_now(&state, &fake, folder.path()).await.offered, 1);
    let image = state.meal_offered(OCR_VERSION).unwrap().remove(0);
    let week = unreadable(&image);
    state
        .meal_record(&image, &week, Some("banner_mismatch"), NOW)
        .unwrap();
    // The same title and image are read once.
    let again = tempfile::tempdir().unwrap();
    let offers = offer_now(&state, &fake, again.path()).await;
    assert_eq!((offers.offered, offers.read), (0, 1));

    // The school corrects the title: the image is read for the new week.
    post(
        &state,
        1,
        "11월 30일 ~ 12월 6일 식단",
        "2026-11-24",
        &["menu"],
    );
    let corrected = tempfile::tempdir().unwrap();
    let offers = offer_now(&state, &fake, corrected.path()).await;
    assert_eq!((offers.offered, offers.read), (1, 0));
    // One row per image URL: the new week replaces the old one, pending.
    let pending = state.meal_offered(OCR_VERSION).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].week, "2026-11-30");
}

fn offered(external_id: &str, week: &str, published: &str) -> Offered {
    Offered {
        image_url: format!("{HOST}{}", image_path(external_id)),
        source_id: SOURCE_ID.into(),
        external_id: external_id.into(),
        week: week.into(),
        published_at: published.into(),
        image_sha256: sha256_hex(external_id),
    }
}

/// An accepted result for the week of 2026-11-23 as meal_ocr.py writes it.
fn accepted(sha: &str) -> serde_json::Value {
    let cell =
        |items: &[&str]| serde_json::json!({"status": "available", "items": items, "time": null});
    let headers: Vec<Vec<String>> = (0..7)
        .map(|i| vec![format!("11월 {}일", 23 + i), format!("{}요일", WEEKDAYS[i])])
        .collect();
    let days: Vec<serde_json::Value> = (0..7)
        .map(|i| {
            serde_json::json!({
                "day": format!("2026-11-{}", 23 + i),
                "breakfast_korean": cell(&["전복죽", "동치미"]),
                "breakfast_simple": cell(&["카라멜애플파이"]),
                "cup_rice": {"status": "available", "items": ["돈까스덮밥"], "time": "11:40"},
                "dinner": {"status": "closed", "items": [], "time": null},
            })
        })
        .collect();
    serde_json::json!({
        "version": OCR_VERSION, "image_sha256": sha, "status": "ok", "reason": null,
        "image_size": [3508, 2480],
        "banner": ["주간메뉴표", "2026.11.23~ 11.29"], "headers": headers, "days": days,
        "breakfast_common": cell(&["그린샐러드&드레싱"]), "drinks": cell(&["디톡스워터"]),
    })
}

fn write_result(folder: &Path, image: &Offered, result: &serde_json::Value) {
    std::fs::write(
        folder.join(format!("{}.result.json", image.image_sha256)),
        result.to_string(),
    )
    .unwrap();
}

fn stored_week(state: &State, week: &str) -> Option<(String, MealWeek)> {
    state
        .with(|c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT external_id,record FROM meal_weeks WHERE week=?1",
                [week],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        })
        .unwrap()
        .map(|(id, record)| (id, serde_json::from_str(&record).unwrap()))
}

fn image_status(state: &State, image: &Offered) -> (String, Option<String>) {
    state
        .with(|c| {
            c.query_row(
                "SELECT status,reason FROM meal_images WHERE image_url=?1",
                [&image.image_url],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .unwrap()
}

#[test]
fn an_accepted_result_becomes_the_week() {
    let state = State::open_in_memory().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let image = offered("10", "2026-11-23", "2026-11-18");
    state.meal_offer(&image, OCR_VERSION, NOW).unwrap();
    write_result(folder.path(), &image, &accepted(&image.image_sha256));
    let applied = apply(&state, folder.path(), NOW).unwrap();
    assert_eq!((applied.ok, applied.unreadable, applied.missing), (1, 0, 0));
    let (_, week) = stored_week(&state, "2026-11-23").unwrap();
    assert_eq!(week.status, MealStatus::Ok);
    assert!(!week.low_resolution);
    assert_eq!(week.image_url, image.image_url);
    assert_eq!(week.days.len(), 7);
    assert_eq!(week.days[6].day, "2026-11-29");
    assert_eq!(week.days[1].cup_rice.time.as_deref(), Some("11:40"));
    assert_eq!(week.days[0].dinner.status, MealCell::Closed);
    assert_eq!(week.drinks.unwrap().items, ["디톡스워터"]);
    assert_eq!(image_status(&state, &image), ("ok".into(), None));
    // Recorded: nothing is pending, and a second apply changes nothing.
    assert_eq!(
        apply(&state, folder.path(), NOW).unwrap(),
        Applied::default()
    );
}

#[test]
fn a_small_image_is_read_but_flagged() {
    let image = offered("10", "2026-11-23", "2026-11-18");
    let mut result = accepted(&image.image_sha256);
    result["image_size"] = serde_json::json!([1088, 787]);
    let week = read_result(result.to_string().as_bytes(), &image)
        .unwrap()
        .unwrap();
    assert_eq!((week.status, week.low_resolution), (MealStatus::Ok, true));
    result["image_size"] = serde_json::json!([2000, 1412]);
    let week = read_result(result.to_string().as_bytes(), &image)
        .unwrap()
        .unwrap();
    assert!(!week.low_resolution);
}

#[test]
fn a_rejected_or_failing_result_publishes_the_week_as_unreadable() {
    type Change = Box<dyn Fn(&mut serde_json::Value)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "unsupported_horizontal_grid",
            Box::new(|r| {
                *r = serde_json::json!({"version": OCR_VERSION, "image_sha256": r["image_sha256"],
                "status": "rejected", "reason": "unsupported_horizontal_grid"});
            }),
        ),
        (
            "invalid_result",
            Box::new(|r| r["status"] = "rejected".into()),
        ),
        (
            "invalid_result",
            Box::new(|r| r["banner"] = serde_json::json!(["2026.11.16~11.22"])),
        ),
        (
            "invalid_result",
            Box::new(|r| r["headers"][2][0] = "11월 26일".into()),
        ),
        (
            "invalid_result",
            Box::new(|r| r["headers"][2][1] = "목요일".into()),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"][3]["day"] = "2026-11-27".into()),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"].as_array_mut().unwrap().truncate(6)),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"][0]["dinner"]["items"] = serde_json::json!(["밥"])),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"][0]["cup_rice"]["time"] = "24:00".into()),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"][0]["breakfast_korean"]["items"] = serde_json::json!([" "])),
        ),
        (
            "invalid_result",
            Box::new(|r| r["days"][0]["breakfast_korean"]["items"] = serde_json::json!(["a\u{7}"])),
        ),
        (
            "invalid_result",
            Box::new(|r| {
                r["days"][0]["breakfast_korean"]["items"] = serde_json::json!(vec!["밥"; 21])
            }),
        ),
        (
            "invalid_result",
            Box::new(|r| r["drinks"] = serde_json::Value::Null),
        ),
    ];
    for (index, (reason, change)) in cases.into_iter().enumerate() {
        let state = State::open_in_memory().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let image = offered("10", "2026-11-23", "2026-11-18");
        state.meal_offer(&image, OCR_VERSION, NOW).unwrap();
        let mut result = accepted(&image.image_sha256);
        change(&mut result);
        write_result(folder.path(), &image, &result);
        let applied = apply(&state, folder.path(), NOW).unwrap();
        assert_eq!((applied.ok, applied.unreadable), (0, 1), "case {index}");
        let (_, week) = stored_week(&state, "2026-11-23").unwrap();
        assert_eq!(week, unreadable(&image), "case {index}");
        assert_eq!(
            image_status(&state, &image),
            ("unreadable".into(), Some(reason.into()))
        );
    }
}

#[test]
fn a_result_that_is_not_this_images_leaves_it_pending() {
    let state = State::open_in_memory().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let image = offered("10", "2026-11-23", "2026-11-18");
    state.meal_offer(&image, OCR_VERSION, NOW).unwrap();
    assert_eq!(apply(&state, folder.path(), NOW).unwrap().missing, 1);
    for change in [
        serde_json::json!({"image_sha256": sha256_hex("other")}),
        serde_json::json!({"version": "meal-ocr-0"}),
        serde_json::json!({"extra": true}),
        // Not a cell at all: the result does not parse.
        serde_json::json!({"breakfast_common": {"status": "maybe", "items": [], "time": null}}),
    ] {
        let mut result = accepted(&image.image_sha256);
        for (key, value) in change.as_object().unwrap() {
            result[key] = value.clone();
        }
        write_result(folder.path(), &image, &result);
        let applied = apply(&state, folder.path(), NOW).unwrap();
        assert_eq!(applied.invalid.len(), 1, "{change}");
        assert_eq!(image_status(&state, &image).0, "pending");
    }
    std::fs::write(
        folder
            .path()
            .join(format!("{}.result.json", image.image_sha256)),
        vec![b' '; MAX_RESULT_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(apply(&state, folder.path(), NOW).unwrap().invalid.len(), 1);
    assert!(stored_week(&state, "2026-11-23").is_none());
}

#[test]
fn a_newer_post_replaces_its_week_even_unreadable_and_an_older_one_never_does() {
    let state = State::open_in_memory().unwrap();
    let older = offered("10", "2026-11-23", "2026-11-17");
    let newer = offered("11", "2026-11-23", "2026-11-19");
    let ok = |image: &Offered| {
        let result = accepted(&image.image_sha256);
        read_result(result.to_string().as_bytes(), image)
            .unwrap()
            .unwrap()
    };
    assert!(state.meal_record(&older, &ok(&older), None, NOW).unwrap());
    assert!(state
        .meal_record(
            &newer,
            &unreadable(&newer),
            Some("conflicting_banner_dates"),
            NOW
        )
        .unwrap());
    assert_eq!(stored_week(&state, "2026-11-23").unwrap().0, "11");
    // The older post's image read again later does not bring its text back.
    assert!(!state.meal_record(&older, &ok(&older), None, NOW).unwrap());
    let (id, week) = stored_week(&state, "2026-11-23").unwrap();
    assert_eq!((id.as_str(), week.status), ("11", MealStatus::Unreadable));
}

#[test]
fn posts_of_one_day_are_ordered_by_their_ids_as_numbers() {
    let state = State::open_in_memory().unwrap();
    let older = offered("999", "2026-11-23", "2026-11-17");
    let newer = offered("1000", "2026-11-23", "2026-11-17");
    for image in [&newer, &older] {
        let week = unreadable(image);
        state
            .meal_record(image, &week, Some("banner_mismatch"), NOW)
            .unwrap();
    }
    assert_eq!(stored_week(&state, "2026-11-23").unwrap().0, "1000");
}

#[test]
fn health_counts_unreadable_images_since_the_last_read_one() {
    let state = State::open_in_memory().unwrap();
    assert!(state.feed_meals_health().unwrap().is_empty());
    let first = offered("10", "2026-11-23", "2026-11-17");
    let second = offered("11", "2026-11-30", "2026-11-24");
    let third = offered("12", "2026-12-07", "2026-12-01");
    for image in [&first, &second, &third] {
        state.meal_offer(image, OCR_VERSION, NOW).unwrap();
    }
    let ok = accepted(&first.image_sha256);
    let week = read_result(ok.to_string().as_bytes(), &first)
        .unwrap()
        .unwrap();
    state
        .meal_record(&first, &week, None, "2026-11-20T00:00:00.000Z")
        .unwrap();
    state
        .meal_record(
            &second,
            &unreadable(&second),
            Some("x"),
            "2026-11-21T00:00:00.000Z",
        )
        .unwrap();
    state
        .meal_record(
            &third,
            &unreadable(&third),
            Some("x"),
            "2026-11-25T00:00:00.000Z",
        )
        .unwrap();
    let health = state.feed_meals_health().unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].source_id, SOURCE_ID);
    assert_eq!(
        health[0].last_success_at.as_deref(),
        Some("2026-11-20T00:00:00.000Z")
    );
    assert_eq!(health[0].consecutive_failures, 2);
}

#[test]
fn the_banner_may_pad_the_month_and_the_day_separately() {
    let week = parse_day("2026-03-02").unwrap();
    for banner in [
        "주간메뉴표 2026.3.2 ~ 3.8",
        "주간메뉴표 2026.03.02 ~ 03.08",
        "주간메뉴표 2026.3.02 ~ 3.08",
        "주간메뉴표 2026/03/2 - 03/8",
    ] {
        assert!(banner_names_week(banner, week), "{banner}");
    }
    for banner in [
        "주간메뉴표 2026.3.9 ~ 3.15",
        "주간메뉴표 2026.3.2 ~ 3.18",
        "주간메뉴표 2026.13.2 ~ 3.8",
        "주간메뉴표 3.2 ~ 3.8",
    ] {
        assert!(!banner_names_week(banner, week), "{banner}");
    }
}
