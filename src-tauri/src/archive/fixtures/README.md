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

`parser_tests.rs` checks these structures and separately constructs invalid/empty
and hostile-content cases. Counts are observations for these fixtures, not parser
constants. Live university content may change; production mismatches must produce
an explicit source/parser error rather than an empty successful archive.
