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
  listing routes, not the unsupported Engineering newsletter.
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

`parser_tests.rs` and `engineering_tests.rs` check these structures and separately
construct invalid/empty and hostile-content cases. Counts are observations for these fixtures, not parser
constants. Live university content may change; production mismatches must produce
an explicit source/parser error rather than an empty successful archive.
