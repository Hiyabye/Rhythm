//! The eClass login lives only in the OS credential store (user decision, #86):
//! never in SQLite, backups, diagnostics or logs.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The production bundle identifier; its service name is the one every saved
/// login already uses.
pub const PRODUCTION_IDENTIFIER: &str = "io.github.hiyabye.rhythm";
const ACCOUNT: &str = "eclass";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Credentials {
    pub id: String,
    pub password: String,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl Credentials {
    pub fn new(id: &str, password: &str) -> Result<Self, String> {
        let id = id.trim();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("사이버캠퍼스 아이디(학번)를 확인해 주세요.".into());
        }
        if password.is_empty() || password.len() > 256 || password.chars().any(char::is_control) {
            return Err("비밀번호를 확인해 주세요.".into());
        }
        Ok(Self {
            id: id.into(),
            password: password.into(),
        })
    }
}

/// The credential-store service of one bundle identifier. A validation build
/// has its own identifier and so its own entry, never the production login.
pub fn service_name(identifier: &str) -> String {
    format!("{identifier}.cybercampus")
}

enum Backend {
    Native(String),
    #[cfg(any(test, feature = "test-support"))]
    Memory(Mutex<Option<String>>),
}
pub struct Vault {
    backend: Backend,
    /// Read once per process so a sync does not query the store for every request.
    cached: Mutex<Option<Option<Credentials>>>,
}
impl Vault {
    pub fn native(identifier: &str) -> Self {
        Self {
            backend: Backend::Native(service_name(identifier)),
            cached: Mutex::new(None),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn memory() -> Self {
        Self {
            backend: Backend::Memory(Mutex::new(None)),
            cached: Mutex::new(None),
        }
    }
    pub fn load(&self) -> Result<Option<Credentials>, String> {
        if let Some(cached) = self.cached.lock().ok().and_then(|value| value.clone()) {
            return Ok(cached);
        }
        let stored = match &self.backend {
            Backend::Native(service) => native_get(service)?,
            #[cfg(any(test, feature = "test-support"))]
            Backend::Memory(value) => value.lock().map_err(|_| LOCKED)?.clone(),
        };
        let credentials = stored.and_then(|value| serde_json::from_str(&value).ok());
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some(credentials.clone());
        }
        Ok(credentials)
    }
    pub fn save(&self, credentials: &Credentials) -> Result<(), String> {
        let encoded = serde_json::to_string(credentials).map_err(|_| LOCKED)?;
        match &self.backend {
            Backend::Native(service) => native_set(service, &encoded)?,
            #[cfg(any(test, feature = "test-support"))]
            Backend::Memory(value) => *value.lock().map_err(|_| LOCKED)? = Some(encoded),
        }
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some(Some(credentials.clone()));
        }
        Ok(())
    }
    pub fn clear(&self) -> Result<(), String> {
        match &self.backend {
            Backend::Native(service) => native_clear(service)?,
            #[cfg(any(test, feature = "test-support"))]
            Backend::Memory(value) => *value.lock().map_err(|_| LOCKED)? = None,
        }
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some(None);
        }
        Ok(())
    }
}
const LOCKED: &str = "사이버캠퍼스 로그인 정보에 접근하지 못했습니다.";

#[cfg(any(target_os = "macos", windows))]
fn entry(service: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, ACCOUNT)
        .map_err(|_| "운영체제 보안 저장소에 접근하지 못했습니다.".to_owned())
}
#[cfg(any(target_os = "macos", windows))]
fn native_get(service: &str) -> Result<Option<String>, String> {
    match entry(service)?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => {
            Err("운영체제 보안 저장소에서 사이버캠퍼스 로그인 정보를 읽지 못했습니다.".into())
        }
    }
}
#[cfg(any(target_os = "macos", windows))]
fn native_set(service: &str, value: &str) -> Result<(), String> {
    entry(service)?
        .set_password(value)
        .map_err(|_| "로그인 정보를 운영체제 보안 저장소에 저장하지 못했습니다.".to_owned())
}
#[cfg(any(target_os = "macos", windows))]
fn native_clear(service: &str) -> Result<(), String> {
    match entry(service)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("로그인 정보를 운영체제 보안 저장소에서 삭제하지 못했습니다.".into()),
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_get(service: &str) -> Result<Option<String>, String> {
    let _ = (service, ACCOUNT);
    Ok(None)
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_set(_service: &str, _value: &str) -> Result<(), String> {
    Err("이 운영체제에서는 사이버캠퍼스 로그인 정보를 저장할 수 없습니다.".into())
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_clear(service: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_identifier_has_its_own_service_and_production_is_unchanged() {
        assert_eq!(
            service_name(PRODUCTION_IDENTIFIER),
            "io.github.hiyabye.rhythm.cybercampus"
        );
        let check = service_name("io.github.hiyabye.rhythm.corecheck");
        let other = service_name("io.github.hiyabye.rhythm.updatecheck");
        assert_eq!(check, "io.github.hiyabye.rhythm.corecheck.cybercampus");
        assert_ne!(check, service_name(PRODUCTION_IDENTIFIER));
        assert_ne!(check, other);
    }
}
