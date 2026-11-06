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
fn entry(id: &str, board: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!("https://se.sogang.ac.kr/board/board_view.php?board_id={board}&no={id}"),
        pinned: false,
        modified_at: None,
    }
}
fn ids(page: &ListingPage) -> Vec<&str> {
    page.entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect()
}

#[test]
fn catalog_names_match_the_live_board_labels() {
    for (id, board, title) in [
        ("se-notices", "notice", "학과 공지사항"),
        ("se-graduate", "notice_master", "대학원 공지사항"),
        ("se-news", "news", "학과소식"),
        ("se-careers", "recruit", "채용/홍보"),
        ("se-industry", "notice_industry", "산학연 공지사항"),
    ] {
        let source = source(id);
        assert!(source.supported);
        assert_eq!(source.family, "semiconductor");
        assert_eq!(source.name, title);
        assert_eq!(
            list_url(&source, 1).unwrap().as_str(),
            format!("https://se.sogang.ac.kr/board/board_list.php?board_id={board}&page=1")
        );
    }
}

#[test]
fn list_and_detail_urls_are_exact_and_bound_to_the_board() {
    let notices = source("se-notices");
    assert_eq!(
        list_url(&notices, 2).unwrap().as_str(),
        "https://se.sogang.ac.kr/board/board_list.php?board_id=notice&page=2"
    );
    assert!(list_url(&notices, 0).is_err());
    let notice_entry = entry("39", "notice");
    assert_eq!(
        detail_url(&notices, &notice_entry).unwrap().as_str(),
        "https://se.sogang.ac.kr/board/board_view.php?board_id=notice&no=39"
    );
    let mut wrong = notice_entry.clone();
    wrong.url = "https://se.sogang.ac.kr/board/board_view.php?board_id=news&no=39".into();
    assert!(detail_url(&notices, &wrong).is_err());
    let mut paged = notice_entry.clone();
    paged.url = "https://se.sogang.ac.kr/board/board_view.php?board_id=notice&no=39&page=1".into();
    assert!(detail_url(&notices, &paged).is_err());
    let mut zero = notice_entry;
    zero.external_id = "0".into();
    zero.url = "https://se.sogang.ac.kr/board/board_view.php?board_id=notice&no=0".into();
    assert!(detail_url(&notices, &zero).is_err());
    assert!(detail_url(&source("se-news"), &entry("39", "notice")).is_err());
}

