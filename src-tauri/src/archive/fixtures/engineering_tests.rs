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
)> {
    vec![
        (
            "eng-general",
            include_str!("eng-notice-list.html"),
            include_str!("eng-notice-detail-723.html"),
            "723",
            "연구활동",
        ),
        (
            "eng-academic",
            include_str!("eng-academic-list.html"),
            include_str!("eng-academic-detail-4.html"),
            "4",
            "수강허가",
        ),
        (
            "eng-careers",
            include_str!("eng-recruit-list.html"),
            include_str!("eng-recruit-detail-676.html"),
            "676",
            "장학생",
        ),
        (
            "eng-research",
            include_str!("eng-research-list.html"),
            include_str!("eng-research-detail-728.html"),
            "728",
            "연구",
        ),
    ]
}
#[test]
fn all_four_boards_parse_real_list_detail_metadata_and_resources() {
    for (id, list, detail, external, word) in cases() {
        let source = source(id);
        let page = listing(&source, 1, list).unwrap();
        let entry = page
            .entries
            .iter()
            .find(|e| e.external_id == external)
            .unwrap();
        let value = article(&source, entry, detail).unwrap();
        assert!(value.title.contains(word), "{id}: {}", value.title);
        assert!(!value.body_text.is_empty());
        assert_eq!(value.author.as_deref(), Some("서강대학교 공과대학"));
        assert_eq!(value.category.as_deref(), Some(source.name.as_str()));
        assert!(!value.body_text.contains("COPYRIGHT"));
        assert!(!value.body_html.contains("style="));
        if id == "eng-academic" {
            assert_eq!(value.published_at, "2026-04-22");
            assert_eq!(value.resources.len(), 3);
            assert_eq!(
                value.resources[0].url,
                "https://eng.sogang.ac.kr/download/708"
            );
            assert_eq!(value.resources[0].filename, "첨부1.수강허가서.hwp");
            assert!(!page.has_next);
        }
        if id == "eng-careers" {
            assert!(value
                .resources
                .iter()
                .any(|r| r.kind == "image" && r.url.contains("/files/")));
        }
        if id == "eng-general" {
            assert!(value.body_html.contains("<table>"));
            assert_eq!(page.total_pages, 36);
            assert_eq!(page.entries.iter().filter(|e| e.pinned).count(), 5);
        }
    }
}
#[test]
fn first_pinned_notice_is_an_article_not_a_navigation_or_empty_record() {
    let source = source("eng-general");
    let page = listing(&source, 1, include_str!("eng-notice-list.html")).unwrap();
    let entry = &page.entries[0];
    assert_eq!(entry.external_id, "20");
    assert!(entry.pinned);
    let article = article(&source, entry, include_str!("eng-notice-detail-20.html")).unwrap();
    assert!(article.title.contains("택배"));
    assert!(article.body_text.contains("참고"));
    assert!(article.resources.iter().any(|r| r.kind == "image"
        && r.url
            .ends_with("/files/f2Mh0ewpzYZ3qSoHOJ9Og/image-940818-1.png")));
    assert_eq!(article.published_at, "2026-09-14");
}
#[test]
fn historical_page_two_omits_first_page_pins_and_advances_regular_rows() {
    let source = source("eng-general");
    let a = listing(&source, 1, include_str!("eng-notice-list.html")).unwrap();
    let b = listing(&source, 2, include_str!("eng-notice-page2.html")).unwrap();
    assert_eq!(a.entries.iter().filter(|e| e.pinned).count(), 5);
    assert_eq!(b.entries.iter().filter(|e| e.pinned).count(), 0);
    assert_eq!(b.total_pages, 36);
    let first: HashSet<_> = a
        .entries
        .iter()
        .filter(|e| !e.pinned)
        .map(|e| &e.external_id)
        .collect();
    assert!(b
        .entries
        .iter()
        .filter(|e| !e.pinned)
        .all(|e| !first.contains(&e.external_id)));
    assert!(listing(&source, 1, include_str!("eng-notice-page2.html")).is_err());
}
#[test]
fn changed_identity_pagination_ordinals_and_resource_links_fail_explicitly() {
    let source = source("eng-academic");
    let list = include_str!("eng-academic-list.html");
    for bad in [
        list.replace("board/academic", "board/research"),
        list.replace("<b>5</b>", "<b>6</b>"),
        list.replace("c-no\">5", "c-no\">4"),
        list.replace("value=\"\"", "value=\"장학\""),
        "<p>서비스 오류</p>".into(),
    ] {
        assert_ne!(bad, list);
        assert!(listing(&source, 1, &bad).is_err());
    }
    let entry = listing(&source, 1, list).unwrap().entries.remove(0);
    let detail = include_str!("eng-academic-detail-4.html");
    for bad in [
        detail.replace("/board/academic/4", "/board/academic/5"),
        detail.replace("/download/708", "https://evil.test/download/708"),
        detail.replace("/download/708", "/download/../708"),
        detail.replace("post-body", "unknown-body"),
        detail.replace("2026.04.22 15:50", "2026.04.21 15:50"),
        detail.replace("첨부파일 <b>3</b>", "첨부파일 <b>4</b>"),
    ] {
        assert_ne!(bad, detail);
        assert!(article(&source, &entry, &bad).is_err());
    }
}
#[test]
fn utc_to_korea_cross_checks_midnight_leap_year_and_invalid_times() {
    for (utc, visible, expected) in [
        ("2026-12-31T15:30:00.000Z", "2027.01.01 00:30", "2027-01-01"),
        ("2024-02-28T23:59:59.000Z", "2024.02.29 08:59", "2024-02-29"),
        ("2026-02-28T15:00:00.000Z", "2026.03.01 00:00", "2026-03-01"),
    ] {
        let html = Html::parse_fragment(&format!("<time datetime=\"{utc}\">{visible}</time>"));
        assert_eq!(
            published(one(html.root_element(), "time").unwrap()).unwrap(),
            expected
        );
    }
    for (utc, visible) in [
        ("2026-04-22T24:00:00.000Z", "2026.04.23 09:00"),
        ("2026-04-22T06:60:00.000Z", "2026.04.22 15:60"),
        ("2026-02-30T06:50:00.000Z", "2026.02.30 15:50"),
        ("2026-04-22T06:50:00.000+09:00", "2026.04.22 15:50"),
        ("2026-04-22T06:50:00.000Z", "2026.04.22 06:50"),
    ] {
        let html = Html::parse_fragment(&format!("<time datetime=\"{utc}\">{visible}</time>"));
        assert!(published(one(html.root_element(), "time").unwrap()).is_err());
    }
}
