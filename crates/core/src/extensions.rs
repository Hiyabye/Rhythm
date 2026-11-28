use crate::storage::Store;
use serde::Serialize;

pub const NOTICES: &str = "sogang_notices";
pub const CYBERCAMPUS: &str = "cybercampus";
pub const SEARCH: &str = "search";
pub const SAVED: &str = "saved";
pub const RECOMMENDATIONS: &str = "recommendations";
pub const WEEK: &str = "week";
pub const CALENDAR: &str = "calendar";
pub const AI: &str = "ai";
pub const PROACTIVE: &str = "proactive";
pub const STUDIUM: &str = "studium";
pub const MEALS: &str = "meals";

#[derive(Serialize)]
pub struct ExtensionInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub available: bool,
    pub enabled: bool,
}

/// Work an extension owns and that its disable stops or settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    /// The archive job controller. A disable cancels only a job the extension
    /// owns: `Controller::cancel_owned_by`.
    Archive,
    /// The feed sync.
    Feed,
    /// Bodies of opened feed notices.
    Bodies,
    /// The Studium generation. Its disable also needs the view close ticket
    /// and `studium::jobs::finish_disable`, which `App::set_extension` runs.
    Studium,
}

/// A background loop an extension owns. It runs only while the extension is
/// on: `App::start` and an enable start it, a disable stops and joins it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    /// The classification loop and its embedding process.
    Classifier,
}

/// Activation tokens a disable clears: one-use records that would otherwise
/// let a stale notification act after the extension was turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tokens {
    StudiumAlerts,
    CalendarReminders,
}

/// The one static declaration of an extension. Extension-specific lifecycle
/// is data here; `Registry`, `App::set_extension` and `Store::set_enabled`
/// read it and name no extension (`docs/ARCHITECTURE.md` "Core and extensions").
pub struct Extension {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    /// Signalled by `Registry::set` under the Store lock when this turns off,
    /// in order.
    pub cancels: &'static [Work],
    /// What `App::set_extension` awaits after that lock is released, in order.
    pub settles: &'static [Work],
    /// Background loops that run only while this is on.
    pub runs: &'static [Loop],
    /// Cleared in the same transaction that records the disable.
    pub clears: &'static [Tokens],
    /// Turning this on moves the notification baseline to the newest archived
    /// revision in the same transaction, so notices that arrived while it was
    /// off are not announced afterwards.
    pub announces_from_enable: bool,
}
impl Extension {
    const fn new(id: &'static str, display_name: &'static str, description: &'static str) -> Self {
        Self {
            id,
            display_name,
            description,
            cancels: &[],
            settles: &[],
            runs: &[],
            clears: &[],
            announces_from_enable: false,
        }
    }
    const fn cancels(mut self, work: &'static [Work]) -> Self {
        self.cancels = work;
        self
    }
    const fn settles(mut self, work: &'static [Work]) -> Self {
        self.settles = work;
        self
    }
    const fn runs(mut self, loops: &'static [Loop]) -> Self {
        self.runs = loops;
        self
    }
    const fn clears(mut self, tokens: &'static [Tokens]) -> Self {
        self.clears = tokens;
        self
    }
    const fn announces_from_enable(mut self) -> Self {
        self.announces_from_enable = true;
        self
    }
    /// Whether the extension feeds the Studium alert pipeline, so that its
    /// disable cancels pending alert handoffs and its enable resumes them.
    pub fn feeds_studium_alerts(&self) -> bool {
        self.clears.contains(&Tokens::StudiumAlerts)
    }
}

