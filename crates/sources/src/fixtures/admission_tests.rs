use super::{ListingEntry, ListingPage, SourceDefinition, Url};
// The public dispatch, as the collector and the app call it.
use crate::parser::{article, detail_url, list_url, listing, missing};
use crate::{catalog, policy::validate_url};

fn source() -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|source| source.id == "admission-notice")
        .unwrap()
}
fn entry(id: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!("https://admission.sogang.ac.kr/enter/html/counsel/noticeView.asp?idx={id}"),
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
fn allowed(url: &str) -> bool {
    validate_url(&Url::parse(url).unwrap()).is_ok()
}
const PINS: [&str; 5] = ["29962", "29005", "29787", "29002", "28940"];
const LIST: &str = include_str!("admission-notice-list.html");
const LAST: &str = include_str!("admission-notice-last.html");
const PAST_END: &str = include_str!("admission-notice-past-end.html");
const TWO_FILES: &str = include_str!("admission-notice-detail-29514.html");
const OLD: &str = include_str!("admission-notice-detail-24472.html");
const MISSING: &str = include_str!("admission-notice-missing.html");

#[test]
fn the_catalog_has_one_admission_board_and_its_urls_are_allowed() {
    let source = source();
    assert_eq!(source.group.as_deref(), Some("sogang-admission"));
    assert_eq!(source.organization, "서강대학교 · 입학처");
    let list = list_url(&source, 2).unwrap();
    assert_eq!(
        list.as_str(),
        "https://admission.sogang.ac.kr/enter/html/counsel/notice.asp?s_page=2"
    );
    let detail = detail_url(&source, &entry("29514")).unwrap();
    assert_eq!(
        detail.as_str(),
        "https://admission.sogang.ac.kr/enter/html/counsel/noticeView.asp?idx=29514"
    );
    for url in [&list, &detail] {
        assert!(validate_url(url).is_ok(), "{url}");
    }
    assert!(allowed("https://admission.sogang.ac.kr/robots.txt"));
    assert!(detail_url(&source, &entry("0")).is_err());
    for blocked in [
        // A category tab, search, another board, extra keys.
        "https://admission.sogang.ac.kr/enter/html/counsel/notice.asp?s_page=1&s_cate=BBS0402",
        "https://admission.sogang.ac.kr/enter/html/counsel/notice.asp?s_page=1&s_data=x",
        "https://admission.sogang.ac.kr/enter/html/rolling/noticeView.asp?idx=29962",
        "https://admission.sogang.ac.kr/enter/html/counsel/noticeView.asp?idx=1&s_page=1",
        "https://admission.sogang.ac.kr/enter/html/counsel/qna.asp",
        // The download script reads a server path: only the board's folder
        // and a plain stored name.
        "https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0004/../../web.config&strFileName=a.pdf",
        "https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0001/A1.PDF&strFileName=a.pdf",
        "https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0004/A1.PDF",
        "https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0004/A-1.PDF&strFileName=a.pdf",
        "https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0004/A1.PDF&strFileName=a.exe",
    ] {
        assert!(!allowed(blocked), "{blocked}");
    }
}

#[test]
fn pins_share_page_one_and_repeat_above_later_pages() {
    let source = source();
    let first = listing(&source, 1, LIST).unwrap();
    assert_eq!(
        ids(&first),
        [&PINS[..], &["29514", "29112", "28828", "28741", "28262"]].concat()
    );
    assert_eq!(
        first.entries.iter().filter(|e| e.pinned).count(),
        PINS.len()
    );
    assert_eq!((first.total_pages, first.has_next), (2, true));
    assert_eq!(first.entries[5].url, entry("29514").url);

    let last = listing(&source, 2, LAST).unwrap();
    assert_eq!(
        ids(&last)[5..],
        [
            "28063", "28057", "27543", "27524", "27049", "26982", "26913", "26490", "25526",
            "24472"
        ]
    );
    assert!(!last.has_next);
    // Past the end the site lists only the pins under an unmarked pager.
    assert!(listing(&source, 3, PAST_END).is_err());
    assert!(listing(&source, 2, LIST).is_err());
    // A category tab would list only part of the board.
    let tab = LIST.replacen("<li class=\"on\">", "<li>", 1);
    assert_ne!(tab, LIST);
    assert!(listing(&source, 1, &tab).is_err());
    // A total that does not match the rows means rows went missing.
    let short = LIST.replacen("20건", "21건", 1);
    assert_ne!(short, LIST);
    assert!(listing(&source, 1, &short).is_err());
}

#[test]
fn articles_keep_the_date_and_the_download_script_files() {
    let source = source();
    let notice = article(&source, &entry("29514"), TWO_FILES).unwrap();
    assert_eq!(notice.title, "2027학년도 정시모집 모집요강");
    assert_eq!(notice.published_at, "2026-09-01");
    assert_eq!(notice.category.as_deref(), Some("공지사항"));
    assert_eq!(
        notice
            .resources
            .iter()
            .map(|r| (r.kind.as_str(), r.filename.as_str()))
            .collect::<Vec<_>>(),
        [
            ("attachment", "2027학년도 서강대학교 정시모집요강.pdf"),
            ("attachment", "2027학년도 서강대학교 정시모집요강.hwp"),
        ]
    );
    assert!(notice.resources[0]
        .url
        .starts_with("https://admission.sogang.ac.kr/common/download.File.asp?strRealFileName=/upload/BBS0004/20260901173332Y7G836.PDF&strFileName="));

    let old = article(&source, &entry("24472"), OLD).unwrap();
    assert_eq!(old.published_at, "2023-03-30");
    assert_eq!(old.resources.len(), 1);
    for article in [&notice, &old] {
        for resource in &article.resources {
            assert!(allowed(&resource.url), "{}", resource.url);
        }
    }
    // A file outside the board's folder fails the article.
    let elsewhere = TWO_FILES.replacen("/upload/BBS0004/", "/upload/../", 1);
    assert_ne!(elsewhere, TWO_FILES);
    assert!(article(&source, &entry("29514"), &elsewhere).is_err());
}

#[test]
fn the_sites_missing_post_answer_counts_as_missing() {
    let source = source();
    assert!(missing(&source, 200, MISSING));
    assert!(!missing(&source, 200, TWO_FILES));
    assert!(!missing(&source, 404, ""));
    assert!(article(&source, &entry("99999999"), MISSING).is_err());
}
