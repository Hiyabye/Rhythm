//! Per-user Windows login entry. Never touches machine-wide startup entries.
use std::path::Path;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
            RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY,
            REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
        },
    },
};

const NAME: PCWSTR = w!("Rhythm.app.rhythm.windows");
const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");

/// Exact per-user values, including a registered but Windows-disabled entry.
pub(crate) struct Snapshot {
    run: Option<(REG_VALUE_TYPE, Vec<u8>)>,
    approved: Option<(REG_VALUE_TYPE, Vec<u8>)>,
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(path: PCWSTR, access: REG_SAM_FLAGS, create: bool) -> Result<Option<Key>, String> {
    let mut key = HKEY::default();
    let code = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, path, None, access, &mut key)
        }
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS {
        return Err(format!("Windows 설정 오류 {}", code.0));
    }
    Ok(Some(Key(key)))
}

fn read(key: &Key) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>, String> {
    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    let code =
        unsafe { RegQueryValueExW(key.0, NAME, None, Some(&mut kind), None, Some(&mut size)) };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS || size > 65536 {
        return Err("자동 시작 설정을 읽지 못했습니다.".into());
    }
    let mut bytes = vec![0u8; size as usize];
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            NAME,
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if code != ERROR_SUCCESS || size as usize > bytes.len() {
        return Err("자동 시작 설정을 읽지 못했습니다.".into());
    }
    bytes.truncate(size as usize);
    Ok(Some((kind, bytes)))
}

fn read_path(path: PCWSTR) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>, String> {
    match open(path, KEY_QUERY_VALUE, false)? {
        Some(key) => read(&key),
        None => Ok(None),
    }
}

fn write(key: &Key, kind: REG_VALUE_TYPE, bytes: &[u8]) -> Result<(), String> {
    let code = unsafe { RegSetValueExW(key.0, NAME, None, kind, Some(bytes)) };
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("Windows 설정 오류 {}", code.0))
    }
}

fn delete(key: &Key) -> Result<(), String> {
    let code = unsafe { RegDeleteValueW(key.0, NAME) };
    if code == ERROR_SUCCESS || code == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("Windows 설정 오류 {}", code.0))
    }
}

fn executable_command(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("실행 파일 경로를 읽지 못했습니다.")?;
    if path.contains('"') {
        return Err("실행 파일 경로가 올바르지 않습니다.".into());
    }
    Ok(format!("\"{path}\" --autostart"))
}

fn registry_string(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut words: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    if words.pop() != Some(0) || words.contains(&0) {
        return None;
    }
    String::from_utf16(&words).ok()
}