/// Every feature beyond the core shell is a built-in extension, listed in the
/// order the UI shows them. Nothing is loaded from outside the app.
pub const BUILT_IN: &[Extension] = &[
    Extension::new(
        NOTICES,
        "서강대학교 공지",
        "학교 게시판 공지를 받아 이 기기에 보관합니다.",
    )
    .cancels(&[Work::Archive, Work::Feed, Work::Bodies])
    .settles(&[Work::Archive, Work::Bodies]),
    Extension::new(
        CYBERCAMPUS,
        "사이버캠퍼스",
        "eClass에서 수강 과목의 공지와 과제·마감을 가져옵니다.",
    )
    .cancels(&[Work::Archive])
    .settles(&[Work::Archive]),
    Extension::new(SEARCH, "검색", "보관한 공지를 검색합니다."),
    Extension::new(SAVED, "보관함", "저장한 공지를 모아 봅니다."),
    Extension::new(
        RECOMMENDATIONS,
        "나를 위한 소식",
        "관심사에 맞는 공지를 골라 줍니다.",
    ),
    Extension::new(
        WEEK,
        "앞으로 7일",
        "다가오는 일주일의 공지와 마감을 보여 줍니다.",
    ),
    Extension::new(
        CALENDAR,
        "캘린더",
        "마감, 공지 속 날짜, 할 일을 달력으로 보여 줍니다.",
    )
    .clears(&[Tokens::CalendarReminders]),
    Extension::new(
        AI,
        "AI 분류",
        "이 기기에서 공지를 주제별로 나눕니다. 비슷한 공지, 뜻으로 찾는 검색, 기회 화면도 함께 켜집니다.",
    )
    .runs(&[Loop::Classifier]),
    Extension::new(
        PROACTIVE,
        "알림·백그라운드",
        "앱이 켜져 있는 동안 새 공지를 알립니다. 로그인 시 실행도 정합니다.",
    )
    .clears(&[Tokens::StudiumAlerts, Tokens::CalendarReminders])
    .announces_from_enable(),
    Extension::new(STUDIUM, "열람실", "열람실의 빈 좌석을 확인합니다.")
        .cancels(&[Work::Studium])
        .settles(&[Work::Studium])
        .clears(&[Tokens::StudiumAlerts]),
    Extension::new(
        MEALS,
        "식단",
        "벨라르미노 학사 식당의 주간 메뉴를 보여 줍니다.",
    ),
];

/// The declaration of built-in extension `id`.
pub fn declaration(id: &str) -> Option<&'static Extension> {
    BUILT_IN.iter().find(|extension| extension.id == id)
}

/// The extension that owns a catalog source family.
pub fn owner(family: &str) -> &'static str {
    if family == rhythm_sources::catalog::CYBERCAMPUS_FAMILY {
        CYBERCAMPUS
    } else {
        NOTICES
    }
}

/// Every built-in extension with its stored on/off state, in UI order.
pub fn list(store: &Store) -> Result<Vec<ExtensionInfo>, String> {
    BUILT_IN
        .iter()
        .map(|extension| {
            Ok(ExtensionInfo {
                id: extension.id,
                display_name: extension.display_name,
                description: extension.description,
                available: true,
                enabled: store.enabled(extension.id)?,
            })
        })
        .collect()
}

pub fn is_built_in(id: &str) -> bool {
    declaration(id).is_some()
}

/// What [`Registry::change`] leaves to await outside the Store lock.
pub struct Changed<T> {
    pub studium: Option<crate::studium::controller::CompletionHandle>,
    pub notifications: Option<crate::studium::notification::Completion>,
    pub captured: Option<T>,
}

