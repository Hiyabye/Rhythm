use super::{ListingEntry, ListingPage, SourceDefinition, Url};
// The public dispatch, as the collector and the app call it.
use crate::parser::{article, detail_url, list_url, listing, missing};
use crate::{catalog, policy::validate_url};

fn source(id: &str) -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|source| source.id == id)
        .unwrap()
}
fn scec_entry(id: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!(
            "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?smode=view&bbs_idx={id}&bbsid=f_notice"
        ),
        pinned: false,
        modified_at: None,
        title: None,
    }
}
fn klec_entry(id: &str, bbsid: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!("https://klec.sogang.ac.kr/index.php?bbs_idx={id}&pagekind=c&bbsid={bbsid}"),
        pinned: false,
        modified_at: None,
        title: None,
    }
}
fn ids(page: &ListingPage) -> Vec<&str> {
    page.entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect()
}
fn pinned(page: &ListingPage) -> Vec<&str> {
    page.entries
        .iter()
        .filter(|entry| entry.pinned)
        .map(|entry| entry.external_id.as_str())
        .collect()
}
fn allowed(url: &str) -> bool {
    validate_url(&Url::parse(url).unwrap()).is_ok()
}

const SCEC_LIST: &str = include_str!("vendor-scec-notice-list.html");
const SCEC_PAGE3: &str = include_str!("vendor-scec-notice-page3.html");
const SCEC_LAST: &str = include_str!("vendor-scec-notice-last.html");
const SCEC_PAST_END: &str = include_str!("vendor-scec-notice-past-end.html");
const SCEC_FILE: &str = include_str!("vendor-scec-notice-detail-90122.html");
const SCEC_NEWS: &str = include_str!("vendor-scec-notice-detail-89659.html");
const SCEC_OLD: &str = include_str!("vendor-scec-notice-detail-45763.html");
const SCEC_MISSING: &str = include_str!("vendor-scec-notice-missing.html");
const KLEC_LIST: &str = include_str!("vendor-klec-notice-list.html");
const KLEC_LAST: &str = include_str!("vendor-klec-notice-last.html");
const KLEC_PAST_END: &str = include_str!("vendor-klec-notice-past-end.html");
const KLEC_NEWS_LIST: &str = include_str!("vendor-klec-news-list.html");
const KLEC_NEWS_LAST: &str = include_str!("vendor-klec-news-last.html");
const KLEC_NEWS_PAST_END: &str = include_str!("vendor-klec-news-past-end.html");
const KLEC_HWP: &str = include_str!("vendor-klec-notice-detail-2978.html");
const KLEC_ZIP: &str = include_str!("vendor-klec-notice-detail-2951.html");
const KLEC_IMAGES: &str = include_str!("vendor-klec-notice-detail-2933.html");
const KLEC_LINK: &str = include_str!("vendor-klec-notice-detail-2757.html");
const KLEC_OLD: &str = include_str!("vendor-klec-notice-detail-344.html");
const KLEC_UNTITLED: &str = include_str!("vendor-klec-notice-untitled.html");
const KLEC_UNTITLED_ARTICLE: &str = include_str!("vendor-klec-notice-detail-1888.html");
const KLEC_PHOTO: &str = include_str!("vendor-klec-news-detail-2988.html");
const KLEC_WRONG_BOARD: &str = include_str!("vendor-klec-notice-wrong-board-2988.html");
const KLEC_DENIED: &str = include_str!("vendor-klec-notice-denied.html");

#[test]
fn the_catalog_lists_three_vendor_boards_in_their_own_groups() {
    let boards: Vec<_> = catalog::definitions()
        .into_iter()
        .filter(|source| source.family == "vendor")
        .map(|source| (source.id, source.group.unwrap(), source.url))
        .collect();
    assert_eq!(
        boards,
        [
            (
                "scec-notice".to_string(),
                "sogang-scec".to_string(),
                "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php".to_string()
            ),
            (
                "klec-notice".into(),
                "sogang-klec".into(),
                "https://klec.sogang.ac.kr/?url=/dep_04/4110.php".into()
            ),
            (
                "klec-news".into(),
                "sogang-klec".into(),
                "https://klec.sogang.ac.kr/?url=/dep_04/4300.php".into()
            ),
        ]
    );
    let groups = catalog::groups();
    for (id, name) in [
        ("sogang-scec", "미래교육원"),
        ("sogang-klec", "한국어교육원"),
    ] {
        let group = groups.iter().find(|group| group.id == id).unwrap();
        assert_eq!(group.name, name);
        assert_eq!(group.parent.as_deref(), Some("sogang-offices"));
    }
}

