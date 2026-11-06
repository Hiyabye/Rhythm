//! The eClass login lives only in the OS credential store (user decision, #86):
//! never in SQLite, backups, diagnostics or logs.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const SERVICE: &str = "io.github.hiyabye.rhythm.cybercampus";
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

enum Backend {
    Native,
    #[cfg(test)]
    Memory(Mutex<Option<String>>),
}
pub struct Vault {
    backend: Backend,
    /// Read once per process so a sync does not query the store for every request.
    cached: Mutex<Option<Option<Credentials>>>,
}
impl Vault {
    pub fn native() -> Self {
        Self {
            backend: Backend::Native,
            cached: Mutex::new(None),
        }
    }
    #[cfg(test)]
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
            Backend::Native => native_get()?,
            #[cfg(test)]
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
            Backend::Native => native_set(&encoded)?,
            #[cfg(test)]
            Backend::Memory(value) => *value.lock().map_err(|_| LOCKED)? = Some(encoded),
        }
        if let Ok(mut cached) = self.cached.lock() {
            *cached = Some(Some(credentials.clone()));
        }
        Ok(())
    }
    pub fn clear(&self) -> Result<(), String> {
        match &self.backend {
            Backend::Native => native_clear()?,
            #[cfg(test)]
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
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| "운영체제 보안 저장소에 접근하지 못했습니다.".to_owned())
}
#[cfg(any(target_os = "macos", windows))]
fn native_get() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => {
            Err("운영체제 보안 저장소에서 사이버캠퍼스 로그인 정보를 읽지 못했습니다.".into())
        }
    }
}
#[cfg(any(target_os = "macos", windows))]
fn native_set(value: &str) -> Result<(), String> {
    entry()?
        .set_password(value)
        .map_err(|_| "로그인 정보를 운영체제 보안 저장소에 저장하지 못했습니다.".to_owned())
}
#[cfg(any(target_os = "macos", windows))]
fn native_clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("로그인 정보를 운영체제 보안 저장소에서 삭제하지 못했습니다.".into()),
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_get() -> Result<Option<String>, String> {
    let _ = (SERVICE, ACCOUNT);
    Ok(None)
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_set(_value: &str) -> Result<(), String> {
    Err("이 운영체제에서는 사이버캠퍼스 로그인 정보를 저장할 수 없습니다.".into())
}
#[cfg(not(any(target_os = "macos", windows)))]
fn native_clear() -> Result<(), String> {
    Ok(())
}
