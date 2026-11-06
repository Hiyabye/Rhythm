# Official-source parser fixtures

These bounded samples were retrieved from public Sogang University pages during the
v0.2.0 implementation (2026-10-24). They are deterministic compatibility fixtures,
not a feed or a copy of the university archive. University-authored content retains
its original rights; Rhythm's MIT license does not relicense it.

- `main-list-exact.json`, `main-page2.json`: pages 1 and 2 of
  `https://www.sogang.ac.kr/api/api/v1/mainKo/BbsData/boardList`, with
  `pageSize=50&bbsConfigFk=2` and the reference collector's empty filter parameters.
- `main-detail.json`: public JSON detail for `pkId=551637`; original HTML body,
  author, category references, publication date, and volatile view-count metadata.
- `computing-*-list.html`: only the original `.board-lnb` and `.board-content`
  elements from the five official `/ko/community/{board}/list?num=1` pages.
- `computing-academic-page2.html`: corresponding page 2. It retains the desktop and
  mobile mirrors to catch incorrect whole-document pin counting.
- `computing-*-detail.html`: original canonical link and `article.board-detail`
  elements for academicNotice/969, graduateNotice/128, externalInfo/1927, news/12,
  and career/3323. Unrelated CSS, navigation, analytics and translation dictionaries
  were removed. The Nuxt `FILE_LIST` data is unchanged; its IIFE was reduced to the
  exact scalar aliases referenced by that list, using the original invocation's
  values. No JavaScript was executed to obtain this data.

The following additional bounded captures were obtained during v0.2.1 verification
(2026-10-25), using public GETs only:

- `computing-news-detail-282.html`: canonical link, original `article.board-detail`,
  and the original Nuxt script from
  `https://computing.sogang.ac.kr/ko/community/news/detail/282`. No JavaScript was
  executed. The public route ID is 282, but `DETAIL.SN` and attachment
  `PARENT_SEQ` resolve to 562; attachment paths still use public ID 282. This
  regression requires both identities to agree with their respective contracts.
- `eng-{notice,academic,recruit,research}-list.html`: original canonical link and
  `<main>` from `https://eng.sogang.ac.kr/board/{board}` (notice/academic include
  `?page=1`). These current SSR pages replace the four corresponding retired CMS
  listing routes. The e뉴스레터 gallery (`/board/newsletter`) is not a source and
  has no fixtures.
- `eng-notice-page2.html`: same extraction from `/board/notice?page=2`. Unlike
  Computing, Engineering pins appear only on page one. Total 361 includes five
  pins; regular ordinals go from 356 on page one to 346 on page two.
- `eng-*-detail-*.html`: canonical link and original `<main>` for notice/20,
  notice/723, academic/4, recruit/676 and research/728. Author, UTC machine timestamps,
  visible Korean date/time, full `.post-body`, tables, attachment names and URLs
  are unchanged. Recruitment's captured article has the inline image
  `/files/fBshpBDiVU3e0JhUuiWvR/image-941579-1.png`; academic/4 has three
  extensionless `/download/{id}` links with authoritative filenames in markup.
  Navigation inside `<main>` remains so scope-extraction regressions are testable;
  unrelated document head, footer, and scripts are omitted. Line endings are
  normalized to LF and trailing line whitespace is removed; these are structural
  compatibility fixtures, not byte-for-byte HTTP response captures.

The following bounded captures were obtained during #78 (2026-11-04) through
Rhythm's own transport and robots policy, using public GETs only:

- `main-detail-551377.json`: unchanged public JSON detail for `pkId=551377`. Its
  body is an empty paragraph plus `div.iframe-wrapper` around a PDF-viewer iframe,
  which the iframe-only rule must recognize. The iframe was not fetched.
- `computing-career-content-396.html`: only the original inner HTML of
  `.sg-content.tiptap-view` from `https://computing.sogang.ac.kr/ko/community/career/detail/396`
  (legacy tables nested about 130 levels deep). The test places it inside the
  `career/3323` page fixture, so its page shell and Nuxt data are not duplicated.

