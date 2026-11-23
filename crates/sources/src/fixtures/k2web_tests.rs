use super::*;

fn board(id: &str) -> &'static K2webBoard {
    K2WEB_BOARDS
        .iter()
        .find(|board| board.source_id == id)
        .unwrap()
}
/// An editable copy of a catalogued board, for tampering tests.
fn copy(board: &K2webBoard) -> K2webBoard {
    K2webBoard { ..*board }
}
fn entry_of(page: &ListingPage, external_id: &str) -> ListingEntry {
    page.entries
        .iter()
        .find(|entry| entry.external_id == external_id)
        .unwrap()
        .clone()
}
fn pins(page: &ListingPage) -> usize {
    page.entries.iter().filter(|entry| entry.pinned).count()
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
        let board = board(id);
        let parsed = listing(board, page, list).unwrap();
        assert_eq!(parsed.total_pages, total_pages, "{id}");
        assert!(parsed.has_next, "{id}");
        let entry = entry_of(&parsed, external);
        let article = article(board, &entry, detail).unwrap();
        assert_eq!(article.external_id, external, "{id}");
        assert_eq!(article.title.trim(), article.title, "{id}");
        assert!(!article.title.is_empty(), "{id}");
        assert_eq!(article.published_at, published, "{id}");
        assert_eq!(article.category.as_deref(), Some(board.name));
        assert!(!article.body_html.contains("<script"), "{id}");
        assert!(!article.body_html.contains("<iframe"), "{id}");
        assert!(
            !article.body_text.is_empty() || !article.resources.is_empty(),
            "{id}"
        );
        assert_eq!(
            detail_url(board, &entry).unwrap().as_str(),
            entry.url,
            "{id}"
        );
    }
}

#[test]
fn the_catalog_and_parser_dispatch_use_the_board_table() {
    let definitions = crate::catalog::definitions();
    for board in K2WEB_BOARDS {
        let source = definitions
            .iter()
            .find(|source| source.id == board.source_id)
            .unwrap();
        assert_eq!(source.family, "k2web");
        assert_eq!(source.name, board.name);
        assert_eq!(source.group.as_deref(), Some(board.group));
        assert_eq!(
            super::super::list_url(source, 2).unwrap(),
            list_url(board, 2).unwrap()
        );
    }
    let general = definitions
        .iter()
        .find(|source| source.id == "dorm-general")
        .unwrap();
    let page =
        super::super::listing(general, 1, include_str!("dormitory-general-list.html")).unwrap();
    assert_eq!(page.total_pages, 19);
}

