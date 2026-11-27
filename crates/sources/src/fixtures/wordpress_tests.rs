use super::{ListingEntry, SourceDefinition, Url};
// The public dispatch, as the collector and the app call it.
use crate::parser::{article, detail_url, list_url, listing, missing};
use crate::{catalog, policy::validate_url};

fn source() -> SourceDefinition {
    catalog::definitions()
        .into_iter()
        .find(|source| source.id == "creative-news")
        .unwrap()
}
fn entry(id: &str) -> ListingEntry {
    ListingEntry {
        external_id: id.into(),
        url: format!("https://creative.sogang.ac.kr/?p={id}"),
        pinned: false,
        modified_at: None,
        title: None,
    }
}
const LIST: &str = include_str!("wordpress-creative-list.json");
const LAST: &str = include_str!("wordpress-creative-last.json");
const PAST_END: &str = include_str!("wordpress-creative-past-end.json");
const TEXT_POST: &str = include_str!("wordpress-creative-post-47154.json");
const IMAGE_POST: &str = include_str!("wordpress-creative-post-47085.json");
const VIDEO_POST: &str = include_str!("wordpress-creative-post-46650.json");

#[test]
fn list_and_post_urls_are_exact_and_allowed() {
    let source = source();
    let list = list_url(&source, 2).unwrap();
    assert_eq!(
        list.as_str(),
        "https://creative.sogang.ac.kr/wp-json/wp/v2/posts?categories=192&orderby=date&order=desc&per_page=50&page=2&_fields=id%2Cmodified_gmt&_envelope="
    );
    let post = detail_url(&source, &entry("47154")).unwrap();
    assert_eq!(
        post.as_str(),
        "https://creative.sogang.ac.kr/wp-json/wp/v2/posts/47154?_fields=id%2Cdate%2Cmodified_gmt%2Cstatus%2Ctype%2Ctitle%2Ccontent%2Ccategories"
    );
    for url in [&list, &post] {
        assert!(validate_url(url).is_ok(), "{url}");
    }
    assert!(validate_url(&Url::parse("https://creative.sogang.ac.kr/robots.txt").unwrap()).is_ok());
    // A post's canonical URL must be its own short link.
    let mut moved = entry("47154");
    moved.url = "https://creative.sogang.ac.kr/2026-art-technology-conference/".into();
    assert!(detail_url(&source, &moved).is_err());
    assert!(detail_url(&source, &entry("0")).is_err());
}

#[test]
fn policy_allows_only_the_bound_queries_and_dated_upload_images() {
    let posts = "https://creative.sogang.ac.kr/wp-json/wp/v2/posts";
    for allowed in [
        "https://creative.sogang.ac.kr/wp-content/uploads/2026/11/%EA%B7%B8%EB%A6%BC2.jpg",
        "https://creative.sogang.ac.kr/wp-content/uploads/2026/02/KakaoTalk_20260219_114309321_01-1024x768.jpg",
    ] {
        assert!(validate_url(&Url::parse(allowed).unwrap()).is_ok(), "{allowed}");
    }
    for blocked in [
        // Another category, more fields, a larger page, a missing envelope.
        format!("{posts}?categories=1&orderby=date&order=desc&per_page=50&page=1&_fields=id%2Cmodified_gmt&_envelope="),
        format!("{posts}?categories=192&orderby=date&order=desc&per_page=50&page=1&_fields=id%2Ccontent&_envelope="),
        format!("{posts}?categories=192&orderby=date&order=desc&per_page=100&page=1&_fields=id%2Cmodified_gmt&_envelope="),
        format!("{posts}?categories=192&orderby=date&order=desc&per_page=50&page=1&_fields=id%2Cmodified_gmt"),
        format!("{posts}?categories=192&orderby=date&order=desc&per_page=50&page=0&_fields=id%2Cmodified_gmt&_envelope="),
        format!("{posts}/47154"),
        format!("{posts}/47154?_fields=id"),
        "https://creative.sogang.ac.kr/wp-json/wp/v2/pages/22003".into(),
        "https://creative.sogang.ac.kr/wp-json/wp/v2/users".into(),
        "https://creative.sogang.ac.kr/feed/".into(),
        "https://creative.sogang.ac.kr/?p=47154".into(),
        "https://creative.sogang.ac.kr/wp-content/uploads/2026/11/a.pdf".into(),
        "https://creative.sogang.ac.kr/wp-content/uploads/a.jpg".into(),
        "https://creative.sogang.ac.kr/wp-content/plugins/x/a.jpg".into(),
        "http://creative.sogang.ac.kr/wp-content/uploads/2026/11/a.jpg".into(),
    ] {
        assert!(validate_url(&Url::parse(&blocked).unwrap()).is_err(), "{blocked}");
    }
}

