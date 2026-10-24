use crate::storage::Store;
use serde::Serialize;

#[derive(Serialize)]
pub struct ExtensionInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub available: bool,
    pub enabled: bool,
}
// Stop signals cancellation synchronously; the owning async command waits for
// in-flight work to settle without holding the database lock.
pub trait Extension: Send + Sync {
    fn metadata(&self) -> ExtensionInfo;
    fn start(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}
struct SogangNotices {
    jobs: std::sync::Arc<crate::archive::sync::Controller>,
}
impl Extension for SogangNotices {
    fn metadata(&self) -> ExtensionInfo {
        ExtensionInfo {
            id: "sogang_notices",
            display_name: "서강대학교 공지",
            description: "구독한 학교 공지의 원문, 첨부 자료와 변경 이력을 이 기기에 보관합니다.",
            available: true,
            enabled: false,
        }
    }
    fn start(&self) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        self.jobs.cancel();
        Ok(())
    }
}
pub struct Registry {
    modules: Vec<Box<dyn Extension>>,
}
impl Registry {
    pub fn new(jobs: std::sync::Arc<crate::archive::sync::Controller>) -> Self {
        Self {
            modules: vec![Box::new(SogangNotices { jobs })],
        }
    }
    pub fn list(&self, store: &Store) -> Result<Vec<ExtensionInfo>, String> {
        self.modules
            .iter()
            .map(|m| {
                let mut info = m.metadata();
                info.enabled = store.enabled(info.id)?;
                Ok(info)
            })
            .collect()
    }
    pub fn set(&self, store: &Store, id: &str, enabled: bool) -> Result<(), String> {
        let module = self
            .modules
            .iter()
            .find(|m| m.metadata().id == id)
            .ok_or("등록되지 않은 확장 기능입니다.")?;
        if enabled && module.metadata().available {
            module.start()?;
        } else {
            module.stop()?;
        }
        store.set_enabled(id, enabled)
    }
    pub fn stop_all(&self) {
        for module in &self.modules {
            if let Err(error) = module.stop() {
                eprintln!("Extension shutdown: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_extension_preserves_preferences_without_automatic_networking() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("rhythm.db")).unwrap();
        let registry = Registry::new(crate::archive::sync::Controller::new());
        registry.set(&store, "sogang_notices", true).unwrap();
        let enabled = registry.list(&store).unwrap();
        assert!(enabled[0].enabled);
        assert!(enabled[0].available);
        registry.set(&store, "sogang_notices", false).unwrap();
        assert!(!registry.list(&store).unwrap()[0].enabled);
        assert_eq!(store.settings().unwrap().theme, "system");
        assert!(registry.set(&store, "unknown", true).is_err());
    }
}
