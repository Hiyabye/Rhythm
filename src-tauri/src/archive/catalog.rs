use super::model::{SourceDefinition, SourceGroup};

/// Settings groups, parents before children, in display order. Every board in
/// `definitions` except Cybercampus belongs to exactly one of them.
pub fn groups() -> Vec<SourceGroup> {
    [
        ("sogang", "서강대학교", None),
        ("sogang-eng", "공과대학", Some("sogang")),
        ("sogang-ee", "전자공학과", Some("sogang-eng")),
        ("sogang-me", "기계공학과", Some("sogang-eng")),
        ("sogang-sse", "시스템반도체공학과", Some("sogang-eng")),
        ("sogang-se", "반도체공학과", Some("sogang-eng")),
        ("sogang-computing", "소프트웨어융합대학", Some("sogang")),
        ("sogang-dorm", "벨라르미노 학사", Some("sogang")),
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
    for (id, menu, name) in [
        ("general", 6975, "일반공지"),
        ("admission", 3947, "입사 및 퇴사 공지"),
        ("menu", 3951, "주간 메뉴"),
    ] {
        add(
            &format!("dorm-{id}"),
            name,
            "서강대학교 · 벨라르미노 학사",
            "dormitory",
            "sogang-dorm",
            format!("https://new-cms.sogang.ac.kr/sgdormitory/{menu}/subview.do"),
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
    // The logged-in user's own course notices, read through the eClass session
    // (#86); not a public board, so it has its own description.
    sources.push(SourceDefinition {
        id: super::cybercampus::SOURCE_ID.into(),
        name: "강의 공지".into(),
        organization: "서강대학교 · 사이버캠퍼스".into(),
        description: "로그인한 계정의 이번 학기 수강 강좌 공지와 첨부 파일을 보관하고, 과제와 마감을 함께 가져옵니다.".into(),
        url: "https://eclass.sogang.ac.kr/ilos/main/main_form.acl".into(),
        family: super::cybercampus::FAMILY.into(),
        supported: true,
        group: None,
    });
    sources
}
