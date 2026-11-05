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
fn clean(parsed: &ArticleInput) {
    assert!(!parsed.title.is_empty());
    assert!(!parsed.body_html.contains("<script"));
    assert!(!parsed.body_html.contains("style="));
    assert!(!parsed.body_html.contains("<iframe"));
    assert!(!parsed.body_text.contains("COPYRIGHT"));
    assert!(!parsed.body_text.is_empty());
    assert_eq!(parsed.author, None);
    assert!(!parsed.unsupported_body);
}

#[test]
fn nine_boards_parse_official_list_and_detail_pages() {
    let cases: [(&str, &str, u32, &str, &str, &str); 9] = [
        (
            "ee-news",
            include_str!("community-ee-news-list.html"),
            1,
            include_str!("community-ee-news-detail-2865.html"),
            "2865",
            "2026-10-30",
        ),
        (
            "ee-general",
            include_str!("community-ee-general-page2.html"),
            2,
            include_str!("community-ee-general-detail-2867.html"),
            "2867",
            "2026-11-02",
        ),
        (
            "ee-academic",
            include_str!("community-ee-academic-list.html"),
            1,
            include_str!("community-ee-academic-detail-2873.html"),
            "2873",
            "2026-11-05",
        ),
        (
            "ee-seminars",
            include_str!("community-ee-seminars-list.html"),
            1,
            include_str!("community-ee-seminars-detail-2841.html"),
            "2841",
            "2026-10-08",
        ),
        (
            "ee-employment",
            include_str!("community-ee-employment-list.html"),
            1,
            include_str!("community-ee-employment-detail-2872.html"),
            "2872",
            "2026-11-04",
        ),
        (
            "ee-recruit",
            include_str!("community-ee-recruit-list.html"),
            1,
            include_str!("community-ee-recruit-detail-2511.html"),
            "2511",
            "2026-04-07",
        ),
        (
            "sse-notices",
            include_str!("community-sse-notices-list.html"),
            1,
            include_str!("community-sse-notices-detail-198.html"),
            "198",
            "2026-04-30",
        ),
        (
            "sse-news",
            include_str!("community-sse-news-list.html"),
            1,
            include_str!("community-sse-news-detail-233.html"),
            "233",
            "2026-10-26",
        ),
        (
            "sse-seminars",
            include_str!("community-sse-seminars-list.html"),
            1,
            include_str!("community-sse-seminars-detail-231.html"),
            "231",
            "2026-10-08",
        ),
    ];
    for (id, list, page, detail, external, published) in cases {
        let source = source(id);
        let page_data = listing(&source, page, list).unwrap();
        let entry = entry_of(&page_data, external);
        let parsed = article(&source, &entry, detail).unwrap();
        clean(&parsed);
        assert_eq!(parsed.external_id, external, "{id}");
        assert_eq!(parsed.published_at, published, "{id}");
        assert_eq!(parsed.category.as_deref(), Some(source.name.as_str()));
        assert_eq!(parsed.modified_at, None);
        assert_eq!(
            detail_url(&source, &entry).unwrap().as_str(),
            entry.url,
            "{id}"
        );
    }
}
#[test]
fn resources_keep_attachments_and_policy_blocked_images() {
    let general = source("ee-general");
    let page_data = listing(&general, 2, include_str!("community-ee-general-page2.html")).unwrap();
    let parsed = article(
        &general,
        &entry_of(&page_data, "2867"),
        include_str!("community-ee-general-detail-2867.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 3);
    let attachment = parsed
        .resources
        .iter()
        .find(|resource| resource.kind == "attachment")
        .unwrap();
    assert_eq!(
        attachment.url,
        "https://ee.sogang.ac.kr/aseoul_bbs/down.php?d=1&idx=2867"
    );
    assert_eq!(
        attachment.filename,
        "[LG이노텍] 27학년도 서강대 Track 산학장학생 모집분야.pdf"
    );
    let images: Vec<_> = parsed
        .resources
        .iter()
        .filter(|resource| resource.kind == "image")
        .collect();
    assert_eq!(images.len(), 2);
    assert!(images.iter().all(|resource| resource
        .url
        .starts_with("https://ee.sogang.ac.kr/se2/photo_uploader/")));

    let sse = source("sse-notices");
    let page_data = listing(&sse, 1, include_str!("community-sse-notices-list.html")).unwrap();
    let parsed = article(
        &sse,
        &entry_of(&page_data, "198"),
        include_str!("community-sse-notices-detail-198.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 2);
    let attachment = parsed
        .resources
        .iter()
        .find(|resource| resource.kind == "attachment")
        .unwrap();
    // robots.txt disallows /bbs/ on this host; the network policy keeps the
    // reference visible as an unsupported resource instead of fetching it.
    assert_eq!(
        attachment.url,
        "https://sse.sogang.ac.kr/bbs/down.php?d=1&idx=198"
    );
    assert!(parsed
        .resources
        .iter()
        .any(|resource| resource.kind == "image" && resource.url.contains("/se2/")));
}
#[test]
fn pins_span_the_first_pages_of_the_combined_pins_first_order() {
    let general = source("ee-general");
    let first = listing(&general, 1, include_str!("community-ee-general-list.html")).unwrap();
    assert_eq!(first.entries.len(), 10);
    assert!(first.entries.iter().all(|entry| entry.pinned));
    assert_eq!(first.total_pages, 31);
    assert!(first.has_next);
    let second = listing(&general, 2, include_str!("community-ee-general-page2.html")).unwrap();
    assert_eq!(second.entries.len(), 10);
    assert_eq!(
        second.entries.iter().filter(|entry| entry.pinned).count(),
        2
    );
    assert_eq!(second.entries[0].external_id, "1914");
    assert_eq!(second.entries[2].external_id, "2867");
    assert!(!second.entries[2].pinned);

    let sse = source("sse-notices");
    let second = listing(&sse, 2, include_str!("community-sse-notices-page2.html")).unwrap();
    assert_eq!(
        second.entries.iter().filter(|entry| entry.pinned).count(),
        3
    );
    assert_eq!(second.entries[3].external_id, "229");
    assert!(!second.entries[3].pinned);
}
#[test]
fn gallery_last_pages_and_single_page_boards_end_cleanly() {
    let news = source("ee-news");
    let last = listing(&news, 16, include_str!("community-ee-news-last.html")).unwrap();
    assert_eq!(last.entries.len(), 7);
    assert_eq!(last.total_pages, 16);
    assert!(!last.has_next);
    assert!(last.entries.iter().all(|entry| !entry.pinned));

    let seminars = source("sse-seminars");
    let single = listing(
        &seminars,
        1,
        include_str!("community-sse-seminars-list.html"),
    )
    .unwrap();
    assert_eq!(single.entries.len(), 5);
    assert_eq!(single.entries[0].external_id, "231");
    assert_eq!(single.total_pages, 1);
    assert!(!single.has_next);
}
#[test]
fn editor_badge_is_dropped_but_a_real_bracket_title_is_kept() {
    let academic = source("ee-academic");
    let page = listing(
        &academic,
        1,
        include_str!("community-ee-academic-list.html"),
    )
    .unwrap();
    let parsed = article(
        &academic,
        &entry_of(&page, "2873"),
        include_str!("community-ee-academic-detail-2873.html"),
    )
    .unwrap();
    assert_eq!(
        parsed.title,
        "대학원 유고결석계 신규 양식 및 제출 방법 안내"
    );

    let notices = source("sse-notices");
    let page = listing(&notices, 1, include_str!("community-sse-notices-list.html")).unwrap();
    let parsed = article(
        &notices,
        &entry_of(&page, "198"),
        include_str!("community-sse-notices-detail-198.html"),
    )
    .unwrap();
    assert_eq!(
        parsed.title,
        "[공지]학과 설계실(K407,TE704)사용규칙 및 운영시간 안내"
    );
}
#[test]
fn list_urls_use_the_current_routes_and_matching_codes() {
    for (id, expected) in [
        (
            "ee-news",
            "https://ee.sogang.ac.kr/kor/community/notice01.php?pNo=3&code=news",
        ),
        (
            "ee-recruit",
            "https://ee.sogang.ac.kr/kor/recruit/recruit.php?pNo=3&code=recruit",
        ),
        (
            "sse-notices",
            "https://sse.sogang.ac.kr/kor/community/notice.php?pNo=3",
        ),
        (
            "sse-news",
            "https://sse.sogang.ac.kr/kor/community/news.php?pNo=3",
        ),
    ] {
        assert_eq!(list_url(&source(id), 3).unwrap().as_str(), expected, "{id}");
    }
}
#[test]
fn detail_urls_reject_other_hosts_missing_codes_and_extra_parameters() {
    let news = source("ee-news");
    let page = listing(&news, 1, include_str!("community-ee-news-list.html")).unwrap();
    let valid = entry_of(&page, "2865");
    assert!(detail_url(&news, &valid).is_ok());
    for url in [
        "https://www.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&pNo=1&code=news",
        "https://ee.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&pNo=1",
        "https://ee.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&pNo=1&code=general",
        "https://ee.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&pNo=1&code=news&x=1",
        "https://ee.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&idx=2866&pNo=1&code=news",
        "http://ee.sogang.ac.kr/kor/community/notice01.php?m=v&idx=2865&pNo=1&code=news",
    ] {
        let broken = ListingEntry {
            url: url.into(),
            ..valid.clone()
        };
        assert!(detail_url(&news, &broken).is_err(), "{url}");
    }
    let sse = source("sse-news");
    let page = listing(&sse, 1, include_str!("community-sse-news-list.html")).unwrap();
    let valid = entry_of(&page, "233");
    assert!(detail_url(&sse, &valid).is_ok());
    let broken = ListingEntry {
        url: "https://sse.sogang.ac.kr/kor/community/news.php?m=v&idx=233&pNo=1&code=news".into(),
        ..valid
    };
    assert!(detail_url(&sse, &broken).is_err());
}
#[test]
fn wrong_page_wrong_board_and_tampered_rows_are_rejected() {
    let general = source("ee-general");
    assert!(listing(&general, 2, include_str!("community-ee-general-list.html")).is_err());
    assert!(listing(&general, 1, include_str!("community-ee-general-page2.html")).is_err());
    let news = source("ee-news");
    assert!(listing(&news, 1, include_str!("community-ee-news-list.html")).is_ok());
    assert!(listing(&news, 1, include_str!("community-sse-news-list.html")).is_err());
    let tampered = include_str!("community-ee-news-list.html").replacen(
        "idx=2844&amp;pNo=1&amp;code=news",
        "idx=2865&amp;pNo=1&amp;code=news",
        1,
    );
    assert!(listing(&news, 1, &tampered).is_err());
    let tampered =
        include_str!("community-ee-general-list.html").replacen(">2026.11.02<", ">2026.13.02<", 1);
    assert!(listing(&general, 1, &tampered).is_err());
}
#[test]
fn article_rejects_a_different_board_and_a_foreign_attachment_identity() {
    let general = source("ee-general");
    let page = listing(&general, 2, include_str!("community-ee-general-page2.html")).unwrap();
    let entry = entry_of(&page, "2867");
    assert!(article(
        &general,
        &entry,
        include_str!("community-ee-news-detail-2865.html")
    )
    .is_err());
    let tampered = include_str!("community-ee-general-detail-2867.html").replacen(
        "down.php?d=1&amp;idx=2867",
        "down.php?d=1&amp;idx=2869",
        1,
    );
    assert!(article(&general, &entry, &tampered).is_err());
    let academic = source("ee-academic");
    let page = listing(
        &academic,
        1,
        include_str!("community-ee-academic-list.html"),
    )
    .unwrap();
    let academic_entry = entry_of(&page, "2873");
    assert!(article(
        &academic,
        &academic_entry,
        include_str!("community-ee-academic-detail-2873.html")
    )
    .is_ok());
    // The detail list link must return to the same listing page and board code.
    let tampered = include_str!("community-ee-academic-detail-2873.html").replacen(
        "notice03.php?pNo=1&amp;code=academic",
        "notice03.php?pNo=2&amp;code=academic",
        1,
    );
    assert!(article(&academic, &academic_entry, &tampered).is_err());
    let tampered = include_str!("community-sse-notices-detail-198.html").replacen(
        "down.php?d=1&amp;idx=198",
        "down.php?d=0&amp;idx=198",
        1,
    );
    let notices = source("sse-notices");
    let page = listing(&notices, 1, include_str!("community-sse-notices-list.html")).unwrap();
    assert!(article(&notices, &entry_of(&page, "198"), &tampered).is_err());
}
