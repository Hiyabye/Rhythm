# Official-source parser fixtures

These are bounded samples from public Sogang University pages, first captured during the v0.2.0 implementation (2026-10-24). They are deterministic compatibility fixtures. They are not a feed or a copy of the university archive.

University-authored content keeps its original rights. Rhythm's GPL-3.0-or-later license does not relicense it.

Unless a section says otherwise, captures are public GETs only, and nothing was executed or retyped. Each section names its capture date and issue.

## Main site (`www.sogang.ac.kr`) and Computing

Captured 2026-10-24 (v0.2.0).

- `main-list-exact.json`, `main-page2.json`: pages 1 and 2 of `https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData/boardList`. They use `pageSize=50&bbsConfigFk=2` and the reference collector's empty filter parameters.
- `main-detail.json`: public JSON detail for `pkId=551637`. It keeps the original HTML body, author, category references, publication date and the volatile view-count metadata.
- `computing-*-list.html`: only the original `.board-lnb` and `.board-content` elements from the five official `/ko/community/{board}/list?num=1` pages.
- `computing-academic-page2.html`: the matching page 2. It keeps the desktop and mobile mirrors, to catch wrong whole-document pin counting.
- `computing-*-detail.html`: the original canonical link and `article.board-detail` for academicNotice/969, graduateNotice/128, externalInfo/1927, news/12 and career/3323.
  - Unrelated CSS, navigation, analytics and translation dictionaries were removed.
  - The Nuxt `FILE_LIST` data is unchanged.
  - Its IIFE was reduced to the exact scalar aliases that list references, with the original invocation's values.
  - No JavaScript was run to get this data.

### Main site boards (#80, 2026-11-22 to 2026-11-24)

