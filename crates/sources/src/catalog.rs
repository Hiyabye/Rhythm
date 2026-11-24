use super::model::{SourceDefinition, SourceGroup};

/// The authenticated eClass connector. The app fetches it with the user's own
/// session; it is listed here so every consumer sees one catalog.
pub const CYBERCAMPUS_SOURCE_ID: &str = "cybercampus";
pub const CYBERCAMPUS_FAMILY: &str = "cybercampus";

/// Settings groups, parents before children, in display order. Every board in
/// `definitions` except Cybercampus belongs to exactly one of them.
pub fn groups() -> Vec<SourceGroup> {
    [
        ("sogang", "서강대학교", None),
        // Unit categories (#80 step 3); departments sit under their college.
        ("sogang-colleges", "단과대학", Some("sogang")),
        ("sogang-liberalarts", "인문대학", Some("sogang-colleges")),
        ("sogang-korea", "국어국문학과", Some("sogang-liberalarts")),
        ("sogang-history", "사학과", Some("sogang-liberalarts")),
        ("sogang-philosophy", "철학과", Some("sogang-liberalarts")),
        ("sogang-religion", "종교학과", Some("sogang-liberalarts")),
        ("sogang-english", "영문학부", Some("sogang-liberalarts")),
        ("sogang-eu", "유럽문화학과", Some("sogang-liberalarts")),
        ("sogang-china", "중국문화학과", Some("sogang-liberalarts")),
        ("sogang-sdoss", "사회과학대학", Some("sogang-colleges")),
        ("sogang-sociology", "사회학과", Some("sogang-sdoss")),
        ("sogang-politics", "정치외교학과", Some("sogang-sdoss")),
        ("sogang-psychology", "심리학과", Some("sogang-sdoss")),
        ("sogang-science", "자연과학대학", Some("sogang-colleges")),
        ("sogang-math", "수학과", Some("sogang-science")),
        ("sogang-physics", "물리학과", Some("sogang-science")),
        ("sogang-chemistry", "화학과", Some("sogang-science")),
        ("sogang-lifescience", "생명과학과", Some("sogang-science")),
        ("sogang-eng", "공과대학", Some("sogang-colleges")),
        ("sogang-ee", "전자공학과", Some("sogang-eng")),
        ("sogang-me", "기계공학과", Some("sogang-eng")),
        ("sogang-sse", "시스템반도체공학과", Some("sogang-eng")),
        ("sogang-se", "반도체공학과", Some("sogang-eng")),
        ("sogang-econ", "경제대학", Some("sogang-colleges")),
        ("sogang-sbs", "경영대학", Some("sogang-colleges")),
        ("sogang-smas", "지식융합미디어대학", Some("sogang-colleges")),
        ("sogang-jas", "신문방송학과", Some("sogang-smas")),
        ("sogang-axt", "아트&테크놀로지학과", Some("sogang-smas")),
        (
            "sogang-computing",
            "소프트웨어융합대학",
            Some("sogang-colleges"),
        ),
        (
            "sogang-sciencebased",
            "SCIENCE기반 자유전공학부",
            Some("sogang-colleges"),
        ),
        ("sogang-grad", "대학원", Some("sogang")),
        ("sogang-gradsch", "일반대학원", Some("sogang-grad")),
        ("sogang-gsinfo", "AISW대학원", Some("sogang-grad")),
        ("sogang-gsm", "가상융합전문대학원", Some("sogang-grad")),
        ("sogang-econ-grad", "경제대학원", Some("sogang-grad")),
        ("sogang-gspp", "공공정책대학원", Some("sogang-grad")),
        ("sogang-gsedu", "교육대학원", Some("sogang-grad")),
        ("sogang-gsis", "국제대학원", Some("sogang-grad")),
        ("sogang-sgmot", "기술경영전문대학원", Some("sogang-grad")),
        (
            "sogang-gscom",
            "미디어커뮤니케이션대학원",
            Some("sogang-grad"),
        ),
        ("sogang-lawschool", "법학전문대학원", Some("sogang-grad")),
        ("sogang-gsot", "신학대학원", Some("sogang-grad")),
        ("sogang-offices", "행정·지원", Some("sogang")),
        (
            "sogang-sogangiis",
            "HUSS 포용사회사업단",
            Some("sogang-offices"),
        ),
        ("sogang-kyomok", "교목처", Some("sogang-offices")),
        ("sogang-purchase", "구매팀", Some("sogang-offices")),
        (
            "sogang-outbound",
            "국제팀 아웃바운드",
            Some("sogang-offices"),
        ),
        ("sogang-oisa", "국제학생팀", Some("sogang-offices")),
        (
            "sogang-globalaiedu",
            "글로벌AI인재양성센터",
            Some("sogang-offices"),
        ),
        (
            "sogang-sogangcouncil",
            "대학평의원회",
            Some("sogang-offices"),
        ),
        ("sogang-innov", "대학혁신추진단", Some("sogang-offices")),
        ("sogang-scec", "미래교육원", Some("sogang-offices")),
        ("sogang-dorm", "벨라르미노 학사", Some("sogang-offices")),
        (
            "sogang-psdic",
            "서강 판교디지털혁신캠퍼스",
            Some("sogang-offices"),
        ),
        (
            "sogang-sustainability",
            "서강글로벌사회공헌원",
            Some("sogang-offices"),
        ),
        ("sogang-yebigun", "예비군연대", Some("sogang-offices")),
        ("sogang-hrgec", "인권·성평등센터", Some("sogang-offices")),
        ("sogang-character", "인성교육센터", Some("sogang-offices")),
        ("sogang-admission", "입학처", Some("sogang-offices")),
        ("sogang-dasoni", "장애학생지원센터", Some("sogang-offices")),
        ("sogang-wholeperson", "전인교육원", Some("sogang-offices")),
        ("sogang-sori", "청년광장", Some("sogang-offices")),
        ("sogang-sggym", "체육관", Some("sogang-offices")),
        ("sogang-sgup", "출판부", Some("sogang-offices")),
        ("sogang-klec", "한국어교육원", Some("sogang-offices")),
        ("sogang-institutes", "연구소", Some("sogang")),
        (
            "sogang-dsri",
            "ICT융합재난안전연구소",
            Some("sogang-institutes"),
        ),
        ("sogang-sgbri", "경영연구소", Some("sogang-institutes")),
        (
            "sogang-cger2",
            "교양교육연구센터",
            Some("sogang-institutes"),
        ),
        ("sogang-siracs", "국제지역문화원", Some("sogang-institutes")),
        ("sogang-iias", "국제지역연구소", Some("sogang-institutes")),
        ("sogang-nano", "나노물질연구소", Some("sogang-institutes")),
        ("sogang-sieas", "동아연구소", Some("sogang-institutes")),
        ("sogang-sgdh", "디지털역사연구소", Some("sogang-institutes")),
        (
            "sogang-mri",
            "메타버스융합연구소",
            Some("sogang-institutes"),
        ),
        ("sogang-lawinst", "법학연구소", Some("sogang-institutes")),
        ("sogang-sgiss", "사회과학연구소", Some("sogang-institutes")),
        (
            "sogang-lifecult",
            "생명문화연구소",
            Some("sogang-institutes"),
        ),
        ("sogang-theoinst", "신학연구소", Some("sogang-institutes")),
        ("sogang-human", "인문과학연구소", Some("sogang-institutes")),
    ]
    .into_iter()
    .map(
        |(id, name, parent): (&str, &str, Option<&str>)| SourceGroup {
            id: id.into(),
            name: name.into(),
            parent: parent.map(Into::into),
            enabled: true,
        },
    )
    .collect()
}

