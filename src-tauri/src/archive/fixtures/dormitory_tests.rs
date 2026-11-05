use super::*;
use crate::archive::catalog;

fn source(id: &str) -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|source| source.id == id)
        .unwrap()
}
fn entry_of(page: &ListingPage, external_id: &str) -> ListingEntry {
    page.entries
        .iter()
        .find(|entry| entry.external_id == external_id)
        .unwrap()
        .clone()
}

#[test]
fn three_boards_parse_official_list_and_detail_pages() {
    let cases = [
        (
            "dorm-general",
            include_str!("dormitory-general-list.html"),
            1,
            19,
            include_str!("dormitory-general-detail-313348.html"),
            "313348",
            "2026-03-26",
        ),
        (
            "dorm-admission",
            include_str!("dormitory-admission-list.html"),
            1,
            41,
            include_str!("dormitory-admission-detail-283698.html"),
            "283698",
            "2023-12-29",
        ),
        (
            "dorm-menu",
            include_str!("dormitory-menu-list.html"),
            1,
            56,
            include_str!("dormitory-menu-detail-374772.html"),
            "374772",
            "2026-11-02",
        ),
    ];
    for (id, list, page, total_pages, detail, external, published) in cases {
        let source = source(id);
        let parsed = listing(&source, page, list).unwrap();
        assert_eq!(parsed.total_pages, total_pages, "{id}");
        assert!(parsed.has_next, "{id}");
        let entry = entry_of(&parsed, external);
        let article = article(&source, &entry, detail).unwrap();
        assert_eq!(article.external_id, external, "{id}");
        assert_eq!(article.title.trim(), article.title, "{id}");
        assert!(!article.title.is_empty(), "{id}");
        assert_eq!(article.published_at, published, "{id}");
        assert_eq!(article.category.as_deref(), Some(source.name.as_str()));
        assert!(!article.body_html.contains("<script"), "{id}");
        assert!(!article.body_html.contains("<iframe"), "{id}");
        assert!(
            !article.body_text.is_empty() || !article.resources.is_empty(),
            "{id}"
        );
        assert_eq!(
            detail_url(&source, &entry).unwrap().as_str(),
            entry.url,
            "{id}"
        );
    }
}