fn command_bytes(command: &str) -> Vec<u8> {
    command
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn registered_command() -> Result<Option<String>, String> {
    let Some(key) = open(RUN, KEY_QUERY_VALUE, false)? else {
        return Ok(None);
    };
    let Some((kind, data)) = read(&key)? else {
        return Ok(None);
    };
    if kind != REG_SZ {
        return Err("자동 시작 항목의 형식이 올바르지 않습니다.".into());
    }
    registry_string(&data)
        .map(Some)
        .ok_or_else(|| "자동 시작 항목의 형식이 올바르지 않습니다.".into())
}

fn approval_entry() -> Result<Option<Vec<u8>>, String> {
    let Some(key) = open(APPROVED, KEY_QUERY_VALUE, false)? else {
        return Ok(None);
    };
    let Some((kind, bytes)) = read(&key)? else {
        return Ok(None);
    };
    if kind != REG_BINARY {
        return Err("Windows 시작 앱 상태를 확인하지 못했습니다.".into());
    }
    approval_flag(&bytes)?;
    Ok(Some(bytes))
}

fn approval_flag(bytes: &[u8]) -> Result<bool, String> {
    if bytes.len() != 12 {
        return Err("Windows 시작 앱 상태를 확인하지 못했습니다.".into());
    }
    match bytes[0] {
        2 | 6 => Ok(true),
        3 | 7 => Ok(false),
        _ => Err("Windows 시작 앱 상태를 확인하지 못했습니다.".into()),
    }
}

fn approval_state() -> Result<bool, String> {
    match approval_entry()? {
        None => Ok(true),
        Some(bytes) => approval_flag(&bytes),
    }
}

fn expected() -> Result<String, String> {
    executable_command(&std::env::current_exe().map_err(|e| e.to_string())?)
}

fn checked_registration(expected: &str) -> Result<bool, String> {
    match registered_command()? {
        None => Ok(false),
        Some(command) if command.eq_ignore_ascii_case(expected) => Ok(true),
        Some(_) => Err("다른 실행 파일이 이 자동 시작 항목을 사용하고 있습니다. Windows 시작 앱 설정을 확인해 주세요.".into()),
    }
}

pub(super) fn status() -> Result<bool, String> {
    if !checked_registration(&expected()?)? {
        return Ok(false);
    }
    approval_state()
}

pub(crate) fn snapshot() -> Result<Snapshot, String> {
    // Do not take ownership of another executable's entry.
    checked_registration(&expected()?)?;
    approval_entry()?;
    Ok(Snapshot {
        run: read_path(RUN)?,
        approved: read_path(APPROVED)?,
    })
}

fn restore_value(path: PCWSTR, value: &Option<(REG_VALUE_TYPE, Vec<u8>)>) -> Result<(), String> {
    let Some(key) = open(path, KEY_QUERY_VALUE | KEY_SET_VALUE, value.is_some())? else {
        return Ok(());
    };
    match value {
        Some((kind, bytes)) => write(&key, *kind, bytes),
        None => delete(&key),
    }
}

pub(crate) fn restore(snapshot: &Snapshot) -> Result<(), String> {
    // A third party may have taken the name since snapshot; never overwrite it.
    checked_registration(&expected()?)?;
    let run_result = restore_value(RUN, &snapshot.run);
    let approval_result = restore_value(APPROVED, &snapshot.approved);
    run_result?;
    approval_result?;
    if read_path(RUN)? != snapshot.run || read_path(APPROVED)? != snapshot.approved {
        return Err("이전 자동 시작 상태가 완전히 복구되지 않았습니다.".into());
    }
    Ok(())
}

pub(super) fn set(enabled: bool) -> Result<bool, String> {
    let command = expected()?;
    let before = snapshot()?;
    let existed = before.run.is_some();
    if !enabled && !existed {
        return Ok(false);
    }
    // Read and validate the OS override before changing anything.
    let approval_disabled = enabled
        && before
            .approved
            .as_ref()
            .is_some_and(|(_, entry)| matches!(entry[0], 3 | 7));
    let run = open(RUN, KEY_QUERY_VALUE | KEY_SET_VALUE, enabled)?
        .ok_or("자동 시작 설정을 열지 못했습니다.")?;
    if enabled {
        write(&run, REG_SZ, &command_bytes(&command))?;
    } else {
        delete(&run)?;
    }
    let operation = (|| -> Result<bool, String> {
        if approval_disabled {
            // This per-user encoding matches the previous auto-launch library.
            // It is changed only for this entry after the user turns it on.
            let key = open(APPROVED, KEY_QUERY_VALUE | KEY_SET_VALUE, false)?
                .ok_or("Windows 시작 앱 상태를 열지 못했습니다.")?;
            write(&key, REG_BINARY, &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
        }
        let actual = status()?;
        if actual != enabled {
            return Err("Windows가 자동 시작 변경을 반영하지 않았습니다.".into());
        }
        Ok(actual)
    })();
    if let Err(error) = operation {
        if restore(&before).is_err() {
            return Err(format!("{error} 이전 자동 시작 상태를 복구하지 못했습니다. Windows 시작 앱 설정을 확인해 주세요."));
        }
        return Err(error);
    }
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_command_quotes_path_and_uses_explicit_login_flag() {
        let path = Path::new(r"C:\Users\이름\Rhythm Folder\rhythm.exe");
        let command = executable_command(path).unwrap();
        assert_eq!(
            command,
            r#""C:\Users\이름\Rhythm Folder\rhythm.exe" --autostart"#
        );
        assert_eq!(registry_string(&command_bytes(&command)), Some(command));
        assert!(registry_string(&[0, 1, 0]).is_none());
        assert!(executable_command(Path::new("bad\"path")).is_err());
    }

    #[test]
    fn startup_approval_distinguishes_enabled_disabled_and_unknown() {
        let mut bytes = [0u8; 12];
        for (value, expected) in [(2, true), (6, true), (3, false), (7, false)] {
            bytes[0] = value;
            assert_eq!(approval_flag(&bytes).unwrap(), expected);
        }
        bytes[0] = 9;
        assert!(approval_flag(&bytes).is_err());
        assert!(approval_flag(&bytes[..3]).is_err());
    }
}
