use super::*;
use crate::catalog;

fn source(id: &str) -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
}
fn cases() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u32,
    usize,
)> {
    vec![
        (
            "computing-academic",
            include_str!("computing-academic-list.html"),
            include_str!("computing-academic-detail.html"),
            "969",
            "현장실습",
            94,
            15,
        ),
        (
            "computing-graduate",
            include_str!("computing-graduate-list.html"),
            include_str!("computing-graduate-detail.html"),
            "128",
            "교과목",
            13,
            12,
        ),
        (
            "computing-external",
            include_str!("computing-external-list.html"),
            include_str!("computing-external-detail.html"),
            "1927",
            "AI인턴십",
            195,
            12,
        ),
        (
            "computing-updates",
            include_str!("computing-updates-list.html"),
            include_str!("computing-updates-detail.html"),
            "12",
            "경제",
            28,
            14,
        ),
        (
            "computing-career",
            include_str!("computing-career-list.html"),
            include_str!("computing-career-detail.html"),
            "3323",
            "현대자동차",
            341,
            13,
        ),
    ]
}
#[test]
fn catalog_has_exact_stable_source_identities_and_honest_support() {
    let sources = catalog::definitions();
    assert_eq!(sources.len(), 173);
    assert_eq!(sources.iter().filter(|s| s.supported).count(), 173);
    assert_eq!(
        sources.iter().map(|s| &s.id).collect::<HashSet<_>>().len(),
        173
    );
    for s in sources {
        assert!(Url::parse(&s.url).is_ok());
        // The Cybercampus connector reads through its session, not a public list.
        if s.family == "cybercampus" {
            assert!(list_url(&s, 1).is_err());
        } else {
            assert!(list_url(&s, 1).is_ok());
        }
    }
}
#[test]
fn main_list_pagination_and_full_original_article_are_verified() {
    let source = source("sogang-academic");
    let page = listing(&source, 1, include_str!("main-list-exact.json")).unwrap();
    assert_eq!(page.entries.len(), 50);
    assert_eq!(page.total_pages, 32);
    assert!(page.has_next);
    let entry = page
        .entries
        .iter()
        .find(|e| e.external_id == "551637")
        .unwrap();
    assert!(entry.pinned);
    assert!(entry.url.contains("/ko/detail/551637?bbsConfigFk=2"));
    let article = article(&source, entry, include_str!("main-detail.json")).unwrap();
    assert_eq!(article.published_at, "2026-10-19");
    assert_eq!(article.author.as_deref(), Some("학사지원팀"));
    assert_eq!(article.category.as_deref(), Some("성적, 학점인정"));
    assert!(article.body_text.contains("강의평가"));
    assert!(article.body_text.contains("Midterm"));
    assert!(article.original.contains("viewCount"));
    assert!(!article.body_html.contains("viewCount"));
    let second = listing(&source, 2, include_str!("main-page2.json")).unwrap();
    assert_eq!(second.entries.len(), 50);
    assert!(second.has_next);
}
#[test]
fn a_main_notice_with_large_inline_images_is_archived_up_to_sixteen_mib() {
    let main = source("sogang-academic");
    let page = listing(&main, 1, include_str!("main-list-exact.json")).unwrap();
    let entry = page
        .entries
        .iter()
        .find(|e| e.external_id == "551637")
        .unwrap();
    let with_image = |bytes: usize| {
        let mut value: Value = serde_json::from_str(include_str!("main-detail.json")).unwrap();
        let content = value["data"]["content"].as_str().unwrap().to_owned();
        value["data"]["content"] = Value::String(format!(
            "{content}<img src=\"data:image/png;base64,{}\">",
            "A".repeat(bytes)
        ));
        value.to_string()
    };
    let notice = article(&main, entry, &with_image(6 * 1024 * 1024)).unwrap();
    assert!(notice.body_text.contains("강의평가"));
    // The inline image is neither kept nor fetched.
    assert!(!notice.body_html.contains("base64"));
    assert!(notice.resources.iter().all(|r| !r.url.starts_with("data:")));
    assert_eq!(article_limit(&main), 16 * 1024 * 1024);
    assert!(article(&main, entry, &with_image(17 * 1024 * 1024)).is_err());
    // Other boards keep the 4 MiB limit.
    assert_eq!(
        article_limit(&source("computing-academic")),
        4 * 1024 * 1024
    );
}
#[test]
fn other_main_site_boards_share_the_api_with_their_own_board_id() {
    let cases = [
        (
            "sogang-general",
            1,
            include_str!("main-general-list.json"),
            70,
            "551892",
            include_str!("main-general-detail-551892.json"),
            "강사",
            4,
        ),
        (
            "sogang-scholarship",
            141,
            include_str!("main-scholarship-list.json"),
            52,
            "551882",
            include_str!("main-scholarship-detail-551882.json"),
            "국가근로",
            2,
        ),
        (
            "sogang-events",
            142,
            include_str!("main-events-list.json"),
            29,
            "551374",
            include_str!("main-events-detail-551374.json"),
            "부르디외",
            1,
        ),
        (
            "sogang-student",
            34,
            include_str!("main-student-list.json"),
            34,
            "551671",
            include_str!("main-student-detail-551671.json"),
            "인파사고",
            1,
        ),
        (
            "sogang-onestop",
            3,
            include_str!("main-onestop-list.json"),
            7,
            "550212",
            include_str!("main-onestop-detail-550212.json"),
            "근로장학생",
            1,
        ),
        (
            "sogang-onestop",
            3,
            include_str!("main-onestop-list.json"),
            7,
            "550598",
            include_str!("main-onestop-detail-550598.json"),
            "대청소",
            0,
        ),
    ];
    for (id, config, list, pages, external, detail, word, files) in cases {
        let board = source(id);
        assert_eq!(board.family, "main");
        assert!(list_url(&board, 1)
            .unwrap()
            .as_str()
            .contains(&format!("bbsConfigFk={config}&")));
        let page = listing(&board, 1, list).unwrap();
        assert_eq!(page.total_pages, pages, "{id}");
        assert!(page.has_next, "{id}");
        let entry = page
            .entries
            .iter()
            .find(|e| e.external_id == external)
            .unwrap_or_else(|| panic!("{id} lists {external}"));
        // The canonical URL is the site's own detail link for that board.
        // Board 3 has no list page on the site (#315): its link is only the board ID.
        let prefix = format!("https://www.sogang.ac.kr/ko/detail/{external}?bbsConfigFk={config}");
        if id == "sogang-onestop" {
            assert_eq!(entry.url, prefix);
        } else {
            assert!(entry.url.starts_with(&format!("{prefix}&")), "{id}");
        }
        assert!(detail_url(&board, entry)
            .unwrap()
            .as_str()
            .ends_with(&format!("BbsData?pkId={external}")));
        let parsed = article(&board, entry, detail).unwrap();
        assert!(
            parsed.title.contains(word) || parsed.body_text.contains(word),
            "{id}"
        );
        let attachments: Vec<_> = parsed
            .resources
            .iter()
            .filter(|r| r.kind == "attachment")
            .collect();
        assert_eq!(attachments.len(), files, "{id}");
        for resource in attachments {
            assert!(
                resource.url.starts_with(&format!(
                    "https://www.sogang.ac.kr/file-fe-prd/board/{config}/"
                )),
                "{id}: {}",
                resource.url
            );
            assert!(crate::policy::validate_url(&Url::parse(&resource.url).unwrap()).is_ok());
        }
        // Another board's list or detail is never accepted under this source.
        let academic = source("sogang-academic");
        assert!(listing(&academic, 1, list).is_err(), "{id}");
        let foreign = ListingEntry {
            url: main_canonical(&MAIN_BOARDS[0], external).into(),
            ..entry.clone()
        };
        assert!(article(&academic, &foreign, detail).is_err(), "{id}");
        assert!(detail_url(&board, &foreign).is_err(), "{id}");
    }
    // The canonical URLs of the original five boards are unchanged, so archived
    // notices keep theirs.
    for (index, expected) in [
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=2&namepage=AcademicNotice&text=%ED%95%99%EC%82%AC+%EC%A7%80%EC%9B%90&data=%255B%255D&title=%ED%95%99%EB%B6%80+%ED%95%99%EC%82%AC%EA%B3%B5%EC%A7%80&redirect=%2Fko%2Facademic-support%2Fnotices",
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=1&namepage=StoryNotificationGeneral&text=%EC%84%9C%EA%B0%95+Story&title=%EC%9D%BC%EB%B0%98%EA%B3%B5%EC%A7%80&redirect=%2Fko%2Fstory%2Fnotification-general",
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=141&namepage=ScholarshipNotice&text=%ED%95%99%EC%82%AC+%EC%A7%80%EC%9B%90&data=%255B%255D&title=%EC%9E%A5%ED%95%99%EA%B3%B5%EC%A7%80&redirect=%2Fko%2Fscholarship-notice",
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=142&namepage=StoryNotificationEvent&text=%EC%84%9C%EA%B0%95+Story&title=%ED%96%89%EC%82%AC%ED%8A%B9%EA%B0%95&redirect=%2Fko%2Fstory%2Fnotification-event",
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=34&namepage=CampusStudentSupportGeneralNotice&text=%EC%BA%A0%ED%8D%BC%EC%8A%A4+%EC%83%9D%ED%99%9C&redirect=%2Fko%2Fcampus%2Fstudent-support%2Fnotice",
        "https://www.sogang.ac.kr/ko/detail/551637?bbsConfigFk=3",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(main_canonical(&MAIN_BOARDS[index], "551637").as_str(), expected);
    }
}
#[test]
fn computing_five_boards_have_scoped_pins_full_text_metadata_and_real_resources() {
    for (id, list, detail, external, word, pages, count) in cases() {
        let source = source(id);
        let page = listing(&source, 1, list).unwrap();
        assert_eq!(page.entries.len(), count, "{id}");
        assert_eq!(page.total_pages, pages, "{id}");
        let entry = page
            .entries
            .iter()
            .find(|e| e.external_id == external)
            .unwrap();
        let article = article(&source, entry, detail).unwrap();
        assert!(article.title.contains(word), "{id}");
        assert!(article.author.is_some(), "{id}");
        assert!(!article.body_text.is_empty(), "{id}");
        assert!(!article.body_text.contains("이전 고정글"));
        assert!(!article.body_text.contains("COPYRIGHT"));
        assert!(
            article.resources.iter().any(|r| r.kind == "attachment"),
            "{id}"
        );
        assert!(
            article.resources.iter().all(|r| r
                .url
                .starts_with("https://computing.sogang.ac.kr/web/file/")),
            "{id}"
        );
        if id == "computing-academic" {
            assert_eq!(article.category.as_deref(), Some("인공지능학과"));
            assert!(article.resources.iter().any(|r| r
                .url
                .ends_with("community/academicNotice/969/20261013163026_oAY34b.pdf")));
        }
        if id == "computing-updates" || id == "computing-career" {
            assert!(article.resources.iter().any(|r| r.kind == "image"), "{id}");
            assert!(article.body_html.contains("data-resource-url="));
            assert!(!article.body_html.contains("<img src="));
        }
    }
}
#[test]
fn computing_historical_page_two_does_not_confuse_repeating_pins_with_recent_rows() {
    let source = source("computing-academic");
    let first = listing(&source, 1, include_str!("computing-academic-list.html")).unwrap();
    let second = listing(&source, 2, include_str!("computing-academic-page2.html")).unwrap();
    assert_eq!(second.entries.len(), 15);
    assert!(second.has_next);
    let first_regular: HashSet<_> = first
        .entries
        .iter()
        .filter(|e| !e.pinned)
        .map(|e| &e.external_id)
        .collect();
    assert!(second
        .entries
        .iter()
        .filter(|e| !e.pinned)
        .all(|e| !first_regular.contains(&e.external_id)));
    assert_eq!(
        first
            .entries
            .iter()
            .filter(|e| e.pinned)
            .map(|e| &e.external_id)
            .collect::<Vec<_>>(),
        second
            .entries
            .iter()
            .filter(|e| e.pinned)
            .map(|e| &e.external_id)
            .collect::<Vec<_>>()
    );
    assert!(listing(&source, 1, include_str!("computing-academic-page2.html")).is_err());
}
#[test]
fn changed_structures_wrong_identity_and_missing_fields_are_errors_not_empty_success() {
    let source = source("computing-academic");
    let list = include_str!("computing-academic-list.html");
    for broken in [
        String::new(),
        "<html><p>서비스 오류</p>".into(),
        list.replace("board-list-date", "unknown-date"),
        list.replace("board-content-header", "unknown-header"),
    ] {
        assert!(listing(&source, 1, &broken).is_err());
    }
    let entry = ListingEntry {
        external_id: "969".into(),
        url: "https://computing.sogang.ac.kr/ko/community/academicNotice/detail/969".into(),
        pinned: true,
        modified_at: None,
        title: None,
    };
    let detail = include_str!("computing-academic-detail.html");
    assert!(article(
        &source,
        &entry,
        &detail.replace("sg-content tiptap-view", "unknown-body")
    )
    .is_err());
    assert!(article(
        &source,
        &entry,
        &detail.replace("/detail/969", "/detail/970")
    )
    .is_err());
    assert!(article(
        &source,
        &entry,
        &detail.replace("SAVE_FILE_NAME:", "MISSING_NAME:")
    )
    .is_err());
    let main = super::tests::source("sogang-academic");
    let mut value: Value = serde_json::from_str(include_str!("main-list-exact.json")).unwrap();
    value["data"]["list"][0]["configId"] = 3.into();
    assert!(listing(&main, 1, &value.to_string()).is_err());
}
#[test]
fn explicit_empty_boards_and_duplicate_main_pins_are_handled() {
    let source = source("sogang-academic");
    assert!(listing(&source,1,r#"{"statusCode":200,"data":{"total":0,"pages":0,"pageNum":1,"pageSize":50,"hasNextPage":false,"list":[]}}"#).unwrap().entries.is_empty());
    let mut value: Value = serde_json::from_str(include_str!("main-list-exact.json")).unwrap();
    let duplicate = value["data"]["list"][0].clone();
    value["data"]["list"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert_eq!(
        listing(&source, 1, &value.to_string())
            .unwrap()
            .entries
            .len(),
        50
    );
    let source = super::tests::source("computing-academic");
    let empty = r#"<aside class="board-lnb"><a aria-current="page" href="/ko/community/academicNotice/list">학사 공지</a></aside><section class="board-content"><header class="board-content-header"><h2>학사 공지</h2></header><strong></strong><div class="board-toolbar"><strong>0</strong><input id="board-search-keyword" value=""></div><div class="board-list"><table><tbody><tr><td>등록된 게시물이 없습니다.</td></tr></tbody></table></div></section>"#;
    assert!(listing(&source, 1, empty).unwrap().entries.is_empty());
    assert!(listing(
        &source,
        1,
        &empty.replace("등록된 게시물이 없습니다.", "로그인이 필요합니다.")
    )
    .is_err());
}
#[test]
fn iframe_only_main_articles_are_explicitly_empty_but_other_empty_results_fail() {
    let source = source("sogang-academic");
    let entry = ListingEntry {
        external_id: "551681".into(),
        url: main_canonical(&MAIN_BOARDS[0], "551681").to_string(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let mut value: Value = serde_json::from_str(include_str!("main-detail.json")).unwrap();
    value["data"]["pkId"] = 551681.into();
    value["data"]["content"] =
        r#"<iframe src="https://embed.example.test/video"></iframe> <iframe src="/other"></iframe>"#
            .into();
    let archived = article(&source, &entry, &value.to_string()).unwrap();
    assert!(archived.body_html.is_empty());
    assert!(archived.body_text.is_empty());
    assert!(archived.resources.is_empty());
    assert!(archived.original.contains("iframe"));
    assert!(archived.unsupported_body);
    value["data"]["content"] =
        r#"<p>안내 내용</p><iframe src="https://embed.example.test/video"></iframe>"#.into();
    let mixed = article(&source, &entry, &value.to_string()).unwrap();
    assert!(mixed.body_text.contains("안내 내용"));
    assert!(!mixed.body_html.contains("<iframe"));

    for content in [
        "",
        "<script>alert(1)</script>",
        "<script>alert(1)</script><iframe src='https://evil.test'></iframe>",
    ] {
        value["data"]["content"] = content.into();
        assert!(article(&source, &entry, &value.to_string()).is_err());
    }
}
#[test]
fn untrusted_html_cannot_execute_or_fetch_remote_content_and_preserves_structure() {
    let input = r#"<h2 onclick="evil()">장학금</h2><p>컴퓨터 <b>연구</b></p><script>alert(1)</script><svg onload="evil()"><text>bad</text></svg><iframe src="https://evil.test"></iframe><a href="javascript:evil()">위험</a><a href="/ko/test?q=&quot;x&quot;">원문</a><img src="/web/file/editor/image.png" onerror="evil()" alt="공지 그림"><table style="background:url(https://evil.test)"><tr><td colspan="2" onclick="evil()">학사 &amp; AI</td></tr></table>"#;
    let (html, plain, resources) = sanitize(
        input,
        "https://computing.sogang.ac.kr/ko/community/news/detail/12",
    )
    .unwrap();
    for banned in [
        "<script",
        "alert(",
        "javascript:",
        "onerror",
        "onclick",
        "<svg",
        "<iframe",
        "style=",
        "src=",
    ] {
        assert!(!html.contains(banned), "{banned}");
    }
    assert!(html.contains("<h2>장학금</h2>"));
    assert!(html.contains("<td colspan=\"2\">학사 &amp; AI</td>"));
    assert!(html.contains("data-external=\"true\""));
    assert!(html.contains("data-resource-url="));
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].kind, "image");
    assert!(plain.contains("컴퓨터 연구"));
    assert!(!plain.contains("bad"));
}
#[test]
fn computing_news_distinguishes_public_route_from_internal_attachment_parent() {
    let source = source("computing-updates");
    let entry = ListingEntry {
        external_id: "282".into(),
        url: "https://computing.sogang.ac.kr/ko/community/news/detail/282".into(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let body = include_str!("computing-news-detail-282.html");
    for broken in [
        body.replace("SN:C,CD_NM:", "SN:999,CD_NM:"),
        body.replace("PARENT_SEQ:C,FILE_PATH:", "PARENT_SEQ:999,FILE_PATH:"),
        body.replace(
            r#"routePath:"\u002Fko\u002Fcommunity\u002Fnews\u002Fdetail\u002F282""#,
            r#"routePath:"\u002Fko\u002Fcommunity\u002Fnews\u002Fdetail\u002F283""#,
        ),
        body.replace(
            r#"community\u002Fnews\u002F282\u002F"#,
            r#"community\u002Fnews\u002F283\u002F"#,
        ),
    ] {
        assert_ne!(broken, body);
        assert!(article(&source, &entry, &broken).is_err());
    }
    let article = article(&source, &entry, body).unwrap();
    assert_eq!(article.external_id, "282");
    assert!(article.title.contains("정연우"));
    assert_eq!(article.published_at, "2026-10-08");
    assert!(article.resources.iter().any(|r| r
        .url
        .ends_with("community/news/282/20261008140703_n3Cf8M.png")));
}
#[test]
fn main_list_rejects_conflicting_duplicates_invalid_pin_and_truncated_metadata() {
    let source = source("sogang-academic");
    let fixture: Value = serde_json::from_str(include_str!("main-list-exact.json")).unwrap();
    for mutation in 0..5 {
        let mut value = fixture.clone();
        match mutation {
            0 => {
                value["data"]["list"][0]["isTop"] = "unknown".into();
            }
            1 => {
                value["data"]["size"] = 49.into();
            }
            2 => {
                value["data"]["pages"] = 31.into();
            }
            3 => {
                value["data"]["list"].as_array_mut().unwrap().pop();
            }
            _ => {
                let mut duplicate = value["data"]["list"][0].clone();
                duplicate["title"] = "다른 제목".into();
                value["data"]["list"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
        }
        assert!(
            listing(&source, 1, &value.to_string()).is_err(),
            "mutation {mutation}"
        );
    }
}
#[test]
fn an_attachment_also_used_inline_retains_its_image_role() {
    let source = source("sogang-academic");
    let entry = ListingEntry {
        external_id: "551637".into(),
        url: main_canonical(&MAIN_BOARDS[0], "551637").into(),
        pinned: true,
        modified_at: None,
        title: None,
    };
    let mut body: Value = serde_json::from_str(include_str!("main-detail.json")).unwrap();
    let url = "https://www.sogang.ac.kr/file-fe-prd/editor/example.png";
    body["data"]["fileValue1"] = url.into();
    body["data"]["content"] = format!("<p>공지 내용</p><img src=\"{url}\" alt=\"안내\">").into();
    let article = article(&source, &entry, &body.to_string()).unwrap();
    assert_eq!(article.resources.len(), 1);
    assert_eq!(article.resources[0].kind, "image");
}
#[test]
fn nuxt_alias_reader_is_data_only_and_rejects_expressions() {
    let (data,aliases)=nuxt_data(r#"window.__NUXT__=(function(a,b){return {FILE_LIST:[],x:"(safe)"}}("community/news/12/",12));"#).unwrap();
    assert!(data.contains("FILE_LIST"));
    assert_eq!(scalar("a", &aliases).unwrap(), "community/news/12/");
    assert_eq!(scalar("b", &aliases).unwrap(), "12");
    assert!(scalar("fetch(\"https://evil.test\")", &aliases).is_err());
    assert!(nuxt_data("window.__NUXT__={FILE_LIST:[]}").is_err());
    assert!(date("2026-02-30").is_err());
    assert!(date("2026-가나다").is_err());
    assert_eq!(date("2024-02-29").unwrap(), "2024-02-29");
}
#[test]
fn wrapped_iframe_body_is_iframe_only_but_wrapped_text_is_not() {
    let source = source("sogang-academic");
    let entry = ListingEntry {
        external_id: "551377".into(),
        url: main_canonical(&MAIN_BOARDS[0], "551377").to_string(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    // Captured body: an empty paragraph and `div.iframe-wrapper` around the viewer.
    let captured = include_str!("main-detail-551377.json");
    let archived = article(&source, &entry, captured).unwrap();
    assert!(archived.unsupported_body);
    assert!(archived.body_html.is_empty() && archived.body_text.is_empty());
    assert_eq!(archived.resources.len(), 3, "attachments stay referenced");
    assert_eq!(viewer_pdf_resources(&archived).len(), 1);
    let mut value: Value = serde_json::from_str(captured).unwrap();
    for content in [
        "<div class=\"iframe-wrapper\"><iframe src=\"/a\"></iframe><p>안내</p></div>",
        "<div><table><tr><td><iframe src=\"/a\"></iframe></td></tr></table></div>",
        &format!("{}<iframe src=\"/a\"></iframe>", "<div>".repeat(100)),
    ] {
        value["data"]["content"] = content.into();
        let parsed = article(&source, &entry, &value.to_string());
        assert!(
            parsed.is_err() || !parsed.unwrap().unsupported_body,
            "{content}"
        );
    }
}
#[test]
fn deeply_nested_legacy_tables_keep_their_text() {
    let source = source("computing-career");
    let deep = include_str!("computing-career-content-396.html");
    let detail = include_str!("computing-career-detail.html");
    // Captured body of career/396 (about 130 levels deep) inside the captured page.
    let open = detail
        .find(r#"<div class="sg-content tiptap-view" data-v-21a8b56e>"#)
        .unwrap();
    let start = detail[open..].find('>').unwrap() + open + 1;
    let end = detail[start..].find("</div></div></div>").unwrap() + start;
    let page = format!("{}{deep}{}", &detail[..start], &detail[end..]);
    let entry = ListingEntry {
        external_id: "3323".into(),
        url: "https://computing.sogang.ac.kr/ko/community/career/detail/3323".into(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let archived = article(&source, &entry, &page).unwrap();
    for text in ["신입사원 모집", "질문하기", "ymjeon@shinhan.com"] {
        assert!(archived.body_text.contains(text), "{text}");
    }
    assert!(!archived.body_html.contains("<script"));

    // Any depth stays bounded: no recursion past the limit, text kept, node cap kept.
    let base = "https://computing.sogang.ac.kr/ko/community/career/detail/1";
    let nested = format!(
        "{}<b>깊은 본문</b><img src=\"/deep.png\" alt=\"그림\"><script>evil()</script>{}",
        "<div><table><tr><td>".repeat(5000),
        "</td></tr></table></div>".repeat(5000)
    );
    let (html, plain, resources) = sanitize(&nested, base).unwrap();
    assert!(plain.contains("깊은 본문"));
    assert!(!html.contains("evil") && !plain.contains("evil"));
    assert_eq!(resources.len(), 1);
    assert!(sanitize(&"<p>가</p>".repeat(60_000), base).is_err());
}
#[test]
fn pdf_viewer_body_references_its_pdf_and_takes_the_pdf_text() {
    let source = source("sogang-academic");
    let entry = ListingEntry {
        external_id: "551680".into(),
        url: main_canonical(&MAIN_BOARDS[0], "551680").to_string(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let captured = include_str!("main-detail-551680.json");
    let mut archived = article(&source, &entry, captured).unwrap();
    assert!(archived.unsupported_body);
    assert!(archived.body_html.is_empty() && archived.body_text.is_empty());
    let viewer = viewer_pdf_resources(&archived);
    assert_eq!(viewer.len(), 1);
    assert_eq!(viewer[0].kind, "attachment");
    assert_eq!(
        viewer[0].filename,
        "2026학년도 동계계절수업 수강신청 안내_공지(학생용).pdf"
    );
    let url = Url::parse(&viewer[0].url).unwrap();
    assert_eq!(
        (url.host_str(), url.path()),
        (Some("scc.sogang.ac.kr"), "/Download3")
    );
    assert_eq!(archived.resources.len(), 1);

    with_pdf_text(&mut archived, &[]);
    assert!(
        archived.unsupported_body,
        "no text keeps the notice metadata-only"
    );
    let pdf = (
        "안내.pdf".to_string(),
        "1. 수강신청 일정\n<b>본 수강신청</b> 11. 25(수)".to_string(),
    );
    with_pdf_text(&mut archived, &[pdf]);
    assert!(!archived.unsupported_body);
    assert_eq!(
        archived.body_text,
        "1. 수강신청 일정 <b>본 수강신청</b> 11. 25(수)"
    );
    assert!(archived
        .body_html
        .ends_with("<p>1. 수강신청 일정</p><p>&lt;b&gt;본 수강신청&lt;/b&gt; 11. 25(수)</p>"));
    assert!(viewer_pdf_resources(&archived).is_empty());

    // Only the school viewer showing an approved file route is a PDF body.
    let mut value: Value = serde_json::from_str(captured).unwrap();
    let file = "https%3A%2F%2Fscc.sogang.ac.kr%2FDownload3%3FpathStr%3DMTAx%26fileName%3Da.pdf%26gubun%3Dcmsfile";
    for src in [
        format!(
            "https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={}",
            file.replace("scc.sogang", "evil")
        ),
        format!("https://evil.test/pdfviewer/web/viewer.html?file={file}"),
        format!("http://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={file}"),
        format!(
            "https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={}",
            file.replace("a.pdf", "a.hwp")
        ),
        format!("https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={file}&file={file}"),
    ] {
        value["data"]["content"] = format!("<iframe src=\"{src}\"></iframe>").into();
        let parsed = article(&source, &entry, &value.to_string()).unwrap();
        assert!(parsed.unsupported_body, "{src}");
        assert!(parsed.resources.is_empty(), "{src}");
    }
    // A viewer next to readable text is not the body and is not fetched.
    value["data"]["content"] = format!(
        "<p>안내</p><iframe src=\"https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file={file}\"></iframe>"
    )
    .into();
    let mixed = article(&source, &entry, &value.to_string()).unwrap();
    assert!(!mixed.unsupported_body && mixed.resources.is_empty());
}
#[test]
fn only_a_family_specific_answer_reports_a_missing_notice() {
    let board = |id: &str| {
        crate::catalog::definitions()
            .into_iter()
            .find(|source| source.id == id)
            .unwrap()
    };
    // Synthetic pages with the structure each site sends for a missing notice
    // (observed 2026-11-17 with a nonexistent ID; no captured page is committed).
    let computing = r#"<html><script>window.__t={noBoardList:"등록된 게시물이 없습니다."}</script><body><section class="board-content"><header class="board-content-header"><h2>학사 공지</h2></header><div class="board-detail-empty"><p>등록된 게시물이 없습니다.</p> <a href="/ko/community/academicNotice/list">목록</a></div></section></body></html>"#;
    let dormitory = r#"<html><head><title>알림메세지</title></head><body><form id="messageForm" action="/bbs/sgdormitory/2231/artclList.do"><div id="errorMessege" class="outLog"><div class="cont"><p class="txt"> <strong>게시물이(가) 없습니다.</strong> </p></div></div></form></body></html>"#;
    assert!(missing(&board("computing-academic"), 200, computing));
    assert!(missing(&board("dorm-general"), 200, dormitory));
    for id in ["sogang-academic", "eng-general", "me-general"] {
        assert!(missing(&board(id), 404, ""), "{id}");
        assert!(missing(&board(id), 410, ""), "{id}");
        for status in [200, 401, 403, 429, 500, 503] {
            assert!(!missing(&board(id), status, ""), "{id} {status}");
        }
    }
    // The phrase in a script bundle alone, a status other than 200, or the
    // message of another board family is not proof.
    let script_only = computing.replace(
        r#"<div class="board-detail-empty"><p>등록된 게시물이 없습니다.</p>"#,
        "<div>",
    );
    assert!(!missing(&board("computing-academic"), 200, &script_only));
    assert!(!missing(&board("computing-academic"), 404, computing));
    assert!(!missing(&board("dorm-general"), 404, dormitory));
    assert!(!missing(
        &board("dorm-general"),
        200,
        &dormitory.replace("게시물이(가) 없습니다.", "권한이 없습니다.")
    ));
    assert!(!missing(&board("dorm-general"), 200, computing));
    // Generic answers never count on community and semiconductor boards.
    let invalid_access =
        "<script>alert('올바른 접근이 아닙니다.'); location.replace('/');</script>";
    for id in ["ee-general", "sse-notices", "se-notices"] {
        assert!(!missing(&board(id), 404, ""), "{id}");
        assert!(!missing(&board(id), 200, invalid_access), "{id}");
        assert!(!missing(&board(id), 200, dormitory), "{id}");
    }
    // Every archived notice page of these families reads as present.
    for (id, page) in [
        (
            "computing-academic",
            include_str!("computing-academic-detail.html"),
        ),
        (
            "computing-career",
            include_str!("computing-career-detail.html"),
        ),
        (
            "computing-external",
            include_str!("computing-external-detail.html"),
        ),
        (
            "computing-graduate",
            include_str!("computing-graduate-detail.html"),
        ),
        (
            "computing-updates",
            include_str!("computing-updates-detail.html"),
        ),
        (
            "computing-updates",
            include_str!("computing-news-detail-282.html"),
        ),
        (
            "dorm-general",
            include_str!("dormitory-general-detail-313348.html"),
        ),
        (
            "dorm-admission",
            include_str!("dormitory-admission-detail-283602.html"),
        ),
        (
            "dorm-admission",
            include_str!("dormitory-admission-detail-283698.html"),
        ),
        (
            "dorm-menu",
            include_str!("dormitory-menu-detail-374772.html"),
        ),
    ] {
        assert!(!missing(&board(id), 200, page), "{id}");
    }
}
