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
fn xml(entries: &str) -> String {
    format!("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{entries}</urlset>")
}

#[test]
fn sitemap_lists_each_board_once_newest_change_first() {
    let general = source("me-general");
    let page = listing(&general, 1, include_str!("mechanical-sitemap.xml")).unwrap();
    assert!(!page.has_next);
    assert_eq!(page.total_pages, 1);
    assert!(page.entries.iter().all(|entry| !entry.pinned));
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| entry.external_id.as_str())
            .collect::<Vec<_>>(),
        ["2420", "2419", "66"]
    );
    assert_eq!(
        page.entries[0].modified_at.as_deref(),
        Some("2026-10-09T15:41:57.173Z")
    );
    assert_eq!(
        page.entries[0].url,
        "https://me.sogang.ac.kr/ko/board/notice/2420"
    );

    let award = listing(
        &source("me-awards"),
        1,
        include_str!("mechanical-sitemap.xml"),
    )
    .unwrap();
    assert_eq!(award.entries.len(), 1);
    assert_eq!(award.entries[0].external_id, "2427");
    let alumni = listing(
        &source("me-alumni"),
        1,
        include_str!("mechanical-sitemap.xml"),
    )
    .unwrap();
    assert_eq!(alumni.entries[0].external_id, "1835");
    let academic = listing(
        &source("me-academic"),
        1,
        include_str!("mechanical-sitemap.xml"),
    )
    .unwrap();
    assert_eq!(academic.entries[0].external_id, "2395");
}

#[test]
fn list_urls_and_detail_urls_are_exact() {
    for (id, board) in [
        ("me-general", "notice"),
        ("me-academic", "academic"),
        ("me-research", "research"),
        ("me-awards", "award"),
        ("me-careers", "scholarship"),
        ("me-events", "events"),
        ("me-alumni", "alumni_news"),
    ] {
        let definition = source(id);
        assert_eq!(
            list_url(&definition, 1).unwrap().as_str(),
            "https://me.sogang.ac.kr/sitemap.xml"
        );
        assert!(list_url(&definition, 2).is_err());
        assert!(list_url(&definition, 0).is_err());
        let entry = ListingEntry {
            external_id: "2419".into(),
            url: format!("https://me.sogang.ac.kr/ko/board/{board}/2419"),
            pinned: false,
            modified_at: None,
        };
        assert_eq!(
            detail_url(&definition, &entry).unwrap().as_str(),
            format!("https://me.sogang.ac.kr/ko/board/{board}/2419")
        );
        let mut wrong = entry.clone();
        wrong.url = "https://me.sogang.ac.kr/ko/board/notice/2419".into();
        if board != "notice" {
            assert!(detail_url(&definition, &wrong).is_err());
        }
        let mut zero = entry;
        zero.external_id = "0".into();
        assert!(detail_url(&definition, &zero).is_err());
    }
    let unknown = source("me-general");
    let mut entry = ListingEntry {
        external_id: "2419".into(),
        url: "https://evil.example/ko/board/notice/2419".into(),
        pinned: false,
        modified_at: None,
    };
    assert!(detail_url(&unknown, &entry).is_err());
    entry.url = "https://me.sogang.ac.kr/ko/board/notice/2419".into();
    assert!(detail_url(&unknown, &entry).is_ok());
}

#[test]
fn notice_detail_keeps_attachments_and_embedded_images() {
    let general = source("me-general");
    let page = listing(&general, 1, include_str!("mechanical-sitemap.xml")).unwrap();
    let entry = entry_of(&page, "2419");
    let parsed = article(
        &general,
        &entry,
        include_str!("mechanical-notice-detail-2419.html"),
    )
    .unwrap();
    assert_eq!(parsed.title, "2026년도 차세대 공학자 양성사업(PRAXIS 사업단)_제1기 실전문제연구팀 모집 공고 (~10/2 13시까지)");
    assert_eq!(parsed.author.as_deref(), Some("기계공학과"));
    assert_eq!(parsed.published_at, "2026-09-28");
    assert_eq!(parsed.category.as_deref(), Some("일반공지"));
    assert_eq!(
        parsed.modified_at.as_deref(),
        Some("2026-09-28T15:31:48.462Z")
    );
    assert!(!parsed.unsupported_body);
    assert!(parsed.body_text.contains("산업체가 제시한"));
    let images: Vec<_> = parsed
        .resources
        .iter()
        .filter(|resource| resource.kind == "image")
        .collect();
    let attachments: Vec<_> = parsed
        .resources
        .iter()
        .filter(|resource| resource.kind == "attachment")
        .collect();
    assert_eq!(images.len(), 1);
    assert!(images[0]
        .url
        .contains("supabase.co/storage/v1/object/public/media/posts/"));
    assert_eq!(attachments.len(), 6);
    assert!(attachments.iter().all(|resource| resource
        .url
        .contains("supabase.co/storage/v1/object/public/media/files/")));
    assert!(attachments
        .iter()
        .any(|resource| resource.filename.ends_with("모집공고최종.docx")));
    // An unlisted file type keeps the notice; the transport never requests it.
    let video = article(
        &general,
        &entry,
        &include_str!("mechanical-notice-detail-2419.html")
            .replace("%EC%B5%9C%EC%A2%85.docx", "%EC%B5%9C%EC%A2%85.mp4"),
    )
    .unwrap();
    assert!(video
        .resources
        .iter()
        .any(|resource| resource.filename.ends_with("모집공고최종.mp4")));
    assert!(parsed
        .body_html
        .contains("data-resource-url=\"https://pvdobbplxndsigatnamu.supabase.co"));
}

