use super::model::SourceDefinition;

/// Stable identities follow the owner's public reference catalog. Catalog presence is
/// not support: only the two families with verified article extraction are enabled.
pub fn definitions() -> Vec<SourceDefinition> {
    let mut sources = Vec::new();
    let mut add = |id: &str,
                   name: &str,
                   organization: &str,
                   family: &str,
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
        });
    };
    add(
        "sogang-academic",
        "학부 학사공지",
        "서강대학교",
        "main",
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
            format!("https://computing.sogang.ac.kr/ko/community/{board}/list"),
            true,
        );
    }
    for (id, board, name) in [
        ("academic", 1628, "학사공지"),
        ("research", 1627, "연구공지"),
        ("general", 1624, "일반공지"),
        ("careers", 8290, "취업정보"),
        ("newsletter", 1621, "뉴스레터"),
    ] {
        add(&format!("eng-{id}"), name, "서강대학교 · 공과대학", "cms", format!("https://eng.sogang.ac.kr/front/cmsboardlist.do?bbsConfigFK={board}&siteId=eng&currentPage=1"), false);
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
            format!("https://ee.sogang.ac.kr/kor/{path}"),
            false,
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
            format!("https://me.sogang.ac.kr/ko/board/{board}"),
            false,
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
            format!("https://sse.sogang.ac.kr/kor/community/{path}.php"),
            false,
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
            format!("https://se.sogang.ac.kr/board/board_list.php?board_id={board}"),
            false,
        );
    }
    sources
}