#[test]
fn notice_paging_keeps_repeated_pins_and_the_descending_ordinals() {
    let notices = source("se-notices");
    let first = listing(&notices, 1, include_str!("semiconductor-notice-list.html")).unwrap();
    assert_eq!(first.total_pages, 3);
    assert!(first.has_next);
    assert_eq!(
        ids(&first),
        [
            "34", "33", "31", "28", "18", "17", "15", "39", "38", "37", "36", "35", "32", "30",
            "29", "27", "24"
        ]
    );
    assert!(first.entries[..7].iter().all(|entry| entry.pinned));
    assert!(first.entries[7..].iter().all(|entry| !entry.pinned));
    assert_eq!(
        first.entries[0].url,
        "https://se.sogang.ac.kr/board/board_view.php?board_id=notice&no=34"
    );

    let second = listing(&notices, 2, include_str!("semiconductor-notice-page2.html")).unwrap();
    assert_eq!(second.total_pages, 3);
    assert!(second.has_next);
    assert_eq!(
        second.entries[..7]
            .iter()
            .map(|entry| entry.external_id.as_str())
            .collect::<Vec<_>>(),
        first.entries[..7]
            .iter()
            .map(|entry| entry.external_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        second.entries[7..]
            .iter()
            .map(|entry| entry.external_id.as_str())
            .collect::<Vec<_>>(),
        ["23", "22", "21", "20", "19", "16", "14", "12", "11", "10"]
    );

    let last = listing(&notices, 3, include_str!("semiconductor-notice-last.html")).unwrap();
    assert_eq!(last.total_pages, 3);
    assert!(!last.has_next);
    assert_eq!(
        ids(&last),
        ["34", "33", "31", "28", "18", "17", "15", "9", "8", "7", "5", "4"]
    );
    assert!(listing(&notices, 4, include_str!("semiconductor-notice-last.html")).is_err());
}

#[test]
fn every_board_has_a_scoped_listing() {
    for (id, fixture, entries, pins, pages, next) in [
        (
            "se-graduate",
            include_str!("semiconductor-graduate-list.html"),
            12,
            5,
            1,
            false,
        ),
        (
            "se-news",
            include_str!("semiconductor-news-list.html"),
            10,
            0,
            2,
            true,
        ),
        (
            "se-careers",
            include_str!("semiconductor-careers-list.html"),
            10,
            0,
            1,
            false,
        ),
        (
            "se-industry",
            include_str!("semiconductor-industry-list.html"),
            7,
            1,
            1,
            false,
        ),
    ] {
        let page = listing(&source(id), 1, fixture).unwrap();
        assert_eq!(page.entries.len(), entries, "{id}");
        assert_eq!(
            page.entries.iter().filter(|entry| entry.pinned).count(),
            pins,
            "{id}"
        );
        assert_eq!(page.total_pages, pages, "{id}");
        assert_eq!(page.has_next, next, "{id}");
    }
    let news = source("se-news");
    let last = listing(&news, 2, include_str!("semiconductor-news-last.html")).unwrap();
    assert_eq!(ids(&last), ["6", "5", "3"]);
    assert!(!last.has_next);
}

#[test]
fn notices_keep_attachments_and_reject_data_uri_editor_images() {
    let notices = source("se-notices");
    let page = listing(&notices, 1, include_str!("semiconductor-notice-list.html")).unwrap();
    let entry = entry_of(&page, "36");
    let parsed = article(
        &notices,
        &entry,
        include_str!("semiconductor-notice-detail-36.html"),
    )
    .unwrap();
    assert_eq!(parsed.title, "TE605 출입 권한 등록 안내");
    assert_eq!(parsed.published_at, "2026-09-17");
    assert_eq!(parsed.category.as_deref(), Some("학과 공지사항"));
    assert_eq!(parsed.author, None);
    assert_eq!(parsed.modified_at, None);
    assert!(!parsed.unsupported_body);
    assert!(parsed.body_text.contains("출입권한 등록방법"));
    assert_eq!(parsed.resources.len(), 1);
    let attachment = &parsed.resources[0];
    assert_eq!(attachment.kind, "attachment");
    assert_eq!(
        attachment.filename,
        "출입권한신청서 TE605 (반도체공학과 학부생 전용).xlsx"
    );
    assert!(attachment
        .url
        .starts_with("https://se.sogang.ac.kr/board/filedown.php?filepath="));
    let query: Vec<_> = Url::parse(&attachment.url)
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    assert_eq!(query.len(), 2);
    assert_eq!(query[0].0, "filepath");
    assert!(!query[0].1.is_empty());
    assert_eq!(query[1].0, "filename");
    assert_eq!(query[1].1, attachment.filename);

    let plain = entry_of(&page, "39");
    let parsed = article(
        &notices,
        &plain,
        include_str!("semiconductor-notice-detail-39.html"),
    )
    .unwrap();
    assert_eq!(parsed.published_at, "2026-10-12");
    assert!(parsed.resources.is_empty());
    assert!(parsed.body_text.contains("연구실 안전교육"));

    // A base64 data URI is not a fetchable resource; the surrounding text stays.
    let mutated = include_str!("semiconductor-notice-detail-39.html").replace(
        "<div class=\"view_con\">",
        "<div class=\"view_con\"><img src=\"data:image/png;base64,iVBORw0KGgo=\">",
    );
    let parsed = article(&notices, &plain, &mutated).unwrap();
    assert!(parsed.resources.is_empty());
    assert!(!parsed.body_html.contains("data:image"));
    assert!(!parsed.body_text.is_empty());
}

#[test]
fn news_records_same_host_and_blocked_external_images() {
    let news = source("se-news");
    let page = listing(&news, 1, include_str!("semiconductor-news-list.html")).unwrap();
    let entry = entry_of(&page, "10");
    let parsed = article(
        &news,
        &entry,
        include_str!("semiconductor-news-detail-10.html"),
    )
    .unwrap();
    assert_eq!(parsed.published_at, "2026-06-02");
    assert_eq!(parsed.resources.len(), 1);
    assert_eq!(parsed.resources[0].kind, "image");
    assert_eq!(
        parsed.resources[0].url,
        "https://se.sogang.ac.kr/smart_editor/upload/1781851969-601781851969295.png"
    );
    assert!(parsed.body_html.contains("data-resource-url="));
    assert!(!parsed.body_html.contains("<img src="));

    let entry = entry_of(&page, "14");
    let parsed = article(
        &news,
        &entry,
        include_str!("semiconductor-news-detail-14.html"),
    )
    .unwrap();
    assert_eq!(parsed.resources.len(), 3);
    assert!(parsed
        .resources
        .iter()
        .all(|resource| resource.kind == "image"
            && resource
                .url
                .starts_with("https://ee.sogang.ac.kr/se2/photo_uploader/upload/")));
}

#[test]
fn careers_keep_multiple_attachments_and_other_boards_parse_scoped_bodies() {
    let careers = source("se-careers");
    let page = listing(&careers, 1, include_str!("semiconductor-careers-list.html")).unwrap();
    let entry = entry_of(&page, "11");
    let parsed = article(
        &careers,
        &entry,
        include_str!("semiconductor-careers-detail-11.html"),
    )
    .unwrap();
    assert_eq!(parsed.category.as_deref(), Some("채용/홍보"));
    assert_eq!(parsed.resources.len(), 4);
    assert!(parsed
        .resources
        .iter()
        .all(|resource| resource.kind == "attachment" && resource.filename.ends_with(".docx")));
    assert!(parsed.body_text.contains("실전문제연구팀"));

    let graduate = source("se-graduate");
    let page = listing(
        &graduate,
        1,
        include_str!("semiconductor-graduate-list.html"),
    )
    .unwrap();
    let entry = entry_of(&page, "7");
    let parsed = article(
        &graduate,
        &entry,
        include_str!("semiconductor-graduate-detail-7.html"),
    )
    .unwrap();
    assert_eq!(parsed.category.as_deref(), Some("대학원 공지사항"));
    assert_eq!(parsed.published_at, "2026-08-19");
    assert!(parsed.resources.is_empty());

    let industry = source("se-industry");
    let page = listing(
        &industry,
        1,
        include_str!("semiconductor-industry-list.html"),
    )
    .unwrap();
    let entry = entry_of(&page, "7");
    let parsed = article(
        &industry,
        &entry,
        include_str!("semiconductor-industry-detail-7.html"),
    )
    .unwrap();
    assert_eq!(parsed.category.as_deref(), Some("산학연 공지사항"));
    assert!(parsed.body_text.contains("삼성 산학트랙"));
}

#[test]
fn article_identity_and_board_label_are_bound() {
    let notices = source("se-notices");
    let page = listing(&notices, 1, include_str!("semiconductor-notice-list.html")).unwrap();
    let entry = entry_of(&page, "36");
    let body = include_str!("semiconductor-notice-detail-36.html");

    // The requested URL and the served board must agree.
    let mut other = entry.clone();
    other.url = "https://se.sogang.ac.kr/board/board_view.php?board_id=news&no=36".into();
    assert!(article(&notices, &other, body).is_err());
    let news = source("se-news");
    let mut page_entry = entry.clone();
    page_entry.url = "https://se.sogang.ac.kr/board/board_view.php?board_id=news&no=36".into();
    assert!(article(&news, &page_entry, body).is_err());

    for (label, broken) in [
        ("board label", body.replace("학과 공지사항", "학과소식")),
        ("body class", body.replace("view_con", "view_content")),
        (
            "back link",
            body.replace("board_id=notice&page=1", "board_id=news&page=1"),
        ),
        ("date", body.replace("2026-09-17", "2026-02-30")),
        (
            "title class",
            body.replace("h3 class=\"v_tit\"", "h3 class=\"title\""),
        ),
    ] {
        assert!(article(&notices, &entry, &broken).is_err(), "{label}");
    }

    // Attachment links must use the download route and agree with the visible name.
    let mismatched = body.replace(
        "출입권한신청서 TE605 (반도체공학과 학부생 전용).xlsx</a>",
        "다른 파일.xlsx</a>",
    );
    assert!(article(&notices, &entry, &mismatched).is_err());
    let unsupported = body.replace(".xlsx", ".exe");
    assert!(article(&notices, &entry, &unsupported).is_err());
    let external = body.replace("/board/filedown.php", "https://evil.example/filedown.php");
    assert!(article(&notices, &entry, &external).is_err());
    let extra_query = body.replace("&filename=", "&extra=1&filename=");
    assert!(article(&notices, &entry, &extra_query).is_err());
}

#[test]
fn changed_list_structures_fail_closed() {
    let notices = source("se-notices");
    let first = include_str!("semiconductor-notice-list.html");
    for broken in [
        String::new(),
        "<html><p>서비스 오류</p>".into(),
        first.replace("학과 공지사항", "학과소식"),
        first.replace("value=\"notice\"", "value=\"news\""),
        first.replace("<p class=\"num\">25</p>", "<p class=\"num\">24</p>"),
        first.replace(
            "<strong class=\"t-key fw7\">25</strong>",
            "<strong class=\"t-key fw7\">26</strong>",
        ),
        first.replace("<div class=\"paging\">", "<div class=\"pages\">"),
        first.replace(
            "board_view.php?board_id=notice&no=39",
            "board_view.php?board_id=notice&no=38",
        ),
        first.replace(
            "board_view.php?board_id=notice&no=39",
            "https://evil.example/board_view.php?board_id=notice&no=39",
        ),
        first.replace("2026-10-12", "2026-02-30"),
    ] {
        assert!(listing(&notices, 1, &broken).is_err());
    }
    // A duplicate identity inside one page is ambiguous.
    let duplicate = first.replace("board_id=notice&no=39", "board_id=notice&no=38");
    assert!(listing(&notices, 1, &duplicate).is_err());
    // A pins-only empty board is a valid empty listing, not an error page.
    let empty = first
        .replace(
            "<p class=\"fs-sub fw7\">총 게시물 수 <strong class=\"t-key fw7\">25</strong> 건</p>",
            "<p class=\"fs-sub fw7\">총 게시물 수 <strong class=\"t-key fw7\">0</strong> 건</p>",
        )
        .replace(
            &first[first.find("<ul class=\"notice_list\">").unwrap()
                ..first.find("</ul>").unwrap() + "</ul>".len()],
            "<ul class=\"notice_list\"></ul>",
        );
    let page = listing(&notices, 1, &empty).unwrap();
    assert!(page.entries.is_empty());
    assert!(!page.has_next);
}