#[test]
fn hero_images_survive_and_embedded_video_frames_do_not() {
    let awards = source("me-awards");
    let entry = ListingEntry {
        external_id: "2427".into(),
        url: "https://me.sogang.ac.kr/ko/board/award/2427".into(),
        pinned: false,
        modified_at: Some("2026-11-04T16:47:29.482Z".into()),
    };
    let parsed = article(
        &awards,
        &entry,
        include_str!("mechanical-award-detail-2427.html"),
    )
    .unwrap();
    assert_eq!(parsed.published_at, "2026-11-04");
    assert_eq!(
        parsed
            .resources
            .iter()
            .filter(|resource| resource.kind == "image")
            .count(),
        1
    );
    assert!(parsed.body_text.contains("미래인재상"));

    let alumni = source("me-alumni");
    let entry = ListingEntry {
        external_id: "1835".into(),
        url: "https://me.sogang.ac.kr/ko/board/alumni_news/1835".into(),
        pinned: false,
        modified_at: None,
    };
    let parsed = article(
        &alumni,
        &entry,
        include_str!("mechanical-alumni-detail-1835.html"),
    )
    .unwrap();
    assert!(!parsed.body_html.contains("iframe"));
    assert!(!parsed.body_html.contains("youtube-nocookie"));
    assert!(parsed.body_html.contains("https://youtu.be/rZI6C7XsnY4"));
    assert!(parsed.body_text.contains("YouTube에서 보기"));
    assert_eq!(parsed.modified_at, None);

    let research = source("me-research");
    let entry = ListingEntry {
        external_id: "2425".into(),
        url: "https://me.sogang.ac.kr/ko/board/research/2425".into(),
        pinned: false,
        modified_at: None,
    };
    let parsed = article(
        &research,
        &entry,
        include_str!("mechanical-research-detail-2425.html"),
    )
    .unwrap();
    assert_eq!(
        parsed
            .resources
            .iter()
            .filter(|resource| resource.kind == "image")
            .count(),
        1
    );

    // This board serves its files from a legacy R2 bucket rather than Supabase;
    // the reference is kept even though the transport never downloads attachments
    // from a file host outside the approved school routes.
    let events = source("me-events");
    let entry = ListingEntry {
        external_id: "1762".into(),
        url: "https://me.sogang.ac.kr/ko/board/events/1762".into(),
        pinned: false,
        modified_at: None,
    };
    let parsed = article(
        &events,
        &entry,
        include_str!("mechanical-events-detail-1762.html"),
    )
    .unwrap();
    assert_eq!(parsed.published_at, "2026-03-13");
    let attachments: Vec<_> = parsed
        .resources
        .iter()
        .filter(|resource| resource.kind == "attachment")
        .collect();
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].url.contains("r2.dev/legacy/"));
    assert!(attachments[0].filename.ends_with(".jpg"));
}

#[test]
fn sitemap_shape_violations_fail_closed() {
    let general = source("me-general");
    let valid = "<url><loc>https://me.sogang.ac.kr/ko/board/notice/1</loc>\
                 <lastmod>2026-09-28T15:31:48.462Z</lastmod></url>";
    assert!(listing(&general, 1, &xml(valid)).is_ok());
    for broken in [
        // Duplicate identity.
        format!("{valid}{valid}"),
        // Query strings are disallowed by robots.txt; no board route has one.
        "<url><loc>https://me.sogang.ac.kr/ko/board/notice/1?page=2</loc></url>".into(),
        // Other hosts, nested paths and non-positive identities are not this board.
        "<url><loc>https://evil.example/ko/board/notice/1</loc></url>".into(),
        "<url><loc>https://me.sogang.ac.kr/ko/board/notice/1/extra</loc></url>".into(),
        "<url><loc>https://me.sogang.ac.kr/ko/board/notice/0</loc></url>".into(),
        // Two change timestamps are ambiguous.
        "<url><loc>https://me.sogang.ac.kr/ko/board/notice/1</loc>\
         <lastmod>2026-09-28T15:31:48.462Z</lastmod>\
         <lastmod>2026-09-29T15:31:48.462Z</lastmod></url>"
            .into(),
        // No article for this board.
        "<url><loc>https://me.sogang.ac.kr/ko/board/gallery/1705</loc></url>".into(),
    ] {
        assert!(listing(&general, 1, &xml(&broken)).is_err(), "{broken}");
    }
    // A non-UTC or malformed change time is ignored, so the article is fetched.
    let parsed = listing(
        &general,
        1,
        &xml("<url><loc>https://me.sogang.ac.kr/ko/board/notice/1</loc>\
             <lastmod>2026-09-28</lastmod></url>"),
    )
    .unwrap();
    assert_eq!(parsed.entries[0].modified_at, None);
    assert!(listing(&general, 2, include_str!("mechanical-sitemap.xml")).is_err());
    assert!(listing(&general, 1, "<html>blocked</html>").is_err());
}

#[test]
fn article_identity_and_board_label_are_bound() {
    let general = source("me-general");
    let page = listing(&general, 1, include_str!("mechanical-sitemap.xml")).unwrap();
    let entry = entry_of(&page, "2419");
    let body = include_str!("mechanical-notice-detail-2419.html");
    let mut swapped = entry.clone();
    swapped.external_id = "2420".into();
    swapped.url = "https://me.sogang.ac.kr/ko/board/notice/2420".into();
    assert!(article(&general, &swapped, body).is_err());
    // Same page served for another board.
    let awards = source("me-awards");
    let mut other = entry.clone();
    other.url = "https://me.sogang.ac.kr/ko/board/award/2419".into();
    assert!(article(&awards, &other, body).is_err());
    let broken = body.replace("일반공지", "학사공지");
    assert!(article(&general, &entry, &broken).is_err());
}
