use super::*;
use crate::archive::catalog;

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
    assert_eq!(sources.len(), 32);
    assert_eq!(sources.iter().filter(|s| s.supported).count(), 10);
    assert_eq!(
        sources.iter().map(|s| &s.id).collect::<HashSet<_>>().len(),
        32
    );
    for s in sources {
        assert!(Url::parse(&s.url).is_ok());
        if !s.supported {
            assert!(list_url(&s, 1).is_err());
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
        url: main_canonical("551637").into(),
        pinned: true,
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