The following bounded captures were obtained during #94 (2026-11-05), using public
GETs only. The Electronic and System Semiconductor department sites run the same
`aseoul` board engine (shared robots.txt rules, `?m=v&idx=` detail links, `pNo`
paging, pins-first ordering) with different skins; the two gallery boards use 12
and 9 rows per page, the other tables 10. Each fixture keeps only the real
`#sub_tit` (ee) or `.ttl01` (sse) title element and the complete `#board` subtree,
with scripts, style blocks and comments removed. Nothing was executed, resized or
retyped; line endings are LF and trailing whitespace is removed.

- `community-ee-{news,general,academic,seminars,employment,recruit}-list.html`:
  page 1 from `/kor/community/{notice01,notice02,notice03,seminar,employment}.php`
  and `/kor/recruit/recruit.php` with `?pNo=1&code=...`. `ee-general` page 1 holds
  only ten pins, so `community-ee-general-page2.html` (two more pins and regular
  ordinals 290-283) is the pin-across-pages regression.
- `community-ee-news-last.html`: the sixteenth (last) gallery page with seven rows
  and no next page.
- `community-ee-{news,general,academic,seminars,employment,recruit}-detail-*`:
  the complete `.board_view` page shell. `general/2867` keeps two inline images
  under `/se2/photo_uploader/upload/` and one `/aseoul_bbs/down.php?d=1&idx=2867`
  attachment; `academic/2873` carries the `[공지]` editor badge as a `<span>`.
- `community-sse-notices-list.html`, `community-sse-notices-page2.html`: page 1
  has ten pins; page 2 begins with three more pins and regular ordinals 117-111.
- `community-sse-news-list.html`: the nine-row gallery.
- `community-sse-seminars-list.html`: the only page (five rows) of a single-page
  board.
- `community-sse-{notices,news,seminars}-detail-*`: the `.view_bx` shell. The
  `notices/198` page keeps one `/se2/` image and one `/bbs/down.php?d=1&idx=198`
  attachment.

Both hosts' robots.txt disallows `/se2/`, `/data/` and `/bbs/`. These fixtures
intentionally retain those URIs so parser tests prove they are recorded, while
the network policy refuses to request them; ee attachment downloads live under
`/aseoul_bbs/`, which no rule covers. Running `openssl s_client
-verify_return_error` returned `0 (ok)` for both hosts on 2026-11-05 (GoGetSSL
chains); no intermediate exception was added.

`parser_tests.rs`, `engineering_tests.rs` and `community_tests.rs` check these
structures and separately construct invalid/empty and hostile-content cases.
Counts are observations for these fixtures, not parser constants. Live university content may change; production mismatches must produce
an explicit source/parser error rather than an empty successful archive.

The following bounded captures were obtained during the dormitory source addition
(2026-11-05), using public GETs only. Each list fixture keeps the original search
form, `.srch_counts`, `.boardWrap` table and `pageForm` pager; each detail fixture
keeps `og:url` and the complete `.board-view` subtree. Nothing was executed or
retyped; line endings are LF.

- `dormitory-general-list.html`, `dormitory-general-page2.html`: pages 1 and 2 of
  `/sgdormitory/6975/subview.do`. Page 1 has six pins plus regular ordinals
  133-127, where pin `313348` is also row 127; page 2 has the same six pins and
  ordinals 126-120. The declared total is 133 and the pager has 19 pages.
- `dormitory-admission-list.html`: page 1 of `/sgdormitory/3947/subview.do`
  (eight pins, ordinals 282-276, total 282, 41 pages).
- `dormitory-menu-list.html`, `dormitory-menu-last.html`: page 1 (one pin,
  ordinals 389-383) and page 56 (one pin, ordinals 4-1, no next page) of
  `/sgdormitory/3951/subview.do`; total 389, 56 pages.
- `dormitory-general-detail-313348.html`: a text notice with no attachments and
  one inline `scc.sogang.ac.kr` HTTP image reference.
- `dormitory-admission-detail-283602.html`: four inline `scc.sogang.ac.kr` HTTP
  image references and no attachments.
- `dormitory-admission-detail-283698.html`: nine `/bbs/sgdormitory/1644/<id>/download.do`
  attachments.
- `dormitory-menu-detail-374772.html`: an image-only weekly menu whose sole body
  image is `/CrossEditor/binary/images/000035/...png` on the same CMS host.