- `main-{general,scholarship,events,student}-list.json` (2026-11-22): page 1 of the same list endpoint with `pageSize=50` and `bbsConfigFk` 1, 141, 142 and 34. Unchanged.
- `main-general-detail-551892.json`, `main-scholarship-detail-551882.json`, `main-events-detail-551374.json`, `main-student-detail-551671.json` (2026-11-22): the unchanged public JSON detail of one listed notice with attachments on each board.
- `main-onestop-list.json`, `main-onestop-detail-550212.json`, `main-onestop-detail-550598.json` (2026-11-24, #315): page 1 of the same list endpoint with `bbsConfigFk=3` (종합봉사실, 302 notices), and the unchanged JSON details of two listed notices (one with an attachment, one without). The site lists no page for this board, so its notice link carries only `bbsConfigFk` (user decision).

## v0.2.1 captures (2026-10-25)

- `computing-news-detail-282.html`: the canonical link, the original `article.board-detail` and the original Nuxt script from `https://computing.sogang.ac.kr/ko/community/news/detail/282`. No JavaScript was run.
  - The public route ID is 282, but `DETAIL.SN` and the attachment `PARENT_SEQ` resolve to 562.
  - Attachment paths still use public ID 282.
  - The regression requires both identities to agree with their own contracts.
- `eng-{notice,academic,recruit,research}-list.html`: the original canonical link and `<main>` from `https://eng.sogang.ac.kr/board/{board}` (notice and academic include `?page=1`).
  - These current SSR pages replace the four retired CMS listing routes.
  - The e뉴스레터 gallery (`/board/newsletter`) is not a source and has no fixtures.
- `eng-notice-page2.html`: the same extraction from `/board/notice?page=2`.
  - Unlike Computing, Engineering pins appear only on page one.
  - The total of 361 includes five pins.
  - Regular ordinals go from 356 on page one to 346 on page two.
- `eng-*-detail-*.html`: the canonical link and original `<main>` for notice/20, notice/723, academic/4, recruit/676 and research/728.
  - Author, UTC machine timestamps, visible Korean date and time, the full `.post-body`, tables, attachment names and URLs are unchanged.
  - recruit/676 has the inline image `/files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png`.
  - academic/4 has three extensionless `/download/{id}` links with the real filenames in the markup.
  - Navigation inside `<main>` stays, so scope-extraction regressions can be tested.
  - The document head, footer and scripts are omitted.
  - Line endings are normalized to LF and trailing line whitespace is removed. These are structural fixtures, not byte-for-byte HTTP captures.

## #78 captures (2026-11-04, 2026-11-08)

Captured through Rhythm's own transport and robots policy.

- `main-detail-551377.json`: unchanged JSON detail for `pkId=551377`. The body is an empty paragraph plus `div.iframe-wrapper` around a PDF-viewer iframe, which the iframe-only rule must recognize. The iframe was not fetched.
- `main-detail-551680.json` (2026-11-08): unchanged JSON detail for `pkId=551680`. The whole body is a PDF-viewer iframe on `scc.sogang.ac.kr`.
- `main-pdf-551680.pdf` (2026-11-08): the PDF that viewer shows, from its `https://scc.sogang.ac.kr/Download3?...&gubun=cmsfile` file URL. Unchanged. 140,458 bytes, SHA-256 `ff614d7ca4bf7f596f94d339084d3c8367fa2c46fc5656ebdd3e2a7ab6154038`. It is a Hancom PDF with a Korean text layer.
- `computing-career-content-396.html`: only the original inner HTML of `.sg-content.tiptap-view` from `https://computing.sogang.ac.kr/ko/community/career/detail/396`. It has legacy tables nested about 130 levels deep. The test places it inside the `career/3323` page fixture, so the page shell and Nuxt data are not duplicated.

## Electronic and System Semiconductor departments (#94, 2026-11-05)

Both sites run the same `aseoul` board engine. They share robots.txt rules, `?m=v&idx=` detail links, `pNo` paging and pins-first ordering, with different skins. The two gallery boards show 12 and 9 rows per page. The other tables show 10.

Each fixture keeps only the real `#sub_tit` (ee) or `.ttl01` (sse) title element and the complete `#board` subtree. Scripts, style blocks and comments were removed. Nothing was run, resized or retyped. Line endings are LF and trailing whitespace is removed.

**ee**

- `community-ee-{news,general,academic,seminars,employment,recruit}-list.html`: page 1 from `/kor/community/{notice01,notice02,notice03,seminar,employment}.php` and `/kor/recruit/recruit.php`, with `?pNo=1&code=...`.
- `community-ee-general-page2.html`: `ee-general` page 1 holds only ten pins. Page 2 has two more pins and regular ordinals 290-283. It is the pin-across-pages regression.
- `community-ee-news-last.html`: the sixteenth (last) gallery page, with seven rows and no next page.
- `community-ee-{news,general,academic,seminars,employment,recruit}-detail-*`: the complete `.board_view` page shell.
  - `general/2867` keeps two inline images under `/se2/photo_uploader/upload/` and one `/aseoul_bbs/down.php?d=1&idx=2867` attachment.
  - `academic/2873` carries the `[공지]` editor badge as a `<span>`.

**sse**

- `community-sse-notices-list.html`, `community-sse-notices-page2.html`: page 1 has ten pins. Page 2 starts with three more pins and regular ordinals 117-111.
- `community-sse-news-list.html`: the nine-row gallery.
- `community-sse-seminars-list.html`: the only page (five rows) of a single-page board.
- `community-sse-{notices,news,seminars}-detail-*`: the `.view_bx` shell. `notices/198` keeps one `/se2/` image and one `/bbs/down.php?d=1&idx=198` attachment.

**robots and TLS**

- Both hosts' robots.txt disallows `/se2/`, `/data/` and `/bbs/`. The fixtures keep those URIs on purpose. Parser tests prove they are recorded, and the network policy refuses to request them.
- ee attachment downloads live under `/aseoul_bbs/`, which no rule covers.
- `openssl s_client -verify_return_error` returned `0 (ok)` for both hosts on 2026-11-05 (GoGetSSL chains). No intermediate exception was added.

`parser_tests.rs`, `engineering_tests.rs` and `community_tests.rs` check these structures. They also build invalid, empty and hostile cases separately. Counts are observations for these fixtures, not parser constants. Live content may change. A production mismatch must produce an explicit source or parser error, never an empty successful archive.

## Dormitory (2026-11-05)

Each list fixture keeps the original search form, `.srch_counts`, the `.boardWrap` table and the `pageForm` pager. Each detail fixture keeps `og:url` and the complete `.board-view` subtree. Line endings are LF.

- `dormitory-general-list.html`, `dormitory-general-page2.html`: pages 1 and 2 of `/sgdormitory/6975/subview.do`.
  - Page 1 has six pins plus regular ordinals 133-127. Pin `313348` is also row 127.
  - Page 2 has the same six pins and ordinals 126-120.
  - The declared total is 133 and the pager has 19 pages.
- `dormitory-admission-list.html`: page 1 of `/sgdormitory/3947/subview.do` (eight pins, ordinals 282-276, total 282, 41 pages).
- `dormitory-menu-list.html`, `dormitory-menu-last.html`: page 1 (one pin, ordinals 389-383) and page 56 (one pin, ordinals 4-1, no next page) of `/sgdormitory/3951/subview.do`. Total 389, 56 pages.
- `dormitory-general-detail-313348.html`: a text notice with no attachments and one inline `scc.sogang.ac.kr` HTTP image reference.
- `dormitory-admission-detail-283602.html`: four inline `scc.sogang.ac.kr` HTTP image references and no attachments.
- `dormitory-admission-detail-283698.html`: nine `/bbs/sgdormitory/1644/<id>/download.do` attachments.
- `dormitory-menu-detail-374772.html`: an image-only weekly menu. Its only body image is `/CrossEditor/binary/images/000035/...png` on the same CMS host.

The CMS host serves `/bbs/sgdormitory/...` attachments and `/CrossEditor/` editor images through a normal public route. Both were downloaded through Rhythm's transport. Inline images on `scc.sogang.ac.kr` are HTTP, and the HTTPS path returns 404. They stay recorded but unsupported references and are not fetched.

## k2web family (#80 step 2, 2026-11-23 to 2026-11-25)

Captured from `https://new-cms.sogang.ac.kr`, the host the family requests. Robots.txt was checked and requests ran one at a time. These are boards of other k2web sites that are not in the catalog, and the tests describe them with their own board rows.

- Lists keep the `<title>` and the search form's parent element (`.board-list` or `.album-list`: search form, count, rows and `pageForm` pager).
- Details keep `og:url` and the complete `.board-view` subtree.
- The message page keeps its `#messageForm`.
- Line endings are LF and trailing whitespace is removed.

Fixtures:

- `k2web-gsot-list.html`, `k2web-gsot-page2.html`, `k2web-gsot-last.html`: pages 1, 2 and 95 of `/gsot/3208/subview.do`. A table of 12 rows, eight pins repeated on every page and partly among the regular rows, category tabs, total 1,129.
- `k2web-gsot-detail-420698.html`: a `분류` item whose value is also shown as the title prefix (`[일반] …`). No attachments.
- `k2web-sociology-list.html`: 13 pins, and all ten regular rows are pins.
- `k2web-sociology-detail-352078.html`: its `og:url` is the literal `?`. The page is 17.4 MB because of one inline `data:image/png;base64,` image. Only that payload was cut to its first 64 base64 characters. Nothing else changed.
- `k2web-sociology-news-page11.html`, `k2web-sociology-news-detail-352050.html` (2026-11-25): page 11 of `/sociology/4376/subview.do` (catalogued `sociology-news`, table, 7 rows, total 85, 14 pins) and its notice 352050. Captured with the collector's user agent after production history passes kept failing on that page. The notice's title was saved with `<strong>` markup. The list nests it in a second `strong`, and the detail's hidden `artclViewTitle` keeps the escaped tags.
- `k2web-sbs-list.html`, `k2web-sbs-detail-425296.html`: no `작성자` item, a `[행사]` title prefix without a `분류` item, a hidden title escaped twice and one `/CrossEditor/` image.
- `k2web-gscom-alumni-list.html` (2026-11-25): page 1 of `/gscom/8106/subview.do` (catalogued `gscom-alumni`, table, 7 rows, total 124). Captured with the collector's user agent after production runs began failing it. It has six pins. Five belong to this board (`2712`) and all are repeated among the regular rows. One is labelled `전체게시판공지` and links to `gscom-notice`'s article `/bbs/gscom/2710/345497/artclView.do`.
- `k2web-science-list.html`, `k2web-science-detail-347256.html`: `board-table horizon4` (another column order), 12 rows.
- `k2web-gsinfo-list.html`, `k2web-gsinfo-detail-374796.html`: 52 pins, two attachments.
- `k2web-liberalarts-list.html`, `k2web-liberalarts-page2.html`, `k2web-liberalarts-last.html` (pages 1, 2, 24), `k2web-liberalarts-detail-425315.html`: an `.albumWrap` gallery (12 cards, `No. N`) and a poster whose body images are under `/sites/liberalarts/atchmnfl/bbs/1722/`.
- `k2web-innov-list.html`, `k2web-innov-detail-405464.html`: a `.thumnailWrap.webzine` gallery (`NO. N`) and a newsletter of third-party images with one attachment.
- `k2web-outbound-list.html`, `k2web-outbound-detail-420716.html`: the CMS message page `접근 권한이 없습니다.` that a login-only post answers with.
- `k2web-gradsch-page3.html` (2026-11-23): page 3 of `/gradsch/1244/subview.do` (일반대학원 공지사항, a catalogued board). Ordinal 770 is a private post, `비공개된 게시물입니다.`, with a `span.clsdr` marker and no link.
- `k2web-sori-detail-347556.html` (2026-11-24): 청년광장 공모전 (`sori-contests`), one public GET, trimmed the same way. The whole body is one inline `data:image/png;base64,` poster, and there is no attachment (`no-file`). Only that payload was cut to its first 64 base64 characters.

A census of all 129 k2web boards active in 2026 (list page 1 and one detail each, through the department host names, not committed) parsed with this family. Every list parsed. Every detail parsed except the login-only post and one whose read was cut at 8 MiB.

## Mechanical engineering (2026-11-05)

- `mechanical-sitemap.xml`: thirteen unchanged `<url>` elements trimmed from `https://me.sogang.ac.kr/sitemap.xml` (826,774 bytes, 4,804 URLs, 4,674 with `lastmod`).
  - It keeps `notice` 2419/2420/66, `award` 2427, `alumni_news` 1835, `research` 2425, `academic` 2395, `scholarship` 2407 and `events` 1762.
  - It also keeps an unrelated `gallery` article, the `notice` list page, an English-locale article and an `/about` page.
  - The sitemap is the only robots-allowed discovery document. The site forbids every `?page=` listing URL (`Disallow: /*?`).
- `mechanical-notice-detail-2419.html`, `mechanical-events-detail-1762.html`, `mechanical-award-detail-2427.html`, `mechanical-alumni-detail-1835.html`, `mechanical-research-detail-2425.html`: the original canonical link and the complete `<article class="container-site">` subtree. View counts and dates are the capture values.
  - 2419: six Supabase attachment links and one inline image.
  - 1762: one legacy R2 `.jpg` attachment.
  - 2427: a hero image outside `div.prose-sg`.
  - 1835: an embedded YouTube iframe and the external `youtu.be` link, which the sanitizer must drop.
  - 2425: one inline image and no attachments.

## Semiconductor (2026-11-06, 2026-11-23)

Only the page chrome outside `#inner_wrap` was removed. Line endings are LF.

- `semiconductor-notice-list.html`, `semiconductor-notice-page2.html`, `semiconductor-notice-last.html`: pages 1-3 of `https://se.sogang.ac.kr/board/board_list.php?board_id=notice`.
  - Page 1: seven repeated pins and ten regular rows (ordinals 25-16).
  - Page 2: the same seven pins and ten regular rows (15-6).
  - Page 3: the pins and five regular rows (5-1).
  - The pager shows first and last links only where they exist.
- `semiconductor-graduate-list.html`: the single page of `?board_id=notice_master`, five pins and seven regular rows.
- `semiconductor-news-list.html`, `semiconductor-news-last.html`: the two pages of `?board_id=news` (ten rows, then three, no pins).
- `semiconductor-careers-list.html`: the single page of `?board_id=recruit` (ten rows).
- `semiconductor-industry-list.html`: the single page of `?board_id=notice_industry`, one pin and six regular rows.
- `semiconductor-notice-detail-36.html`: `?board_id=notice&no=36`. One `/board/filedown.php` attachment (`출입권한신청서 TE605 ... .xlsx`).
- `semiconductor-notice-detail-39.html`: `no=39`. Text body, no attachments.
- `semiconductor-notice-detail-29.html` (2026-11-23, one public GET): `no=29`. Three `.hwp` attachments whose names hold literal `+` characters, written raw in the `filename` query.
- `semiconductor-news-detail-10.html`: `?board_id=news&no=10`. One same-host `/smart_editor/upload/...png` editor image.
- `semiconductor-news-detail-14.html`: `no=14`. Three inline images on `ee.sogang.ac.kr/se2/`, which that host's robots.txt disallows. They stay recorded but unsupported references.
- `semiconductor-graduate-detail-7.html`, `semiconductor-industry-detail-7.html`: text bodies with no attachments, for their board labels.
- `semiconductor-careers-detail-11.html`: `?board_id=recruit&no=11`. Four `.docx` attachments and a large table-formatted body.
  - The editor `data:` URI image in some notices (for example `notice` 34) is not a fetchable resource.
  - The parser test injects one into a fixture to prove it is dropped while the text and original response stay archived.

## Synthetic data

`dates/notices.json` is synthetic. It holds short Korean notice texts written for the #226 date-extractor prototype (`crates/sources/src/dates.rs`). It was not captured from any site.

## WordPress family (#80 step 4, 2026-11-24)

From `creative.sogang.ac.kr` (아트&테크놀로지학과), with the collector's user agent. Saved byte for byte.

- `wordpress-creative-list.json`, `wordpress-creative-last.json`, `wordpress-creative-past-end.json`: the News category (192) list at the exact URL `parser::list_url` builds. Pages 1 (50 posts), 6 (the last, 29 posts) and 7 (the `rest_post_invalid_page_number` envelope). 279 posts in all.
- `wordpress-creative-post-47154.json`, `wordpress-creative-post-47085.json`, `wordpress-creative-post-46650.json`: three posts at the exact `parser::detail_url` route.
  - 47154: text only.
  - 47085: six images under `/wp-content/uploads/2026/11/` (Korean file names).
  - 46650: one upload image and an embedded YouTube iframe the sanitizer must drop.

## Vendor family: scec and klec (#80 step 4, 2026-11-24)

From `https://scec.sogang.ac.kr` and `https://klec.sogang.ac.kr`. scec's robots.txt allows everything but `/nGmaster/`. klec has none. Requests ran one at a time with the collector's user agent. klec pages were sent with CRLF line endings. Saved line endings are LF and trailing whitespace is removed.

What each fixture keeps:

- scec lists: the board heading (`div.title`), the category tabs (`.sub-tabmenu`) and `section.list-wrap`.
- klec lists: the `SearchForm` (rows and pager).
- scec details: `div.viewer` and the `deleteForm`.
- klec details: the `bbs-view` subtree and the comment form up to its hidden `bbsid`. The form is unclosed on the site, so it is closed there.

**scec**

- `vendor-scec-notice-list.html`, `vendor-scec-notice-page3.html`, `vendor-scec-notice-last.html`, `vendor-scec-notice-past-end.html`: pages 1, 3, 39 and 40 of 공지사항 (전체, total 383). 25 pins fill pages 1-3. Page 40 is an empty table.
- `vendor-scec-notice-detail-90122.html`: category `전체`, one `.hwp` attachment, an `/upload/htmlarea/` image.
- `vendor-scec-notice-detail-89659.html`: category `교육원소식`.
- `vendor-scec-notice-detail-45763.html`: from 2006, no category, a `dnview.php` image and the same attachment listed twice.
- `vendor-scec-notice-missing.html`: the empty article frame for `bbs_idx=99999999`.

**klec**

- `vendor-klec-notice-list.html`, `vendor-klec-notice-last.html`, `vendor-klec-notice-past-end.html`: pages 1, 24 and 25 of 일반공지 (one pin, also a regular row).
- `vendor-klec-news-list.html`, `vendor-klec-news-last.html`, `vendor-klec-news-past-end.html`: pages 1, 18 and 19 of the 한국어교육원 소식 gallery.
- `vendor-klec-notice-detail-2978.html`: two `.hwp` attachments.
- `vendor-klec-notice-detail-2951.html`: `.pdf` and `.zip`.
- `vendor-klec-notice-detail-2933.html`: two `dnview.php` images and their files. The site itself sends U+FFFD for the lost emoji.
- `vendor-klec-notice-detail-344.html`: from 2015, with `/univ/upload/bbs/` images.
- `vendor-klec-news-detail-2988.html`: a news post.
- `vendor-klec-notice-untitled.html`, `vendor-klec-notice-detail-1888.html` (2026-11-24): page 14 of 일반공지 and its post 1888, which the site shows with an empty title. Trimmed to the list form and the article. The empty row stopped the board's history in production until the list accepted it.
- `vendor-klec-notice-detail-2757.html`: a second block titled `링크` (one Instagram link) after the two attachments. Found by the bounded live collector run.
- `vendor-klec-notice-wrong-board-2988.html`: news post 2988 requested with 일반공지's `bbsid=00010`. The site serves it, and its hidden `bbsid` still says `00011`.
- `vendor-klec-notice-denied.html`: the `글보기 권한이 없습니다.` page for `bbs_idx=99999999`. A members-only post shows the same page.

## Admission family (#80 step 4, 2026-11-24)

From `https://admission.sogang.ac.kr`. Its robots.txt is the site's HTML not-found page. Requests ran one at a time with the collector's user agent. Pages were sent with CRLF line endings. Saved line endings are LF and trailing whitespace is removed.

- Lists keep `p.pageTitle`, the category tabs, `div.boardSearch` (the declared total), `ul.boardList` and `div.pageArea`.
- Details keep `div.boardView` under the page's `<title>`.

Fixtures:

- `admission-notice-list.html`, `admission-notice-last.html`, `admission-notice-past-end.html`: pages 1, 2 and 3 of `notice.asp?s_page=N` (전체, total 20). Five pins on every page. Five regular rows on page 1 and ten on page 2. Page 3 lists only the pins.
- `admission-notice-detail-29514.html`: a PDF and an HWP through `download.File.asp`.
- `admission-notice-detail-24472.html`: from 2023, one PDF.
- `admission-notice-missing.html`: the whole answer for `idx=99999999`, unchanged.

## Academic calendar (#319, 2026-11-25)

- `academic-calendar.ics`: the unchanged public ICS of the school's 학사 일정 (`calendar::ACADEMIC_CALENDAR_URL`, the 캘린더 구독 link on `https://www.sogang.ac.kr/ko/academic-support/calendar`). 65 all-day events from 2025-11 to 2027-02. Its CRLF line endings are kept.
