use super::{download_key, DownloadKey, ListingEntry, ListingPage, SourceDefinition, Url};
use base64::{engine::general_purpose::STANDARD, Engine};
// The public dispatch, as the collector and the app call it.
use crate::parser::{article, detail_url, list_url, listing, missing};
use crate::{catalog, policy::validate_url};

fn source(id: &str) -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|source| source.id == id)
        .unwrap()
}
fn entry(host: &str, config: u32, site: &str, id: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!(
            "https://{host}/front/cmsboardview.do?bbsConfigFK={config}&siteId={site}&pkid={id}"
        ),
        pinned: false,
        modified_at: None,
        title: None,
    }
}
fn scc(config: u32, id: &str) -> ListingEntry {
    entry("scc.sogang.ac.kr", config, "sweducenter", id)
}
fn convedu(config: u32, id: &str) -> ListingEntry {
    entry("convedu.sogang.ac.kr", config, "convedu", id)
}
fn ids(page: &ListingPage) -> Vec<&str> {
    page.entries
        .iter()
        .map(|entry| entry.external_id.as_str())
        .collect()
}
fn allowed(url: &str) -> bool {
    validate_url(&Url::parse(url).unwrap()).is_ok()
}
/// A `pathStr` the way the site writes it.
fn path_str(pkid: &str, config: &str) -> String {
    let text: String = format!("pkid|{pkid}#!#index|1#!#filePath|{config}")
        .chars()
        .rev()
        .collect();
    let codes: Vec<String> = text.bytes().map(|b| b.to_string()).collect();
    STANDARD.encode(codes.join("##"))
}
const NOTICE_LIST: &str = include_str!("cms-sweducenter-notice-list.html");
const NOTICE_LAST: &str = include_str!("cms-sweducenter-notice-last.html");
const NOTICE_PAST_END: &str = include_str!("cms-sweducenter-notice-past-end.html");
const ONE_FILE: &str = include_str!("cms-sweducenter-notice-detail-941077.html");
const TWO_FILES: &str = include_str!("cms-sweducenter-notice-detail-941521.html");
const PRESS_LIST: &str = include_str!("cms-sweducenter-press-list.html");
const PRESS: &str = include_str!("cms-sweducenter-press-detail-939214.html");
const CONVEDU_LIST: &str = include_str!("cms-convedu-notice-list.html");
const CONVEDU_IMAGE: &str = include_str!("cms-convedu-notice-detail-941911.html");
const CONVEDU_NEWS_LIST: &str = include_str!("cms-convedu-news-list.html");
const CONVEDU_NEWS: &str = include_str!("cms-convedu-news-detail-911675.html");

#[test]
fn the_catalog_has_five_cms_boards_and_their_urls_are_allowed() {
    let boards = [
        ("sweducenter-notice", "공지사항", "sogang-sweducenter", 7250),
        ("sweducenter-news", "소식", "sogang-sweducenter", 7251),
        ("sweducenter-press", "언론보도", "sogang-sweducenter", 7252),
        ("convedu-notice", "공지사항", "sogang-convedu", 5551),
        ("convedu-news", "센터소식", "sogang-convedu", 5550),
    ];
    for (id, name, group, config) in boards {
        let source = source(id);
        assert_eq!(
            (
                source.name.as_str(),
                source.group.as_deref(),
                source.family.as_str()
            ),
            (name, Some(group), "cms")
        );
        let (host, site) = if group == "sogang-convedu" {
            ("convedu.sogang.ac.kr", "convedu")
        } else {
            ("scc.sogang.ac.kr", "sweducenter")
        };
        assert_eq!(
            source.url,
            format!("https://{host}/front/cmsboardlist.do?siteId={site}&bbsConfigFK={config}")
        );
        let list = list_url(&source, 3).unwrap();
        assert_eq!(list.as_str(), format!("{}&currentPage=3", source.url));
        let detail = detail_url(&source, &entry(host, config, site, "941077")).unwrap();
        assert_eq!(
            detail.as_str(),
            format!(
                "https://{host}/front/cmsboardview.do?bbsConfigFK={config}&siteId={site}&pkid=941077"
            )
        );
        for url in [&list, &detail] {
            assert!(validate_url(url).is_ok(), "{url}");
        }
        assert!(allowed(&format!("https://{host}/robots.txt")));
    }
    assert_eq!(
        source("convedu-news").organization,
        "서강대학교 · 융합교육원"
    );
    assert_eq!(
        source("sweducenter-notice").organization,
        "서강대학교 · SW중심대학사업단"
    );
    let notice = source("sweducenter-notice");
    // An entry must name its own board's article route.
    assert!(detail_url(&notice, &scc(7252, "941077")).is_err());
    assert!(detail_url(&notice, &convedu(7250, "941077")).is_err());
    assert!(detail_url(&notice, &scc(7250, "0")).is_err());
}

