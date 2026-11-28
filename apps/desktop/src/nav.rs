//! What each built-in extension adds to the shell, ported from
//! `src/lib/extensions.ts`. Core owns whether an extension is on; this only
//! maps its ID to pages and settings panes. Entries without an extension are
//! the core shell and always shown.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Today,
    Inbox,
    Search,
    Bookmark,
    Opportunities,
    Tasks,
    Calendar,
    Studium,
    Meals,
    Trash,
    Settings(Pane),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Extensions,
    General,
    Boards,
    Cybercampus,
    Personalization,
    Ai,
    Notifications,
    Data,
}

pub(crate) struct Item<T> {
    pub(crate) id: T,
    pub(crate) label: &'static str,
    pub(crate) extension: Option<&'static str>,
}

const fn item<T>(id: T, label: &'static str, extension: Option<&'static str>) -> Item<T> {
    Item {
        id,
        label,
        extension,
    }
}

pub(crate) const NAVIGATION: [Item<Page>; 10] = [
    item(Page::Today, "오늘", None),
    item(Page::Inbox, "받은 소식", Some("sogang_notices")),
    item(Page::Search, "검색", Some("search")),
    item(Page::Bookmark, "보관함", Some("saved")),
    // AI categories with the deadlines found in each notice (#87).
    item(Page::Opportunities, "기회", Some("ai")),
    item(Page::Tasks, "과제·마감", Some("cybercampus")),
    item(Page::Calendar, "캘린더", Some("calendar")),
    item(Page::Studium, "열람실", Some("studium")),
    // The dormitory menu from the feed (#318).
    item(Page::Meals, "식단", Some("meals")),
    // Shown only while a source extension is on or it still holds notices.
    item(Page::Trash, "휴지통", None),
];

pub(crate) const SETTINGS_PANES: [Item<Pane>; 8] = [
    item(Pane::Extensions, "확장 기능", None),
    item(Pane::General, "일반", None),
    item(Pane::Boards, "게시판", Some("sogang_notices")),
    item(Pane::Cybercampus, "사이버캠퍼스", Some("cybercampus")),
    item(Pane::Personalization, "개인화", Some("recommendations")),
    item(Pane::Ai, "AI", Some("ai")),
    item(Pane::Notifications, "알림·백그라운드", Some("proactive")),
    item(Pane::Data, "데이터 및 정보", None),
];

/// Extensions that own archive sources; 휴지통 belongs to them.
const SOURCE_EXTENSIONS: [&str; 2] = ["sogang_notices", "cybercampus"];

fn contributed<T>(items: &'static [Item<T>], enabled: &[&str]) -> Vec<&'static Item<T>> {
    items
        .iter()
        .filter(|item| item.extension.is_none_or(|id| enabled.contains(&id)))
        .collect()
}

/// The sidebar's pages. 휴지통 stays reachable while it still holds notices.
pub(crate) fn primary(enabled: &[&str], trashed: u64) -> Vec<&'static Item<Page>> {
    let sources_on = SOURCE_EXTENSIONS.iter().any(|id| enabled.contains(id));
    let mut items = contributed(&NAVIGATION, enabled);
    items.retain(|item| item.id != Page::Trash || sources_on || trashed > 0);
    items
}

pub(crate) fn panes(enabled: &[&str]) -> Vec<&'static Item<Pane>> {
    contributed(&SETTINGS_PANES, enabled)
}

/// Whether `page` is still reachable, for leaving a page whose extension
/// was just turned off.
pub(crate) fn shown(page: Page, enabled: &[&str], trashed: u64) -> bool {
    match page {
        Page::Settings(pane) => panes(enabled).iter().any(|item| item.id == pane),
        page => primary(enabled, trashed).iter().any(|item| item.id == page),
    }
}

pub(crate) fn heading(page: Page) -> &'static str {
    match page {
        Page::Settings(_) => "설정",
        page => NAVIGATION
            .iter()
            .find(|item| item.id == page)
            .map_or("", |item| item.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(enabled: &[&str], trashed: u64) -> Vec<Page> {
        primary(enabled, trashed)
            .iter()
            .map(|item| item.id)
            .collect()
    }

    fn pane_ids(enabled: &[&str]) -> Vec<Pane> {
        panes(enabled).iter().map(|item| item.id).collect()
    }

    #[test]
    fn all_off_is_the_core_shell() {
        assert_eq!(pages(&[], 0), [Page::Today]);
        assert_eq!(pages(&[], 3), [Page::Today, Page::Trash]);
        assert_eq!(pane_ids(&[]), [Pane::Extensions, Pane::General, Pane::Data]);
    }

    #[test]
    fn each_extension_adds_its_own_items() {
        let cases: [(&str, &[Page], &[Pane]); 11] = [
            (
                "sogang_notices",
                &[Page::Inbox, Page::Trash],
                &[Pane::Boards],
            ),
            (
                "cybercampus",
                &[Page::Tasks, Page::Trash],
                &[Pane::Cybercampus],
            ),
            ("search", &[Page::Search], &[]),
            ("saved", &[Page::Bookmark], &[]),
            ("recommendations", &[], &[Pane::Personalization]),
            ("week", &[], &[]),
            ("calendar", &[Page::Calendar], &[]),
            ("ai", &[Page::Opportunities], &[Pane::Ai]),
            ("proactive", &[], &[Pane::Notifications]),
            ("studium", &[Page::Studium], &[]),
            ("meals", &[Page::Meals], &[]),
        ];
        for (id, added_pages, added_panes) in cases {
            let mut want: Vec<Page> = vec![Page::Today];
            want.extend_from_slice(added_pages);
            assert_eq!(pages(&[id], 0), want, "{id}");
            let mut want = vec![Pane::Extensions, Pane::General];
            want.extend_from_slice(added_panes);
            want.push(Pane::Data);
            assert_eq!(pane_ids(&[id]), want, "{id}");
        }
        // The cases cover exactly core's built-in extensions.
        let mut tested: Vec<&str> = cases.iter().map(|(id, ..)| *id).collect();
        let mut built_in: Vec<&str> = rhythm_core::extensions::BUILT_IN
            .iter()
            .map(|extension| extension.id)
            .collect();
        tested.sort_unstable();
        built_in.sort_unstable();
        assert_eq!(tested, built_in);
    }

    #[test]
    fn a_page_disappears_with_its_extension() {
        assert!(shown(Page::Meals, &["meals"], 0));
        assert!(!shown(Page::Meals, &[], 0));
        assert!(shown(Page::Trash, &[], 1));
        assert!(!shown(Page::Settings(Pane::Ai), &[], 0));
        assert!(shown(Page::Settings(Pane::Data), &[], 0));
    }

    #[test]
    fn headings_follow_the_page() {
        assert_eq!(heading(Page::Opportunities), "기회");
        assert_eq!(heading(Page::Settings(Pane::Ai)), "설정");
    }
}