#[test]
fn list_and_article_urls_are_exact_and_allowed() {
    let scec = source("scec-notice");
    let klec = source("klec-notice");
    let news = source("klec-news");
    let urls = [
        (
            list_url(&scec, 3).unwrap(),
            "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?pageno=3",
        ),
        (
            list_url(&klec, 24).unwrap(),
            "https://klec.sogang.ac.kr/?url=%2Fdep_04%2F4110.php&pageno=24",
        ),
        (
            list_url(&news, 1).unwrap(),
            "https://klec.sogang.ac.kr/?url=%2Fdep_04%2F4300.php&pageno=1",
        ),
        (
            detail_url(&scec, &scec_entry("90122")).unwrap(),
            "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?smode=view&bbs_idx=90122&bbsid=f_notice",
        ),
        (
            detail_url(&klec, &klec_entry("2978", "00010")).unwrap(),
            "https://klec.sogang.ac.kr/index.php?bbs_idx=2978&pagekind=c&bbsid=00010",
        ),
        (
            detail_url(&news, &klec_entry("2988", "00011")).unwrap(),
            "https://klec.sogang.ac.kr/index.php?bbs_idx=2988&pagekind=c&bbsid=00011",
        ),
    ];
    for (url, expected) in urls {
        assert_eq!(url.as_str(), expected);
        assert!(validate_url(&url).is_ok(), "{url}");
    }
    assert!(allowed("https://scec.sogang.ac.kr/robots.txt"));
    assert!(allowed("https://klec.sogang.ac.kr/robots.txt"));
    // An entry must name its own board's article route.
    assert!(detail_url(&klec, &klec_entry("2988", "00011")).is_err());
    assert!(detail_url(&scec, &scec_entry("0")).is_err());
    let mut extra = scec_entry("90122");
    extra.url.push_str("&pageno=1");
    assert!(detail_url(&scec, &extra).is_err());
}

#[test]
fn policy_binds_every_query_key_and_the_file_routes() {
    for blocked in [
        // Another scec board, a category filter, the admin area.
        "https://scec.sogang.ac.kr/fro_end/html/dep_07/7600.php?pageno=1",
        "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?pageno=1&category=x",
        "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?smode=view&bbs_idx=1&bbsid=notice",
        "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php?smode=write&bbs_idx=1&bbsid=f_notice",
        "https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php",
        "https://scec.sogang.ac.kr/nGmaster/index.php",
        "https://scec.sogang.ac.kr/nGsmart/modules/dn.php?downloadname=a.hwp&filename=b.hwp",
        "https://scec.sogang.ac.kr/nGsmart/modules/dnview.php?downloadname=a.hwp&filename=a.hwp",
        "https://scec.sogang.ac.kr/upload/htmlarea/a.php",
        // klec's 학사공지, another page, other routes and keys.
        "https://klec.sogang.ac.kr/?url=%2Fdep_04%2F4120.php&pageno=1",
        "https://klec.sogang.ac.kr/?url=%2Fdep_01%2F1710.php&pageno=1",
        "https://klec.sogang.ac.kr/index.php?bbs_idx=1&pagekind=c&bbsid=00452",
        "https://klec.sogang.ac.kr/index.php?bbs_idx=1&pagekind=w&bbsid=00010",
        "https://klec.sogang.ac.kr/index.php?bbs_idx=1&pagekind=c&bbsid=00010&gopage=x",
        "https://klec.sogang.ac.kr/univ/lms_bbs/board_comm_ok.php",
        "https://klec.sogang.ac.kr/univ/lms_bbs/dn.php?downloadname=a.exe&filename=a.exe&bbs_idx=1",
        "https://klec.sogang.ac.kr/univ/upload/bbs/a.pdf",
        "http://klec.sogang.ac.kr/univ/upload/bbs/a.jpg",
    ] {
        assert!(!allowed(blocked), "{blocked}");
    }
}