#[test]
fn policy_binds_each_site_to_its_own_boards_and_every_query_key() {
    let file = |host: &str, key: &str, name: &str, gubun: &str| {
        format!("https://{host}/Download?pathStr={key}&fileName={name}&gubun={gubun}")
    };
    let scc_key = path_str("941077", "7250");
    let convedu_key = path_str("941689", "5551");
    assert!(allowed(&file(
        "scc.sogang.ac.kr",
        &scc_key,
        "a.hwp",
        "board"
    )));
    assert!(allowed(&file(
        "convedu.sogang.ac.kr",
        &convedu_key,
        "a.pdf",
        "board"
    )));
    for blocked in [
        // Another site's board, an unknown board, the wrong siteId.
        "https://scc.sogang.ac.kr/front/cmsboardlist.do?siteId=sweducenter&bbsConfigFK=5551&currentPage=1".to_string(),
        "https://scc.sogang.ac.kr/front/cmsboardlist.do?siteId=sweducenter&bbsConfigFK=7249&currentPage=1".into(),
        "https://scc.sogang.ac.kr/front/cmsboardlist.do?siteId=convedu&bbsConfigFK=7250&currentPage=1".into(),
        "https://convedu.sogang.ac.kr/front/cmsboardview.do?bbsConfigFK=7250&siteId=sweducenter&pkid=1".into(),
        // Missing, extra and search keys.
        "https://scc.sogang.ac.kr/front/cmsboardlist.do?siteId=sweducenter&bbsConfigFK=7250".into(),
        "https://scc.sogang.ac.kr/front/cmsboardlist.do?siteId=sweducenter&bbsConfigFK=7250&currentPage=1&searchValue=x".into(),
        "https://scc.sogang.ac.kr/front/cmsboardview.do?bbsConfigFK=7250&siteId=sweducenter&pkid=1&currentPage=1".into(),
        "https://scc.sogang.ac.kr/front/cmsboardview.do?bbsConfigFK=7250&siteId=sweducenter&pkid=0".into(),
        // Other routes: writing, the message page, the robots-disallowed area.
        "https://scc.sogang.ac.kr/front/cmsboardwrite.do?bbsConfigFK=7250&siteId=sweducenter".into(),
        "https://scc.sogang.ac.kr/gomsg.do?msgCode=BBS002".into(),
        "https://scc.sogang.ac.kr/cgsi/index.do".into(),
        // Files: another site's key, a broken key, other kinds and names.
        file("scc.sogang.ac.kr", &convedu_key, "a.pdf", "board"),
        file("convedu.sogang.ac.kr", &scc_key, "a.pdf", "board"),
        file("scc.sogang.ac.kr", &path_str("941077", "7249"), "a.pdf", "board"),
        file("scc.sogang.ac.kr", "MTAx", "a.pdf", "board"),
        file("scc.sogang.ac.kr", &scc_key, "a.pdf", "cmsfile"),
        file("scc.sogang.ac.kr", &scc_key, "a.exe", "board"),
        file("scc.sogang.ac.kr", &scc_key, "..%2Fa.pdf", "board"),
        format!("{}&x=1", file("scc.sogang.ac.kr", &scc_key, "a.pdf", "board")),
        // Article images stay unsupported on both hosts.
        "https://scc.sogang.ac.kr/dataview/board/5551/20261015_1644_0810001.png".into(),
        "http://convedu.sogang.ac.kr/front/cmsboardlist.do?siteId=convedu&bbsConfigFK=5551&currentPage=1".into(),
    ] {
        assert!(!allowed(&blocked), "{blocked}");
    }
    // The PDF-viewer route the main boards use is unchanged.
    assert!(allowed(
        "https://scc.sogang.ac.kr/Download3?pathStr=MTAxIyMxMDg%3D&fileName=%EC%95%88%EB%82%B4.pdf&gubun=cmsfile"
    ));
}