#[test]
fn repeated_pins_do_not_distort_regular_rows_or_duplicate_entries() {
    let general = source("dorm-general");
    let page = listing(&general, 1, include_str!("dormitory-general-list.html")).unwrap();
    assert_eq!(page.entries.iter().filter(|e| e.pinned).count(), 6);
    // The sixth pin is also notice 127 in the regular rows; it stays one entry.
    assert_eq!(page.entries.len(), 12);
    assert!(entry_of(&page, "313348").pinned);
    assert!(!entry_of(&page, "313337").pinned);

    let second = listing(&general, 2, include_str!("dormitory-general-page2.html")).unwrap();
    assert_eq!(second.entries.iter().filter(|e| e.pinned).count(), 6);
    assert_eq!(second.entries.len(), 13);
    assert!(!entry_of(&second, "313349").pinned);

    let admission = source("dorm-admission");
    let page = listing(&admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
    assert_eq!(page.entries.iter().filter(|e| e.pinned).count(), 8);
    assert_eq!(page.entries.len(), 12);
    assert!(entry_of(&page, "376167").pinned);
    assert!(!entry_of(&page, "283604").pinned);

    let parsed = article(
        &admission,
        &entry_of(&page, "283602"),
        include_str!("dormitory-admission-detail-283602.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 4);
    assert!(parsed.resources.iter().all(|r| r.kind == "image"));
    assert_eq!(
        parsed.resources[0].url,
        "http://scc.sogang.ac.kr/dataview_sg/board/1182/20260831_0921_5610001.jpg"
    );

    let menu = source("dorm-menu");
    let page = listing(&menu, 1, include_str!("dormitory-menu-list.html")).unwrap();
    assert_eq!(page.entries.iter().filter(|e| e.pinned).count(), 1);
    assert!(entry_of(&page, "297271").pinned);
    assert!(!entry_of(&page, "374772").pinned);

    let last = listing(&menu, 56, include_str!("dormitory-menu-last.html")).unwrap();
    assert!(!last.has_next);
    assert_eq!(last.total_pages, 56);
    assert_eq!(last.entries.iter().filter(|e| e.pinned).count(), 1);
    assert_eq!(last.entries.len(), 5);
}

#[test]
fn beyond_the_last_page_is_rejected_even_though_the_site_serves_pins() {
    let general = source("dorm-general");
    assert!(listing(&general, 20, include_str!("dormitory-general-list.html")).is_err());
    assert!(listing(&general, 0, include_str!("dormitory-general-list.html")).is_err());
    // A different board's page cannot be parsed as this source.
    assert!(listing(&general, 1, include_str!("dormitory-admission-list.html")).is_err());
    assert!(listing(
        &source("dorm-admission"),
        1,
        include_str!("dormitory-menu-list.html")
    )
    .is_err());
}

#[test]
fn list_urls_and_detail_urls_are_exact() {
    let general = source("dorm-general");
    assert_eq!(
        list_url(&general, 3).unwrap().as_str(),
        "https://new-cms.sogang.ac.kr/sgdormitory/6975/subview.do?page=3"
    );
    let page = listing(&general, 1, include_str!("dormitory-general-list.html")).unwrap();
    let entry = entry_of(&page, "313337");
    assert_eq!(
        detail_url(&general, &entry).unwrap().as_str(),
        "https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do"
    );
    let mut other = entry.clone();
    other.url = "https://new-cms.sogang.ac.kr/bbs/sgdormitory/1644/313337/artclView.do".into();
    assert!(detail_url(&general, &other).is_err());
    other.url = "https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do?x=1".into();
    assert!(detail_url(&general, &other).is_err());
}

#[test]
fn article_resources_keep_attachments_and_inline_images() {
    let general = source("dorm-general");
    let page = listing(&general, 1, include_str!("dormitory-general-list.html")).unwrap();
    let parsed = article(
        &general,
        &entry_of(&page, "313348"),
        include_str!("dormitory-general-detail-313348.html"),
    )
    .unwrap();
    assert_eq!(parsed.author.as_deref(), Some("벨라르미노학사"));
    assert!(parsed.resources.is_empty());

    let admission = source("dorm-admission");
    let page = listing(&admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
    let parsed = article(
        &admission,
        &entry_of(&page, "283698"),
        include_str!("dormitory-admission-detail-283698.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 9);
    assert!(parsed.resources.iter().all(|r| r.kind == "attachment"));
    assert_eq!(
        parsed.resources[0].url,
        "https://new-cms.sogang.ac.kr/bbs/sgdormitory/1644/90922/download.do"
    );

    let parsed = article(
        &admission,
        &entry_of(&page, "283602"),
        include_str!("dormitory-admission-detail-283602.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 4);
    assert!(parsed.resources.iter().all(|r| r.kind == "image"));
    assert_eq!(
        parsed.resources[0].url,
        "http://scc.sogang.ac.kr/dataview_sg/board/1182/20260831_0921_5610001.jpg"
    );

    let menu = source("dorm-menu");
    let page = listing(&menu, 1, include_str!("dormitory-menu-list.html")).unwrap();
    let parsed = article(
        &menu,
        &entry_of(&page, "374772"),
        include_str!("dormitory-menu-detail-374772.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 1);
    assert_eq!(parsed.resources[0].kind, "image");
    assert_eq!(
        parsed.resources[0].url,
        "https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000035/20261102092906954_POL76LWU.png"
    );
}

#[test]
fn tampered_list_rows_and_article_identity_fail() {
    let general = source("dorm-general");
    let list = include_str!("dormitory-general-list.html");
    for changed in [
        list.replacen("<strong>133</strong>", "<strong>134</strong>", 1),
        list.replacen("_totPage\">19", "_totPage\">20", 1),
        list.replacen(
            "name=\"layout\" value=\"7367",
            "name=\"layout\" value=\"7368",
            1,
        ),
        list.replacen(
            "/bbs/sgdormitory/2231/313339/artclView.do",
            "/bbs/sgdormitory/2231/313337/artclView.do",
            1,
        ),
    ] {
        assert!(listing(&general, 1, &changed).is_err());
    }
    let page = listing(&general, 1, list).unwrap();
    let entry = entry_of(&page, "313348");
    let detail = include_str!("dormitory-general-detail-313348.html");
    for changed in [
        detail.replace(
            "content=\"https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313348/artclView.do\"",
            "content=\"https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do\"",
        ),
        detail.replace(
            "action=\"/bbs/sgdormitory/2231/artclList.do\"",
            "action=\"/bbs/sgdormitory/1644/artclList.do\"",
        ),
        detail.replace(
            "id=\"artclViewTitle\" value=\"벨라르미노 학사 택배 및 우편물 수령 수칙 안내\"",
            "id=\"artclViewTitle\" value=\"다른 공지\"",
        ),
    ] {
        assert!(article(&general, &entry, &changed).is_err());
    }
    let admission = source("dorm-admission");
    let page = listing(&admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
    let entry = entry_of(&page, "283698");
    let detail = include_str!("dormitory-admission-detail-283698.html");
    for changed in [
        detail.replace(
            "/bbs/sgdormitory/1644/90922/download.do",
            "/bbs/sgdormitory/2231/90922/download.do",
        ),
        detail.replacen(
            "<div class=\"attachment\">",
            "<div class=\"attachment\"><span class=\"no-file\">none</span>",
            1,
        ),
    ] {
        assert!(article(&admission, &entry, &changed).is_err());
    }
}