The CMS host serves `/bbs/sgdormitory/...` attachments and `/CrossEditor/` editor
images with a normal public route; both were downloaded through Rhythm's transport.
Inline images on `scc.sogang.ac.kr` are HTTP (and the HTTPS path returns 404), so
they remain recorded-but-unsupported references rather than being fetched.

The following bounded captures were obtained during the mechanical source addition
(2026-11-05), using public GETs only. Nothing was executed or retyped; line endings
are LF.

- `mechanical-sitemap.xml`: thirteen unchanged `<url>` elements trimmed from
  `https://me.sogang.ac.kr/sitemap.xml` (826,774 bytes, 4,804 URLs, 4,674 with
  `lastmod`). It keeps `notice` 2419/2420/66, `award` 2427, `alumni_news` 1835,
  `research` 2425, `academic` 2395, `scholarship` 2407, `events` 1762, an unrelated
  `gallery` article, the `notice` list page, an English-locale article and an
  `/about` page. The sitemap is the only robots-allowed discovery document: the site
  forbids every `?page=` listing URL (`Disallow: /*?`).
- `mechanical-notice-detail-2419.html`, `mechanical-events-detail-1762.html`,
  `mechanical-award-detail-2427.html`, `mechanical-alumni-detail-1835.html`,
  `mechanical-research-detail-2425.html`: the original canonical link and complete
  `<article class="container-site">` subtree. 2419 keeps six Supabase attachment
  links and one inline image; 1762 keeps one legacy R2 `.jpg` attachment; 2427 has a
  hero image outside `div.prose-sg`; 1835 has an embedded YouTube iframe (and the
  external `youtu.be` link) that the sanitizer must drop; 2425 has one inline image
  and no attachments. View counts and dates are the capture values.


The following bounded captures were obtained during the semiconductor source
addition (2026-11-06), using public GETs only. Nothing was executed; only the page
chrome outside `#inner_wrap` was removed, and line endings are LF.

- `semiconductor-notice-list.html`, `semiconductor-notice-page2.html`,
  `semiconductor-notice-last.html`: pages 1-3 of
  `https://se.sogang.ac.kr/board/board_list.php?board_id=notice`. Page 1 has seven
  repeated pins and ten regular rows (ordinals 25-16); page 2 has the same seven
  pins and ten regular rows (15-6); page 3 has the pins and five regular rows
  (5-1). The pager shows first/last links only where they exist.
- `semiconductor-graduate-list.html`: the single page of `?board_id=notice_master`
  with five pins and seven regular rows. `semiconductor-news-list.html` and
  `semiconductor-news-last.html`: the two pages of `?board_id=news` (ten rows then
  three, no pins). `semiconductor-careers-list.html`: the single page of
  `?board_id=recruit` (ten rows). `semiconductor-industry-list.html`: the single
  page of `?board_id=notice_industry` with one pin and six regular rows.
- `semiconductor-notice-detail-36.html`: `?board_id=notice&no=36`; one
  `/board/filedown.php` attachment (`출입권한신청서 TE605 ... .xlsx`).
  `semiconductor-notice-detail-39.html`: `no=39`; text body and no attachments.
- `semiconductor-news-detail-10.html`: `?board_id=news&no=10`; one same-host
  `/smart_editor/upload/...png` editor image. `semiconductor-news-detail-14.html`:
  `no=14`; three inline images on `ee.sogang.ac.kr/se2/`, which that host's
  robots.txt disallows, so they stay recorded-but-unsupported references.
- `semiconductor-graduate-detail-7.html`, `semiconductor-industry-detail-7.html`:
  text bodies with no attachments for their board labels.
- `semiconductor-careers-detail-11.html`: `?board_id=recruit&no=11`; four
  `.docx` attachments and a large table-formatted body. The editor `data:` URI
  image in some notices (for example `notice` 34) is not a fetchable resource; the
  parser test injects one into a fixture to prove it is dropped while the text and
  original response stay archived.

## Vision fixture

- `vision/poster.png`: wholly synthetic Korean notice poster drawn locally with
  AppKit (2026-11-05) for the local vision (`--mmproj`) evaluation. It is not
  collected university content and contains no real personal data: the title,
  deadline, eligibility, documents, method and the `02-705-0000` phone number are
  invented. It exists to check that an archived image reaches the model and that
  image-derived dates are labelled unverified, not to measure general quality.