#[test]
fn download_keys_decode_the_sites_encoding() {
    // Copied from the 941077 fixture's attachment link.
    assert_eq!(
        download_key("NDgjIzUzIyM1MCMjNTUjIzEyNCMjMTA0IyMxMTYjIzk3IyM4MCMjMTAxIyMxMDgjIzEwNSMjMTAyIyMzNSMjMzMjIzM1IyM0OSMjMTI0IyMxMjAjIzEwMSMjMTAwIyMxMTAjIzEwNSMjMzUjIzMzIyMzNSMjNTUjIzU1IyM0OCMjNDkjIzUyIyM1NyMjMTI0IyMxMDAjIzEwNSMjMTA3IyMxMTI="),
        Some(DownloadKey {
            pkid: "941077".into(),
            config: 7250,
        })
    );
    for broken in [
        String::new(),
        "not base64!".into(),
        STANDARD.encode("112##107##105"),
        STANDARD.encode("pkid|1#!#index|1#!#filePath|7250"),
        path_str("0", "7250"),
        path_str("1", "x"),
    ] {
        assert_eq!(download_key(&broken), None, "{broken}");
    }
}

#[test]
fn lists_put_pins_first_and_check_the_running_numbers() {
    let notice = source("sweducenter-notice");
    let first = listing(&notice, 1, NOTICE_LIST).unwrap();
    assert_eq!(
        ids(&first)[..6],
        ["941512", "941077", "940008", "927525", "941521", "941338"]
    );
    assert_eq!(first.entries.len(), 19);
    assert_eq!(first.entries.iter().filter(|e| e.pinned).count(), 4);
    assert_eq!((first.total_pages, first.has_next), (8, true));
    assert_eq!(first.entries[4].url, scc(7250, "941521").url);

    let last = listing(&notice, 8, NOTICE_LAST).unwrap();
    assert_eq!(
        ids(&last),
        ["916581", "916293", "916027", "914412", "914411", "912697", "912355", "911807"]
    );
    assert!(!last.has_next && last.entries.iter().all(|e| !e.pinned));
    // Past the end the site marks the last page as current and lists nothing.
    assert!(listing(&notice, 99, NOTICE_PAST_END).is_err());
    assert!(listing(&notice, 2, NOTICE_LIST).is_err());
    // Another board's page, even of the same site.
    assert!(listing(&source("sweducenter-press"), 1, NOTICE_LIST).is_err());
    assert!(listing(&source("convedu-notice"), 1, NOTICE_LIST).is_err());
    // A gap in the running numbers means rows went missing.
    let gap = NOTICE_LIST.replacen(">112</div>", ">111</div>", 1);
    assert_ne!(gap, NOTICE_LIST);
    assert!(listing(&notice, 1, &gap).is_err());
    // The last page must end at row 1.
    let row_one = NOTICE_LAST.find("12px;\">1</div>").unwrap();
    let start = NOTICE_LAST[..row_one].rfind("<li>").unwrap();
    let end = row_one + NOTICE_LAST[row_one..].find("</li>").unwrap() + "</li>".len();
    let early = format!("{}{}", &NOTICE_LAST[..start], &NOTICE_LAST[end..]);
    assert!(listing(&notice, 8, &early).is_err());
    // The pager must mark the requested page as current.
    let moved = NOTICE_LIST
        .replacen(
            "<li class=\"on\"><a><span>1</span></a></li>",
            "<li><a href=\"javascript:void(0);\" onclick=\"sendPage('pageForm','1');return false\"><span>1</span></a></li>",
            1,
        )
        .replacen(
            "<li><a href=\"javascript:void(0);\" onclick=\"sendPage('pageForm','2');return false\"><span>2</span></a></li>",
            "<li class=\"on\"><a><span>2</span></a></li>",
            1,
        );
    assert_eq!(moved.matches("class=\"on\"><a><span>2<").count(), 1);
    assert!(listing(&notice, 1, &moved).is_err());

    let press = listing(&source("sweducenter-press"), 1, PRESS_LIST).unwrap();
    assert_eq!(ids(&press)[..2], ["935748", "939214"]);
    assert_eq!(press.entries.len(), 9);
    assert!(press.entries[0].pinned && !press.has_next);

    let convedu_notice = listing(&source("convedu-notice"), 1, CONVEDU_LIST).unwrap();
    assert_eq!(
        (convedu_notice.total_pages, convedu_notice.has_next),
        (10, true)
    );
    assert_eq!(convedu_notice.entries[0].url, convedu(5551, "941911").url);
    let news = listing(&source("convedu-news"), 1, CONVEDU_NEWS_LIST).unwrap();
    assert_eq!(ids(&news), ["911675", "911673", "882726"]);
    assert!(!news.has_next);
}