#[test]
fn scec_pins_fill_the_leading_slots_of_the_first_pages() {
    let scec = source("scec-notice");
    let first = listing(&scec, 1, SCEC_LIST).unwrap();
    assert_eq!(
        ids(&first),
        [
            "90262", "90247", "90231", "90174", "90173", "90162", "90161", "90122", "90093",
            "90066"
        ]
    );
    assert_eq!(pinned(&first).len(), 10);
    assert_eq!((first.total_pages, first.has_next), (39, true));
    assert_eq!(first.entries[0].url, scec_entry("90262").url);

    let third = listing(&scec, 3, SCEC_PAGE3).unwrap();
    assert_eq!(
        pinned(&third),
        ["89865", "89578", "89016", "88301", "87922"]
    );
    assert_eq!(
        ids(&third)[5..],
        ["89906", "89905", "89780", "89763", "89759"]
    );

    let last = listing(&scec, 39, SCEC_LAST).unwrap();
    assert_eq!(ids(&last), ["84854", "84834", "45763"]);
    assert!(!last.has_next && pinned(&last).is_empty());
    // The site serves an empty table past the end; the declared total of 383
    // already ends at page 39, and the wrong page number is rejected too.
    assert!(listing(&scec, 40, SCEC_PAST_END).is_err());
    assert!(listing(&scec, 2, SCEC_LIST).is_err());
}

#[test]
fn klec_tables_keep_a_repeated_pin_once_and_count_down_to_one() {
    let klec = source("klec-notice");
    let first = listing(&klec, 1, KLEC_LIST).unwrap();
    assert_eq!(
        ids(&first),
        ["2903", "2978", "2970", "2951", "2933", "2896", "2873", "2863", "2828", "2823"]
    );
    assert_eq!(pinned(&first), ["2903"]);
    assert_eq!((first.total_pages, first.has_next), (24, true));

    let last = listing(&klec, 24, KLEC_LAST).unwrap();
    assert_eq!(ids(&last), ["356", "351", "345", "344"]);
    assert_eq!((last.total_pages, last.has_next), (24, false));
    // Past the end the pager keeps the last block without an active page.
    assert!(listing(&klec, 25, KLEC_PAST_END).is_err());
    assert!(listing(&klec, 2, KLEC_LIST).is_err());
    // The other klec board's page is not this board's list.
    assert!(listing(&klec, 1, KLEC_NEWS_LIST).is_err());
}

#[test]
fn klec_lists_an_untitled_post_whose_article_is_then_refused() {
    let klec = source("klec-notice");
    // Page 14 holds post 1888, which the site shows with an empty title; the
    // page must still parse, or the board's history stops there.
    let page = listing(&klec, 14, KLEC_UNTITLED).unwrap();
    assert_eq!(
        ids(&page),
        ["1903", "1900", "1899", "1898", "1897", "1896", "1895", "1894", "1888", "1868"]
    );
    assert_eq!((page.total_pages, page.has_next), (24, true));
    // The article has no title either, so it is refused as a permanent parse
    // failure, which the collector records as skipped.
    assert!(article(&klec, &klec_entry("1888", "00010"), KLEC_UNTITLED_ARTICLE).is_err());
}

#[test]
fn klec_gallery_reads_twelve_cards_per_page() {
    let news = source("klec-news");
    let first = listing(&news, 1, KLEC_NEWS_LIST).unwrap();
    assert_eq!(first.entries.len(), 12);
    assert_eq!(&ids(&first)[..3], ["2988", "2877", "2876"]);
    assert_eq!((first.total_pages, first.has_next), (18, true));
    assert!(pinned(&first).is_empty());
    let last = listing(&news, 18, KLEC_NEWS_LAST).unwrap();
    assert_eq!(
        ids(&last),
        ["365", "364", "363", "362", "361", "360", "359", "358"]
    );
    assert!(!last.has_next);
    assert!(listing(&news, 19, KLEC_NEWS_PAST_END).is_err());
}