/// Stable identities follow the owner's public reference catalog. Catalog presence is
/// not support: each enabled family needs full article and local-resource verification.
pub fn definitions() -> Vec<SourceDefinition> {
    let mut sources = Vec::new();
    let mut add = |id: &str,
                   name: &str,
                   organization: &str,
                   family: &str,
                   group: &str,
                   url: String,
                   supported| {
        sources.push(SourceDefinition {
            id: id.into(),
            name: name.into(),
            organization: organization.into(),
            description: if supported {
                format!("{organization}의 {name}. 공개 원문과 접근 가능한 과거 공지를 보관합니다.")
            } else {
                format!("{organization}의 {name}. 원문 수집기 검증 전으로 아직 구독할 수 없습니다.")
            },
            url,
            family: family.into(),
            supported,
            group: Some(group.into()),
        });
    };
    add(
        "sogang-academic",
        "학부 학사공지",
        "서강대학교",
        "main",
        "sogang",
        "https://www.sogang.ac.kr/ko/academic-support/notices".into(),
        true,
    );
    // Other boards of the same BbsData API (#80); `parser::MAIN_BOARDS` holds
    // their board IDs. 보도자료 is left out: its list marks every row pinned.
    for (id, name, path) in [
        ("general", "공지사항", "story/notification-general"),
        ("scholarship", "장학공지", "scholarship-notice"),
        ("events", "행사특강", "story/notification-event"),
        ("student", "학생 일반공지", "campus/student-support/notice"),
        // The site lists no page for this board (#315): its URL is the
        // service-room info page, and notice links carry only the board ID.
        ("onestop", "종합봉사실 공지", "service-room"),
    ] {
        add(
            &format!("sogang-{id}"),
            name,
            "서강대학교",
            "main",
            "sogang",
            format!("https://www.sogang.ac.kr/ko/{path}"),
            true,
        );
    }
    for (id, board, name) in [
        ("academic", "academicNotice", "학사 공지"),
        ("graduate", "graduateNotice", "대학원 공지"),
        ("external", "externalInfo", "대외정보"),
        ("updates", "news", "소식"),
        ("career", "career", "취업·인턴십"),
    ] {
        add(
            &format!("computing-{id}"),
            name,
            "서강대학교 · 소프트웨어융합대학",
            "computing",
            "sogang-computing",
            format!("https://computing.sogang.ac.kr/ko/community/{board}/list"),
            true,
        );
    }
    for (id, board, name) in [
        ("academic", "academic", "학사공지"),
        ("research", "research", "연구공지"),
        ("general", "notice", "일반공지"),
        ("careers", "recruit", "취업정보"),
    ] {
        add(
            &format!("eng-{id}"),
            name,
            "서강대학교 · 공과대학",
            "engineering",
            "sogang-eng",
            format!("https://eng.sogang.ac.kr/board/{board}"),
            true,
        );
    }
    // k2web CMS boards are one row each of `parser::K2WEB_BOARDS`.
    for board in crate::parser::K2WEB_BOARDS {
        add(
            board.source_id,
            board.name,
            board.organization,
            "k2web",
            board.group,
            format!(
                "https://new-cms.sogang.ac.kr/{}/{}/subview.do",
                board.site, board.menu
            ),
            true,
        );
    }
    for (id, path, name) in [
        ("news", "community/notice01.php", "학과소식"),
        ("general", "community/notice02.php", "일반공지"),
        ("academic", "community/notice03.php", "학사공지"),
        ("seminars", "community/seminar.php", "세미나"),
        ("employment", "community/employment.php", "취업정보"),
        ("recruit", "recruit/recruit.php", "채용공고"),
    ] {
        add(
            &format!("ee-{id}"),
            name,
            "서강대학교 · 전자공학과",
            "community",
            "sogang-ee",
            format!("https://ee.sogang.ac.kr/kor/{path}"),
            true,
        );
    }
    for (id, board, name) in [
        ("general", "notice", "일반공지"),
        ("academic", "academic", "학사공지"),
        ("research", "research", "연구성과"),
        ("awards", "award", "수상"),
        ("careers", "scholarship", "장학·취업정보"),
        ("events", "events", "외부 행사"),
        ("alumni", "alumni_news", "기계공학과 동문 소식"),
    ] {
        add(
            &format!("me-{id}"),
            name,
            "서강대학교 · 기계공학과",
            "mechanical",
            "sogang-me",
            format!("https://me.sogang.ac.kr/ko/board/{board}"),
            true,
        );
    }
    for (id, path, name) in [
        ("notices", "notice", "학과공지"),
        ("news", "news", "학과소식"),
        ("seminars", "seminar", "일반공지(세미나,홍보 등)"),
    ] {
        add(
            &format!("sse-{id}"),
            name,
            "서강대학교 · 시스템반도체공학과",
            "community",
            "sogang-sse",
            format!("https://sse.sogang.ac.kr/kor/community/{path}.php"),
            true,
        );
    }
    for (id, board, name) in [
        ("notices", "notice", "학과 공지사항"),
        ("graduate", "notice_master", "대학원 공지사항"),
        ("news", "news", "학과소식"),
        ("careers", "recruit", "채용/홍보"),
        ("industry", "notice_industry", "산학연 공지사항"),
    ] {
        add(
            &format!("se-{id}"),
            name,
            "서강대학교 · 반도체공학과",
            "semiconductor",
            "sogang-se",
            format!("https://se.sogang.ac.kr/board/board_list.php?board_id={board}"),
            true,
        );
    }
    // The department's WordPress News category (#80 step 4), read through its
    // REST API; `parser::WORDPRESS_BOARDS` holds the category.
    add(
        "creative-news",
        "News & Events",
        "서강대학교 · 아트&테크놀로지학과",
        "wordpress",
        "sogang-axt",
        "https://creative.sogang.ac.kr/blog/newsevents/".into(),
        true,
    );
    // The continuing-education sites on one outside vendor's board engine
    // (#80 step 4), one row each of `parser::VENDOR_BOARDS`.
    // 입학처 공지사항, every admission category as one board (#80 step 4).
    add(
        crate::parser::ADMISSION_SOURCE_ID,
        crate::parser::ADMISSION_NAME,
        "서강대학교 · 입학처",
        "admission",
        "sogang-admission",
        "https://admission.sogang.ac.kr/enter/html/counsel/notice.asp".into(),
        true,
    );
    for board in crate::parser::VENDOR_BOARDS.iter() {
        add(
            board.source_id,
            board.name,
            board.organization,
            "vendor",
            board.group,
            crate::parser::vendor_board_url(board),
            true,
        );
    }
    // The logged-in user's own course notices, read through the eClass session
    // (#86); not a public board, so it has its own description.
    sources.push(SourceDefinition {
        id: CYBERCAMPUS_SOURCE_ID.into(),
        name: "강의 공지".into(),
        organization: "서강대학교 · 사이버캠퍼스".into(),
        description: "로그인한 계정의 이번 학기 수강 강좌 공지와 첨부 파일을 보관하고, 과제와 마감을 함께 가져옵니다.".into(),
        url: "https://eclass.sogang.ac.kr/ilos/main/main_form.acl".into(),
        family: CYBERCAMPUS_FAMILY.into(),
        supported: true,
        group: None,
    });
    sources
}