/// Records extension choices and stops the work of an extension turned off.
pub struct Registry {
    jobs: std::sync::Arc<crate::archive::sync::Controller>,
    studium: std::sync::Arc<crate::studium::controller::Controller>,
    feed: std::sync::Arc<crate::feed::Controller>,
}
impl Registry {
    pub fn new(
        jobs: std::sync::Arc<crate::archive::sync::Controller>,
        studium: std::sync::Arc<crate::studium::controller::Controller>,
        feed: std::sync::Arc<crate::feed::Controller>,
    ) -> Self {
        Self {
            jobs,
            studium,
            feed,
        }
    }
    pub fn list(&self, store: &Store) -> Result<Vec<ExtensionInfo>, String> {
        list(store)
    }
    /// Records the choice. Disabling a source extension signals archive
    /// cancellation under the caller's store lock, the same lock job admission
    /// takes, so no job of its sources can start or continue afterwards. Work
    /// that needs to await (jobs, loops, lifecycle) is settled by the caller.
    pub fn set(&self, store: &Store, id: &str, enabled: bool) -> Result<(), String> {
        let Some(extension) = declaration(id) else {
            return Err("등록되지 않은 확장 기능입니다.".into());
        };
        // The caller holds Store's lock, which also protects admission. Commit
        // the preference before irreversible cancellation/clearing so a failed
        // write leaves the currently enabled extension and its work intact.
        store.set_enabled(id, enabled)?;
        if !enabled {
            for work in extension.cancels {
                match work {
                    Work::Archive => self.jobs.cancel_owned_by(id),
                    Work::Feed => self.feed.cancel(),
                    Work::Bodies => self.jobs.bodies.cancel(),
                    Work::Studium => {
                        self.studium.cancel();
                        self.studium.clear();
                    }
                }
            }
        }
        Ok(())
    }
    /// Turns `id` on or off under the Store lock that job admission also
    /// takes, and returns what a disable must await after the lock is
    /// released: the Studium generation, its alert handoffs and whatever
    /// `capture` took under the same lock when Studium turned off (the app's
    /// mini window close ticket).
    pub fn change<T>(
        &self,
        shared_store: &crate::storage::SharedStore,
        id: &str,
        enabled: bool,
        capture: impl FnOnce() -> Option<T>,
    ) -> Result<Changed<T>, String> {
        let store = shared_store
            .lock()
            .map_err(|_| "저장소에 접근할 수 없습니다.")?;
        self.set(&store, id, enabled)?;
        // `set` accepted `id`, so it has a declaration.
        let extension = declaration(id).ok_or("등록되지 않은 확장 기능입니다.")?;
        let notifications = if !enabled && extension.feeds_studium_alerts() {
            self.studium.cancel_notifications()
        } else {
            None
        };
        let studium_off = !enabled && extension.cancels.contains(&Work::Studium);
        let captured = if studium_off { capture() } else { None };
        Ok(Changed {
            studium: if studium_off {
                self.studium.completion()
            } else {
                None
            },
            notifications,
            captured,
        })
    }
    pub fn stop_all(&self) {
        self.jobs.cancel();
        self.studium.cancel();
        self.feed.cancel();
        self.jobs.bodies.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cybercampus_family_has_its_own_extension() {
        assert_eq!(
            owner(rhythm_sources::catalog::CYBERCAMPUS_FAMILY),
            CYBERCAMPUS
        );
        assert_eq!(owner("engineering"), NOTICES);
    }

    #[test]
    fn operational_extension_preserves_preferences_without_automatic_networking() {
        let (_directory, store) = crate::storage::temp_store();
        let registry = Registry::new(
            crate::archive::sync::Controller::new(),
            crate::studium::controller::Controller::new(),
            Default::default(),
        );
        let listed = registry.list(&store).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.id).collect::<Vec<_>>(),
            [
                NOTICES,
                CYBERCAMPUS,
                SEARCH,
                SAVED,
                RECOMMENDATIONS,
                WEEK,
                CALENDAR,
                AI,
                PROACTIVE,
                STUDIUM,
                MEALS
            ]
        );
        // A new profile starts with every extension off.
        assert!(listed.iter().all(|e| !e.enabled && e.available));
        registry.set(&store, NOTICES, true).unwrap();
        registry.set(&store, AI, true).unwrap();
        let enabled = registry.list(&store).unwrap();
        let on = |listing: &[ExtensionInfo], id: &str| {
            listing.iter().find(|e| e.id == id).unwrap().enabled
        };
        assert!(on(&enabled, NOTICES) && on(&enabled, AI));
        assert!(!on(&enabled, CYBERCAMPUS));
        registry.set(&store, NOTICES, false).unwrap();
        assert!(!on(&registry.list(&store).unwrap(), NOTICES));
        assert_eq!(store.settings().unwrap().theme, "system");
        assert!(registry.set(&store, "unknown", true).is_err());
    }

    #[test]
    fn studium_can_toggle_without_changing_targets_or_starting_work() {
        let (_directory, store) = crate::storage::temp_store();
        let registry = Registry::new(
            crate::archive::sync::Controller::new(),
            crate::studium::controller::Controller::new(),
            Default::default(),
        );
        assert!(!store.enabled(STUDIUM).unwrap());
        registry.set(&store, STUDIUM, true).unwrap();
        store
            .studium_save_preferences(&["5".into(), "6".into()], "6")
            .unwrap();
        registry.set(&store, STUDIUM, false).unwrap();
        assert!(!store.enabled(STUDIUM).unwrap());
        assert_eq!(store.studium_preferences().unwrap().view_room, "6");
        assert_eq!(store.studium_preferences().unwrap().room_ids, ["5", "6"]);
    }
}