#[test]
fn repeated_pins_do_not_distort_regular_rows_or_duplicate_entries() {
    let general = board("dorm-general");
    let page = listing(general, 1, include_str!("dormitory-general-list.html")).unwrap();
    assert_eq!(pins(&page), 6);
    // The sixth pin is also notice 127 in the regular rows; it stays one entry.
    assert_eq!(page.entries.len(), 12);
    assert!(entry_of(&page, "313348").pinned);
    assert!(!entry_of(&page, "313337").pinned);

    let second = listing(general, 2, include_str!("dormitory-general-page2.html")).unwrap();
    assert_eq!(pins(&second), 6);
    assert_eq!(second.entries.len(), 13);
    assert!(!entry_of(&second, "313349").pinned);

    let admission = board("dorm-admission");
    let page = listing(admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
    assert_eq!(pins(&page), 8);
    assert_eq!(page.entries.len(), 12);
    assert!(entry_of(&page, "376167").pinned);
    assert!(!entry_of(&page, "283604").pinned);

    let menu = board("dorm-menu");
    let page = listing(menu, 1, include_str!("dormitory-menu-list.html")).unwrap();
    assert_eq!(pins(&page), 1);
    assert!(entry_of(&page, "297271").pinned);
    assert!(!entry_of(&page, "374772").pinned);

    let last = listing(menu, 56, include_str!("dormitory-menu-last.html")).unwrap();
    assert!(!last.has_next);
    assert_eq!(last.total_pages, 56);
    assert_eq!(pins(&last), 1);
    assert_eq!(last.entries.len(), 5);
}

#[test]
fn table_boards_of_other_sites_page_through_their_history() {
    // 1,129 notices at 12 rows: 95 pages, eight pins repeated on each page,
    // some of them also among the regular rows.
    let first = listing(
        board("gsot-notice"),
        1,
        include_str!("k2web-gsot-list.html"),
    )
    .unwrap();
    assert_eq!((first.total_pages, first.has_next), (95, true));
    assert_eq!((pins(&first), first.entries.len()), (8, 15));
    let second = listing(
        board("gsot-notice"),
        2,
        include_str!("k2web-gsot-page2.html"),
    )
    .unwrap();
    assert_eq!((pins(&second), second.entries.len()), (8, 17));
    let last = listing(
        board("gsot-notice"),
        95,
        include_str!("k2web-gsot-last.html"),
    )
    .unwrap();
    assert_eq!((last.total_pages, last.has_next), (95, false));
    // Ordinal 1 is the only regular row on the last page.
    assert_eq!((pins(&last), last.entries.len()), (8, 9));

    // 52 pins; nine of the ten regular rows are pins too.
    let gsinfo = listing(
        board("gsinfo-notice"),
        1,
        include_str!("k2web-gsinfo-list.html"),
    )
    .unwrap();
    assert_eq!(
        (gsinfo.total_pages, pins(&gsinfo), gsinfo.entries.len()),
        (59, 52, 53)
    );
    let sociology = listing(
        board("sociology-events"),
        1,
        include_str!("k2web-sociology-list.html"),
    )
    .unwrap();
    assert_eq!(
        (
            sociology.total_pages,
            pins(&sociology),
            sociology.entries.len()
        ),
        (7, 13, 13)
    );
    let sbs = listing(board("sbs-notice"), 1, include_str!("k2web-sbs-list.html")).unwrap();
    assert_eq!(
        (sbs.total_pages, pins(&sbs), sbs.entries.len()),
        (37, 3, 13)
    );
    let science = listing(
        board("science-newsletter"),
        1,
        include_str!("k2web-science-list.html"),
    )
    .unwrap();
    assert_eq!(
        (science.total_pages, pins(&science), science.entries.len()),
        (3, 0, 12)
    );
    let outbound = listing(
        board("outbound-selection"),
        1,
        include_str!("k2web-outbound-list.html"),
    )
    .unwrap();
    assert_eq!(
        (
            outbound.total_pages,
            outbound.has_next,
            outbound.entries.len()
        ),
        (1, false, 10)
    );

    // A wrong page size disagrees with the declared page count.
    let mut wrong = copy(board("gsot-notice"));
    wrong.rows = 10;
    assert!(listing(&wrong, 1, include_str!("k2web-gsot-list.html")).is_err());
}

#[test]
fn gallery_boards_read_cards_and_their_ordinals() {
    let first = listing(
        board("liberalarts-posters"),
        1,
        include_str!("k2web-liberalarts-list.html"),
    )
    .unwrap();
    assert_eq!(
        (first.total_pages, first.has_next, first.entries.len()),
        (24, true, 12)
    );
    assert_eq!(pins(&first), 0);
    assert_eq!(
        first.entries[0].url,
        "https://new-cms.sogang.ac.kr/bbs/liberalarts/1722/425315/artclView.do"
    );
    let second = listing(
        board("liberalarts-posters"),
        2,
        include_str!("k2web-liberalarts-page2.html"),
    )
    .unwrap();
    assert_eq!(second.entries.len(), 12);
    let last = listing(
        board("liberalarts-posters"),
        24,
        include_str!("k2web-liberalarts-last.html"),
    )
    .unwrap();
    assert_eq!((last.has_next, last.entries.len()), (false, 11));

    // The webzine variant: `NO. 5` and labels without classes.
    let innov = listing(
        board("innov-newsletter"),
        1,
        include_str!("k2web-innov-list.html"),
    )
    .unwrap();
    assert_eq!(
        (innov.total_pages, innov.has_next, innov.entries.len()),
        (1, false, 5)
    );

    let list = include_str!("k2web-liberalarts-list.html");
    // The first card's `<li>`; the category tabs above it are list items too.
    let card = list[..list.find("/bbs/liberalarts/1722/425315/").unwrap()]
        .rfind("<li>")
        .unwrap();
    let pinned_card = format!(
        "{}<li class=\"notice\">{}",
        &list[..card],
        &list[card + 4..]
    );
    for changed in [
        list.replacen("No. 286", "No. 285", 1),
        pinned_card,
        list.replacen(
            "<strong>등록일</strong>2026.11.20",
            "<strong>등록일</strong>",
            1,
        ),
        list.replacen(
            "/bbs/liberalarts/1722/423617/artclView.do",
            "/bbs/liberalarts/1723/423617/artclView.do",
            1,
        ),
    ] {
        assert_ne!(changed, list);
        assert!(listing(board("liberalarts-posters"), 1, &changed).is_err());
    }
    // A table board's page is not a gallery, and the other way round.
    let mut table = copy(board("liberalarts-posters"));
    table.skin = Skin::Table;
    assert!(listing(&table, 1, list).is_err());
}

#[test]
fn beyond_the_last_page_is_rejected_even_though_the_site_serves_pins() {
    let general = board("dorm-general");
    assert!(listing(general, 20, include_str!("dormitory-general-list.html")).is_err());
    assert!(listing(general, 0, include_str!("dormitory-general-list.html")).is_err());
    // A different board's page cannot be parsed as this source.
    assert!(listing(general, 1, include_str!("dormitory-admission-list.html")).is_err());
    assert!(listing(
        board("dorm-admission"),
        1,
        include_str!("dormitory-menu-list.html")
    )
    .is_err());
    // Same page title and page size, another site.
    let mut other_site = copy(board("gsot-notice"));
    other_site.site = "gsinfo";
    assert!(listing(&other_site, 1, include_str!("k2web-gsot-list.html")).is_err());
}

#[test]
fn list_urls_and_detail_urls_are_exact() {
    let general = board("dorm-general");
    assert_eq!(
        list_url(general, 3).unwrap().as_str(),
        "https://new-cms.sogang.ac.kr/sgdormitory/6975/subview.do?page=3"
    );
    assert_eq!(
        list_url(board("gsot-notice"), 1).unwrap().as_str(),
        "https://new-cms.sogang.ac.kr/gsot/3208/subview.do?page=1"
    );
    let page = listing(general, 1, include_str!("dormitory-general-list.html")).unwrap();
    let entry = entry_of(&page, "313337");
    assert_eq!(
        detail_url(general, &entry).unwrap().as_str(),
        "https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do"
    );
    let mut other = entry.clone();
    other.url = "https://new-cms.sogang.ac.kr/bbs/sgdormitory/1644/313337/artclView.do".into();
    assert!(detail_url(general, &other).is_err());
    other.url = "https://new-cms.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do?x=1".into();
    assert!(detail_url(general, &other).is_err());
    // The department host name is never requested (user decision, #80).
    other.url = "https://dormitory.sogang.ac.kr/bbs/sgdormitory/2231/313337/artclView.do".into();
    assert!(detail_url(general, &other).is_err());
}

#[test]
fn article_resources_keep_attachments_and_inline_images() {
    let general = board("dorm-general");
    let page = listing(general, 1, include_str!("dormitory-general-list.html")).unwrap();
    let parsed = article(
        general,
        &entry_of(&page, "313348"),
        include_str!("dormitory-general-detail-313348.html"),
    )
    .unwrap();
    assert_eq!(parsed.author.as_deref(), Some("벨라르미노학사"));
    assert!(parsed.resources.is_empty());

    let admission = board("dorm-admission");
    let page = listing(admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
    let parsed = article(
        admission,
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
        admission,
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

    let menu = board("dorm-menu");
    let page = listing(menu, 1, include_str!("dormitory-menu-list.html")).unwrap();
    let parsed = article(
        menu,
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
fn detail_variants_of_other_sites_parse() {
    let entry = |board: &K2webBoard, id: &str| ListingEntry {
        external_id: id.into(),
        url: format!(
            "https://new-cms.sogang.ac.kr/bbs/{}/{}/{id}/artclView.do",
            board.site, board.bbs
        ),
        pinned: false,
        modified_at: None,
        title: None,
    };
    // `분류` is an extra label; the category stays the board name.
    let gsot = article(
        board("gsot-notice"),
        &entry(board("gsot-notice"), "420698"),
        include_str!("k2web-gsot-detail-420698.html"),
    )
    .unwrap();
    assert_eq!(
        (gsot.published_at.as_str(), gsot.author.as_deref()),
        ("2026-11-17", Some("신학대학원"))
    );
    assert_eq!(gsot.category.as_deref(), Some("공지사항"));
    // Shown as `[일반] …`; the category prefix is not part of the title.
    assert_eq!(
        gsot.title,
        "2027학년도 논술전형 논술시험에 따른 학교 출입 통제 안내(11/21토 ~ 11/22일)"
    );
    let detail = include_str!("k2web-gsot-detail-420698.html");
    let label = detail.find("<span>분류</span>").unwrap();
    let wrong_class = format!(
        "{}{}",
        &detail[..label],
        detail[label..].replacen("일반", "학사", 1)
    );
    assert_ne!(wrong_class, detail);
    assert!(article(
        board("gsot-notice"),
        &entry(board("gsot-notice"), "420698"),
        &wrong_class
    )
    .is_err());
    assert!(gsot.resources.is_empty());

    // No `작성자` item.
    let sbs = article(
        board("sbs-notice"),
        &entry(board("sbs-notice"), "425296"),
        include_str!("k2web-sbs-detail-425296.html"),
    )
    .unwrap();
    assert_eq!(
        (sbs.published_at.as_str(), sbs.author),
        ("2026-11-20", None)
    );
    // Shown as `[행사] [판교캠퍼스사업단] …` without a `분류` item; the hidden
    // title (escaped twice) keeps the second bracket as part of the title.
    assert_eq!(
        sbs.title,
        "[판교캠퍼스사업단] \"파워브랜드가 소비자에게 미치는 힘\" 특강 (2026.11.26.(목))"
    );
    assert_eq!(
        sbs.resources[0].url,
        "https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000047/20261120171034423_KF60TXFT.png"
    );

    // `og:url` is the literal `?` on this site; the list form and hidden title
    // still bind the page.
    let detail = include_str!("k2web-sociology-detail-352078.html");
    let sociology = article(
        board("sociology-events"),
        &entry(board("sociology-events"), "352078"),
        detail,
    )
    .unwrap();
    assert_eq!(sociology.published_at, "2026-04-17");
    let mut other_board = copy(board("sociology-events"));
    other_board.bbs = 2779;
    assert!(article(&other_board, &entry(&other_board, "352078"), detail).is_err());

    // Boards without files render no attachment block (3 of 129 census boards).
    let without = detail.replace(
        r#"<div class="attachment">"#,
        r#"<div class="not-attachment">"#,
    );
    assert_ne!(without, detail);
    assert!(article(
        board("sociology-events"),
        &entry(board("sociology-events"), "352078"),
        &without
    )
    .is_ok());

    let science = article(
        board("science-newsletter"),
        &entry(board("science-newsletter"), "347256"),
        include_str!("k2web-science-detail-347256.html"),
    )
    .unwrap();
    assert_eq!(
        science.resources[0].url,
        "https://new-cms.sogang.ac.kr/bbs/science/2740/125186/download.do"
    );

    let gsinfo = article(
        board("gsinfo-notice"),
        &entry(board("gsinfo-notice"), "374796"),
        include_str!("k2web-gsinfo-detail-374796.html"),
    )
    .unwrap();
    assert_eq!(
        gsinfo
            .resources
            .iter()
            .filter(|r| r.kind == "attachment")
            .count(),
        2
    );

    // A poster post: its body is images in the site's own folder.
    let poster = article(
        board("liberalarts-posters"),
        &entry(board("liberalarts-posters"), "425315"),
        include_str!("k2web-liberalarts-detail-425315.html"),
    )
    .unwrap();
    let images: Vec<_> = poster
        .resources
        .iter()
        .filter(|r| r.kind == "image")
        .map(|r| r.url.as_str())
        .collect();
    assert_eq!(
        images,
        [
            "https://new-cms.sogang.ac.kr/sites/liberalarts/atchmnfl/bbs/1722/temp_1795392463736100.png",
            "https://new-cms.sogang.ac.kr/sites/liberalarts/atchmnfl/bbs/1722/temp_1795392463872100.jpg",
        ]
    );

    let innov = article(
        board("innov-newsletter"),
        &entry(board("innov-newsletter"), "405464"),
        include_str!("k2web-innov-detail-405464.html"),
    )
    .unwrap();
    assert_eq!(innov.published_at, "2026-11-17");
    assert_eq!(
        innov
            .resources
            .iter()
            .filter(|r| r.kind == "attachment")
            .count(),
        1
    );
}

#[test]
fn a_login_only_post_is_a_permanent_failure_not_a_missing_notice() {
    let page = include_str!("k2web-outbound-detail-420716.html");
    let entry = ListingEntry {
        external_id: "420716".into(),
        url: "https://new-cms.sogang.ac.kr/bbs/outbound/617/420716/artclView.do".into(),
        pinned: false,
        modified_at: None,
        title: None,
    };
    let error = article(board("outbound-selection"), &entry, page).unwrap_err();
    assert_ne!(error, PARSER_ERROR);
    assert!(error.contains("로그인"), "{error}");
    assert!(!missing(page));
}

#[test]
fn tampered_list_rows_and_article_identity_fail() {
    let general = board("dorm-general");
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
        assert!(listing(general, 1, &changed).is_err());
    }
    let page = listing(general, 1, list).unwrap();
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
        assert!(article(general, &entry, &changed).is_err());
    }
    let admission = board("dorm-admission");
    let page = listing(admission, 1, include_str!("dormitory-admission-list.html")).unwrap();
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
        assert!(article(admission, &entry, &changed).is_err());
    }
}

#[test]
fn the_widget_slot_may_vary_but_site_and_menu_are_bound() {
    // `sgdormitory@@6975@@fnct1`; one census board (sgiss) shows its board as
    // the page's second widget, `fnct2`.
    let general = board("dorm-general");
    let list = include_str!("dormitory-general-list.html");
    let token = "7367646f726d69746f72794040363937354040666e6374";
    assert!(list.contains(&format!("{token}31")));
    let second = list.replace(&format!("{token}31"), &format!("{token}32"));
    assert!(listing(general, 1, &second).is_ok());
    for changed in [
        list.replace(&format!("{token}31"), &format!("{token}30")),
        list.replace(&format!("{token}31"), &format!("{token}313233")),
        list.replace(&format!("{token}31"), token),
        list.replace(
            &format!("{token}31"),
            &format!("{}31", token.to_uppercase()),
        ),
    ] {
        assert!(listing(general, 1, &changed).is_err());
    }
}

#[test]
fn a_link_post_keeps_its_ordinal_but_is_not_this_boards_notice() {
    // Seen on oisa 1718: a card that shows another board's article through
    // the CMS's encoded menu link.
    let list = include_str!("k2web-liberalarts-list.html");
    let original = "/bbs/liberalarts/1722/423617/artclView.do";
    assert_eq!(list.matches(original).count(), 1);
    let linked = |href: &str| list.replace(original, href);
    let page = listing(
        board("liberalarts-posters"),
        1,
        &linked("https://oisa.sogang.ac.kr/oisa/1386/subview.do?enc=Zm5jdDF8QEB8"),
    )
    .unwrap();
    assert_eq!(page.entries.len(), 11);
    assert!(page
        .entries
        .iter()
        .all(|entry| entry.external_id != "423617"));
    for href in [
        "https://example.com/oisa/1386/subview.do?enc=Zm5jdDF8QEB8",
        "https://oisa.sogang.ac.kr/oisa/1386/subview.do?enc=Zm5j&x=1",
        "https://oisa.sogang.ac.kr/oisa/1386/subview.do",
        "http://oisa.sogang.ac.kr/oisa/1386/subview.do?enc=Zm5jdDF8QEB8",
        "https://oisa.sogang.ac.kr/oisa/1386/list.do?enc=Zm5jdDF8QEB8",
    ] {
        assert!(
            listing(board("liberalarts-posters"), 1, &linked(href)).is_err(),
            "{href}"
        );
    }
}

/// The collector skips a known notice while its list title equals the archived
/// one (#80 step 3), so a list row must show exactly the title the article
/// page yields, including a board whose category sits beside the title.
#[test]
fn list_titles_match_the_archived_article_titles() {
    let cases: [(&K2webBoard, &str, &str, &str); 10] = [
        (
            board("dorm-general"),
            include_str!("dormitory-general-list.html"),
            include_str!("dormitory-general-detail-313348.html"),
            "313348",
        ),
        (
            board("dorm-admission"),
            include_str!("dormitory-admission-list.html"),
            include_str!("dormitory-admission-detail-283698.html"),
            "283698",
        ),
        (
            board("dorm-menu"),
            include_str!("dormitory-menu-list.html"),
            include_str!("dormitory-menu-detail-374772.html"),
            "374772",
        ),
        (
            board("gsot-notice"),
            include_str!("k2web-gsot-list.html"),
            include_str!("k2web-gsot-detail-420698.html"),
            "420698",
        ),
        (
            board("sbs-notice"),
            include_str!("k2web-sbs-list.html"),
            include_str!("k2web-sbs-detail-425296.html"),
            "425296",
        ),
        (
            board("science-newsletter"),
            include_str!("k2web-science-list.html"),
            include_str!("k2web-science-detail-347256.html"),
            "347256",
        ),
        (
            board("gsinfo-notice"),
            include_str!("k2web-gsinfo-list.html"),
            include_str!("k2web-gsinfo-detail-374796.html"),
            "374796",
        ),
        (
            board("sociology-events"),
            include_str!("k2web-sociology-list.html"),
            include_str!("k2web-sociology-detail-352078.html"),
            "352078",
        ),
        (
            board("liberalarts-posters"),
            include_str!("k2web-liberalarts-list.html"),
            include_str!("k2web-liberalarts-detail-425315.html"),
            "425315",
        ),
        (
            board("innov-newsletter"),
            include_str!("k2web-innov-list.html"),
            include_str!("k2web-innov-detail-405464.html"),
            "405464",
        ),
    ];
    for (board, list, detail, external) in cases {
        let page = listing(board, 1, list).unwrap();
        assert!(
            page.entries.iter().all(|entry| entry
                .title
                .as_deref()
                .is_some_and(|title| !title.is_empty())),
            "{}",
            board.site
        );
        let entry = entry_of(&page, external);
        let archived = article(board, &entry, detail).unwrap();
        assert_eq!(
            entry.title.as_deref(),
            Some(archived.title.as_str()),
            "{}",
            board.site
        );
    }
}