#[test]
fn scec_articles_keep_category_author_files_and_images() {
    let scec = source("scec-notice");
    let file = article(&scec, &scec_entry("90122"), SCEC_FILE).unwrap();
    assert_eq!(file.title, "서강대학교 미래교육원 계약직원 채용공고");
    assert_eq!(file.published_at, "2026-10-30");
    assert_eq!(file.author.as_deref(), Some("행정실"));
    // `전체` is no category: the board name stands in.
    assert_eq!(file.category.as_deref(), Some("공지사항"));
    let resources: Vec<_> = file
        .resources
        .iter()
        .map(|r| (r.kind.as_str(), r.filename.as_str(), r.url.as_str()))
        .collect();
    assert_eq!(
        resources,
        [
            (
                "attachment",
                "미래교육원입사지원서및자기소개서양식.hwp",
                "https://scec.sogang.ac.kr/nGsmart/modules/dn.php?downloadname=%EB%AF%B8%EB%9E%98%EA%B5%90%EC%9C%A1%EC%9B%90%EC%9E%85%EC%82%AC%EC%A7%80%EC%9B%90%EC%84%9C%EB%B0%8F%EC%9E%90%EA%B8%B0%EC%86%8C%EA%B0%9C%EC%84%9C%EC%96%91%EC%8B%9D.hwp&filename=20261030150301.hwp&bbs_idx=90122"
            ),
            (
                "image",
                "202610405414_editor_image.jpg",
                "https://scec.sogang.ac.kr/upload/htmlarea/202610405414_editor_image.jpg"
            ),
        ]
    );

    let news = article(&scec, &scec_entry("89659"), SCEC_NEWS).unwrap();
    assert_eq!(news.category.as_deref(), Some("교육원소식"));
    assert_eq!(news.published_at, "2026-08-05");

    // A 2006 post: no category tag, the same attachment listed twice and a
    // file-view image.
    let old = article(&scec, &scec_entry("45763"), SCEC_OLD).unwrap();
    assert_eq!(old.title, "International Culture Festival");
    assert_eq!(old.category.as_deref(), Some("공지사항"));
    assert_eq!(old.published_at, "2006-11-10");
    assert_eq!(
        old.resources
            .iter()
            .map(|r| r.kind.as_str())
            .collect::<Vec<_>>(),
        ["attachment", "image"]
    );
    for article in [&file, &news, &old] {
        assert!(!article.body_text.is_empty() || !article.resources.is_empty());
        for resource in &article.resources {
            assert!(allowed(&resource.url), "{}", resource.url);
        }
    }
}

#[test]
fn scec_reports_a_gone_post_and_binds_the_article() {
    let scec = source("scec-notice");
    assert!(missing(&scec, 200, SCEC_MISSING));
    assert!(!missing(&scec, 200, SCEC_FILE));
    assert!(!missing(&scec, 404, ""));
    assert!(article(&scec, &scec_entry("99999999"), SCEC_MISSING).is_err());
    // The page names its post in the delete form.
    assert!(article(&scec, &scec_entry("90123"), SCEC_FILE).is_err());
}

#[test]
fn klec_articles_keep_files_and_images() {
    let klec = source("klec-notice");
    let hwp = article(&klec, &klec_entry("2978", "00010"), KLEC_HWP).unwrap();
    assert_eq!(
        hwp.title,
        "서강대학교 한국어교육원 전임강사 채용 공고 (지원마감일 : 2026.10.28 22:00)"
    );
    assert_eq!(hwp.published_at, "2026-10-23");
    assert_eq!(hwp.category.as_deref(), Some("일반공지"));
    assert_eq!(hwp.author, None);
    assert_eq!(
        hwp.resources
            .iter()
            .map(|r| r.filename.as_str())
            .collect::<Vec<_>>(),
        [
            "(양식)한국어교육원 전임강사 입사지원서.hwp",
            "(양식)한국어교육원 추천서.hwp"
        ]
    );
    assert!(hwp.body_text.contains("전임 강사를 채용합니다"));

    let zip = article(&klec, &klec_entry("2951", "00010"), KLEC_ZIP).unwrap();
    assert_eq!(zip.resources.len(), 2);

    let images = article(&klec, &klec_entry("2933", "00010"), KLEC_IMAGES).unwrap();
    assert_eq!(
        images
            .resources
            .iter()
            .map(|r| (r.kind.as_str(), r.url.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "attachment",
                "https://klec.sogang.ac.kr/univ/lms_bbs/dn.php?downloadname=%EC%8A%AC%EB%9D%BC%EC%9D%B4%EB%93%9C1.PNG&filename=YYYY1_1.png&bbs_idx=2933"
            ),
            (
                "attachment",
                "https://klec.sogang.ac.kr/univ/lms_bbs/dn.php?downloadname=%EC%8A%AC%EB%9D%BC%EC%9D%B4%EB%93%9C2.PNG&filename=YYYY2_1.png&bbs_idx=2933"
            ),
            (
                "image",
                "https://klec.sogang.ac.kr/univ/lms_bbs/dnview.php?downloadname=YYYY1_1.png&filename=YYYY1_1.png"
            ),
            (
                "image",
                "https://klec.sogang.ac.kr/univ/lms_bbs/dnview.php?downloadname=YYYY2_1.png&filename=YYYY2_1.png"
            ),
        ]
    );

    // A 2015 post whose images sit under /univ/upload/bbs/ (Korean names).
    let old = article(&klec, &klec_entry("344", "00010"), KLEC_OLD).unwrap();
    assert_eq!(old.published_at, "2015-11-18");
    assert_eq!(old.resources.len(), 2);
    assert!(old.resources.iter().all(|r| r.kind == "image"));

    let photo = article(
        &source("klec-news"),
        &klec_entry("2988", "00011"),
        KLEC_PHOTO,
    )
    .unwrap();
    assert_eq!(photo.title, "DANCE & SONG: SOGANG K-STAR CHALLENGE");
    assert_eq!(photo.category.as_deref(), Some("한국어교육원 소식"));
    for article in [&hwp, &zip, &images, &old, &photo] {
        for resource in &article.resources {
            assert!(allowed(&resource.url), "{}", resource.url);
        }
    }
}