#[test]
fn listing_reads_the_envelope_and_each_post_change_time() {
    let source = source();
    let page = listing(&source, 1, LIST).unwrap();
    assert_eq!(page.entries.len(), 50);
    assert!(page.has_next);
    assert_eq!(page.total_pages, 6);
    let first = &page.entries[0];
    assert_eq!(first.external_id, "47154");
    assert_eq!(first.url, "https://creative.sogang.ac.kr/?p=47154");
    assert_eq!(first.modified_at.as_deref(), Some("2026-11-20T07:26:08Z"));
    assert_eq!(page.entries[1].external_id, "47101");
    // Edits arrive through the change time, never through a listed title.
    assert!(page.entries.iter().all(|e| !e.pinned && e.title.is_none()));

    let last = listing(&source, 6, LAST).unwrap();
    assert_eq!(last.entries.len(), 29);
    assert!(!last.has_next);
    assert_eq!(last.entries[28].external_id, "16362");
    assert_eq!(
        last.entries[28].modified_at.as_deref(),
        Some("2023-10-13T19:19:37Z")
    );
    // The same body read as another page no longer matches its row count.
    assert!(listing(&source, 1, LAST).is_err());
    assert!(listing(&source, 5, LAST).is_err());
    // A total that does not match the last page's rows.
    let grown = LAST.replace("\"X-WP-Total\":279", "\"X-WP-Total\":280");
    assert_ne!(grown, LAST);
    assert!(listing(&source, 6, &grown).is_err());
    // A total too small for the declared pages is an error, not an underflow.
    let shrunk = LAST.replace("\"X-WP-Total\":279", "\"X-WP-Total\":10");
    assert_ne!(shrunk, LAST);
    assert!(listing(&source, 6, &shrunk).is_err());
}

#[test]
fn a_page_past_the_end_ends_the_listing() {
    let source = source();
    let page = listing(&source, 7, PAST_END).unwrap();
    assert!(page.entries.is_empty());
    assert!(!page.has_next);
    // The first page always exists; its error is a real failure.
    assert!(listing(&source, 1, PAST_END).is_err());
    let other_error = PAST_END.replace("rest_post_invalid_page_number", "rest_forbidden");
    assert!(listing(&source, 7, &other_error).is_err());
}

#[test]
fn malformed_listings_are_rejected() {
    let source = source();
    for body in [
        LIST.replace("\"X-WP-TotalPages\":6", "\"X-WP-TotalPages\":7"),
        LIST.replace("{\"id\":47101,", "{\"id\":47154,"),
        LIST.replace(
            "\"modified_gmt\":\"2026-11-20T07:26:08\"",
            "\"modified_gmt\":\"2026-11-20\"",
        ),
        LIST.replace("\"status\":200", "\"status\":500"),
        "<html>blocked</html>".into(),
    ] {
        assert_ne!(body, LIST);
        assert!(listing(&source, 1, &body).is_err());
    }
}

#[test]
fn a_post_is_archived_from_its_rendered_fields() {
    let source = source();
    let article = article(&source, &entry("47154"), TEXT_POST).unwrap();
    assert_eq!(
        article.title,
        "2026 Art & Technology Conference (ATC) 《레시피 바꾸지 말 것 *》"
    );
    assert_eq!(
        article.canonical_url,
        "https://creative.sogang.ac.kr/?p=47154"
    );
    // The local date: 07:26 UTC is 16:26 in Korea on the same day.
    assert_eq!(article.published_at, "2026-11-20");
    assert_eq!(article.modified_at.as_deref(), Some("2026-11-20T07:26:08Z"));
    assert_eq!(article.category.as_deref(), Some("News & Events"));
    assert!(article
        .body_text
        .contains("서강대학교 아트&테크놀로지학과에서 《레시피 바꾸지 말 것 *》를 주제로"));
    assert!(article.resources.is_empty());
    assert!(!article.unsupported_body);
    assert_eq!(article.original, TEXT_POST);
}

#[test]
fn upload_images_are_kept_and_embedded_video_is_dropped() {
    let source = source();
    let images = article(&source, &entry("47085"), IMAGE_POST).unwrap();
    assert_eq!(images.title, "Processing 25주년, PCD @ Seoul 2026 성료");
    assert_eq!(images.published_at, "2026-11-11");
    assert_eq!(images.resources.len(), 6);
    for resource in &images.resources {
        assert_eq!(resource.kind, "image");
        assert!(
            validate_url(&Url::parse(&resource.url).unwrap()).is_ok(),
            "{}",
            resource.url
        );
    }
    assert!(images.resources[0]
        .url
        .starts_with("https://creative.sogang.ac.kr/wp-content/uploads/2026/11/"));

    let video = article(&source, &entry("46650"), VIDEO_POST).unwrap();
    assert!(video.title.contains("헌정필름 ‘비하인드 신’ 전달"));
    assert!(!video.body_html.contains("<iframe"));
    assert!(!video.body_html.contains("youtube.com/embed"));
    assert!(video.body_text.contains("원로배우 신영균"));
    assert!(video
        .resources
        .iter()
        .any(|r| r.url.ends_with("-1024x768.jpg")));
}

#[test]
fn a_post_outside_the_board_is_not_its_notice() {
    let source = source();
    for body in [
        TEXT_POST.replace("\"categories\":[460,192]", "\"categories\":[460]"),
        TEXT_POST.replace("\"status\":\"publish\"", "\"status\":\"draft\""),
        TEXT_POST.replace("\"type\":\"post\"", "\"type\":\"page\""),
        TEXT_POST.replace("\"protected\":false", "\"protected\":true"),
    ] {
        assert_ne!(body, TEXT_POST);
        assert!(article(&source, &entry("47154"), &body).is_err());
    }
    // Another post's document under this post's identity.
    assert!(article(&source, &entry("47085"), TEXT_POST).is_err());
}

#[test]
fn only_a_404_or_410_reports_a_post_missing() {
    let source = source();
    assert!(missing(&source, 404, ""));
    assert!(missing(&source, 410, ""));
    assert!(!missing(&source, 200, TEXT_POST));
    assert!(!missing(&source, 500, ""));
}
