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
// Lifecycle implementations must cancel their own jobs and notifications before stop returns.
// No connector is active in v0.1.0; saved opt-in is independent from availability.
pub trait Extension: Send + Sync {
    fn metadata(&self) -> ExtensionInfo;
    fn start(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}
struct SogangNotices;
impl Extension for SogangNotices {
    fn metadata(&self) -> ExtensionInfo {
        ExtensionInfo{id:"sogang_notices",display_name:"서강대학교 공지",description:"학교 공지를 모아 원문과 변경 이력을 보관합니다. 동기화는 v0.2.0에서 제공할 예정입니다.",available:false,enabled:false}
    }
    fn start(&self) -> Result<(), String> {
        Err("공지 동기화는 아직 지원하지 않습니다.".into())
    }
    fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}
pub struct Registry {
    modules: Vec<Box<dyn Extension>>,
}
impl Registry {
    pub fn new() -> Self {
        Self {
            modules: vec![Box::new(SogangNotices)],
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
    fn unavailable_extension_saves_opt_in_without_starting_or_deleting_data() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("rhythm.db")).unwrap();
        let registry = Registry::new();
        registry.set(&store, "sogang_notices", true).unwrap();
        let enabled = registry.list(&store).unwrap();
        assert!(enabled[0].enabled);
        assert!(!enabled[0].available);
        registry.set(&store, "sogang_notices", false).unwrap();
        assert!(!registry.list(&store).unwrap()[0].enabled);
        assert_eq!(store.settings().unwrap().theme, "system");
        assert!(registry.set(&store, "unknown", true).is_err());
    }
}