#[test]
fn klec_binds_the_board_and_cannot_tell_missing_from_members_only() {
    let klec = source("klec-notice");
    // The site served the news board's post for a 일반공지 request.
    assert!(article(&klec, &klec_entry("2988", "00010"), KLEC_WRONG_BOARD).is_err());
    assert!(article(&klec, &klec_entry("2979", "00010"), KLEC_HWP).is_err());
    let denied = article(&klec, &klec_entry("99999999", "00010"), KLEC_DENIED).unwrap_err();
    assert!(denied.contains("로그인"), "{denied}");
    assert!(!missing(&klec, 200, KLEC_DENIED));
}

#[test]
fn tampered_list_links_fail() {
    let scec = source("scec-notice");
    let other_board = SCEC_LAST.replacen("bbsid=f_notice", "bbsid=notice", 1);
    assert!(listing(&scec, 39, &other_board).is_err());
    let other_host = SCEC_LAST.replacen(
        "href=\"/fro_end/html/dep_05/5100.php?smode=view",
        "href=\"https://example.com/fro_end/html/dep_05/5100.php?smode=view",
        1,
    );
    assert!(listing(&scec, 39, &other_host).is_err());
    // A pin after a regular row breaks the leading-slot layout.
    // Edited without line endings, which a Windows checkout turns into CRLF.
    let cell = SCEC_PAGE3.find("<td class=\"numb\">354</td>").unwrap();
    let row = SCEC_PAGE3[..cell].rfind("<tr class=\"\">").unwrap();
    let late_pin = format!(
        "{}<tr class=\"bbs-bbs-noti\">{}<td class=\"numb\">공지</td>{}",
        &SCEC_PAGE3[..row],
        &SCEC_PAGE3[row + "<tr class=\"\">".len()..cell],
        &SCEC_PAGE3[cell + "<td class=\"numb\">354</td>".len()..]
    );
    assert_ne!(late_pin, SCEC_PAGE3);
    assert!(listing(&scec, 3, &late_pin).is_err());
    let klec = source("klec-notice");
    let other_board = KLEC_LAST.replacen("bbsid=00010&keyword", "bbsid=00011&keyword", 1);
    assert!(listing(&klec, 24, &other_board).is_err());
    // A skipped ordinal means a row is missing.
    let gap = KLEC_LAST.replacen(
        "<td data-label=\"번호\">3</td>",
        "<td data-label=\"번호\">9</td>",
        1,
    );
    assert!(listing(&klec, 24, &gap).is_err());
}

#[test]
fn a_klec_link_block_joins_the_body_and_keeps_the_files() {
    let klec = source("klec-notice");
    let notice = article(&klec, &klec_entry("2757", "00010"), KLEC_LINK).unwrap();
    assert_eq!(
        notice.title,
        "서강대학교판교캠퍼스 방문 후기 SNS 이벤트(5/8일까지)"
    );
    assert_eq!(
        notice
            .resources
            .iter()
            .filter(|r| r.kind == "attachment")
            .map(|r| r.filename.as_str())
            .collect::<Vec<_>>(),
        ["0423 sns 이벤트(한국어).png", "0423 sns 이벤트(중국어).png"]
    );
    // The body names the account only as text; the block adds the link.
    assert!(notice
        .body_html
        .contains("<a href=\"https://www.instagram.com/sogangsdic/\""));
    // Another block title is not a known layout.
    let unknown = KLEC_LINK.replacen(">링크<", ">관련 자료<", 1);
    assert_ne!(unknown, KLEC_LINK);
    assert!(article(&klec, &klec_entry("2757", "00010"), &unknown).is_err());
}
