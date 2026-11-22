//! Which URLs and addresses automatic collection may reach. Pure checks with no
//! I/O, shared by every fetcher; the transport applies them before each request
//! and redirect hop.
use std::net::IpAddr;
use url::Url;

pub const BLOCKED: &str = "검증된 학교 공개 자료 주소가 아니어서 자동 다운로드하지 않았습니다.";
/// Any public HTTPS host name except the school's, on the default port. Bytes
/// are verified as a raster image after download; DNS answers must be public.
pub fn third_party_image(url: &Url) -> bool {
    let Some(url::Host::Domain(host)) = url.host() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && host.contains('.')
        && host != "sogang.ac.kr"
        && !host.ends_with(".sogang.ac.kr")
        && !path.contains(['\\', '\0'])
        && !path.contains("%2f")
        && !path.contains("%5c")
}
fn raster_name(name: &str) -> bool {
    name.len() <= 512
        && [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|extension| name.to_ascii_lowercase().ends_with(extension))
}
/// Automatic fetching is deliberately narrower than user-triggered external navigation.
pub fn validate_url(url: &Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or("");
    let path = url.path();
    let allowed = match host {
        "www.sogang.ac.kr" => {
            path == "/api/api/v1/mainKo/BbsData"
                || path == "/api/api/v1/mainKo/BbsData/boardList"
                // Attachments of the catalogued BbsData boards only.
                || {
                    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
                    // Deeper paths stay allowed, as the earlier board-2 prefix rule did.
                    parts.len() >= 4
                        && parts[..2] == ["file-fe-prd", "board"]
                        && crate::parser::MAIN_BOARDS
                            .iter()
                            .any(|board| parts[2] == board.config.to_string())
                        && !parts[3].is_empty()
                }
                || path.starts_with("/file-fe-prd/editor/")
                || path.starts_with("/file-fe-prd/upload/")
                // Inline notice images (user decision, #78), e.g.
                // /dataview/board/2/<uuid>.jpg and /dataview/board/551608/<uuid>.jpg.
                || {
                    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
                    parts.len() == 4
                        && parts[..2] == ["dataview", "board"]
                        && positive_id(parts[2])
                        && raster_name(parts[3])
                }
                || path == "/robots.txt"
        }
        "computing.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            let boards = [
                "academicNotice",
                "graduateNotice",
                "externalInfo",
                "news",
                "career",
            ];
            let article = parts.len() >= 4
                && parts[..2] == ["ko", "community"]
                && boards.contains(&parts[2])
                && ((parts.len() == 4 && parts[3] == "list")
                    || (parts.len() == 5 && parts[3] == "detail" && positive_id(parts[4])));
            let attachment = parts.len() == 6
                && parts[..3] == ["web", "file", "community"]
                && boards.contains(&parts[3])
                && positive_id(parts[4])
                && !parts[5].is_empty();
            article || attachment || path.starts_with("/web/file/editor/") || path == "/robots.txt"
        }
        "eng.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            let board = (parts.len() == 2 || parts.len() == 3)
                && parts[0] == "board"
                && ["notice", "recruit", "academic", "research"].contains(&parts[1])
                && (parts.len() == 2 || positive_id(parts[2]));
            let download = parts.len() == 2 && parts[0] == "download" && positive_id(parts[1]);
            // Observed official recruit article /board/recruit/676 embeds
            // /files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png. Do not allow
            // generic asset, application, or arbitrary nested file routes.
            let image = parts.len() == 3
                && parts[0] == "files"
                && !parts[1].is_empty()
                && parts[1].len() <= 128
                && parts[1]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                && parts[2].len() <= 512
                && [".png", ".jpg", ".jpeg", ".gif", ".webp"]
                    .iter()
                    .any(|extension| parts[2].to_ascii_lowercase().ends_with(extension));
            board || download || image || path == "/robots.txt"
        }
        "ee.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt disallows /se2/ (inline editor images) and /data/; the
            // /aseoul_bbs/ attachment route is not covered by any rule.
            let article = parts.len() == 3
                && ((parts[0] == "kor"
                    && parts[1] == "community"
                    && [
                        "notice01.php",
                        "notice02.php",
                        "notice03.php",
                        "seminar.php",
                        "employment.php",
                    ]
                    .contains(&parts[2]))
                    || parts[..2] == ["kor", "recruit"] && parts[2] == "recruit.php");
            let attachment = parts.len() == 2 && parts[0] == "aseoul_bbs" && parts[1] == "down.php";
            article || attachment || path == "/robots.txt"
        }
        "me.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt forbids every query string (`Disallow: /*?`), so board
            // paging is never requested. Discovery uses the query-free sitemap;
            // article pages stay under the verified `ko/board/<board>/<id>` route.
            let article = parts.len() == 4
                && parts[..2] == ["ko", "board"]
                && [
                    "notice",
                    "academic",
                    "research",
                    "award",
                    "scholarship",
                    "events",
                    "alumni_news",
                ]
                .contains(&parts[2])
                && positive_id(parts[3]);
            path == "/sitemap.xml" || article || path == "/robots.txt"
        }
        "se.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt allows every path, but only these exact routes are
            // requested: the board list/detail query routes, the filedown
            // attachment route and same-host editor images. Query keys and values
            // are bound so an arbitrary query never reaches another PHP route.
            let list = parts.len() == 2
                && parts[..2] == ["board", "board_list.php"]
                && exact_query(
                    url,
                    &[("board_id", semiconductor_board), ("page", positive_id)],
                );
            let article = parts.len() == 2
                && parts[..2] == ["board", "board_view.php"]
                && exact_query(
                    url,
                    &[("board_id", semiconductor_board), ("no", positive_id)],
                );
            let attachment = parts.len() == 2
                && parts[..2] == ["board", "filedown.php"]
                && exact_query(
                    url,
                    &[("filepath", bounded_value), ("filename", downloadable_name)],
                );
            let image = parts.len() == 3
                && parts[..2] == ["smart_editor", "upload"]
                && editor_image(parts[2]);
            list || article || attachment || image || path == "/robots.txt"
        }
        "sse.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // robots.txt disallows /se2/, /data/ and /bbs/, so attachments and
            // inline images are never requested from this host.
            (parts.len() == 3
                && parts[0] == "kor"
                && parts[1] == "community"
                && ["notice.php", "news.php", "seminar.php"].contains(&parts[2]))
                || path == "/robots.txt"
        }
        "new-cms.sogang.ac.kr" => {
            let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
            // Only the three verified dormitory menus and their board ids. Inline
            // editor images under /CrossEditor/ are the only article resources this
            // host serves; other school hosts' images stay blocked by policy.
            let menus = ["6975", "3947", "3951"];
            let boards = ["2231", "1644", "1648"];
            let list = parts.len() == 3
                && parts[0] == "sgdormitory"
                && menus.contains(&parts[1])
                && parts[2] == "subview.do";
            let board_file = parts.len() == 5
                && parts[..2] == ["bbs", "sgdormitory"]
                && boards.contains(&parts[2])
                && positive_id(parts[3])
                && ["artclView.do", "download.do"].contains(&parts[4]);
            let image = parts.len() == 5
                && parts[..3] == ["CrossEditor", "binary", "images"]
                && (1..=12).contains(&parts[3].len())
                && parts[3].bytes().all(|b| b.is_ascii_digit())
                && raster_name(parts[4]);
            list || board_file || image || path == "/robots.txt"
        }
        // The CMS file route behind the main board's PDF-viewer iframes
        // (user decision): a notice whose whole body is that viewer is archived
        // from the PDF's text. Article images on this host stay unsupported.
        "scc.sogang.ac.kr" => {
            let pdf = path == "/Download3"
                && exact_query(
                    url,
                    &[
                        ("pathStr", |v| {
                            bounded_value(v)
                                && v.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                        }),
                        ("fileName", |v| {
                            downloadable_name(v) && v.to_ascii_lowercase().ends_with(".pdf")
                        }),
                        ("gubun", |v| v == "cmsfile"),
                    ],
                );
            pdf || path == "/robots.txt"
        }
        _ => false,
    };
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !allowed
        || path.contains(['\\', '\0'])
        || path.to_ascii_lowercase().contains("%2f")
        || path.to_ascii_lowercase().contains("%5c")
    {
        return Err(BLOCKED.into());
    }
    Ok(())
}
fn positive_id(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|id| id > 0)
}
fn semiconductor_board(value: &str) -> bool {
    [
        "notice",
        "notice_master",
        "news",
        "recruit",
        "notice_industry",
    ]
    .contains(&value)
}
fn bounded_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}
fn downloadable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && !value.contains(['/', '\\'])
        && !value.chars().any(char::is_control)
        && {
            let lower = value.to_ascii_lowercase();
            [
                ".pdf", ".hwp", ".hwpx", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip",
                ".txt", ".png", ".jpg", ".jpeg", ".gif", ".webp",
            ]
            .iter()
            .any(|extension| lower.ends_with(extension))
        }
}
fn editor_image(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && raster_name(name)
}
type QueryRule = (&'static str, fn(&str) -> bool);
/// Requires exactly the listed query keys once each, with a validating predicate
/// per key. The order is not significant.
fn exact_query(url: &Url, allowed: &[QueryRule]) -> bool {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if pairs.len() != allowed.len() {
        return false;
    }
    for (name, check) in allowed {
        let mut matches = pairs.iter().filter(|(key, _)| key.as_str() == *name);
        let Some((_, value)) = matches.next() else {
            return false;
        };
        if matches.next().is_some() || !check(value) {
            return false;
        }
    }
    true
}
pub fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            // Permit only ordinary global unicast; exclude documentation, local,
            // multicast and transition mechanisms which could tunnel private IPv4.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn main_site_attachments_are_limited_to_catalogued_boards() {
        for board in [1, 2, 34, 141, 142] {
            let url =
                format!("https://www.sogang.ac.kr/file-fe-prd/board/{board}/551882_1.pdf?sg=a.pdf");
            assert!(validate_url(&Url::parse(&url).unwrap()).is_ok(), "{url}");
        }
        for path in [
            "/file-fe-prd/board/104/551415_1.pdf",
            "/file-fe-prd/board/3/1_1.pdf",
            "/file-fe-prd/board/141/",
            "/file-fe-prd/board/x/1.pdf",
        ] {
            let url = format!("https://www.sogang.ac.kr{path}");
            assert!(validate_url(&Url::parse(&url).unwrap()).is_err(), "{url}");
        }
    }
    #[test]
    fn address_policy_rejects_private_redirect_targets_and_unverified_domains() {
        for value in [
            "http://www.sogang.ac.kr/api/api/v1/mainKo/BbsData",
            "https://127.0.0.1/a",
            "https://www.sogang.ac.kr.evil.test/api/api/v1/mainKo/BbsData",
            "https://computing.sogang.ac.kr:8443/ko/community/news/list",
            "https://user@computing.sogang.ac.kr/ko/community/news/list",
            "https://computing.sogang.ac.kr/web/file/editor/a#x",
            "https://computing.sogang.ac.kr/admin",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
                "{value}"
            );
        }
        for value in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(value.parse().unwrap()), "{value}");
        }
        assert!(public_ip("163.239.1.1".parse().unwrap()));
        assert!(validate_url(
            &Url::parse("https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData?pkId=1").unwrap()
        )
        .is_ok());
    }
    #[test]
    fn pdf_viewer_files_are_the_only_approved_scc_route() {
        let file = "https://scc.sogang.ac.kr/Download3?pathStr=MTAxIyMxMDg%3D&fileName=%EC%95%88%EB%82%B4.pdf&gubun=cmsfile";
        for value in [file, "https://scc.sogang.ac.kr/robots.txt"] {
            assert!(validate_url(&Url::parse(value).unwrap()).is_ok(), "{value}");
        }
        for value in [
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.hwp&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=other",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile&x=1",
            "https://scc.sogang.ac.kr/Download3?pathStr=..%2F..&fileName=a.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=..%2Fa.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/Download2?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile",
            "https://scc.sogang.ac.kr/pdfviewer/web/viewer.html?file=x",
            "https://scc.sogang.ac.kr/dataview/temp/a.jpg",
            "http://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
                "{value}"
            );
        }
    }
    #[test]
    fn engineering_policy_is_limited_to_verified_public_routes() {
        for path in [
            "/board/notice",
            "/board/recruit/676",
            "/board/academic/4",
            "/board/research/728",
            "/download/708",
            "/files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png",
            "/robots.txt",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://eng.sogang.ac.kr{path}")).unwrap())
                    .is_ok(),
                "{path}"
            );
        }
        for path in [
            "/adm",
            "/login",
            "/search",
            "/board/unknown",
            "/board/notice/0",
            "/board/notice/1/private",
            "/download/0",
            "/download/708/nested",
            "/files/token/page.html",
            "/files/token/nested/image.png",
            "/files/token/a%2fimage.png",
            "/assets/js/site.js",
        ] {
            assert!(
                validate_url(&Url::parse(&format!("https://eng.sogang.ac.kr{path}")).unwrap())
                    .is_err(),
                "{path}"
            );
        }
    }
}