#[test]
fn articles_keep_date_author_and_the_posts_own_files() {
    let notice = source("sweducenter-notice");
    let one = article(&notice, &scc(7250, "941077"), ONE_FILE).unwrap();
    assert_eq!(
        one.title,
        "AI인턴십 장학금 지원 프로그램 참여자 모집(~10/16)"
    );
    assert_eq!(one.published_at, "2026-09-21");
    assert_eq!(one.author.as_deref(), Some("SW중심대학사업단"));
    assert_eq!(one.category.as_deref(), Some("공지사항"));
    assert!(one.body_text.contains("2026.10.16(금)까지"));
    // The editor's embedded document JSON is a comment, never body text.
    assert!(!one.body_text.contains("documentPr"));
    assert_eq!(
        one.resources
            .iter()
            .map(|r| (r.kind.as_str(), r.filename.as_str()))
            .collect::<Vec<_>>(),
        [("attachment", "[AI중심대학]AI인턴십 장학금 신청 서식.hwp")]
    );

    let two = article(&notice, &scc(7250, "941521"), TWO_FILES).unwrap();
    assert_eq!(two.published_at, "2026-10-02");
    assert_eq!(
        two.resources
            .iter()
            .map(|r| r.filename.as_str())
            .collect::<Vec<_>>(),
        [
            "붙임2. Cooperation Profile.docx",
            "붙임3. 신청기관 정보 활용동의서.docx"
        ]
    );
    let news = article(
        &source("convedu-news"),
        &convedu(5550, "911675"),
        CONVEDU_NEWS,
    )
    .unwrap();
    assert_eq!(news.published_at, "2024-08-08");
    assert_eq!(news.resources.len(), 1);
    for parsed in [&one, &two, &news] {
        for resource in &parsed.resources {
            assert!(allowed(&resource.url), "{}", resource.url);
        }
    }
    // A file whose key names another post or board fails the article.
    let link_key = "NDgjIzUzIyM1MCMjNTUjIzEyNCMjMTA0IyMxMTYjIzk3IyM4MCMjMTAxIyMxMDgjIzEwNSMjMTAyIyMzNSMjMzMjIzM1IyM0OSMjMTI0IyMxMjAjIzEwMSMjMTAwIyMxMTAjIzEwNSMjMzUjIzMzIyMzNSMjNTUjIzU1IyM0OCMjNDkjIzUyIyM1NyMjMTI0IyMxMDAjIzEwNSMjMTA3IyMxMTI=";
    for other in [path_str("941078", "7250"), path_str("941077", "7251")] {
        let changed = ONE_FILE.replace(link_key, &other);
        assert_ne!(changed, ONE_FILE);
        assert!(article(&notice, &scc(7250, "941077"), &changed).is_err());
    }
    // The page shows any post whatever board the URL names; the title binds
    // the response to the board that was asked for.
    assert!(article(&notice, &scc(7250, "939214"), PRESS).is_err());
}

#[test]
fn article_images_are_kept_as_references() {
    let press = article(&source("sweducenter-press"), &scc(7252, "939214"), PRESS).unwrap();
    assert_eq!(press.published_at, "2026-08-11");
    assert!(press
        .title
        .starts_with("AI·SW 미래인재 가렸다…서강대 'RADAI'"));
    assert!(press
        .resources
        .iter()
        .any(|r| r.kind == "image" && r.url.starts_with("https://imgnews.pstatic.net/")));

    let image = article(
        &source("convedu-notice"),
        &convedu(5551, "941911"),
        CONVEDU_IMAGE,
    )
    .unwrap();
    assert_eq!(image.published_at, "2026-10-15");
    // The site links its images over HTTP on the scc host; the reference is
    // kept, but the route is never approved.
    let [reference] = &image.resources[..] else {
        panic!("{:?}", image.resources);
    };
    assert_eq!(
        reference.url,
        "http://scc.sogang.ac.kr/dataview/board/5551/20261015_1644_0810001.png"
    );
    assert!(!allowed(&reference.url));
}

#[test]
fn a_missing_post_never_counts_as_missing() {
    // A missing post redirects to a generic message page.
    let source = source("sweducenter-notice");
    assert!(!missing(&source, 302, ""));
    assert!(!missing(&source, 200, ONE_FILE));
    assert!(!missing(&source, 404, ""));
}
